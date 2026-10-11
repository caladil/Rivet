//! The boot partition's contents. `ADR-002` §4 is the authority.
//!
//! The Raspberry Pi boot chain removes bootloader work from the project
//! entirely (`ADR-002` §4): there is no U-Boot, no GRUB and no bootloader
//! source tree, and the whole of Rivet's boot configuration is a FAT32
//! partition holding somebody else's firmware, a kernel, a device tree and
//! **five text files this module generates**.
//!
//! # What this module is, and is not
//!
//! It produces the complete contents of one boot partition as data — a list of
//! `(name, bytes)` — and performs **no I/O at all**. The firmware, the kernel
//! and the device tree are *inputs*: they are somebody else's bytes, and a
//! function that returns bytes is testable where one that copies files is not.
//! Writing the result is [`crate::disk`]'s job.
//!
//! # The three traps, all of them already paid for
//!
//! 1. **`tryboot.txt` is not optional.** `ADR-018` §5.3, verified on hardware
//!    2026-08-22: in tryboot mode the firmware reads `tryboot.txt` **instead
//!    of** `config.txt`. `config.txt` carries `arm_64bit=1` and there is no
//!    32-bit kernel on the partition, so a tryboot reboot with no `tryboot.txt`
//!    attempts a 32-bit boot and loses a headless board. This module emits the
//!    two as **byte-identical copies** — see [`tryboot_txt`].
//! 2. **`cmdline.txt` must not carry `cgroup_disable=memory`.** Raspberry Pi OS
//!    ships it, and `ADR-024` §8.1 found that it silently removes the memory
//!    controller `ADR-015` §5.1 and `ADR-017` §3.1.3 both depend on: the card
//!    reads `cgroup2fs` correctly while `cgroup.controllers` reads
//!    `cpuset cpu io pids`, so a memory limit is accepted and ignored. Rivet
//!    writes its own `cmdline.txt`, so this is a deletion Rivet simply never
//!    makes — and the test `the_command_line_never_disables_the_memory_cgroup`
//!    is what keeps it never made.
//! 3. **`root=` differs per slot**, which is exactly why `ADR-002` §4's
//!    `cmdline.txt` example is marked stale. It is derived from
//!    [`Slot::system_partition`](crate::layout::Slot::system_partition) via the
//!    frozen table in [`crate::layout`], never written out by hand. See
//!    [`root_spec`] for why it is a partition **label**.
//!
//! # Determinism
//!
//! `ADR-007` §3.1 wants two machines in the same state to produce
//! byte-identical documents. It matters more here than it does for
//! configuration: a diff between two cards should show only what actually
//! differs. Nothing in this module reads a clock, a random number or an
//! environment variable, and the order of the emitted files does not depend on
//! the order of the inputs — [`contents`] sorts the firmware by name.
//!
//! # Line endings: LF, and why
//!
//! Every generated file ends its lines with a single `\n`, and every generated
//! file is pure ASCII.
//!
//! * The firmware accepts both, and stock Raspberry Pi OS images are shipped
//!   with LF.
//! * `cmdline.txt` is the one file whose bytes *become* something else — the
//!   kernel command line — and a stray `\r` is the one byte that could be
//!   carried into it. There is nothing to gain by putting one there.
//! * `ADR-018` §5.3 records that PID 1 preserves line endings byte-for-byte
//!   when it rewrites `autoboot.txt`, so what Rivet emits is what survives,
//!   and an operator who edits the file from Windows and leaves CRLF behind
//!   keeps CRLF. Emitting LF therefore constrains nobody.
//! * Modern Windows editors — Notepad included since 2018 — read LF.
//!
//! # What is unproven here
//!
//! **Nothing in this module has booted anything.** Every file below is
//! generated from the design record and from Raspberry Pi's published
//! `config.txt` documentation, and `ADR-018` §5.3 is explicit that the
//! `autoboot.txt` syntax is documentation-derived: that a `[tryboot]` section
//! selects a different `boot_partition` was never tested, because the board
//! used for the hardware pass has one boot partition to select. `AGENTS.md`
//! §10.5. The cross-check that *is* real is against `rivet-adapter-image`:
//! ~~PID 1's parser is vendored into the tests from its own source file, with a
//! guard that fails the moment the two drift.~~ **Since 2026-08-23 the tests
//! call that parser directly** — it moved into a library crate and the copy it
//! needed a guard for is gone.

// Temporary, exactly as in `crate::layout`: `crate::disk` does not consume this
// module yet, so every item here is unreachable from `main`. Delete this the
// moment it does — a standing allow is how a genuinely unused function
// survives for a year.
#![allow(dead_code)]

use crate::layout::{BOOT_SIZE_MIB, PartitionSpec, Slot, partition_table};

// -- the names on the partition ------------------------------------------------

/// The 64-bit kernel. `ADR-002` §4.
///
/// Also the name the firmware looks for by default once `arm_64bit=1` is set,
/// but [`config_txt`] names it anyway: `ADR-011` — the file that decides is the
/// one that should say so.
pub const KERNEL_FILE: &str = "kernel8.img";

/// The device tree for the Pi 4 Model B, which `ADR-002` §3 makes the primary
/// target.
pub const DEVICE_TREE_FILE: &str = "bcm2711-rpi-4-b.dtb";

/// Read by the firmware on a normal boot.
pub const CONFIG_FILE: &str = "config.txt";

/// Read by the firmware **instead of** [`CONFIG_FILE`] in tryboot mode.
/// `ADR-018` §5.3.
pub const TRYBOOT_FILE: &str = "tryboot.txt";

/// The A/B selector. `ADR-018` §5.1, and rewritten by PID 1 to commit a slot —
/// `rivet-adapter-image` (it was `cmd/rivet-initd/src/confirm.rs`, then
/// that crate's `slots.rs`, both on 2026-08-23).
pub const AUTOBOOT_FILE: &str = "autoboot.txt";

/// One line, and the entire interface between the firmware and Rivet.
pub const CMDLINE_FILE: &str = "cmdline.txt";

/// First-boot provisioning. `ADR-004` §6, read by
/// `cmd/rivet-initd/src/provision.rs`.
pub const PROVISIONING_FILE: &str = "rivet.toml";

/// The firmware files that must be on the card for the board to boot at all.
///
/// On the Pi 4 the *first* stage lives in an SPI EEPROM rather than on the card
/// (`ADR-002` §4), which is why there is no `bootcode.bin` here — but the
/// EEPROM's bootloader still loads `start4.elf` from the FAT partition, and
/// `fixup4.dat` with it. A partition missing either does not boot, so this
/// module refuses to produce one rather than producing a card that fails in the
/// field. `AGENTS.md` §3.
///
/// Sorted, because [`contents`] emits firmware in sorted order and a constant
/// that is already in that order is one fewer thing to keep in step.
pub const REQUIRED_FIRMWARE: &[&str] = &["fixup4.dat", "start4.elf"];

/// Where PID 1 lives, and therefore what `init=` names.
///
/// `ADR-002` §4 flagged `init=/sbin/rivet-initd` as one of three stale values
/// in its own example: `ADR-008` §3 and `ADR-016` §3 put binaries under
/// `/system/bin`. This is the same reasoning that produced
/// `supervise::SUPERVISOR_PATH` in `cmd/rivet-initd`, and
/// the test `the_init_path_is_a_sibling_of_the_supervisor_rivet_initd_spawns`
/// reads that constant out of its own source file to check the two still agree.
pub const INIT_PATH: &str = "/system/bin/rivet-initd";

/// What every generated file ends its lines with. See the module docs.
pub const EOL: &str = "\n";

// -- the inputs and the output -------------------------------------------------

/// One file destined for a boot partition.
///
/// Also the shape of the firmware handed *in*: firmware is copied through
/// untouched, so there is no second type for it to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

impl BootFile {
    pub fn new(name: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            name: name.into(),
            bytes: bytes.into(),
        }
    }
}

/// The bytes this module does not generate: somebody else's firmware, kernel
/// and device tree.
///
/// They are handed in rather than read, which is what keeps [`contents`] a pure
/// function. `tools/qemu/fetch-kernel.sh` records where the kernel and the
/// device tree in this repository came from — a running Raspberry Pi, so that
/// the loop runs the exact kernel the hardware runs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BootInputs {
    pub kernel: Vec<u8>,
    pub device_tree: Vec<u8>,
    /// `start4.elf`, `fixup4.dat`, and anything else the firmware wants.
    pub firmware: Vec<BootFile>,
    /// The device's TLS identity, if the operator supplied one — the three
    /// files `cmd/rivet-initd`'s `identity` module copies to `/state/secrets`
    /// and then blanks.
    ///
    /// **Three files rather than keys in the document**, and that module's
    /// header carries the argument: `ADR-020` §4's `Secret<T>` is not built, so
    /// a seed in the configuration schema would render in `export`, in
    /// `--explain` and in any error that quoted a value. Nothing in
    /// `rivet-config` ever sees these, so nothing in `rivet-config` can print
    /// them.
    ///
    /// **All three or none.** A seed with no certificate is a card whose
    /// `rivet-mcpd` cannot start, and the failure would appear on the board
    /// rather than here — see [`Identity`], whose three fields travel together
    /// by construction rather than as three independent options.
    pub identity: Option<Identity>,
    /// The staging bundle, if the operator supplied one — `ADR-049`.
    ///
    /// **The same shape as [`identity`](Self::identity) and for the same
    /// reason**: material that cannot be in the payload because it is
    /// per-machine, written to a partition PID 1 reads before any service runs,
    /// and destroyed on the card once it is safely on `/state`. It differs in
    /// exactly one thing, which is the whole of `ADR-049`: an identity belongs
    /// to the MACHINE and lands in a root-owned directory, and this belongs to
    /// an APPLICATION whose uid nothing at card time can know.
    ///
    /// **Not validated here.** `cmd/rivet-image` does not depend on
    /// `rivet-config` and this crate copies bytes; `rivet_core::staging::parse`
    /// is the authority and `cmd/rivet-provision` is what runs it before the
    /// card is built. The same named seam
    /// [`document`](Self::document) carries.
    pub staged: Option<String>,
    /// The signed application payloads this card carries — `ADR-050` §4.
    ///
    /// **Already named and already verified.** Each [`BootFile`]'s name is
    /// `rivet_core::carried::card_file` of the product out of the payload's own
    /// verified header, and `crate::application::verify` is what put it there:
    /// this module copies bytes and names nothing, exactly as it does for the
    /// firmware.
    ///
    /// **Unlike [`staged`](Self::staged), these are not secret and are not
    /// destroyed on the card.** `rivet_core::carried` carries the argument — a
    /// `.rvt` is protected by a signature rather than by secrecy, and leaving
    /// it means the card still says what it was built to carry when somebody
    /// puts it in a reader. What PID 1 does instead of destroying it is skip a
    /// payload whose record directory is already on `/state`.
    pub applications: Vec<BootFile>,
    /// The first-boot provisioning document, if the operator supplied one.
    ///
    /// `None` writes [`provisioning_toml`]'s template, which is what every card
    /// carried before 2026-08-24 and which declares a schema version and nothing
    /// else. A card built that way **boots to a machine nobody can reach**: with
    /// no `[network]` table it takes no address at all, so nothing can open a
    /// connection to it.
    ///
    /// That was survivable while the boot partition was the only thing anyone
    /// wrote — it is FAT and mounts on any workstation — but editing a FAT
    /// partition by hand is a step performed at the wrong end of the day, and
    /// this project already treats *a machine you can log into* as the
    /// exit criterion rather than the convenience.
    ///
    /// **Not validated here, and that is a named seam rather than an oversight.**
    /// `cmd/rivet-image` does not depend on `rivet-config`, so nothing in
    /// this crate can say whether the bytes parse — the same seam
    /// [`provisioning_toml`] documents about its own template. The refusal an
    /// operator gets for a bad document therefore comes from the *board*, on the
    /// first boot, which is exactly the wrong place; closing it needs a
    /// `Cargo.toml` change this module does not own.
    pub document: Option<String>,
}

/// Which partition is being built, and which slot the card boots by default.
///
/// Two `Slot`s that mean different things, so they are named fields rather than
/// two positional arguments — transposing them produces a card that boots the
/// wrong system and still looks right.
///
/// * `slot` decides `root=` in [`cmdline_txt`], because each boot partition
///   names its own `/system`.
/// * `default_slot` decides `autoboot.txt`, which is a property of the **card**
///   rather than of the partition: [`autoboot_txt`] emits the same bytes
///   whichever partition they are written to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootPartitionSpec {
    pub slot: Slot,
    pub default_slot: Slot,
}

impl BootPartitionSpec {
    /// A freshly written card: this slot's partition, and this slot is the
    /// default.
    pub const fn fresh(slot: Slot) -> Self {
        Self {
            slot,
            default_slot: slot,
        }
    }
}

/// Why a boot partition could not be produced.
///
/// Every one of these is a card that would not boot, or would boot something
/// other than what was asked for. `AGENTS.md` §3: refuse rather than guess, and
/// refuse rather than write a partially-correct card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootError {
    /// No kernel bytes were supplied.
    EmptyKernel,
    /// No device-tree bytes were supplied.
    EmptyDeviceTree,
    /// A file the board cannot boot without is not among the firmware.
    MissingFirmware { name: &'static str },
    /// Two files claim the same name on a FAT filesystem.
    DuplicateName { name: String },
    /// A name FAT cannot hold, or that would not survive being written.
    UnusableName { name: String, why: &'static str },
    /// The contents do not fit the partition `crate::layout` reserves.
    TooLarge { bytes: u64, capacity: u64 },
    /// `crate::layout`'s table does not describe the partition this slot needs.
    /// Unreachable unless the frozen contract changes underneath this module,
    /// which is exactly why it is an error and not an `unwrap`.
    LayoutMismatch { partition: u32 },
}

impl core::fmt::Display for BootError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::EmptyKernel => write!(f, "no kernel was supplied; {KERNEL_FILE} would be empty"),
            Self::EmptyDeviceTree => {
                write!(
                    f,
                    "no device tree was supplied; {DEVICE_TREE_FILE} would be empty"
                )
            }
            Self::MissingFirmware { name } => write!(
                f,
                "the firmware file {name} is missing; the board would not boot (ADR-002 section 4)"
            ),
            Self::DuplicateName { name } => {
                write!(
                    f,
                    "two files are called {name}; one would overwrite the other"
                )
            }
            Self::UnusableName { name, why } => {
                write!(
                    f,
                    "{name:?} cannot be a file name on the boot partition: {why}"
                )
            }
            Self::TooLarge { bytes, capacity } => write!(
                f,
                "the boot partition holds {capacity} bytes and these contents are {bytes}"
            ),
            Self::LayoutMismatch { partition } => write!(
                f,
                "crate::layout describes no partition {partition}; the shared contract changed"
            ),
        }
    }
}

impl std::error::Error for BootError {}

// -- the generated files -------------------------------------------------------

/// `config.txt`, read by the firmware on a normal boot.
///
/// # What is here, and why each line earns its place
///
/// `ADR-001`'s minimalism is about **surface**, so the question asked of every
/// line was not "does this hurt" but "what does Rivet depend on". Four
/// settings are dependencies and three are refusals:
///
/// | Setting | Why |
/// | :-- | :-- |
/// | `arm_64bit=1` | `ADR-002` §3: the target is 64-bit. `ADR-018` §5.3 records what its absence costs |
/// | `kernel=` | the partition holds exactly one kernel; naming it beats a firmware default |
/// | `device_tree=` | likewise, and the board is fixed by `ADR-002` §3 |
/// | `enable_uart=1` | `ADR-002` §4 names it, and `ADR-004` §7 makes serial the path for panics |
/// | `camera_auto_detect=0` | stock sets `1`. Rivet has no camera; probing for one is surface |
/// | `display_auto_detect=0` | stock sets `1`. Likewise |
/// | `auto_initramfs=0` | stock sets `1`. Rivet ships no initramfs — see the note below |
///
/// # What is deliberately absent
///
/// * **`dtparam=audio=on`.** Stock Raspberry Pi OS sets it; the device-tree
///   default is off, so Rivet omits the line rather than writing `off` and
///   restating a default.
/// * **`dtoverlay=vc4-kms-v3d`.** Stock enables the KMS driver. Rivet has no
///   GUI — `PLAN.md` §4.13 ruled that a system extension — and the HDMI console
///   `ADR-002` §2.2 wants comes from the firmware's framebuffer. **Unproven:**
///   that the console appears without the overlay on this board. If it does
///   not, the overlay is the known remedy and this is where it goes.
/// * **Conditional sections.** `[pi4]`, `[cm4]` and `[all]` filter by board;
///   `ADR-002` §3 fixes the board, so a flat file applies unconditionally and
///   there is no section for a setting to hide in.
///
/// # The `auto_initramfs=0` note
///
/// `ADR-024` §8.1 found `EROFS_FS=m` in the stock kernel, and
/// [`cmdline_txt`] asks that kernel to mount an `erofs` root. A module cannot
/// be loaded before the filesystem holding it is mounted, so the stock kernel
/// needs either an initramfs or `EROFS_FS=y`. `ADR-024` §8's custom build is
/// the answer the record already chose; this line is `0` because there is no
/// initramfs to find today, and it is the line that changes if that answer
/// changes.
pub fn config_txt() -> String {
    let mut s = String::new();
    s.push_str(&banner());
    s.push_str(&format!(
        "#\n\
         # {TRYBOOT_FILE} is a byte-identical copy of this file. ADR-018 section\n\
         # 5.3: in tryboot mode the firmware reads that file INSTEAD of this\n\
         # one, so a setting present in only one of them is a slot that boots\n\
         # differently from the slot it replaces.\n\
         \n\
         # ADR-002 section 3: the target is 64-bit. Its absence is what would\n\
         # lose a headless board on a tryboot reboot.\n\
         arm_64bit=1\n\
         kernel={KERNEL_FILE}\n\
         device_tree={DEVICE_TREE_FILE}\n\
         \n\
         # ADR-002 section 4 and ADR-004 section 7: serial is where a panic goes.\n\
         enable_uart=1\n\
         \n\
         # Stock Raspberry Pi OS enables all three. Rivet has no camera, no\n\
         # display to detect and no initramfs, and ADR-001's minimalism is about\n\
         # surface: firmware that probes for hardware Rivet does not use is\n\
         # surface.\n\
         camera_auto_detect=0\n\
         display_auto_detect=0\n\
         auto_initramfs=0\n"
    ));
    s
}

/// `tryboot.txt` — **byte-identical to [`config_txt`], by construction.**
///
/// `ADR-002` §4 and `ADR-018` §5.3 both require the file to exist and to be
/// bootable. Making it a copy rather than a second document is `ADR-011`
/// applied: two files that must agree and are written separately are two
/// authorities, and the one that drifts is the one nobody boots until the day
/// an update needs to roll back.
///
/// There is a second reason, and it is the reason the copy is *identical*
/// rather than merely equivalent. `ADR-018` §5.3 records that `tryboot_a_b=1`
/// — which [`autoboot_txt`] sets — makes a tryboot boot read the normal
/// `config.txt` rather than `tryboot.txt`, while the hardware pass observed
/// `tryboot.txt` being read on a card with no `autoboot.txt`. **Which file the
/// firmware reads therefore depends on state this module cannot see**, and
/// identical bytes make the question stop mattering.
pub fn tryboot_txt() -> String {
    config_txt()
}

/// `cmdline.txt` — one line, and the entire interface between the firmware and
/// Rivet.
///
/// `ADR-002` §4's example is stale in all three of the ways it says it is, and
/// this function is the correction:
///
/// * `root=` names **this slot's** `/system`, derived from [`root_spec`].
/// * `rootfstype=` comes from the frozen table, which makes it `erofs`
///   (`ADR-018` §3) rather than `ext4`.
/// * `init=` is [`INIT_PATH`], under `/system/bin` per `ADR-008` §3.
///
/// # What is absent, deliberately
///
/// * **`cgroup_disable=memory`.** `ADR-024` §8.1. Rivet never writes it —
///   **and that was never enough, which is why `cgroup_enable=memory` moved
///   from the absent list to the line itself on 2026-08-23.**
///
/// # What is present because absence was not enough
///
/// * **`cgroup_enable=memory`.** This entry previously sat in the list above,
///   reasoning that the controller "is available by default when `CONFIG_MEMCG`
///   is compiled in". `CONFIG_MEMCG` *is* compiled in — and the controller was
///   still missing on the board, on a card whose `cmdline.txt` this function
///   wrote and which contained no `cgroup_disable` at all.
///
///   **The Raspberry Pi firmware composes its own kernel command line and
///   appends `cmdline.txt` to it**, and the firmware's half carries
///   `cgroup_disable=memory`. Read from a Pi 4 on 2026-08-23, `/proc/cmdline`
///   holds it among `coherent_pool`, `8250.nr_uarts`, `snd_bcm2835.*` and
///   `vc_mem.*` — firmware-composed parameters, *before* the `console=` seam
///   where `cmdline.txt` begins. Owning `cmdline.txt` therefore cannot delete
///   it; there is nothing there to delete. It can only be **countered**, and
///   `cgroup_enable=` is what counters `cgroup_disable=` — later wins, and
///   `cmdline.txt` is appended last, which is the one piece of ordering luck in
///   this.
///
///   This is also why the emulator disagreed with the board on the same kernel
///   binary: QEMU has no Raspberry Pi firmware, so nothing injected the
///   parameter and the controller was simply there.
///
///   `ADR-024` §7 still owns the *verification* — this flag is a request, and
///   PID 1 reading `cgroup.controllers` remains the authority for whether it
///   was granted. A flag that asserted the outcome would be the second answer
///   the old comment rightly warned about; a flag that asks for it is not.
/// * **`ro`.** `ADR-018` §3's whole point is that `/system` is read-only *by
///   format* rather than by flag. A mount flag restating a format property is
///   the weaker of the two claims sitting next to the stronger one. (The kernel
///   mounts root read-only by default in any case.)
/// * **`panic=`.** A headless board that panics should reboot, and `ADR-018`
///   §5.2 step 5 makes an unattended reboot the thing that completes a
///   rollback. It is left out **for Stage 0 only**, because a board that
///   reboots ten seconds after a panic erases the panic message from the HDMI
///   console that `ADR-002` §2.2 keeps the board for. This is a real trade-off
///   and belongs to the maintainer, not to this function.
///
/// # `console=serial0`
///
/// Stock-derived and unverified by this code. `serial0` is a device-tree alias
/// resolved by the kernel, which is what keeps the line correct whether the
/// port is the PL011 or the mini-UART — the same reasoning that makes
/// [`root_spec`] a label. `console=tty1` comes last, so `/dev/console` is the
/// HDMI console `ADR-002` §2.2 wants PID 1 to speak on.
pub fn cmdline_txt(slot: Slot) -> Result<String, BootError> {
    let system = system_partition(slot)?;
    Ok(format!(
        "console=serial0,115200 console=tty1 root=PARTLABEL={label} rootfstype={fstype} \
         rootwait cgroup_enable=memory init={INIT_PATH}{EOL}",
        label = system.label,
        fstype = system.fstype.as_str(),
    ))
}

/// How `cmdline.txt` names the root filesystem: `PARTLABEL=system-a`.
///
/// Three candidates, and the choice has to survive `ADR-018` §5's slot swap:
///
/// | Form | Verdict |
/// | :-- | :-- |
/// | `root=/dev/mmcblk0p3` | **No.** It names the *medium*. `ADR-002` §2.2 keeps USB boot as a reason for choosing this board, and the same card in a USB enclosure is `/dev/sda3` |
/// | `root=PARTUUID=…` | Workable, and what Raspberry Pi OS uses — but the GUID is written by [`crate::disk`], so this module could only be handed one, and a generated GUID would make the output non-deterministic (`ADR-007` §3.1) |
/// | `root=PARTLABEL=system-a` | **Chosen.** The label is in the frozen contract already, it names the *slot* rather than the device, and it is legible in a diff between two cards |
///
/// **What a swap does and does not move.** `ADR-018` §5 swaps which slot is
/// *default*; it never renumbers or relabels a partition. Each boot partition
/// carries its own `cmdline.txt` naming its own `/system`, so after a swap the
/// firmware boots the other boot partition and reads the other `cmdline.txt`,
/// which already names the right label. A device path would survive the swap
/// too — and would still be wrong the first time the card is booted from USB.
///
/// **Verified, and by what method:** the exact kernel this repository boots
/// (`tools/qemu/kernel8.img`, fetched from the board by
/// `tools/qemu/fetch-kernel.sh`) contains the literal `PARTLABEL=` and the GPT
/// parser's own diagnostics, read with `strings` over the decompressed image.
/// That says the kernel's early root lookup **has** the code path and that it
/// parses GPT labels; it does not say that this board boots a GPT-labelled card
/// — `ADR-018` §9 item 1 records that as unproven and belongs to image
/// assembly.
pub fn root_spec(slot: Slot) -> Result<String, BootError> {
    Ok(format!("PARTLABEL={}", system_partition(slot)?.label))
}

/// `autoboot.txt` for a freshly written card. `ADR-018` §5.1.
///
/// ```text
/// [all]
/// tryboot_a_b=1
/// boot_partition=1
///
/// [tryboot]
/// boot_partition=2
/// ```
///
/// `boot_partition=` under `[all]` is the default slot; the `[tryboot]`
/// override is the slot a `reboot "0 tryboot"` tries once. `tryboot_a_b=1` is
/// what makes the pair an A/B pair rather than two unrelated partitions.
///
/// # It is a property of the card, not of the partition
///
/// The same bytes belong on both boot partitions. Nothing here depends on which
/// partition the file is written to — only on which slot the card boots by
/// default — which is what lets [`contents`] be called twice for a mirrored
/// fresh card and produce one `autoboot.txt`.
///
/// # What PID 1 does to this file, and what it does not
///
/// `rivet_adapter_image::PiFirmware::commit` moves the `[all]` value to the
/// running partition to commit a slot, refusing any file whose shape it does
/// not recognise. This file is generated to be recognised, and the test
/// `rivet_initd_parser_accepts_the_autoboot_file_this_module_writes`
/// runs that crate's own parser over these bytes.
///
/// **It deliberately leaves the `[tryboot]` override alone**, which is worth
/// stating here because it is visible from this file and not from that one:
/// after the first commit the override names the slot that is now current, so
/// the *next* update's trial boot would try the running slot rather than the
/// freshly written one. Whatever writes the inactive slot has to write this
/// file too. ~~That mechanism does not exist yet, and it is not this module's
/// to invent.~~
///
/// **It exists since 2026-08-23, and it is not the slot writer.**
/// `rivet_adapter_image`'s `arm_trial_boot` moves the override, on
/// the argument that arming is the moment the target slot is known and
/// committing is not. ~~**Nothing calls it yet**, so what
/// this module writes is still the whole of the override's life on a card;~~
/// **`cmd/rivet` calls it since later the same day** (`ADR-018` §5.3.1), so
/// what this module writes is the override's *starting* value and an operator
/// can move it. `ADR-018` §5.3 item 2 — that a `[tryboot]` section selects a
/// *different* `boot_partition` at all — remains untested on any board.
///
/// **`tryboot_a_b=1` below is load-bearing in a new way.** It was written here
/// because `ADR-018` §5.3 says a tryboot boot without it reads `tryboot.txt`
/// instead of `config.txt`; it is now also the thing an arming *checks for*, and
/// `rivet_adapter_image::require_tryboot_a_b` refuses a card that lacks it.
/// Removing this line from a fresh card would make every arming refuse — which
/// is the safe direction, and is the point.
pub fn autoboot_txt(default_slot: Slot) -> String {
    let mut s = String::new();
    s.push_str(&banner());
    s.push_str(&format!(
        "#\n\
         # ADR-018 section 5.1. boot_partition under [all] is the default slot;\n\
         # the [tryboot] override is the slot a one-shot tryboot reboot tries.\n\
         # PID 1 moves the [all] value to commit a slot, and refuses this file\n\
         # if it is not the shape it expects.\n\
         [all]\n\
         tryboot_a_b=1\n\
         boot_partition={default}\n\
         \n\
         [tryboot]\n\
         boot_partition={other}\n",
        default = default_slot.boot_partition(),
        other = default_slot.other().boot_partition(),
    ));
    s
}

/// The device's Ed25519 seed, as `cargo rivet identity new` writes it.
///
/// **The only secret this project has ever deliberately put on a FAT
/// partition**, and it is there for as long as it takes a board to boot once:
/// `ADR-019` §3.1 has PID 1 copy it to `/state/secrets` and delete this copy.
/// `cmd/rivet-initd`'s `identity` module is that step, and its ordering — copy,
/// sync, only then delete — is the whole of the safety.
pub const IDENTITY_SEED_FILE: &str = "device.seed";

/// The self-signed certificate carrying that seed's public half. Not a secret;
/// it is the thing a peer pins (`ADR-035` §7.2).
pub const IDENTITY_CERTIFICATE_FILE: &str = "device.crt";

/// The Ed25519 public keys allowed to reach the control plane, one hex key per
/// line. `ADR-025` §2.3: public by definition, and what matters is integrity.
pub const IDENTITY_PEERS_FILE: &str = "peers";

/// The staging bundle — `ADR-049`, and the SECOND thing on this partition that
/// carries a private key.
///
/// `rivet_core::staging` owns the name and the format; this is a re-export
/// rather than a second spelling, because a card written with one name and read
/// with another is a machine that silently provisions nothing (`ADR-011`).
pub const STAGED_FILE: &str = rivet_core::staging::BUNDLE_FILE;

/// What an operator hands a card build so the machine arrives enrolled.
///
/// `ADR-019` §2: *"a machine arrives enrolled … credentials never traverse a
/// network to enroll — they are written to a card by the operator's own
/// tooling"*, and `ADR-025` §3 puts generation in the fleet tool. This is that
/// hand-off, and this crate neither mints nor validates it — it copies bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// Thirty-two raw bytes.
    pub seed: Vec<u8>,
    /// DER.
    pub certificate: Vec<u8>,
    /// Text, one hex key per line.
    pub peers: Vec<u8>,
}

/// `rivet.toml`, the first-boot provisioning document. `ADR-004` §6.
///
/// A template, not a configuration: it declares the schema version and nothing
/// else, because `ADR-007` §4.3 is explicit that **no Stage 0 field is
/// specified anywhere in the record**, and `rivet-config` refuses an unknown
/// key rather than ignoring it (§3.6.5). Wireless credentials and a fleet
/// endpoint arrive with `ADR-019` §3; the operator's editor is where they will
/// be typed, and this file is the thing that is there to be edited.
///
/// Absence is a valid state — `provision.rs` says so in as many words, and
/// `ADR-004` §6 step 3 has PID 1 remove the credential after the first boot —
/// so an operator may delete this file and nothing breaks.
///
/// **A known seam.** `rivet-image` does not depend on `rivet-config`, so
/// this text is a second hand-written spelling of a schema that crate owns, and
/// nothing here can prove `rivet-config` accepts it. A `dev-dependency` on
/// `rivet-config` plus a parse test would close that, and it needs a
/// `Cargo.toml` change this module does not own.
pub fn provisioning_toml() -> String {
    let mut s = String::new();
    s.push_str(&banner());
    s.push_str(
        "#\n\
         # First-boot provisioning. ADR-004 section 6: PID 1 mounts this\n\
         # partition read-only, reads this file, and applies it. Deleting the\n\
         # file is safe -- an absent document is a valid state and is the steady\n\
         # state of every boot after the first.\n\
         #\n\
         # Unknown keys are refused rather than ignored (ADR-007 section 3.6.5),\n\
         # so a misspelling is reported instead of silently doing nothing.\n\
         \n\
         # The schema version this document is written against. Required.\n\
         version = 1\n",
    );
    s
}

/// The header every generated file carries. One place, so every file says the
/// same thing about who wrote it.
fn banner() -> String {
    format!(
        "# Generated by rivet-image. Edits are lost the next time this\n# partition is written. See {CONFIG_FILE}'s module in cmd/rivet-image.\n"
    )
}

// -- the whole partition -------------------------------------------------------

/// Every file that belongs on one boot partition, in a deterministic order.
///
/// The order is: firmware sorted by name, then the kernel, the device tree, and
/// the five generated text files in the order `ADR-002` §4 lists them. Sorting
/// the firmware is what makes the output independent of the order the caller
/// happened to collect it in — see the module docs on determinism.
///
/// # What it refuses
///
/// A missing kernel or device tree, missing `start4.elf` or `fixup4.dat`, two
/// files with the same name, a name FAT cannot hold, and contents larger than
/// the partition `crate::layout` reserves. Every one of those is a card that
/// does not boot, and `AGENTS.md` §3 has this refuse rather than write one.
pub fn contents(spec: &BootPartitionSpec, inputs: &BootInputs) -> Result<Vec<BootFile>, BootError> {
    if inputs.kernel.is_empty() {
        return Err(BootError::EmptyKernel);
    }
    if inputs.device_tree.is_empty() {
        return Err(BootError::EmptyDeviceTree);
    }
    for required in REQUIRED_FIRMWARE {
        if !inputs.firmware.iter().any(|f| f.name == *required) {
            return Err(BootError::MissingFirmware { name: required });
        }
    }

    let mut firmware = inputs.firmware.clone();
    firmware.sort_by(|a, b| a.name.cmp(&b.name));

    let mut files = firmware;
    files.push(BootFile::new(KERNEL_FILE, inputs.kernel.clone()));
    files.push(BootFile::new(DEVICE_TREE_FILE, inputs.device_tree.clone()));
    files.push(BootFile::new(CONFIG_FILE, config_txt()));
    files.push(BootFile::new(TRYBOOT_FILE, tryboot_txt()));
    files.push(BootFile::new(
        AUTOBOOT_FILE,
        autoboot_txt(spec.default_slot),
    ));
    files.push(BootFile::new(CMDLINE_FILE, cmdline_txt(spec.slot)?));
    files.push(BootFile::new(
        PROVISIONING_FILE,
        // The operator's document if there is one, the template otherwise.
        // Never both, and never a merge: a document assembled from two sources
        // is one no reader can predict, and `ADR-007` §3.6.5 refuses an unknown
        // key rather than ignoring it — so a template's comments appended to a
        // real document would be harmless and a template's *keys* would not.
        inputs
            .document
            .clone()
            .unwrap_or_else(provisioning_toml),
    ));

    // **A machine arrives enrolled, or it arrives without an identity.** There
    // is no half: `cmd/rivet-initd` refuses a seed with no certificate, and it
    // refuses it on the BOARD, which is the wrong end to discover a card was
    // written wrong.
    if let Some(identity) = &inputs.identity {
        files.push(BootFile::new(IDENTITY_SEED_FILE, identity.seed.clone()));
        files.push(BootFile::new(
            IDENTITY_CERTIFICATE_FILE,
            identity.certificate.clone(),
        ));
        files.push(BootFile::new(IDENTITY_PEERS_FILE, identity.peers.clone()));
    }

    // **The staging bundle rides beside the identity, and is independent of
    // it.** A member machine takes a register and no new identity; a fleet
    // server takes both. Coupling them would make one role impossible to
    // express and neither is a precondition of the other — PID 1 handles them
    // in two separate passes for that reason.
    if let Some(staged) = &inputs.staged {
        files.push(BootFile::new(STAGED_FILE, staged.clone().into_bytes()));
    }

    // **The applications, sorted by name** — `ADR-050` §4.
    //
    // Sorted for the firmware's reason one paragraph up: the output must not
    // depend on the order the caller happened to collect the payloads in, or
    // two cards built from the same inputs would differ (`ADR-007` §3). It
    // does not decide install order — that is the supervisor's, and
    // `ADR-017` §3 has no dependency graph for it to express.
    //
    // A duplicate name falls into the check below and is refused there, which
    // is where every other duplicate on this partition is refused.
    let mut applications = inputs.applications.clone();
    applications.sort_by(|a, b| a.name.cmp(&b.name));
    files.extend(applications);

    for file in &files {
        check_name(&file.name)?;
    }
    for (i, file) in files.iter().enumerate() {
        if files
            .iter()
            .skip(i.saturating_add(1))
            .any(|o| o.name == file.name)
        {
            return Err(BootError::DuplicateName {
                name: file.name.clone(),
            });
        }
    }

    let bytes = total_bytes(&files);
    let capacity = BOOT_SIZE_MIB.saturating_mul(1024).saturating_mul(1024);
    if bytes > capacity {
        return Err(BootError::TooLarge { bytes, capacity });
    }

    Ok(files)
}

/// How much of the partition the contents occupy, before FAT's own overhead.
///
/// Deliberately an underestimate of the space needed: directory entries and
/// cluster slack are the filesystem's business and [`crate::disk`]'s, not this
/// module's. It exists so [`contents`] can refuse the obviously-impossible case
/// rather than hand out a partition's worth of files that cannot fit in it.
pub fn total_bytes(files: &[BootFile]) -> u64 {
    files
        .iter()
        .map(|f| f.bytes.len() as u64)
        .fold(0u64, u64::saturating_add)
}

/// This slot's `/system` partition, read out of the frozen table rather than
/// assumed.
fn system_partition(slot: Slot) -> Result<PartitionSpec, BootError> {
    let number = slot.system_partition();
    partition_table()
        .into_iter()
        .find(|p| p.number == number)
        .ok_or(BootError::LayoutMismatch { partition: number })
}

/// Whether a name can be a file on the boot partition.
///
/// FAT's illegal set, plus the two entries a directory always has, plus a rule
/// that keeps the whole partition ASCII: the firmware's parser is not a Unicode
/// implementation and nothing here needs one.
fn check_name(name: &str) -> Result<(), BootError> {
    let reject = |why: &'static str| {
        Err(BootError::UnusableName {
            name: name.to_string(),
            why,
        })
    };
    if name.is_empty() {
        return reject("it is empty");
    }
    if name.len() > 255 {
        return reject("it is longer than 255 bytes");
    }
    if name == "." || name == ".." {
        return reject("a directory always has that entry");
    }
    if name.starts_with(' ') || name.ends_with(' ') || name.ends_with('.') {
        return reject("FAT does not keep a leading space or a trailing space or dot");
    }
    if !name.is_ascii() {
        return reject("the boot partition is kept ASCII");
    }
    if name.chars().any(|c| c.is_ascii_control()) {
        return reject("it contains a control character");
    }
    if name.chars().any(|c| "\"*/:<>?\\|".contains(c)) {
        return reject("FAT forbids one of \" * / : < > ? \\ |");
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// ~~PID 1's `autoboot.txt` parser, vendored.~~ **Deleted 2026-08-23, and the
// reason it existed is worth keeping.**
//
// The other side of `autoboot.txt` refuses any file it does not recognise,
// deliberately: a corrupted file on a headless board is not recoverable over the
// network. It and this module are written against the same file by different
// people, so this is where they drift — and the tests below are what catch it.
//
// The parser was **copied verbatim into this file**, behind a test that
// extracted both copies from both source files and compared their text, because
// it lived in `cmd/rivet-initd` and *"a binary crate has no library target, so
// it cannot be depended on"*. That sentence stopped being true when the
// boot-slot port moved to `rivet-adapter-image`. The copy, its drift guard and
// the guard's own can-it-fail test are therefore gone, and the tests below call
// `rivet_adapter_image::rewrite_autoboot_default` itself — which is what the
// guard was a proxy for.
//
// It moved twice in one day: `confirm.rs` → `cmd/rivet-initd/src/slots.rs` →
// `rivet-adapter-image`. The function was not edited on either move.
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// PID 1's supervisor, which owns `SUPERVISOR_PATH`.
    const SUPERVISE_SOURCE: &str = include_str!("../../rivet-initd/src/supervise.rs");

    fn a_spec() -> BootPartitionSpec {
        BootPartitionSpec { slot: Slot::A, default_slot: Slot::A }
    }

    fn inputs() -> BootInputs {
        BootInputs {
            kernel: b"kernel8".to_vec(),
            device_tree: b"dtb".to_vec(),
            firmware: vec![
                BootFile::new("start4.elf", b"start".to_vec()),
                BootFile::new("fixup4.dat", b"fixup".to_vec()),
            ],
            document: None,
            identity: None,
            staged: None,
            applications: Vec::new(),
        }
    }

    fn names(files: &[BootFile]) -> Vec<String> {
        files.iter().map(|f| f.name.clone()).collect()
    }

    fn file<'a>(files: &'a [BootFile], name: &str) -> Option<&'a BootFile> {
        files.iter().find(|f| f.name == name)
    }

    fn text(files: &[BootFile], name: &str) -> String {
        file(files, name).map_or_else(String::new, |f| {
            String::from_utf8_lossy(&f.bytes).to_string()
        })
    }

    fn fresh(slot: Slot) -> Vec<BootFile> {
        contents(&BootPartitionSpec::fresh(slot), &inputs()).unwrap_or_default()
    }

    /// Every line of a generated text file that is a setting rather than a
    /// comment or a section header.
    fn settings(text: &str) -> Vec<&str> {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('['))
            .collect()
    }

    // -- config.txt and tryboot.txt: ADR-018 §5.3's trap ----------------------

    /// The setting whose absence loses a headless board.
    #[test]
    fn config_carries_arm_64bit_and_so_does_tryboot() {
        assert!(
            settings(&config_txt()).contains(&"arm_64bit=1"),
            "{}",
            config_txt()
        );
        assert!(settings(&tryboot_txt()).contains(&"arm_64bit=1"));
    }

    /// `ADR-002` §4: "Image assembly writes the two together or it has built a
    /// rollback mechanism that bricks the rollback."
    #[test]
    fn tryboot_is_a_byte_identical_copy_of_config() {
        assert_eq!(config_txt(), tryboot_txt());

        let files = fresh(Slot::A);
        assert_eq!(
            file(&files, CONFIG_FILE).map(|f| &f.bytes),
            file(&files, TRYBOOT_FILE).map(|f| &f.bytes),
            "the two files on the partition must not differ either"
        );
    }

    #[test]
    fn a_partition_without_tryboot_txt_is_not_a_partition_this_module_produces() {
        assert!(names(&fresh(Slot::A)).contains(&TRYBOOT_FILE.to_string()));
        assert!(names(&fresh(Slot::B)).contains(&TRYBOOT_FILE.to_string()));
    }

    /// `config.txt` names files; the partition must actually hold them.
    #[test]
    fn config_names_the_kernel_and_device_tree_that_are_on_the_partition() {
        let files = fresh(Slot::A);
        let config = text(&files, CONFIG_FILE);
        assert!(settings(&config).contains(&format!("kernel={KERNEL_FILE}").as_str()));
        assert!(settings(&config).contains(&format!("device_tree={DEVICE_TREE_FILE}").as_str()));
        assert!(names(&files).contains(&KERNEL_FILE.to_string()));
        assert!(names(&files).contains(&DEVICE_TREE_FILE.to_string()));
    }

    #[test]
    fn config_turns_off_the_autodetection_stock_turns_on() {
        let config = config_txt();
        let s = settings(&config);
        for off in [
            "camera_auto_detect=0",
            "display_auto_detect=0",
            "auto_initramfs=0",
        ] {
            assert!(s.contains(&off), "{off} missing from:\n{config}");
        }
        assert!(!config_txt().contains("camera_auto_detect=1"));
    }

    #[test]
    fn config_enables_the_uart_the_panics_come_out_of() {
        assert!(settings(&config_txt()).contains(&"enable_uart=1"));
    }

    #[test]
    fn every_config_setting_is_a_single_key_equals_value() {
        for line in settings(&config_txt()) {
            let parts: Vec<&str> = line.splitn(2, '=').collect();
            assert_eq!(parts.len(), 2, "{line:?} is not key=value");
            assert!(
                !parts.first().is_some_and(|k| k.is_empty()),
                "{line:?} has no key"
            );
        }
    }

    // -- cmdline.txt: ADR-024 §8.1's trap -------------------------------------

    /// `ADR-024` §8.1 item 4: Raspberry Pi OS ships `cgroup_disable=memory`, and
    /// it silently removes the controller `ADR-015` §5.1 needs. Rivet writes
    /// its own command line, so this is a deletion Rivet must never make.
    #[test]
    fn the_command_line_never_disables_the_memory_cgroup() {
        for slot in [Slot::A, Slot::B] {
            let line = cmdline_txt(slot).unwrap_or_default();
            assert!(!line.contains("cgroup_disable"), "{line}");
        }
    }

    #[test]
    fn the_command_line_is_exactly_one_line_with_no_carriage_return() {
        for slot in [Slot::A, Slot::B] {
            let line = cmdline_txt(slot).unwrap_or_default();
            assert!(
                !line.contains('\r'),
                "a stray CR would reach the kernel: {line:?}"
            );
            assert_eq!(line.matches('\n').count(), 1, "{line:?}");
            assert!(line.ends_with('\n'), "{line:?}");
        }
    }

    /// `ADR-002` §4: `root=/dev/mmcblk0p2` was wrong because `p2` is boot-B.
    /// Each slot names its own `/system`, and the numbers come from the frozen
    /// contract.
    #[test]
    fn root_names_this_slots_system_partition_and_not_the_other_ones() {
        assert_eq!(root_spec(Slot::A), Ok("PARTLABEL=system-a".to_string()));
        assert_eq!(root_spec(Slot::B), Ok("PARTLABEL=system-b".to_string()));

        let a = cmdline_txt(Slot::A).unwrap_or_default();
        let b = cmdline_txt(Slot::B).unwrap_or_default();
        assert!(a.contains("root=PARTLABEL=system-a"), "{a}");
        assert!(b.contains("root=PARTLABEL=system-b"), "{b}");
        assert!(!a.contains("system-b"), "{a}");
        assert!(!b.contains("system-a"), "{b}");
    }

    /// The label has to be the one `crate::layout` actually puts on the
    /// partition, not a matching string that happens to agree today.
    #[test]
    fn the_root_label_is_the_label_the_frozen_table_gives_that_partition() {
        for slot in [Slot::A, Slot::B] {
            let expected = partition_table()
                .into_iter()
                .find(|p| p.number == slot.system_partition())
                .map(|p| p.label.to_string());
            assert_eq!(
                root_spec(slot),
                expected
                    .map(|l| format!("PARTLABEL={l}"))
                    .ok_or(BootError::LayoutMismatch {
                        partition: slot.system_partition()
                    })
            );
        }
    }

    /// `ADR-002` §4's `rootfstype=ext4` was wrong when it was written, for a
    /// different reason than it is right today.
    ///
    /// Two separate things are asserted here and both earn their place:
    ///
    /// 1. **The command line agrees with the table.** This is the drift check —
    ///    `cmdline.txt` must never name a filesystem the partition was not made
    ///    with, whatever that filesystem is.
    /// 2. **The table says `ext4`.** `ADR-018` §3 makes `/system` erofs, and
    ///    **§3.1 suspends that for the first cards** because the stock kernel
    ///    cannot mount erofs as root (`EROFS_FS=m`, and a module cannot be
    ///    loaded from the filesystem it is needed to mount).
    ///
    /// The second assertion looks redundant and is not. **It is a forcing
    /// function**: when `ADR-024` §8's custom kernel arrives and
    /// `layout::SYSTEM_FSTYPE` flips back to erofs, this test fails, and
    /// whoever flips it has to come here and change it on purpose. Deleting the
    /// pin instead would let the stopgap become permanent silently, which is
    /// exactly how stopgaps become permanent.
    #[test]
    fn rootfstype_is_the_filesystem_the_frozen_table_gives_that_partition() {
        for slot in [Slot::A, Slot::B] {
            let expected = partition_table()
                .into_iter()
                .find(|p| p.number == slot.system_partition())
                .map(|p| p.fstype.as_str().to_string())
                .unwrap_or_default();
            assert_eq!(
                expected, "ext4",
                "ADR-018 §3.1 — the stopgap. If this now says erofs, the custom \
                 kernel has landed: update this assertion and delete \
                 layout::SYSTEM_FSTYPE."
            );
            let line = cmdline_txt(slot).unwrap_or_default();
            assert!(line.contains(&format!("rootfstype={expected}")), "{line}");
        }
    }

    /// `ADR-002` §4: `init=/sbin/rivet-initd` was wrong; `ADR-008` §3 and
    /// `ADR-016` §3 put binaries under `/system/bin`. Cross-checked against
    /// `rivet-initd`'s own constant, read out of its source.
    #[test]
    fn the_init_path_is_a_sibling_of_the_supervisor_rivet_initd_spawns() {
        let marker = "pub const SUPERVISOR_PATH: &str = \"";
        let start = SUPERVISE_SOURCE
            .find(marker)
            .map(|i| i.saturating_add(marker.len()));
        let supervisor = start
            .and_then(|i| SUPERVISE_SOURCE.get(i..))
            .and_then(|rest| rest.find('"').and_then(|end| rest.get(..end)))
            .unwrap_or_default();

        assert_eq!(
            supervisor, "/system/bin/rivet-sysd",
            "read from supervise.rs"
        );
        let dir = |p: &str| {
            p.rsplit_once('/')
                .map(|(d, _)| d.to_string())
                .unwrap_or_default()
        };
        assert_eq!(dir(INIT_PATH), dir(supervisor));
        assert!(
            cmdline_txt(Slot::A)
                .unwrap_or_default()
                .contains(&format!("init={INIT_PATH}"))
        );
    }

    #[test]
    fn the_two_slots_command_lines_differ_only_in_the_root_term() {
        let a = cmdline_txt(Slot::A).unwrap_or_default();
        let b = cmdline_txt(Slot::B).unwrap_or_default();
        let strip = |s: &str| {
            s.split_whitespace()
                .filter(|t| !t.starts_with("root="))
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(strip(&a), strip(&b));
        assert_ne!(a, b);
    }

    #[test]
    fn the_command_line_carries_the_terms_the_firmware_contract_needs() {
        let line = cmdline_txt(Slot::A).unwrap_or_default();
        for term in ["console=serial0,115200", "console=tty1", "rootwait"] {
            assert!(line.contains(term), "{term} missing from {line}");
        }
        // tty1 last, so /dev/console is the HDMI console PID 1 speaks on.
        let serial = line.find("console=serial0");
        let tty = line.find("console=tty1");
        assert!(serial < tty, "{line}");
    }

    // -- autoboot.txt, and the other side of it -------------------------------

    #[test]
    fn a_fresh_card_defaults_to_its_own_slot_and_tries_the_other_one() {
        let a = autoboot_txt(Slot::A);
        assert!(settings(&a).contains(&"boot_partition=1"), "{a}");
        assert!(settings(&a).contains(&"boot_partition=2"), "{a}");
        assert!(settings(&a).contains(&"tryboot_a_b=1"), "{a}");

        // The default is the setting before the [tryboot] section header; the
        // override is the one after it. Split on the header *line*, so the
        // comment that mentions the section by name is not mistaken for it.
        let split = |text: &str| {
            let before = text
                .split("\n[tryboot]\n")
                .next()
                .unwrap_or_default()
                .to_string();
            let after = text
                .split("\n[tryboot]\n")
                .nth(1)
                .unwrap_or_default()
                .to_string();
            (before, after)
        };
        let (before, after) = split(&a);
        assert!(settings(&before).contains(&"boot_partition=1"), "{a}");
        assert!(settings(&after).contains(&"boot_partition=2"), "{a}");

        let b = autoboot_txt(Slot::B);
        let (before, after) = split(&b);
        assert!(settings(&before).contains(&"boot_partition=2"), "{b}");
        assert!(settings(&after).contains(&"boot_partition=1"), "{b}");
    }

    /// It describes the card, so both partitions of a pair get the same bytes.
    #[test]
    fn autoboot_depends_on_the_default_slot_and_not_on_which_partition_holds_it() {
        let a = contents(
            &BootPartitionSpec {
                slot: Slot::A,
                default_slot: Slot::A,
            },
            &inputs(),
        )
        .unwrap_or_default();
        let b = contents(
            &BootPartitionSpec {
                slot: Slot::B,
                default_slot: Slot::A,
            },
            &inputs(),
        )
        .unwrap_or_default();
        assert_eq!(text(&a, AUTOBOOT_FILE), text(&b, AUTOBOOT_FILE));
    }

    /// **The test the two modules exist to disagree in.** `confirm.rs` refuses
    /// any file it does not recognise; this runs its own parser over the file
    /// this module writes.
    #[test]
    fn rivet_initd_parser_accepts_the_autoboot_file_this_module_writes() {
        use rivet_adapter_image::rewrite_autoboot_default;

        let fresh_a = autoboot_txt(Slot::A);
        // Booted from partition 1 on a card that already defaults to it:
        // recognised, and nothing to write.
        assert_eq!(rewrite_autoboot_default(&fresh_a, 1), Ok(None), "{fresh_a}");

        // Booted from partition 2 after a tryboot: the default moves.
        let committed = rewrite_autoboot_default(&fresh_a, 2);
        assert!(matches!(committed, Ok(Some(_))), "{committed:?}");
        let committed = committed.ok().flatten().unwrap_or_default();
        assert!(
            settings(&committed).contains(&"boot_partition=2"),
            "{committed}"
        );

        // And the file PID 1 leaves behind is still one it will accept.
        assert_eq!(
            rewrite_autoboot_default(&committed, 2),
            Ok(None),
            "{committed}"
        );
    }

    #[test]
    fn rivet_initd_parser_accepts_both_slots_files() {
        use rivet_adapter_image::rewrite_autoboot_default;
        for (default, partition) in [(Slot::A, 1u32), (Slot::B, 2u32)] {
            let text = autoboot_txt(default);
            assert_eq!(
                rewrite_autoboot_default(&text, partition),
                Ok(None),
                "{text}"
            );
        }
    }

    // -- the carried applications, ADR-050 §4 ---------------------------------

    /// **The payload reaches the partition byte-for-byte**, under the name
    /// `rivet_core::carried` composes — which is the name PID 1 looks for.
    #[test]
    fn a_carried_payload_is_on_the_partition_unchanged_and_under_its_own_name() {
        let payload = vec![0xABu8; 4096];
        let mut inputs = inputs();
        inputs.applications = vec![BootFile::new(
            rivet_core::carried::card_file("rivet-fleetd"),
            payload.clone(),
        )];
        let files = contents(&a_spec(), &inputs).expect("a card carrying one application");

        let found = file(&files, "app-rivet-fleetd.rvt").expect("the payload is on the partition");
        assert_eq!(found.bytes, payload, "the bytes were not carried unchanged");
        assert_eq!(
            rivet_core::carried::product_of(&found.name).as_deref(),
            Some("rivet-fleetd"),
            "PID 1 would not recognise {} as a payload",
            found.name
        );
    }

    /// And it does not displace anything: a card that carries applications
    /// still carries a kernel, a document and the rest.
    #[test]
    fn carrying_applications_leaves_the_rest_of_the_partition_alone() {
        let mut inputs = inputs();
        inputs.applications = vec![
            BootFile::new(rivet_core::carried::card_file("rivet-fleetd"), vec![1u8; 16]),
            BootFile::new(rivet_core::carried::card_file("rivet-proxyd"), vec![2u8; 16]),
        ];
        let with = names(&contents(&a_spec(), &inputs).unwrap_or_default());
        let without = names(&fresh(Slot::A));
        for name in &without {
            assert!(with.contains(name), "{name} was displaced by a payload");
        }
        assert_eq!(with.len(), without.len() + 2, "{with:?}");
    }

    /// **The order does not depend on the caller's**, for the firmware's
    /// reason: two builds of the same inputs must produce the same bytes
    /// (`ADR-007` §3).
    #[test]
    fn the_payloads_are_emitted_in_a_deterministic_order() {
        let one = BootFile::new(rivet_core::carried::card_file("rivet-proxyd"), vec![2u8; 8]);
        let two = BootFile::new(rivet_core::carried::card_file("rivet-fleetd"), vec![1u8; 8]);

        let mut a = inputs();
        a.applications = vec![one.clone(), two.clone()];
        let mut b = inputs();
        b.applications = vec![two, one];
        assert_eq!(
            contents(&a_spec(), &a).unwrap_or_default(),
            contents(&a_spec(), &b).unwrap_or_default()
        );
    }

    /// A card that carries nothing is byte-identical to one built before this
    /// existed — the ordinary headless card, and every card built before
    /// 2026-09-07.
    #[test]
    fn a_card_that_carries_no_application_is_the_card_it_always_was() {
        let mut inputs = inputs();
        inputs.applications = Vec::new();
        assert_eq!(contents(&a_spec(), &inputs), contents(&a_spec(), &inputs));
        assert!(
            !names(&fresh(Slot::A))
                .iter()
                .any(|n| n.starts_with(rivet_core::carried::PREFIX)),
            "an empty carry put something on the partition"
        );
    }

    /// Two payloads under one name is refused where every other duplicate on
    /// this partition is refused, rather than one silently overwriting the
    /// other.
    #[test]
    fn two_payloads_with_the_same_name_are_refused() {
        let mut inputs = inputs();
        let name = rivet_core::carried::card_file("rivet-fleetd");
        inputs.applications = vec![
            BootFile::new(name.clone(), vec![1u8; 8]),
            BootFile::new(name.clone(), vec![2u8; 8]),
        ];
        assert_eq!(
            contents(&a_spec(), &inputs),
            Err(BootError::DuplicateName { name })
        );
    }

    // -- the provisioning file ------------------------------------------------

    #[test]
    fn the_provisioning_file_declares_the_schema_version_and_nothing_else() {
        let toml = provisioning_toml();
        assert_eq!(settings(&toml), vec!["version = 1"], "{toml}");
    }

    /// **A supplied document replaces the template outright.** Never a merge:
    /// a document assembled from two sources is one no reader can predict, and
    /// `ADR-007` §3.6.5 refuses an unknown key rather than ignoring it — so the
    /// template's `version = 1` landing on top of an operator's document would
    /// be a duplicate key rather than a harmless comment.
    #[test]
    fn a_supplied_document_is_written_instead_of_the_template_and_not_beside_it() {
        let theirs = "version = 1
[network.eth0]
method = \"static\"
address = \"10.0.0.2/24\"
";
        let mut given = inputs();
        given.document = Some(theirs.to_string());
        let files = contents(&BootPartitionSpec::fresh(Slot::A), &given).expect("a card");
        let written = file(&files, PROVISIONING_FILE).expect("rivet.toml is on the card");
        assert_eq!(String::from_utf8_lossy(&written.bytes), theirs);
        assert!(
            !String::from_utf8_lossy(&written.bytes).contains("Generated by rivet-image"),
            "the template's banner survived into an operator's document"
        );
    }

    /// **Both slots get it.** A card is provisioned, not a slot: an update
    /// swaps which boot partition the firmware reads, and a machine that lost
    /// its address and its keys on the first update would be unreachable at
    /// exactly the moment somebody wanted to look at it.
    #[test]
    fn the_document_is_written_to_both_boot_partitions() {
        let theirs = "version = 1
";
        let mut given = inputs();
        given.document = Some(theirs.to_string());
        for slot in [Slot::A, Slot::B] {
            let files = contents(&BootPartitionSpec::fresh(slot), &given).expect("a card");
            let written = file(&files, PROVISIONING_FILE).expect("rivet.toml");
            assert_eq!(String::from_utf8_lossy(&written.bytes), theirs, "{slot:?}");
        }
    }

    /// No document is still the template, which is every card built before
    /// 2026-08-24 and is what `--no-document` would have meant if it existed.
    #[test]
    fn no_document_leaves_the_template_exactly_as_it_was() {
        let files = contents(&BootPartitionSpec::fresh(Slot::A), &inputs()).expect("a card");
        let written = file(&files, PROVISIONING_FILE).expect("rivet.toml");
        assert_eq!(String::from_utf8_lossy(&written.bytes), provisioning_toml());
    }

    /// `ADR-020` §4 and `AGENTS.md` §8.3: the file will hold a passphrase one
    /// day, and nothing that looks like one is generated into it today.
    #[test]
    fn the_provisioning_template_carries_no_credential_shaped_placeholder() {
        let toml = provisioning_toml().to_lowercase();
        for word in ["passphrase", "password", "psk", "secret", "token"] {
            assert!(!toml.contains(word), "{word} appears in the template");
        }
    }

    // -- the file list --------------------------------------------------------

    #[test]
    fn the_partition_holds_exactly_adr_002_4s_files() {
        let mut got = names(&fresh(Slot::A));
        got.sort();
        let mut want = vec![
            "bcm2711-rpi-4-b.dtb",
            "kernel8.img",
            "fixup4.dat",
            "start4.elf",
            "config.txt",
            "tryboot.txt",
            "autoboot.txt",
            "cmdline.txt",
            "rivet.toml",
        ];
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn the_order_does_not_depend_on_the_order_the_firmware_arrived_in() {
        let mut shuffled = inputs();
        shuffled.firmware.reverse();
        shuffled
            .firmware
            .push(BootFile::new("start4x.elf", b"x".to_vec()));
        shuffled.firmware.swap(0, 2);

        let mut ordered = inputs();
        ordered
            .firmware
            .push(BootFile::new("start4x.elf", b"x".to_vec()));
        ordered.firmware.sort_by(|a, b| a.name.cmp(&b.name));

        let spec = BootPartitionSpec::fresh(Slot::A);
        assert_eq!(contents(&spec, &shuffled), contents(&spec, &ordered));
    }

    #[test]
    fn the_same_inputs_produce_byte_identical_output() {
        let spec = BootPartitionSpec::fresh(Slot::B);
        assert_eq!(contents(&spec, &inputs()), contents(&spec, &inputs()));
    }

    /// Two cards should differ only where they actually differ — `ADR-007`
    /// §3.1's property, applied to a diff between boot partitions.
    #[test]
    fn the_two_slots_partitions_differ_only_in_cmdline_txt() {
        let a = contents(
            &BootPartitionSpec {
                slot: Slot::A,
                default_slot: Slot::A,
            },
            &inputs(),
        )
        .unwrap_or_default();
        let b = contents(
            &BootPartitionSpec {
                slot: Slot::B,
                default_slot: Slot::A,
            },
            &inputs(),
        )
        .unwrap_or_default();

        assert_eq!(names(&a), names(&b));
        let differ: Vec<String> = a
            .iter()
            .zip(b.iter())
            .filter(|(x, y)| x.bytes != y.bytes)
            .map(|(x, _)| x.name.clone())
            .collect();
        assert_eq!(differ, vec![CMDLINE_FILE.to_string()]);
    }

    #[test]
    fn every_generated_file_is_ascii_with_lf_endings_and_no_tabs() {
        let files = fresh(Slot::A);
        for name in [
            CONFIG_FILE,
            TRYBOOT_FILE,
            AUTOBOOT_FILE,
            CMDLINE_FILE,
            PROVISIONING_FILE,
        ] {
            let body = text(&files, name);
            assert!(!body.is_empty(), "{name} is empty");
            assert!(body.is_ascii(), "{name} is not ASCII");
            assert!(!body.contains('\r'), "{name} has a CR");
            assert!(!body.contains('\t'), "{name} has a tab");
            assert!(body.ends_with(EOL), "{name} does not end with a newline");
        }
    }

    #[test]
    fn the_firmware_and_kernel_bytes_are_copied_through_untouched() {
        let files = fresh(Slot::A);
        assert_eq!(
            file(&files, KERNEL_FILE).map(|f| f.bytes.clone()),
            Some(b"kernel8".to_vec())
        );
        assert_eq!(
            file(&files, DEVICE_TREE_FILE).map(|f| f.bytes.clone()),
            Some(b"dtb".to_vec())
        );
        assert_eq!(
            file(&files, "start4.elf").map(|f| f.bytes.clone()),
            Some(b"start".to_vec())
        );
    }

    // -- refusals -------------------------------------------------------------

    #[test]
    fn a_missing_kernel_or_device_tree_is_refused() {
        let spec = BootPartitionSpec::fresh(Slot::A);
        let mut no_kernel = inputs();
        no_kernel.kernel.clear();
        assert_eq!(contents(&spec, &no_kernel), Err(BootError::EmptyKernel));

        let mut no_dtb = inputs();
        no_dtb.device_tree.clear();
        assert_eq!(contents(&spec, &no_dtb), Err(BootError::EmptyDeviceTree));
    }

    #[test]
    fn a_partition_without_start4_elf_or_fixup4_dat_is_refused() {
        let spec = BootPartitionSpec::fresh(Slot::A);
        for missing in REQUIRED_FIRMWARE {
            let mut inputs = inputs();
            inputs.firmware.retain(|f| f.name != *missing);
            assert_eq!(
                contents(&spec, &inputs),
                Err(BootError::MissingFirmware { name: missing })
            );
        }
    }

    #[test]
    fn a_firmware_file_that_collides_with_a_generated_one_is_refused() {
        let spec = BootPartitionSpec::fresh(Slot::A);
        let mut inputs = inputs();
        inputs
            .firmware
            .push(BootFile::new(CONFIG_FILE, b"not ours".to_vec()));
        assert_eq!(
            contents(&spec, &inputs),
            Err(BootError::DuplicateName {
                name: CONFIG_FILE.to_string()
            })
        );
    }

    #[test]
    fn two_firmware_files_with_the_same_name_are_refused() {
        let spec = BootPartitionSpec::fresh(Slot::A);
        let mut inputs = inputs();
        inputs
            .firmware
            .push(BootFile::new("start4.elf", b"other".to_vec()));
        assert!(matches!(
            contents(&spec, &inputs),
            Err(BootError::DuplicateName { .. })
        ));
    }

    #[test]
    fn a_name_fat_cannot_hold_is_refused() {
        let spec = BootPartitionSpec::fresh(Slot::A);
        for bad in [
            "",
            ".",
            "..",
            "a/b.elf",
            "a:b.elf",
            "trailing ",
            " leading",
            "dot.",
            "é.elf",
        ] {
            let mut inputs = inputs();
            inputs.firmware.push(BootFile::new(bad, b"x".to_vec()));
            assert!(
                matches!(
                    contents(&spec, &inputs),
                    Err(BootError::UnusableName { .. })
                ),
                "{bad:?} was accepted"
            );
        }
    }

    #[test]
    fn contents_larger_than_the_partition_are_refused_rather_than_truncated() {
        let spec = BootPartitionSpec::fresh(Slot::A);
        let mut inputs = inputs();
        let capacity = BOOT_SIZE_MIB.saturating_mul(1024).saturating_mul(1024);
        inputs.kernel = vec![0u8; usize::try_from(capacity).unwrap_or(usize::MAX)];
        inputs.kernel.push(0);
        assert!(matches!(
            contents(&spec, &inputs),
            Err(BootError::TooLarge { .. })
        ));
    }

    #[test]
    fn a_real_sized_kernel_fits_the_partition_layout_reserves() {
        let spec = BootPartitionSpec::fresh(Slot::A);
        let mut inputs = inputs();
        // The kernel in tools/qemu is about 10 MiB; the firmware is a few more.
        inputs.kernel = vec![0u8; 12 * 1024 * 1024];
        inputs.firmware = vec![
            BootFile::new("start4.elf", vec![0u8; 3 * 1024 * 1024]),
            BootFile::new("fixup4.dat", vec![0u8; 8 * 1024]),
        ];
        assert!(contents(&spec, &inputs).is_ok());
    }

    #[test]
    fn every_refusal_says_what_would_have_gone_wrong() {
        let errors = [
            BootError::EmptyKernel,
            BootError::EmptyDeviceTree,
            BootError::MissingFirmware { name: "start4.elf" },
            BootError::DuplicateName { name: "x".into() },
            BootError::UnusableName {
                name: "x".into(),
                why: "because",
            },
            BootError::TooLarge {
                bytes: 2,
                capacity: 1,
            },
            BootError::LayoutMismatch { partition: 9 },
        ];
        for e in errors {
            let said = e.to_string();
            assert!(said.len() > 20, "{e:?} says only {said:?}");
        }
    }

    #[test]
    fn total_bytes_counts_every_file() {
        let files = fresh(Slot::A);
        let sum: u64 = files.iter().map(|f| f.bytes.len() as u64).sum();
        assert_eq!(total_bytes(&files), sum);
        assert!(total_bytes(&[]) == 0);
    }

    /// **A card can arrive enrolled** — `ADR-019` §2, `ADR-025` §3. The three
    /// files go on the boot partition, where PID 1 finds them on first boot and
    /// where it then deletes the seed (`ADR-019` §3.1).
    #[test]
    fn an_identity_reaches_the_boot_partition_as_three_files() {
        let mut inputs = inputs();
        inputs.identity = Some(Identity {
            seed: vec![7u8; 32],
            certificate: vec![0x30, 0x82, 9],
            peers: b"aabb
".to_vec(),
        });
        let files = contents(&a_spec(), &inputs).expect("a valid boot partition");
        let named = |n: &str| files.iter().find(|f| f.name == n);
        assert_eq!(named(IDENTITY_SEED_FILE).map(|f| f.bytes.len()), Some(32));
        assert_eq!(named(IDENTITY_CERTIFICATE_FILE).map(|f| f.bytes.as_slice()), Some(&[0x30u8, 0x82, 9][..]));
        assert!(named(IDENTITY_PEERS_FILE).is_some(), "the pin file did not reach the card");
    }

    /// And a card without one carries none of the three, rather than empty
    /// placeholders that PID 1 would then have to tell apart from a real
    /// identity.
    #[test]
    fn a_card_with_no_identity_carries_none_of_the_three_files() {
        let files = contents(&a_spec(), &inputs()).expect("a valid boot partition");
        for name in [IDENTITY_SEED_FILE, IDENTITY_CERTIFICATE_FILE, IDENTITY_PEERS_FILE] {
            assert!(
                !files.iter().any(|f| f.name == name),
                "{name} is on a card that was given no identity"
            );
        }
    }

    /// **A card can arrive carrying an application's credentials too** —
    /// `ADR-049`. The bundle is one file, and it is on the partition PID 1
    /// reads before any service runs.
    #[test]
    fn a_staging_bundle_reaches_the_boot_partition_as_one_file() {
        let mut inputs = inputs();
        let staged = rivet_core::staging::render(&[
            rivet_core::staging::Staged::new("rivet-fleetd", "server.crt", b"pem".to_vec())
                .expect("a legal entry"),
        ]);
        inputs.staged = Some(staged.clone());
        let files = contents(&a_spec(), &inputs).expect("a valid boot partition");
        let found = files
            .iter()
            .find(|f| f.name == STAGED_FILE)
            .expect("the staging bundle did not reach the card");
        assert_eq!(found.bytes, staged.into_bytes());
    }

    /// And the bundle does not depend on an identity being present: a fleet
    /// MEMBER is given the fleet's register and no new keypair.
    #[test]
    fn a_staging_bundle_reaches_the_card_without_an_identity_beside_it() {
        let mut inputs = inputs();
        inputs.staged = Some("version = 1\n".to_owned());
        let files = contents(&a_spec(), &inputs).expect("a valid boot partition");
        assert!(files.iter().any(|f| f.name == STAGED_FILE));
        assert!(!files.iter().any(|f| f.name == IDENTITY_SEED_FILE));
    }

    #[test]
    fn a_card_with_nothing_staged_carries_no_bundle() {
        let files = contents(&a_spec(), &inputs()).expect("a valid boot partition");
        assert!(
            !files.iter().any(|f| f.name == STAGED_FILE),
            "{STAGED_FILE} is on a card that was given nothing to stage"
        );
    }

    /// The bundle's name has to survive this module's own FAT rules, and it is
    /// chosen in another crate.
    #[test]
    fn the_staging_bundles_name_is_one_this_partition_accepts() {
        check_name(STAGED_FILE).expect("rivet_core::staging named a file FAT cannot hold");
    }
}
