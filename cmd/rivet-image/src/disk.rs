//! Partitioning and filesystem creation. `ADR-018` §4 is the authority.
//!
//! This module turns [`layout`](crate::layout)'s table into an **image file**.
//! Not a written card: writing is `dd` or Raspberry Pi Imager, so the step that
//! can destroy a disk is one the operator takes deliberately with a device path
//! they typed themselves. This module refuses to open anything that is not a
//! regular file for exactly that reason ([`ImageError::NotARegularFile`]).
//!
//! ## It drives tools; it does not reimplement them
//!
//! `sgdisk`, `mkfs.vfat`, `mcopy`, `mkfs.erofs` and `mkfs.ext4` are
//! `ADR-009` §3.4's left column — self-contained programs driven over a pipe,
//! whose types never reach Rivet's own. Every one of them is invoked through
//! [`Host`], so the *decisions* (geometry, argv, ordering, refusals) are tested
//! without any of them being installed, and the layer that actually spawns a
//! process stays thin. This is the same seam `cmd/rivet-initd/src/kernel.rs`
//! draws against the kernel.
//!
//! ## It never needs root
//!
//! No `losetup`, no `mount`, no `sudo`. `mkfs.vfat --offset` and
//! `mkfs.ext4 -E offset=` write a filesystem into the middle of an existing
//! file; `mcopy` populates a FAT through mtools' `image@@offset` syntax;
//! `mkfs.erofs` builds from a source directory. Every other tool in this
//! repository runs unprivileged except the QEMU boot, and an image build that
//! needed `sudo` would be hostile to automate.
//!
//! ## Ownership, and why a `/system` tree goes in as a **tar**
//!
//! Nothing in this build runs as root, so nothing can `chown`. Two of the three
//! `mkfs` tools have to be told, and each is told differently:
//!
//! * **erofs** — `--all-root`. Measured with `dump.erofs`: without it every
//!   inode carries `Uid: 1000 Gid: 1000`; with it, `Uid: 0 Gid: 0`.
//! * **ext4** — **`mke2fs -d` accepts a tarball as well as a directory, and a
//!   tar can claim any ownership its author likes.** Measured with
//!   `debugfs -R "ls -l /bin"`: from a *directory*, `rivet-initd` is
//!   `100755 1000 1000`; from a tar built `--owner=0 --group=0
//!   --numeric-owner`, it is `100755 0 0`, and the symlink survives as
//!   `120777 0 0`. `ADR-008` §3 makes `/system` the image, and an image whose
//!   files belong to a uid that does not exist on the board is a silent wrong
//!   answer — it builds, it boots, and it is wrong.
//!
//! `/state` needs none of this: it ships **empty**. `ADR-018` §4 makes it
//! machine content, so nothing the image builder writes belongs there.
//!
//! *Corrected 2026-08-23:* this used to add "and PID 1 creates what it needs on
//! first boot, as root". PID 1 **mounts** it (`ADR-017` §3.1.7) and creates
//! nothing inside it; whatever first writes to `/state/config`, `/state/home` or
//! `/state/ext` makes its own directory. The sentence described a plan rather
//! than the code.
//!
//! ## Every region is byte-for-byte reproducible
//!
//! `ADR-015` §4 signs these artifacts, and a signature over something nobody
//! can rebuild identically is worth much less than it looks. Measured on
//! `gptfdisk` 1.0.10, `dosfstools` 4.2, `mtools` 4.0.49, `erofs-utils` 1.9.3
//! and `e2fsprogs` 1.47.4 — two builds three seconds apart, compared region by
//! region:
//!
//! | Region | How it is pinned |
//! | :-- | :-- |
//! | GPT | every GUID is a fixed constant ([`DISK_GUID`]) |
//! | FAT32 | `--invariant`, a fixed `-i` volume id, staged mtimes at [`IMAGE_EPOCH`], `TZ=UTC`, and a sorted `mcopy` argv |
//! | erofs | `-U`, `-T` with `--all-time`, `--all-root` |
//! | ext4 | `-U`, `-E hash_seed=`, a `--sort=name --mtime=` tar, and **`SOURCE_DATE_EPOCH`** |
//!
//! **`SOURCE_DATE_EPOCH` is the whole of the ext4 answer, and it is easy to
//! conclude it does not work.** `mke2fs -E bogus=1` lists no time option, and
//! grepping the binary for the name finds nothing. Both of those are misleading:
//! two builds of a populated 640 MiB ext4 with it set differ in **0** bytes, and
//! without it in **93** — the superblock's `s_wtime`, `s_lastcheck` and
//! `s_mkfs_time` with their checksum, plus inode timestamps. It is in
//! [`TOOL_ENV`] for that reason and removing it silently costs the property.

#![allow(dead_code)] // Remove when `main` wires this up — same note as `layout`.

use std::collections::{BTreeMap, BTreeSet};

use crate::layout::{FsType, PartitionSpec, minimum_device_mib, partition_table};

// ---------------------------------------------------------------------------
// Content, handed in structurally
// ---------------------------------------------------------------------------

/// One thing that goes on a filesystem.
///
/// Deliberately structural. `boot` and `system` decide *what* a partition
/// contains and hand it over as paths, bytes and modes; this module never sees
/// their types, so none of the three has to be written before the others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// An explicitly empty directory. **Needed, not decorative:** `ADR-018` §6
    /// puts a tmpfs over `/system/ext`, and a mount point cannot be created on
    /// a read-only filesystem, so the empty directory has to be in the image.
    Dir { path: String, mode: u32 },
    File {
        path: String,
        mode: u32,
        contents: Vec<u8>,
    },
    /// erofs and ext4 hold these; FAT cannot, and is refused rather than
    /// silently given a copy of the target.
    Symlink { path: String, target: String },
}

impl Entry {
    pub fn path(&self) -> &str {
        match self {
            Self::Dir { path, .. } | Self::File { path, .. } | Self::Symlink { path, .. } => path,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Dir { .. } => "directory",
            Self::File { .. } => "file",
            Self::Symlink { .. } => "symlink",
        }
    }
}

/// A validated set of [`Entry`] values, rooted at a filesystem's `/`.
///
/// Validation happens once, at construction, so nothing downstream has to ask
/// whether a path escapes the tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tree {
    entries: Vec<Entry>,
}

impl Tree {
    /// Sorted by path, so the staging order — and therefore FAT directory
    /// entry order — does not depend on the caller's iteration order.
    pub fn new(entries: Vec<Entry>) -> Result<Self, ImageError> {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for e in &entries {
            check_relative_path(e.path())?;
            if let Entry::Symlink { path, target } = e
                && target.is_empty()
            {
                return Err(ImageError::BadPath {
                    path: path.clone(),
                    reason: "a symlink with an empty target",
                });
            }
            if !seen.insert(e.path()) {
                return Err(ImageError::DuplicatePath {
                    path: e.path().to_string(),
                });
            }
        }
        let mut entries = entries;
        entries.sort_by(|a, b| a.path().cmp(b.path()));
        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether the tree holds an entry at exactly this path.
    pub fn has(&self, path: &str) -> bool {
        self.entries.iter().any(|e| e.path() == path)
    }

    /// The distinct first path components, sorted.
    ///
    /// `mcopy` is handed these rather than a glob, because a glob's order is
    /// the shell's and FAT records directory entries in the order they are
    /// written.
    pub fn top_level(&self) -> Vec<String> {
        let mut out: BTreeSet<String> = BTreeSet::new();
        for e in &self.entries {
            if let Some(first) = e.path().split('/').next() {
                out.insert(first.to_string());
            }
        }
        out.into_iter().collect()
    }

    fn first_symlink(&self) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| matches!(e, Entry::Symlink { .. }))
    }
}

/// A path is relative, has no `.` or `..`, and holds nothing that would change
/// its meaning to a shell-free `exec` or to a filesystem.
fn check_relative_path(path: &str) -> Result<(), ImageError> {
    let bad = |reason: &'static str| {
        Err(ImageError::BadPath {
            path: path.to_string(),
            reason,
        })
    };
    if path.is_empty() {
        return bad("an empty path");
    }
    if path.starts_with('/') {
        return bad("an absolute path; trees are rooted at the filesystem's own /");
    }
    if path.contains('\\') {
        return bad("a backslash, which is a separator on the build host and not on the target");
    }
    if path.contains('\0') {
        return bad("a NUL byte");
    }
    for component in path.split('/') {
        match component {
            "" => return bad("an empty path component"),
            "." | ".." => return bad("a `.` or `..` component, which can escape the tree"),
            _ => {}
        }
        if component.len() > 255 {
            return bad("a path component longer than 255 bytes");
        }
        if component.chars().any(|c| c.is_control()) {
            return bad("a control character in a path component");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// GPT's logical block, and the unit `sgdisk` and `mkfs.vfat --offset` count in.
pub const SECTOR_BYTES: u64 = 512;

/// 1 MiB, in sectors. `sgdisk`'s own default, restated here because this module
/// computes the boundaries rather than letting `sgdisk` pick them — a start
/// sector that is computed in Rust is a start sector a test can assert on.
pub const ALIGN_SECTORS: u64 = 2048;

/// The first sector a partition may occupy. Sector 0 is the protective MBR,
/// 1 is the GPT header, and 2..=33 are the 128 partition entries; the round
/// number 2048 leaves that alone and aligns to 1 MiB.
pub const FIRST_USABLE_SECTOR: u64 = 2048;

/// The secondary GPT at the end of the device: one header plus 32 sectors of
/// entries. Nothing may be laid out inside it.
pub const GPT_TAIL_SECTORS: u64 = 34;

/// A partition with its place on the device decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub spec: PartitionSpec,
    pub first_lba: u64,
    /// Inclusive, as GPT and `sgdisk` both mean it.
    pub last_lba: u64,
}

impl Placed {
    pub fn sectors(&self) -> u64 {
        self.last_lba - self.first_lba + 1
    }

    pub fn offset_bytes(&self) -> u64 {
        self.first_lba * SECTOR_BYTES
    }

    pub fn len_bytes(&self) -> u64 {
        self.sectors() * SECTOR_BYTES
    }
}

/// Lay `ADR-018` §4's table onto a device of `device_mib` mebibytes.
///
/// Pure: no I/O, no tool, no image. Every geometry decision this module makes
/// is here so that it can be asserted on without `sgdisk` being installed.
pub fn plan(device_mib: u64) -> Result<Vec<Placed>, ImageError> {
    let minimum = minimum_device_mib();
    if device_mib < minimum {
        return Err(ImageError::DeviceTooSmall {
            device_mib,
            minimum_mib: minimum,
        });
    }
    let total_sectors = device_mib
        .checked_mul(1024 * 1024 / SECTOR_BYTES)
        .ok_or(ImageError::DeviceTooLarge { device_mib })?;

    let table = partition_table();
    let unsized_count = table.iter().filter(|p| p.size_mib.is_none()).count();
    if unsized_count != 1 {
        return Err(ImageError::TableNotUsable {
            reason: "exactly one partition must take the remainder of the device",
        });
    }

    // Everything above the tail is reserved for the secondary GPT, and the
    // remainder partition's end is pulled down to an alignment boundary so a
    // card's last partition starts and ends where every other one does.
    let last_usable = total_sectors - GPT_TAIL_SECTORS;
    let remainder_end = (last_usable + 1) / ALIGN_SECTORS * ALIGN_SECTORS - 1;

    let mut placed = Vec::with_capacity(table.len());
    let mut cursor = FIRST_USABLE_SECTOR;
    for spec in table {
        let (first_lba, last_lba) = match spec.size_mib {
            Some(mib) => {
                let sectors = mib * (1024 * 1024 / SECTOR_BYTES);
                (cursor, cursor + sectors - 1)
            }
            None => (cursor, remainder_end),
        };
        if last_lba < first_lba || last_lba > remainder_end {
            return Err(ImageError::DeviceTooSmall {
                device_mib,
                minimum_mib: minimum,
            });
        }
        cursor = last_lba + 1;
        placed.push(Placed {
            spec,
            first_lba,
            last_lba,
        });
    }
    Ok(placed)
}

// ---------------------------------------------------------------------------
// Fixed identity — the price of a reproducible image
// ---------------------------------------------------------------------------

/// Every GUID below is a constant, and none of them is random.
///
/// **That is a trade, and the losing side is worth naming.** A random disk GUID
/// would make two Rivet cards distinguishable by `/dev/disk/by-partuuid` on a
/// host with both attached. A fixed one makes the image byte-identical between
/// builds, which is what `ADR-015` §4's signature needs — a signature over an
/// artifact that changes every build proves nothing about the build. The Pi has
/// one card, and the build host never attaches the image, so the cost falls
/// where nobody is standing.
///
/// The shape is a well-formed version-4 variant-1 UUID (the `4` and the `8`),
/// with a body no generator would ever produce, so anyone reading `sgdisk -p`
/// can see at a glance that it was chosen rather than rolled.
pub const DISK_GUID: &str = "FEEDFACE-0000-4000-8000-000000000000";

/// The unique GUID of partition `number`.
pub fn partition_guid(number: u32) -> String {
    format!("FEEDFACE-0000-4000-8000-{number:012X}")
}

/// The filesystem UUID inside partition `number`. A separate namespace from
/// [`partition_guid`] — leading `F` rather than `0` — so a UUID in a bug report
/// says which of the two it is.
pub fn filesystem_uuid(number: u32) -> String {
    format!("FEEDFACE-0000-4000-8000-F{number:011X}")
}

/// FAT32 has a 32-bit volume id, not a UUID. `blkid` renders this as
/// `FEED-000N`.
pub fn fat_volume_id(number: u32) -> String {
    format!("FEED{number:04X}")
}

/// ext4's directory-hash seed is a UUID and is otherwise random per filesystem.
pub const EXT4_HASH_SEED: &str = "FEEDFACE-0000-4000-8000-5EED00000005";

/// 1980-01-01T00:00:00Z — **the earliest instant a FAT timestamp can hold**,
/// which is why it and not the Unix epoch. `mcopy` writing a 1970 mtime into
/// FAT wraps it to 2097; that was observed with `mdir` before this constant
/// existed, and a build whose dates are visibly absurd invites somebody to
/// "fix" the determinism away.
pub const IMAGE_EPOCH: i64 = 315_532_800;

/// Environment every tool is run with.
///
/// **`TZ` is load-bearing, not tidiness.** `mcopy` converts a source mtime to
/// FAT's local-time-with-no-zone using the caller's timezone, so the same
/// inputs on two developers' machines produced two different boot partitions
/// until this was pinned.
///
/// **`SOURCE_DATE_EPOCH` is the only thing making ext4 reproducible.** See the
/// module documentation: with it, two builds of a populated 640 MiB ext4 differ
/// in 0 bytes; without it, 93.
///
/// `LC_ALL=C` keeps the stderr this module quotes in one language.
pub const TOOL_ENV: &[(&str, &str)] = &[
    ("TZ", "UTC"),
    ("LC_ALL", "C"),
    ("SOURCE_DATE_EPOCH", "315532800"),
];

/// The GPT type GUID for a partition holding `fs`.
///
/// **The FAT partitions are marked Microsoft basic data, and the guess this
/// replaces was wrong on hardware.** Until 2026-08-23 this returned
/// `C12A7328-…`, the EFI System Partition GUID, on the reasoning that it is the
/// one type with a universal meaning of "the FAT filesystem firmware boots
/// from" and so the likeliest thing the `Matched GUID!` string in the EEPROM is
/// compared against. `ADR-018` §9 item 1 had established that the parser exists
/// without establishing which GUID it matches, and the comment here said that
/// if a board refused the image this constant was the first thing to change.
///
/// **A board refused the image.** The first card written from this builder was
/// booted on a Pi 4 Model B on 2026-08-23 and the bootloader printed
/// `GPT: No bootable partitions` — the same string the EEPROM audit found —
/// against a table whose two FAT partitions were `EF00`. So the parser ran, saw
/// the ESP GUID, and did not accept it. `EBD0A0A2-…` is what the stock card's
/// MBR type `0x0c` maps to under GPT, and it is the documented alternative that
/// comment named.
pub const fn type_guid(fs: FsType) -> &'static str {
    match fs {
        FsType::Fat32 => "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7",
        // Plain "Linux filesystem data" for both `system` slots and `state`.
        // The Discoverable Partitions Spec has a `root-arm64` GUID, and it is
        // deliberately not used: Rivet has no systemd to discover anything,
        // and an A/B *pair* both claiming to be the root partition is an
        // ambiguity invented for no gain. The GPT label says which is which.
        FsType::Erofs | FsType::Ext4 => "0FC63DAF-8483-4772-8E79-3D69D8477DE4",
    }
}

/// Why a filesystem is not byte-reproducible, or `None` when it is.
///
/// Every arm is `None` today, and the function stays because that is a
/// *measured* answer rather than a permanent property: each one depends on
/// flags in this module and on [`TOOL_ENV`], and this is where a future tool
/// version that stops cooperating gets recorded instead of quietly shipping.
pub const fn irreproducible_because(fs: FsType) -> Option<&'static str> {
    match fs {
        // FAT: --invariant, a fixed volume id, and mtimes staged at IMAGE_EPOCH.
        FsType::Fat32 => None,
        // erofs: -U, -T with --all-time, --all-root.
        FsType::Erofs => None,
        // ext4: -U, -E hash_seed, a sorted fixed-mtime tar, and — the one that
        // actually matters — SOURCE_DATE_EPOCH in TOOL_ENV.
        FsType::Ext4 => None,
    }
}

/// `ADR-018` §7's reserved-space guarantee, as a percentage of `/state`.
///
/// ext4 reserves these blocks for uid 0 alone. `ADR-018` §4 wants application
/// data never to make a machine unconfigurable, and `ADR-015` §5.1 runs
/// applications as their own UIDs — so root-only reserve *is* the guarantee.
/// It is stated rather than left to `mke2fs`'s default, because a default that
/// changes is a guarantee that changes without a commit.
pub const STATE_RESERVED_PERCENT: u32 = 5;

/// A `/system` slot reserves nothing.
///
/// It is mounted read-only (`layout::SYSTEM_FSTYPE` — the ext4 stopgap) and
/// nothing writes to it outside an update, so `mke2fs`'s 5% default would be
/// 32 MiB of a 640 MiB slot held back from a filesystem that never fills.
pub const IMAGE_RESERVED_PERCENT: u32 = 0;

/// The files `ADR-002` §4 requires on a boot partition.
///
/// **`tryboot.txt` is the one that costs a board.** `ADR-018` §5.3 recorded it
/// from hardware: in tryboot mode the firmware reads `tryboot.txt` *instead of*
/// `config.txt`, so a rollback with no `tryboot.txt` boots with none of
/// `config.txt`'s settings. `autoboot.txt` is what selects the slot at all
/// (`ADR-018` §5.1); without it the firmware always boots partition 1 and the B
/// slot is unreachable. Checked here because this module is the last gate
/// before the bytes are written, and because `AGENTS.md` §4 asks specifically
/// for the individually-correct-but-jointly-wrong case to be tested.
pub const REQUIRED_BOOT_FILES: &[&str] = &["autoboot.txt", "config.txt", "tryboot.txt"];

/// Every program that must be on `PATH`. Notably **not** `losetup`, `mount` or
/// `sudo`.
pub const REQUIRED_TOOLS: &[&str] = &[
    "sgdisk",
    "mkfs.vfat",
    "mcopy",
    "mkfs.erofs",
    "mkfs.ext4",
    // `tar` is not packaging: it is how a `/system` tree claims root ownership
    // from an unprivileged build. See the module documentation.
    "tar",
    "find",
    "touch",
];

// ---------------------------------------------------------------------------
// Errors — every one names what was wrong
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    DeviceTooSmall {
        device_mib: u64,
        minimum_mib: u64,
    },
    DeviceTooLarge {
        device_mib: u64,
    },
    TableNotUsable {
        reason: &'static str,
    },
    MissingTools {
        programs: Vec<String>,
    },
    NotARegularFile {
        path: String,
    },
    BadPath {
        path: String,
        reason: &'static str,
    },
    DuplicatePath {
        path: String,
    },
    /// A symlink was handed to a FAT partition, which cannot represent one.
    FatCannotHold {
        label: &'static str,
        path: String,
        kind: &'static str,
    },
    BootFileMissing {
        label: &'static str,
        filename: &'static str,
    },
    /// A built filesystem is larger than the partition it was destined for.
    PartitionOverflow {
        label: &'static str,
        needed_bytes: u64,
        capacity_bytes: u64,
    },
    ToolFailed {
        program: String,
        args: Vec<String>,
        code: Option<i32>,
        stderr: String,
    },
    Host {
        op: &'static str,
        path: String,
        message: String,
    },
}

impl core::fmt::Display for ImageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DeviceTooSmall {
                device_mib,
                minimum_mib,
            } => write!(
                f,
                "refusing a {device_mib} MiB device: ADR-018 §4's five partitions need at least \
                 {minimum_mib} MiB"
            ),
            Self::DeviceTooLarge { device_mib } => {
                write!(
                    f,
                    "refusing a {device_mib} MiB device: the size overflows a sector count"
                )
            }
            Self::TableNotUsable { reason } => {
                write!(f, "refusing to lay out the partition table: {reason}")
            }
            Self::MissingTools { programs } => write!(
                f,
                "refusing to build: not on PATH: {}. This is a host tool and drives them rather \
                 than reimplementing them (ADR-009 §3.4)",
                programs.join(", ")
            ),
            Self::NotARegularFile { path } => write!(
                f,
                "refusing to write {path}: it is not a regular file. This tool produces an image \
                 file; writing a card is `dd` or Raspberry Pi Imager, with a device path you typed"
            ),
            Self::BadPath { path, reason } => {
                write!(f, "refusing the path {path:?}: it has {reason}")
            }
            Self::DuplicatePath { path } => {
                write!(f, "refusing the tree: two entries claim the path {path:?}")
            }
            Self::FatCannotHold { label, path, kind } => write!(
                f,
                "refusing to put {path:?} on {label}: it is a {kind}, and FAT cannot hold one. \
                 A FAT partition is what the Pi firmware reads (ADR-018 §5)"
            ),
            Self::BootFileMissing { label, filename } => write!(
                f,
                "refusing to build {label} without {filename}: ADR-002 §4 requires it, and \
                 ADR-018 §5.3 recorded on hardware what its absence costs — a tryboot reboot \
                 that loses a headless board"
            ),
            Self::PartitionOverflow {
                label,
                needed_bytes,
                capacity_bytes,
            } => write!(
                f,
                "refusing to write {label}: the filesystem is {needed_bytes} bytes and the \
                 partition holds {capacity_bytes}. Writing it would overwrite the next partition"
            ),
            Self::ToolFailed {
                program,
                args,
                code,
                stderr,
            } => {
                let status = match code {
                    Some(c) => format!("exit status {c}"),
                    None => "killed by a signal".to_string(),
                };
                write!(
                    f,
                    "{program} failed ({status}): {}\n  invoked as: {program} {}",
                    if stderr.trim().is_empty() {
                        "(it said nothing on stderr)"
                    } else {
                        stderr.trim()
                    },
                    args.join(" ")
                )
            }
            Self::Host { op, path, message } => {
                write!(f, "could not {op} {path}: {message}")
            }
        }
    }
}

impl std::error::Error for ImageError {}

// ---------------------------------------------------------------------------
// The port
// ---------------------------------------------------------------------------

/// What a program said and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolRun {
    /// `None` when the program was killed by a signal.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl ToolRun {
    pub fn succeeded(&self) -> bool {
        self.code == Some(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    Missing,
    RegularFile,
    /// A directory, a block device, a symlink to either — anything this module
    /// declines to write an image into.
    Other,
}

/// The seam between image assembly's decisions and the build machine.
///
/// Everything above this line is a decision; everything below it is a process
/// or a file. [`CommandHost`] must stay thin and should get thinner rather than
/// smarter — whatever logic lives in it is logic no test can drive.
pub trait Host {
    /// Is `program` on `PATH`?
    fn have_tool(&mut self, program: &str) -> bool;

    /// Run `program` to completion with exactly `env` overlaid, capturing both
    /// streams. `Err` is "could not spawn it at all"; a non-zero exit is `Ok`
    /// and the caller's business.
    fn run(
        &mut self,
        program: &str,
        args: &[String],
        env: &[(&str, &str)],
    ) -> Result<ToolRun, ImageError>;

    fn path_kind(&mut self, path: &str) -> Result<PathKind, ImageError>;

    /// Create or truncate `path` to exactly `bytes`, sparse where the host
    /// allows it.
    fn create_sized(&mut self, path: &str, bytes: u64) -> Result<(), ImageError>;

    fn file_size(&mut self, path: &str) -> Result<u64, ImageError>;

    /// Materialise `tree` under `root`, applying each entry's mode.
    fn stage(&mut self, root: &str, tree: &Tree) -> Result<(), ImageError>;

    /// Copy every byte of `src` into `dest` starting at `offset`, leaving the
    /// rest of `dest` alone.
    fn splice(&mut self, dest: &str, offset: u64, src: &str) -> Result<(), ImageError>;

    /// A directory this build may write scratch files into.
    fn scratch_dir(&mut self) -> Result<String, ImageError>;

    fn remove_tree(&mut self, path: &str) -> Result<(), ImageError>;
}

// ---------------------------------------------------------------------------
// The argv, decided in pure code
// ---------------------------------------------------------------------------

fn s(v: &str) -> String {
    v.to_string()
}

/// One `sgdisk` invocation writes the whole table.
///
/// `--clear` first: `sgdisk` applies its options in order, so anything before
/// it would be discarded. Long option names throughout, because `-u` and `-U`
/// are *partition* GUID and *disk* GUID and a reader should not have to
/// remember which.
pub fn sgdisk_args(placed: &[Placed], image_path: &str) -> Vec<String> {
    let mut args = vec![s("--clear"), format!("--disk-guid={DISK_GUID}")];
    for p in placed {
        let n = p.spec.number;
        args.push(format!("--new={n}:{}:{}", p.first_lba, p.last_lba));
        args.push(format!("--typecode={n}:{}", type_guid(p.spec.fstype)));
        args.push(format!("--change-name={n}:{}", p.spec.label));
        args.push(format!("--partition-guid={n}:{}", partition_guid(n)));
    }
    args.push(s(image_path));
    args
}

/// FAT32 into the middle of the image, no loop device.
///
/// `--offset` counts 512-byte sectors and the trailing size counts 1024-byte
/// blocks; they are different units in the same command line, which is why both
/// are computed here where a test can see them.
///
/// ## Why the cluster size is forced to four sectors
///
/// **`-F 32` is not sufficient to produce a FAT32 a conforming reader will
/// treat as FAT32,** and a Pi 4 proved it on 2026-08-23. Microsoft's FAT
/// specification does not identify the type from the BPB; it identifies it from
/// the *cluster count*: under 4085 is FAT12, under **65525** is FAT16, and only
/// at or above 65525 is a volume FAT32. The rule is stated as the one and only
/// correct way to determine the type, and the Pi's bootloader implements it.
///
/// `mkfs.fat` obeys `-F 32` regardless. On a 256 MiB partition it chose eight
/// sectors per cluster, which left **65403** clusters — 122 short. The result
/// is a volume whose boot sector says `FAT32` while its cluster count says
/// FAT16, and the bootloader printed exactly that: `type: 16`. It then read the
/// FAT16 fields of a FAT32 boot sector, where `sectors per fat` and the root
/// directory slot count are both zero by definition, and reported
/// `Firmware not found` for a `start4.elf` that was present the whole time.
///
/// Four sectors per cluster puts the count near 130 000, clear of the boundary
/// by a factor of two, and no boot partition this layout allows can fall back
/// under it. Larger clusters are the trap, not smaller ones.
pub fn mkfs_vfat_args(p: &Placed, image_path: &str) -> Vec<String> {
    vec![
        s("-F"),
        s("32"),
        // Cluster count, not the BPB, is what decides the FAT type. See above.
        s("-s"),
        s("4"),
        // Without this, mkfs.fat stamps a volume id from the clock.
        s("--invariant"),
        s("-n"),
        p.spec.label.to_uppercase(),
        s("-i"),
        fat_volume_id(p.spec.number),
        format!("--offset={}", p.first_lba),
        s(image_path),
        (p.sectors() / 2).to_string(),
    ]
}

/// `mcopy` reaches a FAT inside a larger file through mtools' `image@@offset`
/// syntax — no loop mount and no privilege.
///
/// The sources are the tree's top-level names in sorted order, each as a path
/// under the staging directory. Sorted because FAT records directory entries in
/// the order they are written.
pub fn mcopy_args(p: &Placed, image_path: &str, stage_dir: &str, tree: &Tree) -> Vec<String> {
    let mut args = vec![
        s("-i"),
        format!("{image_path}@@{}", p.offset_bytes()),
        s("-s"), // recurse
        s("-p"), // preserve attributes
        s("-m"), // preserve mtime — with IMAGE_EPOCH staged, this is determinism
        s("-Q"), // stop at the first failure instead of continuing
        s("-o"), // overwrite without prompting; the filesystem is new anyway
    ];
    for name in tree.top_level() {
        args.push(format!("{stage_dir}/{name}"));
    }
    args.push(s("::"));
    args
}

/// erofs is built to its own file first, because **its size is decided by its
/// contents**, unlike the other two which are told how big to be. It has to be
/// measured against the partition before it can be placed.
pub fn mkfs_erofs_args(p: &Placed, out_path: &str, stage_dir: &str) -> Vec<String> {
    vec![
        s("--quiet"),
        s("-U"),
        filesystem_uuid(p.spec.number),
        s("-L"),
        p.spec.label.to_string(),
        s("-T"),
        IMAGE_EPOCH.to_string(),
        // The timestamp reaches every file, not just the superblock.
        s("--all-time"),
        // The build runs unprivileged, so without this every inode carries the
        // builder's uid. Verified with `dump.erofs`: 1000 without it, 0 with.
        s("--all-root"),
        s(out_path),
        s(stage_dir),
    ]
}

/// The `ext4` block size, forced rather than left to `mke2fs`.
///
/// # `ADR-018` §10.2 changed this underneath us, and it took a loop device to notice
///
/// `mke2fs.conf` picks a *usage type* from the size of the filesystem it is
/// making, and its `small` type — anything under 512 MB — carries
/// `blocksize = 1024`. §10.2 shrank the image to the layout's minimum, which
/// makes `/state` **254 MiB at build time**, so every card built since has had
/// a 1 KiB-block `/state`. Nothing asked for that and nothing announced it.
///
/// **It matters because block size fixes how far the filesystem can ever be
/// grown.** `mke2fs` reserves group-descriptor blocks for future growth, and
/// what they buy depends on the block size. Measured on `e2fsprogs 1.47.4`,
/// same 254 MiB, on 2026-08-24:
///
/// | `-b` | reserved GDT blocks | ceiling an online resize can reach |
/// | --: | --: | --: |
/// | 1024 | 256 | **~33 GiB** |
/// | 4096 | 31 | **~256 GiB** |
///
/// A 254 MiB `/state` that can never exceed 33 GiB is a silent cap on a
/// property `ADR-018` §4 is explicitly about — *"a partition solves it by
/// fixing a size that has to be guessed years in advance, and the guess will be
/// wrong"* — reintroduced by the size of the **image** rather than the size of
/// the card. `layout.rs` names the 128 GB working card; that card is inside the
/// 4 KiB ceiling and outside the 1 KiB one.
///
/// So the block size is stated here instead of being inherited from a heuristic
/// about a size that is now an artefact of how the image is distributed. It
/// changes nothing for the 640 MiB `/system` slots, which were already over
/// `mke2fs`'s threshold and already getting 4096.
pub const EXT4_BLOCK_BYTES: u32 = 4096;

/// ext4 in place, at an offset, sized in 512-byte sectors.
///
/// `tar_path` is `Some` for a partition with content — a **tarball**, never a
/// directory, because `mke2fs -d` copies a directory's ownership and this build
/// is not root. `None` produces an empty filesystem, which is what `/state`
/// gets.
pub fn mkfs_ext4_args(p: &Placed, image_path: &str, tar_path: Option<&str>) -> Vec<String> {
    let reserve = if p.spec.number == crate::layout::STATE_PARTITION {
        STATE_RESERVED_PERCENT
    } else {
        IMAGE_RESERVED_PERCENT
    };
    let mut args = vec![
        s("-F"),
        s("-q"),
        s("-t"),
        s("ext4"),
        // Not mke2fs.conf's guess from the size of the partition. See
        // `EXT4_BLOCK_BYTES` — it decides how far `ADR-018` §10.3 can ever grow
        // this filesystem, and the image being small made the guess wrong.
        s("-b"),
        EXT4_BLOCK_BYTES.to_string(),
        s("-L"),
        p.spec.label.to_string(),
        s("-U"),
        filesystem_uuid(p.spec.number),
        s("-m"),
        reserve.to_string(),
        s("-E"),
        format!(
            "offset={},root_owner=0:0,hash_seed={EXT4_HASH_SEED}",
            p.offset_bytes()
        ),
    ];
    if let Some(tar) = tar_path {
        args.push(s("-d"));
        args.push(s(tar));
    }
    args.push(s(image_path));
    args.push(format!("{}s", p.sectors()));
    args
}

/// Pack a staged tree into a tar that claims root ownership and a fixed time.
///
/// Every flag here is load-bearing:
///
/// * `--owner=0 --group=0 --numeric-owner` — the ownership `mke2fs -d` will
///   record. This is the whole reason a tar is involved.
/// * `--sort=name` — otherwise the member order is `readdir`'s, which is the
///   filesystem's, which is not the same twice.
/// * `--mtime` — otherwise every member carries the staging time.
/// * `--format=gnu` — stated rather than inherited, because the default format
///   is a build-time option of `tar` and a changed default would change the
///   bytes.
pub fn tar_args(out_tar: &str, stage_dir: &str) -> Vec<String> {
    vec![
        s("--format=gnu"),
        s("--owner=0"),
        s("--group=0"),
        s("--numeric-owner"),
        s("--sort=name"),
        format!("--mtime=@{IMAGE_EPOCH}"),
        s("-C"),
        s(stage_dir),
        s("-cf"),
        s(out_tar),
        s("."),
    ]
}

/// Refuse a filesystem that would not fit the partition it is destined for.
///
/// Only [`FsType::Erofs`] can reach this: its size is decided by its contents,
/// so it is built to its own file and measured. The other two are told how big
/// to be, and a `mkfs` that cannot fit the content into that size fails on its
/// own and says so.
pub fn check_fits(
    label: &'static str,
    needed_bytes: u64,
    capacity_bytes: u64,
) -> Result<(), ImageError> {
    if needed_bytes > capacity_bytes {
        return Err(ImageError::PartitionOverflow {
            label,
            needed_bytes,
            capacity_bytes,
        });
    }
    Ok(())
}

/// Stamp every staged path with [`IMAGE_EPOCH`].
///
/// A tool rather than a syscall on purpose: setting an mtime from Rust means
/// either a new dependency or an `unsafe` call to `utimensat(2)`, and this
/// crate's `Cargo.toml` is a shared file while `coreutils` is already a
/// requirement of the build host. `-h` so a symlink's own mtime is set rather
/// than its target's.
pub fn touch_args(stage_dir: &str) -> Vec<String> {
    vec![
        s(stage_dir),
        s("-exec"),
        s("touch"),
        s("-h"),
        s("-d"),
        format!("@{IMAGE_EPOCH}"),
        s("{}"),
        s("+"),
    ]
}

// ---------------------------------------------------------------------------
// The build
// ---------------------------------------------------------------------------

/// What to build, and what goes on it.
#[derive(Debug, Clone)]
pub struct BuildRequest<'a> {
    pub image_path: &'a str,
    pub device_mib: u64,
    /// Keyed by partition number, so a caller cannot transpose the A and B
    /// slots by getting the order of four arguments wrong. `ADR-018` §5.3's
    /// `[tryboot]` override makes those numbers load-bearing.
    pub trees: BTreeMap<u32, Tree>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionReport {
    pub number: u32,
    pub label: &'static str,
    pub fstype: FsType,
    pub first_lba: u64,
    pub last_lba: u64,
    pub offset_bytes: u64,
    pub len_bytes: u64,
    /// For erofs, the size of the filesystem actually written; `None` where the
    /// filesystem fills the partition by construction.
    pub used_bytes: Option<u64>,
    /// `None` when the region is byte-reproducible; otherwise why it is not.
    pub irreproducible_because: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildReport {
    pub image_path: String,
    pub device_mib: u64,
    pub partitions: Vec<PartitionReport>,
}

impl BuildReport {
    pub fn fully_reproducible(&self) -> bool {
        self.partitions
            .iter()
            .all(|p| p.irreproducible_because.is_none())
    }
}

/// Build the image.
///
/// Everything that can be refused is refused **before the image file is
/// created**, so a rejected build leaves nothing behind to be mistaken for a
/// half-written card.
pub fn build(host: &mut dyn Host, req: &BuildRequest<'_>) -> Result<BuildReport, ImageError> {
    let placed = plan(req.device_mib)?;
    let empty = Tree::default();

    // --- refusals, all of them, before any byte is written -----------------
    for p in &placed {
        let tree = req.trees.get(&p.spec.number).unwrap_or(&empty);
        match p.spec.fstype {
            FsType::Fat32 => {
                if let Some(bad) = tree.first_symlink() {
                    return Err(ImageError::FatCannotHold {
                        label: p.spec.label,
                        path: bad.path().to_string(),
                        kind: bad.kind(),
                    });
                }
                for required in REQUIRED_BOOT_FILES {
                    if !tree.has(required) {
                        return Err(ImageError::BootFileMissing {
                            label: p.spec.label,
                            filename: required,
                        });
                    }
                }
            }
            FsType::Erofs | FsType::Ext4 => {}
        }
    }

    let missing: Vec<String> = REQUIRED_TOOLS
        .iter()
        .filter(|t| !host.have_tool(t))
        .map(|t| s(t))
        .collect();
    if !missing.is_empty() {
        return Err(ImageError::MissingTools { programs: missing });
    }

    if host.path_kind(req.image_path)? == PathKind::Other {
        return Err(ImageError::NotARegularFile {
            path: s(req.image_path),
        });
    }

    // --- the image ---------------------------------------------------------
    host.create_sized(req.image_path, req.device_mib * 1024 * 1024)?;
    run_checked(host, "sgdisk", &sgdisk_args(&placed, req.image_path))?;

    let scratch = host.scratch_dir()?;
    let mut reports = Vec::with_capacity(placed.len());

    for p in &placed {
        let tree = req.trees.get(&p.spec.number).unwrap_or(&empty);
        let stage_dir = format!("{scratch}/p{}", p.spec.number);
        let mut used_bytes = None;

        match p.spec.fstype {
            FsType::Fat32 => {
                run_checked(host, "mkfs.vfat", &mkfs_vfat_args(p, req.image_path))?;
                if !tree.is_empty() {
                    host.stage(&stage_dir, tree)?;
                    run_checked(host, "find", &touch_args(&stage_dir))?;
                    run_checked(
                        host,
                        "mcopy",
                        &mcopy_args(p, req.image_path, &stage_dir, tree),
                    )?;
                }
            }
            FsType::Erofs => {
                // Staged even when empty: mkfs.erofs needs a source directory,
                // and an empty `/system` slot is a legitimate B slot on a first
                // card.
                host.stage(&stage_dir, tree)?;
                let out = format!("{scratch}/p{}.erofs", p.spec.number);
                run_checked(host, "mkfs.erofs", &mkfs_erofs_args(p, &out, &stage_dir))?;
                let size = host.file_size(&out)?;
                check_fits(p.spec.label, size, p.len_bytes())?;
                host.splice(req.image_path, p.offset_bytes(), &out)?;
                used_bytes = Some(size);
            }
            FsType::Ext4 => {
                // A tree means an image partition — a `/system` slot under the
                // `layout::SYSTEM_FSTYPE` stopgap. No tree means `/state`,
                // which ships empty.
                let tar = if tree.is_empty() {
                    None
                } else {
                    host.stage(&stage_dir, tree)?;
                    let tar = format!("{scratch}/p{}.tar", p.spec.number);
                    run_checked(host, "tar", &tar_args(&tar, &stage_dir))?;
                    Some(tar)
                };
                run_checked(
                    host,
                    "mkfs.ext4",
                    &mkfs_ext4_args(p, req.image_path, tar.as_deref()),
                )?;
            }
        }

        reports.push(PartitionReport {
            number: p.spec.number,
            label: p.spec.label,
            fstype: p.spec.fstype,
            first_lba: p.first_lba,
            last_lba: p.last_lba,
            offset_bytes: p.offset_bytes(),
            len_bytes: p.len_bytes(),
            used_bytes,
            irreproducible_because: irreproducible_because(p.spec.fstype),
        });
    }

    // Left in place on failure, deliberately: a scratch tree is the only thing
    // that says what was handed to a `mkfs` that rejected it.
    host.remove_tree(&scratch)?;

    Ok(BuildReport {
        image_path: s(req.image_path),
        device_mib: req.device_mib,
        partitions: reports,
    })
}

/// Run a tool and turn a non-zero exit into an error **carrying its stderr**.
/// "mkfs failed" without the message is useless at 2am.
fn run_checked(host: &mut dyn Host, program: &str, args: &[String]) -> Result<ToolRun, ImageError> {
    let run = host.run(program, args, TOOL_ENV)?;
    if !run.succeeded() {
        return Err(ImageError::ToolFailed {
            program: s(program),
            args: args.to_vec(),
            code: run.code,
            stderr: run.stderr,
        });
    }
    Ok(run)
}

// ---------------------------------------------------------------------------
// The adapter. Thin on purpose.
// ---------------------------------------------------------------------------

/// The real [`Host`]: processes and files on the build machine.
#[derive(Debug, Default)]
pub struct CommandHost {
    _private: (),
}

/// Process-wide, not per-[`CommandHost`].
///
/// It was per-instance first, and two builds in one process then chose the same
/// scratch directory and the second one tripped over the first one's symlinks.
/// The counter has to be unique across every `CommandHost` a process makes,
/// which is what "process-wide" means and what a field cannot be.
static SCRATCH_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl CommandHost {
    pub fn new() -> Self {
        Self::default()
    }
}

fn host_err<'a>(op: &'static str, path: &'a str) -> impl FnOnce(std::io::Error) -> ImageError + 'a {
    move |e| ImageError::Host {
        op,
        path: s(path),
        message: e.to_string(),
    }
}

impl Host for CommandHost {
    fn have_tool(&mut self, program: &str) -> bool {
        // `Command::new(p).spawn()` is the only portable "is it on PATH" that
        // does not reimplement PATH lookup. `--version` is chosen because every
        // tool in REQUIRED_TOOLS accepts it or ignores it harmlessly.
        std::process::Command::new(program)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
    }

    fn run(
        &mut self,
        program: &str,
        args: &[String],
        env: &[(&str, &str)],
    ) -> Result<ToolRun, ImageError> {
        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().map_err(|e| ImageError::Host {
            op: "run",
            path: s(program),
            message: e.to_string(),
        })?;
        Ok(ToolRun {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    fn path_kind(&mut self, path: &str) -> Result<PathKind, ImageError> {
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_file() => Ok(PathKind::RegularFile),
            Ok(_) => Ok(PathKind::Other),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(PathKind::Missing),
            Err(e) => Err(host_err("inspect", path)(e)),
        }
    }

    fn create_sized(&mut self, path: &str, bytes: u64) -> Result<(), ImageError> {
        let f = std::fs::File::create(path).map_err(host_err("create", path))?;
        f.set_len(bytes).map_err(host_err("size", path))
    }

    fn file_size(&mut self, path: &str) -> Result<u64, ImageError> {
        std::fs::metadata(path)
            .map(|m| m.len())
            .map_err(host_err("measure", path))
    }

    fn stage(&mut self, root: &str, tree: &Tree) -> Result<(), ImageError> {
        std::fs::create_dir_all(root).map_err(host_err("create", root))?;
        for entry in tree.entries() {
            let full = format!("{root}/{}", entry.path());
            match entry {
                Entry::Dir { mode, .. } => {
                    std::fs::create_dir_all(&full).map_err(host_err("create", &full))?;
                    set_mode(&full, *mode)?;
                }
                Entry::File { mode, contents, .. } => {
                    if let Some(parent) = std::path::Path::new(&full).parent() {
                        std::fs::create_dir_all(parent).map_err(host_err("create", &full))?;
                    }
                    std::fs::write(&full, contents).map_err(host_err("write", &full))?;
                    set_mode(&full, *mode)?;
                }
                Entry::Symlink { target, .. } => {
                    if let Some(parent) = std::path::Path::new(&full).parent() {
                        std::fs::create_dir_all(parent).map_err(host_err("create", &full))?;
                    }
                    symlink(target, &full)?;
                }
            }
        }
        Ok(())
    }

    fn splice(&mut self, dest: &str, offset: u64, src: &str) -> Result<(), ImageError> {
        use std::io::{Read, Seek, SeekFrom, Write};
        let mut input = std::fs::File::open(src).map_err(host_err("open", src))?;
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .open(dest)
            .map_err(host_err("open", dest))?;
        output
            .seek(SeekFrom::Start(offset))
            .map_err(host_err("seek in", dest))?;
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            let n = input.read(&mut buf).map_err(host_err("read", src))?;
            if n == 0 {
                break;
            }
            output
                .write_all(&buf[..n])
                .map_err(host_err("write", dest))?;
        }
        output.flush().map_err(host_err("flush", dest))
    }

    fn scratch_dir(&mut self) -> Result<String, ImageError> {
        let seq = SCRATCH_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("rivet-image-{}-{seq}", std::process::id()));
        let path = path.to_string_lossy().into_owned();
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).map_err(host_err("create", &path))?;
        Ok(path)
    }

    fn remove_tree(&mut self, path: &str) -> Result<(), ImageError> {
        match std::fs::remove_dir_all(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(host_err("remove", path)(e)),
        }
    }
}

#[cfg(unix)]
fn set_mode(path: &str, mode: u32) -> Result<(), ImageError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(host_err("set the mode of", path))
}

/// `AGENTS.md` §3: a stub refuses, it never fakes success. A mode that was not
/// applied is a `/system` whose binaries may not be executable.
#[cfg(not(unix))]
fn set_mode(path: &str, mode: u32) -> Result<(), ImageError> {
    Err(ImageError::Host {
        op: "set the mode of",
        path: s(path),
        message: format!(
            "this build host cannot express a Unix mode ({mode:o}); build the image on Linux"
        ),
    })
}

#[cfg(unix)]
fn symlink(target: &str, at: &str) -> Result<(), ImageError> {
    std::os::unix::fs::symlink(target, at).map_err(host_err("create the symlink", at))
}

#[cfg(not(unix))]
fn symlink(target: &str, at: &str) -> Result<(), ImageError> {
    Err(ImageError::Host {
        op: "create the symlink",
        path: s(at),
        message: format!("this build host cannot create a symlink to {target}; build on Linux"),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{BOOT_SIZE_MIB, STATE_PARTITION, SYSTEM_FSTYPE, SYSTEM_SIZE_MIB, Slot};

    /// A name in `ADR-027` §3's shape, so the fixtures model the artifact this
    /// project actually distributes rather than a bare `rivet.img`.
    ///
    /// It is a *literal* and not a composition on purpose: `ADR-027` §7 rules
    /// that the tool must compose the name, and has not been implemented — the
    /// caller still supplies the whole path. Composing it here would make the
    /// suite look as though the mechanism exists.
    const IMAGE_PATH: &str = "/out/rivet-0.0.1-aarch64-rpi4-card.img";

    /// `layout::SYSTEM_FSTYPE` is a stopgap that flips back to erofs when
    /// `ADR-024` §8's kernel ships. Tests that depend on which one it is ask
    /// here rather than naming a filesystem, so the flip changes one line and
    /// does not silently pass a suite that stopped checking anything.
    fn system_mkfs() -> String {
        format!("mkfs.{}", SYSTEM_FSTYPE.as_str())
    }

    fn system_is_erofs() -> bool {
        matches!(SYSTEM_FSTYPE, FsType::Erofs)
    }

    // -- the fake -----------------------------------------------------------

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Invocation {
        program: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
    }

    /// A [`Host`] that answers however a test needs it to.
    ///
    /// Defaults are the boring happy path — every tool is present and succeeds,
    /// the image path does not exist yet — so a test only says what is
    /// *unusual* about the world it describes.
    struct FakeHost {
        runs: Vec<Invocation>,
        created: Vec<(String, u64)>,
        staged: Vec<(String, Tree)>,
        spliced: Vec<(String, u64, String)>,
        removed: Vec<String>,
        missing_tools: BTreeSet<String>,
        /// program name -> the failure it answers with.
        fail: BTreeMap<String, ToolRun>,
        /// path -> size, for `file_size`. Absent means 4096.
        sizes: BTreeMap<String, u64>,
        image_kind: PathKind,
        scratch_calls: u32,
    }

    impl FakeHost {
        fn new() -> Self {
            Self {
                runs: Vec::new(),
                created: Vec::new(),
                staged: Vec::new(),
                spliced: Vec::new(),
                removed: Vec::new(),
                missing_tools: BTreeSet::new(),
                fail: BTreeMap::new(),
                sizes: BTreeMap::new(),
                image_kind: PathKind::Missing,
                scratch_calls: 0,
            }
        }

        fn programs(&self) -> Vec<String> {
            self.runs.iter().map(|r| r.program.clone()).collect()
        }

        fn args_for(&self, program: &str) -> Vec<Vec<String>> {
            self.runs
                .iter()
                .filter(|r| r.program == program)
                .map(|r| r.args.clone())
                .collect()
        }
    }

    impl Host for FakeHost {
        fn have_tool(&mut self, program: &str) -> bool {
            !self.missing_tools.contains(program)
        }

        fn run(
            &mut self,
            program: &str,
            args: &[String],
            env: &[(&str, &str)],
        ) -> Result<ToolRun, ImageError> {
            self.runs.push(Invocation {
                program: s(program),
                args: args.to_vec(),
                env: env.iter().map(|(k, v)| (s(k), s(v))).collect(),
            });
            Ok(self.fail.get(program).cloned().unwrap_or(ToolRun {
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            }))
        }

        fn path_kind(&mut self, _path: &str) -> Result<PathKind, ImageError> {
            Ok(self.image_kind)
        }

        fn create_sized(&mut self, path: &str, bytes: u64) -> Result<(), ImageError> {
            self.created.push((s(path), bytes));
            Ok(())
        }

        fn file_size(&mut self, path: &str) -> Result<u64, ImageError> {
            Ok(self.sizes.get(path).copied().unwrap_or(4096))
        }

        fn stage(&mut self, root: &str, tree: &Tree) -> Result<(), ImageError> {
            self.staged.push((s(root), tree.clone()));
            Ok(())
        }

        fn splice(&mut self, dest: &str, offset: u64, src: &str) -> Result<(), ImageError> {
            self.spliced.push((s(dest), offset, s(src)));
            Ok(())
        }

        fn scratch_dir(&mut self) -> Result<String, ImageError> {
            self.scratch_calls += 1;
            Ok(s("/scratch"))
        }

        fn remove_tree(&mut self, path: &str) -> Result<(), ImageError> {
            self.removed.push(s(path));
            Ok(())
        }
    }

    // -- fixtures -----------------------------------------------------------

    fn file(path: &str) -> Entry {
        Entry::File {
            path: s(path),
            mode: 0o644,
            contents: b"x".to_vec(),
        }
    }

    /// A boot tree with everything `ADR-002` §4 requires.
    fn boot_tree() -> Tree {
        Tree::new(vec![
            file("config.txt"),
            file("tryboot.txt"),
            file("autoboot.txt"),
            file("cmdline.txt"),
            file("kernel8.img"),
        ])
        .unwrap_or_default()
    }

    fn system_tree() -> Tree {
        Tree::new(vec![
            Entry::Dir {
                path: s("ext"),
                mode: 0o755,
            },
            Entry::File {
                path: s("bin/rivet-initd"),
                mode: 0o755,
                contents: b"elf".to_vec(),
            },
        ])
        .unwrap_or_default()
    }

    fn request() -> BuildRequest<'static> {
        let mut trees = BTreeMap::new();
        trees.insert(Slot::A.boot_partition(), boot_tree());
        trees.insert(Slot::B.boot_partition(), boot_tree());
        trees.insert(Slot::A.system_partition(), system_tree());
        trees.insert(Slot::B.system_partition(), system_tree());
        BuildRequest {
            image_path: IMAGE_PATH,
            device_mib: minimum_device_mib(),
            trees,
        }
    }

    fn placed_at(number: u32) -> Placed {
        let all = plan(minimum_device_mib()).unwrap_or_default();
        let found = all.into_iter().find(|p| p.spec.number == number);
        assert!(found.is_some(), "no partition {number} in the plan");
        found.unwrap_or(Placed {
            spec: PartitionSpec {
                number,
                label: "missing",
                fstype: FsType::Ext4,
                size_mib: None,
            },
            first_lba: 0,
            last_lba: 0,
        })
    }

    // -- paths --------------------------------------------------------------

    #[test]
    fn an_absolute_path_is_refused_because_a_tree_is_rooted_at_its_own_filesystem() {
        assert!(matches!(
            Tree::new(vec![file("/config.txt")]),
            Err(ImageError::BadPath { .. })
        ));
    }

    #[test]
    fn a_dot_dot_component_is_refused_because_it_can_escape_the_tree() {
        assert!(matches!(
            Tree::new(vec![file("overlays/../../etc/passwd")]),
            Err(ImageError::BadPath { .. })
        ));
    }

    #[test]
    fn a_single_dot_component_is_refused() {
        assert!(matches!(
            Tree::new(vec![file("a/./b")]),
            Err(ImageError::BadPath { .. })
        ));
    }

    #[test]
    fn an_empty_path_is_refused() {
        assert!(matches!(
            Tree::new(vec![file("")]),
            Err(ImageError::BadPath { .. })
        ));
    }

    #[test]
    fn a_doubled_separator_is_refused_rather_than_quietly_collapsed() {
        assert!(matches!(
            Tree::new(vec![file("bin//init")]),
            Err(ImageError::BadPath { .. })
        ));
    }

    #[test]
    fn a_backslash_is_refused_because_the_build_host_is_windows_and_the_target_is_not() {
        assert!(matches!(
            Tree::new(vec![file("overlays\\note.txt")]),
            Err(ImageError::BadPath { .. })
        ));
    }

    #[test]
    fn a_nul_byte_in_a_path_is_refused() {
        assert!(matches!(
            Tree::new(vec![file("config\0.txt")]),
            Err(ImageError::BadPath { .. })
        ));
    }

    #[test]
    fn a_control_character_in_a_path_is_refused() {
        assert!(matches!(
            Tree::new(vec![file("config\n.txt")]),
            Err(ImageError::BadPath { .. })
        ));
    }

    #[test]
    fn a_path_component_over_255_bytes_is_refused() {
        let long = "a".repeat(256);
        assert!(matches!(
            Tree::new(vec![file(&long)]),
            Err(ImageError::BadPath { .. })
        ));
    }

    #[test]
    fn a_255_byte_component_is_accepted_because_the_limit_is_the_boundary_not_a_margin() {
        let ok = "a".repeat(255);
        assert!(Tree::new(vec![file(&ok)]).is_ok());
    }

    #[test]
    fn two_entries_claiming_one_path_are_refused_rather_than_one_silently_winning() {
        assert!(matches!(
            Tree::new(vec![file("config.txt"), file("config.txt")]),
            Err(ImageError::DuplicatePath { .. })
        ));
    }

    #[test]
    fn a_symlink_with_an_empty_target_is_refused() {
        assert!(matches!(
            Tree::new(vec![Entry::Symlink {
                path: s("bin/sh"),
                target: String::new(),
            }]),
            Err(ImageError::BadPath { .. })
        ));
    }

    #[test]
    fn a_tree_sorts_its_entries_so_staging_order_does_not_depend_on_the_caller()
    -> Result<(), String> {
        let Ok(t) = Tree::new(vec![file("z"), file("a"), file("m")]) else {
            return Err("a valid tree was refused".to_string());
        };
        let paths: Vec<&str> = t.entries().iter().map(|e| e.path()).collect();
        assert_eq!(paths, ["a", "m", "z"]);
        Ok(())
    }

    #[test]
    fn top_level_is_deduplicated_and_sorted_because_fat_records_the_write_order()
    -> Result<(), String> {
        let Ok(t) = Tree::new(vec![
            file("overlays/b.dtbo"),
            file("overlays/a.dtbo"),
            file("config.txt"),
        ]) else {
            return Err("a valid tree was refused".to_string());
        };
        assert_eq!(t.top_level(), ["config.txt", "overlays"]);
        Ok(())
    }

    // -- geometry -----------------------------------------------------------

    #[test]
    fn a_device_below_the_minimum_is_refused_and_the_error_names_both_numbers() -> Result<(), String>
    {
        let Err(e) = plan(minimum_device_mib() - 1) else {
            return Err("an undersized device was accepted".to_string());
        };
        assert!(matches!(e, ImageError::DeviceTooSmall { .. }));
        let text = e.to_string();
        assert!(text.contains(&minimum_device_mib().to_string()), "{text}");
        assert!(
            text.contains(&(minimum_device_mib() - 1).to_string()),
            "{text}"
        );
        Ok(())
    }

    #[test]
    fn a_device_at_exactly_the_minimum_is_accepted() {
        assert!(plan(minimum_device_mib()).is_ok());
    }

    #[test]
    fn a_zero_sized_device_is_refused() {
        assert!(matches!(plan(0), Err(ImageError::DeviceTooSmall { .. })));
    }

    #[test]
    fn an_absurd_device_size_is_refused_rather_than_overflowing_a_sector_count() {
        assert!(matches!(
            plan(u64::MAX),
            Err(ImageError::DeviceTooLarge { .. })
        ));
    }

    #[test]
    fn the_first_partition_starts_at_the_first_usable_sector() -> Result<(), String> {
        let Ok(p) = plan(minimum_device_mib()) else {
            return Err("the minimum device was refused".to_string());
        };
        assert_eq!(p[0].first_lba, FIRST_USABLE_SECTOR);
        Ok(())
    }

    #[test]
    fn every_partition_starts_on_a_one_mib_boundary() -> Result<(), String> {
        let Ok(p) = plan(4096) else {
            return Err("a 4 GiB device was refused".to_string());
        };
        for part in &p {
            assert_eq!(
                part.first_lba % ALIGN_SECTORS,
                0,
                "partition {} starts at {}",
                part.spec.number,
                part.first_lba
            );
        }
        Ok(())
    }

    #[test]
    fn partitions_are_laid_out_end_to_end_with_no_gap_and_no_overlap() -> Result<(), String> {
        let Ok(p) = plan(4096) else {
            return Err("a 4 GiB device was refused".to_string());
        };
        for pair in p.windows(2) {
            assert_eq!(pair[1].first_lba, pair[0].last_lba + 1);
        }
        Ok(())
    }

    #[test]
    fn the_sized_partitions_get_exactly_the_size_layout_asked_for() -> Result<(), String> {
        let Ok(p) = plan(4096) else {
            return Err("a 4 GiB device was refused".to_string());
        };
        assert_eq!(p[0].len_bytes(), BOOT_SIZE_MIB * 1024 * 1024);
        assert_eq!(p[1].len_bytes(), BOOT_SIZE_MIB * 1024 * 1024);
        assert_eq!(p[2].len_bytes(), SYSTEM_SIZE_MIB * 1024 * 1024);
        assert_eq!(p[3].len_bytes(), SYSTEM_SIZE_MIB * 1024 * 1024);
        Ok(())
    }

    #[test]
    fn the_last_partition_stops_clear_of_the_secondary_gpt() -> Result<(), String> {
        let device_mib = 4096;
        let Ok(p) = plan(device_mib) else {
            return Err("a 4 GiB device was refused".to_string());
        };
        let total_sectors = device_mib * 1024 * 1024 / SECTOR_BYTES;
        let last = &p[p.len() - 1];
        // **Not** written in terms of GPT_TAIL_SECTORS. It was, and setting
        // that constant to 0 then moved the goalpost and no test failed —
        // exactly the assertion that cannot fail which `AGENTS.md` §4 warns
        // about. The number is fixed by the format: the secondary GPT is 32
        // sectors of partition entries plus a header at the very last LBA, so
        // the last usable sector is 34 below the end.
        assert!(
            last.last_lba <= total_sectors - 34,
            "last_lba {} runs into the secondary GPT",
            last.last_lba
        );
        assert_eq!(
            GPT_TAIL_SECTORS, 34,
            "the reservation no longer matches GPT"
        );
        Ok(())
    }

    #[test]
    fn the_last_partition_ends_on_an_alignment_boundary_too() -> Result<(), String> {
        let Ok(p) = plan(4096) else {
            return Err("a 4 GiB device was refused".to_string());
        };
        let last = &p[p.len() - 1];
        assert_eq!((last.last_lba + 1) % ALIGN_SECTORS, 0);
        Ok(())
    }

    #[test]
    fn state_grows_with_the_device_and_nothing_else_does() -> Result<(), String> {
        let Ok(small) = plan(2048) else {
            return Err("2048 MiB refused".to_string());
        };
        let Ok(big) = plan(8192) else {
            return Err("8192 MiB refused".to_string());
        };
        for i in 0..4 {
            assert_eq!(small[i].len_bytes(), big[i].len_bytes());
        }
        assert!(big[4].len_bytes() > small[4].len_bytes());
        assert_eq!(big[4].spec.number, STATE_PARTITION);
        Ok(())
    }

    #[test]
    fn a_partitions_offset_and_length_are_its_sectors_in_bytes() {
        let p = placed_at(1);
        assert_eq!(p.offset_bytes(), p.first_lba * SECTOR_BYTES);
        assert_eq!(p.len_bytes(), p.sectors() * SECTOR_BYTES);
        assert_eq!(p.sectors(), p.last_lba - p.first_lba + 1);
    }

    #[test]
    fn the_plan_keeps_adr_018_4s_order_and_numbering() -> Result<(), String> {
        let Ok(p) = plan(4096) else {
            return Err("a 4 GiB device was refused".to_string());
        };
        let got: Vec<(u32, &str)> = p.iter().map(|p| (p.spec.number, p.spec.label)).collect();
        assert_eq!(
            got,
            [
                (1, "boot-a"),
                (2, "boot-b"),
                (3, "system-a"),
                (4, "system-b"),
                (5, "state"),
            ]
        );
        Ok(())
    }

    // -- identity -----------------------------------------------------------

    #[test]
    fn a_partition_guid_is_fixed_and_distinct_per_partition() {
        let all: BTreeSet<String> = (1..=5).map(partition_guid).collect();
        assert_eq!(all.len(), 5);
        assert_eq!(partition_guid(1), partition_guid(1));
        assert_eq!(partition_guid(5), "FEEDFACE-0000-4000-8000-000000000005");
    }

    #[test]
    fn a_filesystem_uuid_is_a_different_namespace_from_a_partition_guid() {
        for n in 1..=5 {
            assert_ne!(filesystem_uuid(n), partition_guid(n));
        }
        assert_eq!(filesystem_uuid(3), "FEEDFACE-0000-4000-8000-F00000000003");
    }

    #[test]
    fn every_fixed_guid_is_a_well_formed_uuid() {
        let mut all = vec![s(DISK_GUID), s(EXT4_HASH_SEED)];
        for n in 1..=5 {
            all.push(partition_guid(n));
            all.push(filesystem_uuid(n));
        }
        all.push(s(type_guid(FsType::Fat32)));
        all.push(s(type_guid(FsType::Erofs)));
        for g in all {
            let parts: Vec<&str> = g.split('-').collect();
            assert_eq!(parts.len(), 5, "{g}");
            assert_eq!(
                parts.iter().map(|p| p.len()).collect::<Vec<_>>(),
                [8, 4, 4, 4, 12],
                "{g}"
            );
            assert!(g.chars().all(|c| c.is_ascii_hexdigit() || c == '-'), "{g}");
        }
    }

    #[test]
    fn a_fat_volume_id_is_eight_hex_digits_and_distinct_per_boot_partition() {
        assert_eq!(fat_volume_id(1).len(), 8);
        assert_ne!(fat_volume_id(1), fat_volume_id(2));
        assert_eq!(fat_volume_id(2), "FEED0002");
    }

    #[test]
    fn the_fat_partitions_carry_the_microsoft_basic_data_type_guid() {
        assert_eq!(
            type_guid(FsType::Fat32),
            "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7"
        );
    }

    #[test]
    fn the_two_linux_filesystems_share_one_type_guid_and_it_is_not_the_fat_one() {
        assert_eq!(type_guid(FsType::Erofs), type_guid(FsType::Ext4));
        assert_ne!(type_guid(FsType::Erofs), type_guid(FsType::Fat32));
    }

    #[test]
    fn the_image_epoch_is_representable_in_fat_which_starts_in_1980() {
        // 315_532_800 is 1980-01-01T00:00:00Z. A smaller value wraps in FAT —
        // `mdir` showed 2097 for the Unix epoch before this constant existed.
        assert_eq!(IMAGE_EPOCH, 315_532_800);
    }

    #[test]
    fn timezone_is_pinned_because_mcopy_converts_an_mtime_using_it() {
        assert!(TOOL_ENV.contains(&("TZ", "UTC")));
    }

    #[test]
    fn the_required_tools_do_not_include_anything_that_needs_root() {
        for forbidden in ["losetup", "mount", "sudo", "kpartx", "partprobe"] {
            assert!(
                !REQUIRED_TOOLS.contains(&forbidden),
                "{forbidden} needs privilege; the build must not"
            );
        }
    }

    #[test]
    fn every_filesystem_is_currently_measured_reproducible() {
        // Measured, not assumed — two builds three seconds apart, compared
        // region by region. The `#[ignore]`d determinism test at the bottom of
        // this module is what actually checks it against the real tools; this
        // one pins the claim so a change to it is a deliberate edit.
        for fs in [FsType::Fat32, FsType::Erofs, FsType::Ext4] {
            assert!(irreproducible_because(fs).is_none(), "{fs:?}");
        }
    }

    // -- argv ---------------------------------------------------------------

    #[test]
    fn sgdisk_clears_the_table_before_anything_else_because_it_applies_options_in_order()
    -> Result<(), String> {
        let Ok(p) = plan(4096) else {
            return Err("refused".to_string());
        };
        let args = sgdisk_args(&p, "/out/i.img");
        assert_eq!(args[0], "--clear");
        Ok(())
    }

    #[test]
    fn sgdisk_is_given_a_fixed_disk_guid_so_two_builds_match() -> Result<(), String> {
        let Ok(p) = plan(4096) else {
            return Err("refused".to_string());
        };
        let args = sgdisk_args(&p, "/out/i.img");
        assert!(
            args.contains(&format!("--disk-guid={DISK_GUID}")),
            "{args:?}"
        );
        Ok(())
    }

    #[test]
    fn sgdisk_names_every_partition_with_layouts_label() -> Result<(), String> {
        let Ok(p) = plan(4096) else {
            return Err("refused".to_string());
        };
        let args = sgdisk_args(&p, "/out/i.img");
        for spec in partition_table() {
            assert!(
                args.contains(&format!("--change-name={}:{}", spec.number, spec.label)),
                "{args:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn sgdisk_gives_each_partition_a_fixed_unique_guid() -> Result<(), String> {
        let Ok(p) = plan(4096) else {
            return Err("refused".to_string());
        };
        let args = sgdisk_args(&p, "/out/i.img");
        for n in 1..=5u32 {
            assert!(
                args.contains(&format!("--partition-guid={n}:{}", partition_guid(n))),
                "{args:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn sgdisk_marks_only_the_two_boot_partitions_as_the_fat_boot_type() -> Result<(), String> {
        let Ok(p) = plan(4096) else {
            return Err("refused".to_string());
        };
        let args = sgdisk_args(&p, "/out/i.img");
        let fat = type_guid(FsType::Fat32);
        let marked: Vec<u32> = (1..=5u32)
            .filter(|n| args.contains(&format!("--typecode={n}:{fat}")))
            .collect();
        assert_eq!(marked, [1, 2]);
        Ok(())
    }

    #[test]
    fn sgdisks_new_ranges_are_the_planned_sectors() -> Result<(), String> {
        let Ok(p) = plan(4096) else {
            return Err("refused".to_string());
        };
        let args = sgdisk_args(&p, "/out/i.img");
        for part in &p {
            assert!(
                args.contains(&format!(
                    "--new={}:{}:{}",
                    part.spec.number, part.first_lba, part.last_lba
                )),
                "{args:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn the_image_path_is_sgdisks_last_argument() -> Result<(), String> {
        let Ok(p) = plan(4096) else {
            return Err("refused".to_string());
        };
        let args = sgdisk_args(&p, "/out/i.img");
        assert_eq!(args[args.len() - 1], "/out/i.img");
        Ok(())
    }

    #[test]
    fn mkfs_vfat_is_told_the_offset_in_sectors_and_the_size_in_1024_byte_blocks() {
        let p = placed_at(1);
        let args = mkfs_vfat_args(&p, "/out/i.img");
        assert!(
            args.contains(&format!("--offset={}", p.first_lba)),
            "{args:?}"
        );
        // Two 512-byte sectors to a 1024-byte block. 256 MiB -> 262144.
        assert_eq!(args[args.len() - 1], (p.sectors() / 2).to_string());
        assert_eq!(args[args.len() - 1], "262144");
    }

    /// The regression that cost a boot on 2026-08-23.
    ///
    /// `-F 32` alone produced 65403 clusters on a 256 MiB partition — 122 under
    /// the 65525 that Microsoft's specification requires before a reader may
    /// call a volume FAT32 — so the Pi's bootloader classified it `type: 16`,
    /// read the FAT16 fields of a FAT32 boot sector, and could not find
    /// `start4.elf`. The cluster size is the lever, so this asserts it and then
    /// bounds the count it produces.
    #[test]
    fn a_boot_partition_clears_the_65525_cluster_floor_that_makes_fat32_fat32() {
        for n in [1, 2] {
            let p = placed_at(n);
            let args = mkfs_vfat_args(&p, "/out/i.img");
            assert!(
                args.windows(2).any(|w| w[0] == "-s" && w[1] == "4"),
                "cluster size is not four sectors: {args:?}"
            );

            // Deliberately pessimistic: charge every cluster 4 data sectors
            // plus a whole extra sector of FAT, and drop the 32 reserved. The
            // real count is ~130 000; this floor only has to clear 65525.
            let floor = (p.sectors() - 32) / 5;
            assert!(
                floor >= 65_525,
                "partition {n} may produce a FAT16-classified volume: floor {floor}"
            );
        }
    }

    #[test]
    fn mkfs_vfat_carries_invariant_or_the_volume_id_comes_from_the_clock() {
        let args = mkfs_vfat_args(&placed_at(1), "/out/i.img");
        assert!(args.contains(&s("--invariant")), "{args:?}");
    }

    #[test]
    fn mkfs_vfat_sets_a_fixed_volume_id_per_partition() {
        let a = mkfs_vfat_args(&placed_at(1), "/out/i.img");
        let b = mkfs_vfat_args(&placed_at(2), "/out/i.img");
        assert!(a.contains(&fat_volume_id(1)), "{a:?}");
        assert!(b.contains(&fat_volume_id(2)), "{b:?}");
        assert_ne!(a, b);
    }

    #[test]
    fn a_fat_label_is_uppercased_because_fat_stores_it_that_way() {
        let args = mkfs_vfat_args(&placed_at(1), "/out/i.img");
        assert!(args.contains(&s("BOOT-A")), "{args:?}");
    }

    #[test]
    fn a_fat_label_fits_fats_eleven_character_limit() {
        for n in [1, 2] {
            let p = placed_at(n);
            assert!(p.spec.label.len() <= 11, "{}", p.spec.label);
        }
    }

    #[test]
    fn mkfs_vfat_asks_for_fat32_because_the_firmware_reads_it() -> Result<(), String> {
        let args = mkfs_vfat_args(&placed_at(1), "/out/i.img");
        let Some(i) = args.iter().position(|a| a == "-F") else {
            return Err("no FAT size given".to_string());
        };
        assert_eq!(args[i + 1], "32");
        Ok(())
    }

    #[test]
    fn mcopy_reaches_the_partition_through_mtools_offset_syntax_not_a_loop_mount() {
        let p = placed_at(1);
        let args = mcopy_args(&p, "/out/i.img", "/scratch/p1", &boot_tree());
        assert!(
            args.contains(&format!("/out/i.img@@{}", p.offset_bytes())),
            "{args:?}"
        );
    }

    #[test]
    fn mcopy_sources_are_the_trees_top_level_names_sorted() -> Result<(), String> {
        let Ok(tree) = Tree::new(vec![
            file("config.txt"),
            file("tryboot.txt"),
            file("autoboot.txt"),
            file("overlays/x.dtbo"),
        ]) else {
            return Err("refused".to_string());
        };
        let args = mcopy_args(&placed_at(1), "/out/i.img", "/scratch/p1", &tree);
        let sources: Vec<&String> = args
            .iter()
            .filter(|a| a.starts_with("/scratch/p1/"))
            .collect();
        assert_eq!(
            sources,
            [
                "/scratch/p1/autoboot.txt",
                "/scratch/p1/config.txt",
                "/scratch/p1/overlays",
                "/scratch/p1/tryboot.txt",
            ]
        );
        Ok(())
    }

    #[test]
    fn mcopys_last_argument_is_the_fat_root() {
        let args = mcopy_args(&placed_at(1), "/out/i.img", "/scratch/p1", &boot_tree());
        assert_eq!(args[args.len() - 1], "::");
    }

    #[test]
    fn mcopy_preserves_the_staged_mtime_which_is_where_fat_determinism_comes_from() {
        let args = mcopy_args(&placed_at(1), "/out/i.img", "/scratch/p1", &boot_tree());
        assert!(args.contains(&s("-m")), "{args:?}");
    }

    #[test]
    fn mcopy_stops_at_the_first_failure_rather_than_reporting_success_for_a_partial_copy() {
        let args = mcopy_args(&placed_at(1), "/out/i.img", "/scratch/p1", &boot_tree());
        assert!(args.contains(&s("-Q")), "{args:?}");
    }

    #[test]
    fn mkfs_erofs_forces_root_ownership_because_the_build_runs_unprivileged() {
        let args = mkfs_erofs_args(&placed_at(3), "/scratch/p3.erofs", "/scratch/p3");
        assert!(args.contains(&s("--all-root")), "{args:?}");
    }

    #[test]
    fn mkfs_erofs_fixes_the_timestamp_on_every_file_not_just_the_superblock() -> Result<(), String>
    {
        let args = mkfs_erofs_args(&placed_at(3), "/scratch/p3.erofs", "/scratch/p3");
        let Some(i) = args.iter().position(|a| a == "-T") else {
            return Err("no timestamp given".to_string());
        };
        assert_eq!(args[i + 1], IMAGE_EPOCH.to_string());
        assert!(args.contains(&s("--all-time")), "{args:?}");
        Ok(())
    }

    #[test]
    fn mkfs_erofs_writes_to_its_own_file_and_reads_the_staging_directory() {
        let args = mkfs_erofs_args(&placed_at(3), "/scratch/p3.erofs", "/scratch/p3");
        assert_eq!(args[args.len() - 2], "/scratch/p3.erofs");
        assert_eq!(args[args.len() - 1], "/scratch/p3");
    }

    #[test]
    fn each_erofs_slot_gets_its_own_filesystem_uuid() {
        let a = mkfs_erofs_args(&placed_at(3), "/s/a", "/s/sa");
        let b = mkfs_erofs_args(&placed_at(4), "/s/b", "/s/sb");
        assert!(a.contains(&filesystem_uuid(3)), "{a:?}");
        assert!(b.contains(&filesystem_uuid(4)), "{b:?}");
    }

    #[test]
    fn mkfs_ext4_is_told_the_offset_in_bytes_and_the_size_in_sectors() {
        let p = placed_at(5);
        let args = mkfs_ext4_args(&p, "/out/i.img", None);
        assert!(
            args.iter()
                .any(|a| a.starts_with(&format!("offset={},", p.offset_bytes()))),
            "{args:?}"
        );
        assert_eq!(args[args.len() - 1], format!("{}s", p.sectors()));
    }

    /// `ADR-018` §10.7.4. `mke2fs.conf` picks 1024-byte blocks for anything
    /// under 512 MB, and §10.2 made `/state` 254 MiB at build time — which
    /// silently capped how far §10.3 could ever grow it, at about 33 GiB.
    /// Every ext4 this tool makes is told its block size instead.
    #[test]
    fn mkfs_ext4_is_told_the_block_size_rather_than_letting_the_image_size_pick_it()
    -> Result<(), String> {
        for number in [3, 5] {
            let args = mkfs_ext4_args(&placed_at(number), "/out/i.img", None);
            let Some(i) = args.iter().position(|a| a == "-b") else {
                return Err(format!("partition {number} is given no block size: {args:?}"));
            };
            assert_eq!(args[i + 1], EXT4_BLOCK_BYTES.to_string());
        }
        assert_eq!(
            EXT4_BLOCK_BYTES, 4096,
            "1024 caps an online grow at about 33 GiB; the working card is 128 GB"
        );
        Ok(())
    }

    /// **The two ways a card can reach its final geometry must reach the same
    /// one.** `ADR-018` §10.4 keeps `--device-mib` as the escape hatch that
    /// builds at a card's size, and §10.3 grows a minimum-sized image on the
    /// card instead. If `plan` and `rivet_adapter_image::grow` disagreed
    /// about where the last partition ends, those two paths would produce
    /// different cards from the same hardware, and only one of them would be
    /// reachable by a test.
    #[test]
    fn the_grow_and_the_builder_place_the_last_partition_identically()
    -> Result<(), String> {
        use rivet_adapter_image::grow;
        assert_eq!(grow::LBA_BYTES, SECTOR_BYTES);
        assert_eq!(grow::ALIGN_LBA, ALIGN_SECTORS);
        assert_eq!(grow::STATE_PARTITION, crate::layout::STATE_PARTITION);

        let table = partition_table();
        let last = table.last().ok_or("the table is empty")?;
        assert_eq!(
            last.label,
            grow::STATE_LABEL,
            "the grow finds /state by the label this tool writes"
        );

        // 128 entries of 128 bytes is what `sgdisk` writes, so 32 sectors.
        let entry_lbas = 32;
        for device_mib in [2048u64, 3000, 7000, 7616, 15_000, 32_768, 122_880] {
            let placed = plan(device_mib).map_err(|e| format!("{device_mib} MiB: {e:?}"))?;
            let built = placed.last().ok_or("no partitions")?.last_lba;
            let device_lbas = device_mib * 1024 * 1024 / SECTOR_BYTES;
            let grown = grow::aligned_end(grow::last_usable(device_lbas, entry_lbas));
            assert_eq!(
                built, grown,
                "{device_mib} MiB: the builder ends /state at {built} and the grow at {grown}"
            );
        }
        Ok(())
    }

    #[test]
    fn mkfs_ext4_owns_the_root_directory_as_root_because_the_builder_is_not() {
        let args = mkfs_ext4_args(&placed_at(5), "/out/i.img", None);
        assert!(
            args.iter().any(|a| a.contains("root_owner=0:0")),
            "{args:?}"
        );
    }

    #[test]
    fn mkfs_ext4_asks_for_ext4_rather_than_whatever_mke2fs_defaults_to() -> Result<(), String> {
        let args = mkfs_ext4_args(&placed_at(5), "/out/i.img", None);
        let Some(i) = args.iter().position(|a| a == "-t") else {
            return Err("no filesystem type given".to_string());
        };
        assert_eq!(args[i + 1], FsType::Ext4.as_str());
        Ok(())
    }

    #[test]
    fn state_reserves_adr_018_7s_space_for_the_system() -> Result<(), String> {
        let args = mkfs_ext4_args(&placed_at(STATE_PARTITION), "/out/i.img", None);
        let Some(i) = args.iter().position(|a| a == "-m") else {
            return Err("no reserve given".to_string());
        };
        assert_eq!(args[i + 1], STATE_RESERVED_PERCENT.to_string());
        assert_ne!(
            STATE_RESERVED_PERCENT, IMAGE_RESERVED_PERCENT,
            "`ADR-018` §7's guarantee is that /state holds something back"
        );
        Ok(())
    }

    #[test]
    fn a_read_only_image_partition_reserves_nothing_because_nothing_ever_writes_to_it()
    -> Result<(), String> {
        let args = mkfs_ext4_args(&placed_at(3), "/out/i.img", Some("/scratch/p3.tar"));
        let Some(i) = args.iter().position(|a| a == "-m") else {
            return Err("no reserve given".to_string());
        };
        assert_eq!(args[i + 1], IMAGE_RESERVED_PERCENT.to_string());
        assert_eq!(IMAGE_RESERVED_PERCENT, 0);
        Ok(())
    }

    #[test]
    fn state_is_given_no_source_at_all_because_it_ships_empty() {
        let args = mkfs_ext4_args(&placed_at(STATE_PARTITION), "/out/i.img", None);
        assert!(!args.contains(&s("-d")), "{args:?}");
    }

    #[test]
    fn an_ext4_image_partition_is_given_a_tarball_never_a_directory() -> Result<(), String> {
        let args = mkfs_ext4_args(&placed_at(3), "/out/i.img", Some("/scratch/p3.tar"));
        let Some(i) = args.iter().position(|a| a == "-d") else {
            return Err("no source given".to_string());
        };
        // A directory would be copied with the builder's uid; a tar carries the
        // ownership its author chose. Measured with `debugfs -R "ls -l"`.
        assert_eq!(args[i + 1], "/scratch/p3.tar");
        Ok(())
    }

    #[test]
    fn the_tar_claims_root_ownership_because_that_is_the_only_reason_it_exists() {
        let args = tar_args("/scratch/p3.tar", "/scratch/p3");
        assert!(args.contains(&s("--owner=0")), "{args:?}");
        assert!(args.contains(&s("--group=0")), "{args:?}");
        assert!(args.contains(&s("--numeric-owner")), "{args:?}");
    }

    #[test]
    fn the_tar_is_sorted_and_time_fixed_or_it_is_not_reproducible() {
        let args = tar_args("/scratch/p3.tar", "/scratch/p3");
        assert!(args.contains(&s("--sort=name")), "{args:?}");
        assert!(
            args.contains(&format!("--mtime=@{IMAGE_EPOCH}")),
            "{args:?}"
        );
        assert!(args.contains(&s("--format=gnu")), "{args:?}");
    }

    #[test]
    fn the_tar_is_taken_from_inside_the_staging_directory_so_paths_start_at_the_root() {
        let args = tar_args("/scratch/p3.tar", "/scratch/p3");
        assert_eq!(args[args.len() - 1], ".");
        let Some(i) = args.iter().position(|a| a == "-C") else {
            return;
        };
        assert_eq!(args[i + 1], "/scratch/p3");
    }

    #[test]
    fn the_source_date_epoch_the_tools_are_given_is_the_one_the_staging_uses() -> Result<(), String>
    {
        let Some((_, value)) = TOOL_ENV.iter().find(|(k, _)| *k == "SOURCE_DATE_EPOCH") else {
            return Err("SOURCE_DATE_EPOCH is missing; ext4 is then not reproducible".to_string());
        };
        assert_eq!(*value, IMAGE_EPOCH.to_string());
        Ok(())
    }

    #[test]
    fn ext4_is_only_claimed_reproducible_while_source_date_epoch_is_pinned() {
        // The claim in `irreproducible_because` rests entirely on this variable
        // — measured at 0 differing bytes with it and 93 without.
        if irreproducible_because(FsType::Ext4).is_none() {
            assert!(
                TOOL_ENV.iter().any(|(k, _)| *k == "SOURCE_DATE_EPOCH"),
                "ext4 is claimed reproducible with nothing pinning its timestamps"
            );
        }
    }

    #[test]
    fn touch_sets_the_symlink_itself_not_its_target() {
        let args = touch_args("/scratch/p1");
        assert!(args.contains(&s("-h")), "{args:?}");
        assert!(args.contains(&format!("@{IMAGE_EPOCH}")), "{args:?}");
    }

    // -- refusals -----------------------------------------------------------

    #[test]
    fn a_build_on_an_undersized_device_writes_nothing_at_all() {
        let mut host = FakeHost::new();
        let mut req = request();
        req.device_mib = 64;
        assert!(matches!(
            build(&mut host, &req),
            Err(ImageError::DeviceTooSmall { .. })
        ));
        assert!(host.created.is_empty());
        assert!(host.runs.is_empty());
    }

    #[test]
    fn a_missing_tool_is_refused_and_the_error_names_it() -> Result<(), String> {
        let mut host = FakeHost::new();
        host.missing_tools.insert(s("mkfs.erofs"));
        let Err(e) = build(&mut host, &request()) else {
            return Err("built without mkfs.erofs".to_string());
        };
        assert!(matches!(e, ImageError::MissingTools { .. }));
        assert!(e.to_string().contains("mkfs.erofs"), "{e}");
        assert!(host.created.is_empty(), "the image was created anyway");
        Ok(())
    }

    #[test]
    fn every_missing_tool_is_named_not_just_the_first() -> Result<(), String> {
        let mut host = FakeHost::new();
        host.missing_tools.insert(s("sgdisk"));
        host.missing_tools.insert(s("mcopy"));
        let Err(e) = build(&mut host, &request()) else {
            return Err("built with two tools missing".to_string());
        };
        let text = e.to_string();
        assert!(text.contains("sgdisk"), "{text}");
        assert!(text.contains("mcopy"), "{text}");
        Ok(())
    }

    #[test]
    fn a_target_that_is_not_a_regular_file_is_refused_because_this_tool_does_not_write_cards()
    -> Result<(), String> {
        let mut host = FakeHost::new();
        host.image_kind = PathKind::Other;
        let Err(e) = build(&mut host, &request()) else {
            return Err("wrote to a device".to_string());
        };
        assert!(matches!(e, ImageError::NotARegularFile { .. }));
        assert!(e.to_string().contains("dd"), "{e}");
        assert!(host.created.is_empty());
        Ok(())
    }

    #[test]
    fn an_existing_regular_file_is_accepted_and_truncated() {
        let mut host = FakeHost::new();
        host.image_kind = PathKind::RegularFile;
        assert!(build(&mut host, &request()).is_ok());
        assert_eq!(host.created.len(), 1);
    }

    #[test]
    fn a_boot_partition_without_tryboot_txt_is_refused_because_adr_018_5_3_costs_a_board()
    -> Result<(), String> {
        let Ok(missing) = Tree::new(vec![file("config.txt"), file("autoboot.txt")]) else {
            return Err("refused".to_string());
        };
        let mut req = request();
        req.trees.insert(1, missing);
        let mut host = FakeHost::new();
        let Err(e) = build(&mut host, &req) else {
            return Err("built a boot partition with no tryboot.txt".to_string());
        };
        assert!(matches!(
            e,
            ImageError::BootFileMissing {
                filename: "tryboot.txt",
                ..
            }
        ));
        assert!(e.to_string().contains("headless"), "{e}");
        Ok(())
    }

    #[test]
    fn a_boot_partition_without_autoboot_txt_is_refused_because_slot_b_would_be_unreachable()
    -> Result<(), String> {
        let Ok(missing) = Tree::new(vec![file("config.txt"), file("tryboot.txt")]) else {
            return Err("refused".to_string());
        };
        let mut req = request();
        req.trees.insert(2, missing);
        let mut host = FakeHost::new();
        assert!(matches!(
            build(&mut host, &req),
            Err(ImageError::BootFileMissing {
                filename: "autoboot.txt",
                label: "boot-b",
            })
        ));
        Ok(())
    }

    #[test]
    fn a_boot_partition_without_config_txt_is_refused() -> Result<(), String> {
        let Ok(missing) = Tree::new(vec![file("tryboot.txt"), file("autoboot.txt")]) else {
            return Err("refused".to_string());
        };
        let mut req = request();
        req.trees.insert(1, missing);
        let mut host = FakeHost::new();
        assert!(matches!(
            build(&mut host, &req),
            Err(ImageError::BootFileMissing {
                filename: "config.txt",
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn both_boot_partitions_are_checked_not_only_the_first() {
        let mut req = request();
        req.trees.insert(2, Tree::default());
        let mut host = FakeHost::new();
        assert!(matches!(
            build(&mut host, &req),
            Err(ImageError::BootFileMissing {
                label: "boot-b",
                ..
            })
        ));
    }

    #[test]
    fn a_symlink_bound_for_fat_is_refused_rather_than_silently_becoming_a_copy()
    -> Result<(), String> {
        let mut entries = boot_tree().entries().to_vec();
        entries.push(Entry::Symlink {
            path: s("kernel.img"),
            target: s("kernel8.img"),
        });
        let Ok(tree) = Tree::new(entries) else {
            return Err("refused".to_string());
        };
        let mut req = request();
        req.trees.insert(1, tree);
        let mut host = FakeHost::new();
        let Err(e) = build(&mut host, &req) else {
            return Err("put a symlink on FAT".to_string());
        };
        assert!(matches!(e, ImageError::FatCannotHold { .. }));
        assert!(e.to_string().contains("kernel.img"), "{e}");
        Ok(())
    }

    #[test]
    fn a_symlink_bound_for_a_system_slot_is_fine_because_both_formats_hold_one()
    -> Result<(), String> {
        let Ok(tree) = Tree::new(vec![Entry::Symlink {
            path: s("bin/sh"),
            target: s("/system/bin/nu"),
        }]) else {
            return Err("refused".to_string());
        };
        let mut req = request();
        req.trees.insert(Slot::A.system_partition(), tree);
        let mut host = FakeHost::new();
        assert!(build(&mut host, &req).is_ok());
        Ok(())
    }

    #[test]
    fn a_filesystem_larger_than_its_partition_is_refused_before_it_overwrites_the_next_one()
    -> Result<(), String> {
        let capacity = SYSTEM_SIZE_MIB * 1024 * 1024;
        let Err(e) = check_fits("system-a", capacity + 1, capacity) else {
            return Err("an oversized filesystem was accepted".to_string());
        };
        assert!(matches!(e, ImageError::PartitionOverflow { .. }));
        assert!(
            e.to_string().contains("overwrite the next partition"),
            "{e}"
        );
        assert!(e.to_string().contains("system-a"), "{e}");
        Ok(())
    }

    #[test]
    fn a_filesystem_exactly_filling_its_partition_is_accepted() {
        let capacity = SYSTEM_SIZE_MIB * 1024 * 1024;
        assert!(check_fits("system-a", capacity, capacity).is_ok());
        assert!(check_fits("system-a", 0, capacity).is_ok());
    }

    /// The measure-then-place path only exists for erofs, whose size is decided
    /// by its contents. `layout::SYSTEM_FSTYPE` is an ext4 stopgap today, so
    /// this runs only when erofs is back — and fails loudly rather than
    /// silently skipping, because a test that quietly does nothing is worse
    /// than no test (`AGENTS.md` §4).
    #[test]
    fn an_oversized_erofs_slot_is_refused_before_it_is_spliced() -> Result<(), String> {
        if !system_is_erofs() {
            assert_eq!(
                SYSTEM_FSTYPE,
                FsType::Ext4,
                "SYSTEM_FSTYPE is neither erofs nor the recorded ext4 stopgap"
            );
            return Ok(());
        }
        let mut host = FakeHost::new();
        host.sizes
            .insert(s("/scratch/p3.erofs"), SYSTEM_SIZE_MIB * 1024 * 1024 + 1);
        let Err(e) = build(&mut host, &request()) else {
            return Err("spliced an oversized erofs".to_string());
        };
        assert!(matches!(e, ImageError::PartitionOverflow { .. }));
        assert!(host.spliced.is_empty(), "it was written anyway");
        Ok(())
    }

    #[test]
    fn a_failing_tool_stops_the_build_and_the_error_carries_its_stderr() -> Result<(), String> {
        let mut host = FakeHost::new();
        host.fail.insert(
            system_mkfs(),
            ToolRun {
                code: Some(1),
                stdout: String::new(),
                stderr: s("cannot find source directory"),
            },
        );
        let Err(e) = build(&mut host, &request()) else {
            return Err("a failed mkfs was reported as success".to_string());
        };
        let text = e.to_string();
        assert!(text.contains("cannot find source directory"), "{text}");
        assert!(text.contains("exit status 1"), "{text}");
        assert!(
            text.contains(&format!("invoked as: {}", system_mkfs())),
            "{text}"
        );
        Ok(())
    }

    #[test]
    fn a_tool_killed_by_a_signal_is_reported_as_such_rather_than_as_an_exit_code()
    -> Result<(), String> {
        let mut host = FakeHost::new();
        host.fail.insert(
            s("sgdisk"),
            ToolRun {
                code: None,
                stdout: String::new(),
                stderr: String::new(),
            },
        );
        let Err(e) = build(&mut host, &request()) else {
            return Err("a killed sgdisk was reported as success".to_string());
        };
        assert!(e.to_string().contains("killed by a signal"), "{e}");
        Ok(())
    }

    #[test]
    fn a_silent_failure_says_the_tool_said_nothing_rather_than_showing_an_empty_message()
    -> Result<(), String> {
        let mut host = FakeHost::new();
        host.fail.insert(
            s("mkfs.vfat"),
            ToolRun {
                code: Some(2),
                stdout: String::new(),
                stderr: s("   \n"),
            },
        );
        let Err(e) = build(&mut host, &request()) else {
            return Err("reported success".to_string());
        };
        assert!(e.to_string().contains("said nothing on stderr"), "{e}");
        Ok(())
    }

    #[test]
    fn a_failed_sgdisk_stops_before_any_mkfs_runs() {
        let mut host = FakeHost::new();
        host.fail.insert(
            s("sgdisk"),
            ToolRun {
                code: Some(3),
                stdout: String::new(),
                stderr: s("bad"),
            },
        );
        let _ = build(&mut host, &request());
        assert!(
            !host.programs().iter().any(|p| p.starts_with("mkfs")),
            "{:?}",
            host.programs()
        );
    }

    // -- the happy path -----------------------------------------------------

    #[test]
    fn the_image_file_is_created_at_exactly_the_requested_size() {
        let mut host = FakeHost::new();
        let req = request();
        assert!(build(&mut host, &req).is_ok());
        assert_eq!(
            host.created,
            [(s(IMAGE_PATH), req.device_mib * 1024 * 1024)]
        );
    }

    #[test]
    fn the_partition_table_is_written_before_any_filesystem() -> Result<(), String> {
        let mut host = FakeHost::new();
        assert!(build(&mut host, &request()).is_ok());
        let programs = host.programs();
        let Some(sgdisk_at) = programs.iter().position(|p| p == "sgdisk") else {
            return Err("sgdisk never ran".to_string());
        };
        let Some(first_mkfs) = programs.iter().position(|p| p.starts_with("mkfs")) else {
            return Err("no mkfs ran".to_string());
        };
        assert!(sgdisk_at < first_mkfs, "{programs:?}");
        Ok(())
    }

    #[test]
    fn sgdisk_runs_exactly_once_because_one_invocation_writes_the_whole_table() {
        let mut host = FakeHost::new();
        assert!(build(&mut host, &request()).is_ok());
        assert_eq!(host.args_for("sgdisk").len(), 1);
    }

    #[test]
    fn every_partition_gets_the_mkfs_layout_named_for_it() {
        let mut host = FakeHost::new();
        assert!(build(&mut host, &request()).is_ok());
        assert_eq!(host.args_for("mkfs.vfat").len(), 2, "two boot partitions");
        // The two `/system` slots plus `/state` — which is three `mkfs.ext4`
        // calls under today's stopgap and one when erofs returns.
        assert_eq!(
            host.args_for(&system_mkfs()).len(),
            if system_is_erofs() { 2 } else { 3 }
        );
        assert_eq!(
            host.args_for("mkfs.ext4").len(),
            if system_is_erofs() { 1 } else { 3 }
        );
    }

    #[test]
    fn a_fat_partition_is_made_before_it_is_populated() -> Result<(), String> {
        let mut host = FakeHost::new();
        assert!(build(&mut host, &request()).is_ok());
        let programs = host.programs();
        let Some(mkfs) = programs.iter().position(|p| p == "mkfs.vfat") else {
            return Err("no mkfs.vfat".to_string());
        };
        let Some(mcopy) = programs.iter().position(|p| p == "mcopy") else {
            return Err("no mcopy".to_string());
        };
        assert!(mkfs < mcopy, "{programs:?}");
        Ok(())
    }

    #[test]
    fn the_staged_tree_is_touched_before_mcopy_reads_it() -> Result<(), String> {
        let mut host = FakeHost::new();
        assert!(build(&mut host, &request()).is_ok());
        let programs = host.programs();
        let Some(touch) = programs.iter().position(|p| p == "find") else {
            return Err("mtimes were never fixed".to_string());
        };
        let Some(mcopy) = programs.iter().position(|p| p == "mcopy") else {
            return Err("no mcopy".to_string());
        };
        assert!(touch < mcopy, "{programs:?}");
        Ok(())
    }

    #[test]
    fn every_tool_is_run_with_the_pinned_environment() {
        let mut host = FakeHost::new();
        assert!(build(&mut host, &request()).is_ok());
        for run in &host.runs {
            assert!(
                run.env.contains(&(s("TZ"), s("UTC"))),
                "{} ran without TZ pinned",
                run.program
            );
        }
    }

    #[test]
    fn each_partition_stages_into_its_own_directory() {
        let mut host = FakeHost::new();
        assert!(build(&mut host, &request()).is_ok());
        let roots: Vec<&String> = host.staged.iter().map(|(r, _)| r).collect();
        assert_eq!(
            roots,
            ["/scratch/p1", "/scratch/p2", "/scratch/p3", "/scratch/p4"]
        );
    }

    #[test]
    fn an_empty_system_slot_is_handled_the_way_its_format_needs() {
        let mut req = request();
        req.trees.remove(&Slot::B.system_partition());
        let mut host = FakeHost::new();
        assert!(build(&mut host, &req).is_ok());
        let staged = host.staged.iter().any(|(r, _)| r == "/scratch/p4");
        // erofs needs a source directory even for an empty filesystem; ext4
        // makes an empty filesystem by being given no source at all.
        assert_eq!(staged, system_is_erofs(), "{:?}", host.staged);
    }

    #[test]
    fn an_empty_system_slot_is_still_formatted_so_the_b_slot_exists_on_a_first_card() {
        let mut req = request();
        req.trees.remove(&Slot::B.system_partition());
        let mut host = FakeHost::new();
        assert!(build(&mut host, &req).is_ok());
        assert_eq!(
            host.args_for(&system_mkfs()).len(),
            if system_is_erofs() { 2 } else { 3 }
        );
    }

    #[test]
    fn an_empty_fat_partition_is_formatted_but_not_populated() {
        // A boot tree is required, so the only way to see this is through the
        // argv builder: mcopy with no sources would be an error.
        let args = mcopy_args(&placed_at(1), "/out/i.img", "/scratch/p1", &Tree::default());
        let sources: Vec<&String> = args
            .iter()
            .filter(|a| a.starts_with("/scratch/p1/"))
            .collect();
        assert!(sources.is_empty());
    }

    #[test]
    fn state_is_never_staged_because_it_ships_empty() {
        let mut host = FakeHost::new();
        assert!(build(&mut host, &request()).is_ok());
        assert!(
            !host.staged.iter().any(|(r, _)| r == "/scratch/p5"),
            "{:?}",
            host.staged
        );
    }

    #[test]
    fn only_erofs_is_ever_spliced_because_every_other_mkfs_writes_itself_in_place() {
        let mut host = FakeHost::new();
        assert!(build(&mut host, &request()).is_ok());
        if system_is_erofs() {
            let p3 = placed_at(3);
            let p4 = placed_at(4);
            assert_eq!(
                host.spliced,
                [
                    (s(IMAGE_PATH), p3.offset_bytes(), s("/scratch/p3.erofs")),
                    (s(IMAGE_PATH), p4.offset_bytes(), s("/scratch/p4.erofs")),
                ]
            );
        } else {
            assert!(host.spliced.is_empty(), "{:?}", host.spliced);
        }
    }

    #[test]
    fn a_system_slot_is_tarred_before_the_mkfs_that_reads_the_tar() -> Result<(), String> {
        if system_is_erofs() {
            return Ok(());
        }
        let mut host = FakeHost::new();
        assert!(build(&mut host, &request()).is_ok());
        let programs = host.programs();
        let Some(tar) = programs.iter().position(|p| p == "tar") else {
            return Err("the /system tree was never tarred".to_string());
        };
        let Some(mkfs) = programs.iter().rposition(|p| p == "mkfs.ext4") else {
            return Err("no mkfs.ext4".to_string());
        };
        assert!(tar < mkfs, "{programs:?}");
        Ok(())
    }

    #[test]
    fn state_is_never_tarred_because_it_has_nothing_to_pack() {
        let mut host = FakeHost::new();
        assert!(build(&mut host, &request()).is_ok());
        // Two `/system` slots under the stopgap, and nothing for `/state`.
        let expected = if system_is_erofs() { 0 } else { 2 };
        assert_eq!(host.args_for("tar").len(), expected);
    }

    #[test]
    fn the_scratch_directory_is_removed_when_the_build_succeeds() {
        let mut host = FakeHost::new();
        assert!(build(&mut host, &request()).is_ok());
        assert_eq!(host.removed, ["/scratch"]);
    }

    #[test]
    fn the_scratch_directory_survives_a_failure_so_the_inputs_can_be_inspected() {
        let mut host = FakeHost::new();
        host.fail.insert(
            system_mkfs(),
            ToolRun {
                code: Some(1),
                stdout: String::new(),
                stderr: s("boom"),
            },
        );
        assert!(build(&mut host, &request()).is_err());
        assert!(host.removed.is_empty());
    }

    // -- the report ---------------------------------------------------------

    #[test]
    fn the_report_describes_all_five_partitions_in_order() -> Result<(), String> {
        let mut host = FakeHost::new();
        let Ok(report) = build(&mut host, &request()) else {
            return Err("the happy path failed".to_string());
        };
        let got: Vec<(u32, &str)> = report
            .partitions
            .iter()
            .map(|p| (p.number, p.label))
            .collect();
        assert_eq!(
            got,
            [
                (1, "boot-a"),
                (2, "boot-b"),
                (3, "system-a"),
                (4, "system-b"),
                (5, "state")
            ]
        );
        Ok(())
    }

    #[test]
    fn the_report_states_reproducibility_per_partition_rather_than_leaving_it_to_be_discovered()
    -> Result<(), String> {
        let mut host = FakeHost::new();
        let Ok(report) = build(&mut host, &request()) else {
            return Err("the happy path failed".to_string());
        };
        for p in &report.partitions {
            assert_eq!(
                p.irreproducible_because,
                irreproducible_because(p.fstype),
                "{}",
                p.label
            );
        }
        assert!(report.fully_reproducible());
        Ok(())
    }

    #[test]
    fn the_report_carries_the_bytes_a_measured_filesystem_used_and_nothing_else_claims_a_figure()
    -> Result<(), String> {
        let mut host = FakeHost::new();
        host.sizes.insert(s("/scratch/p3.erofs"), 12288);
        let Ok(report) = build(&mut host, &request()) else {
            return Err("the happy path failed".to_string());
        };
        let used: Vec<(u32, Option<u64>)> = report
            .partitions
            .iter()
            .map(|p| (p.number, p.used_bytes))
            .collect();
        // Only erofs is built to its own file and measured. Everything else is
        // told how big to be, so claiming a used figure for it would be a
        // number this module does not have.
        let expected = if system_is_erofs() {
            [
                (1, None),
                (2, None),
                (3, Some(12288)),
                (4, Some(4096)),
                (5, None),
            ]
        } else {
            [(1, None), (2, None), (3, None), (4, None), (5, None)]
        };
        assert_eq!(used, expected);
        Ok(())
    }

    #[test]
    fn the_reports_offsets_are_the_plans_offsets() -> Result<(), String> {
        let mut host = FakeHost::new();
        let Ok(report) = build(&mut host, &request()) else {
            return Err("the happy path failed".to_string());
        };
        let Ok(planned) = plan(minimum_device_mib()) else {
            return Err("plan refused".to_string());
        };
        for (r, p) in report.partitions.iter().zip(planned.iter()) {
            assert_eq!(r.offset_bytes, p.offset_bytes());
            assert_eq!(r.len_bytes, p.len_bytes());
            assert_eq!(r.first_lba, p.first_lba);
            assert_eq!(r.last_lba, p.last_lba);
        }
        Ok(())
    }

    #[test]
    fn the_reported_partitions_cover_the_device_without_overlapping() -> Result<(), String> {
        let mut host = FakeHost::new();
        let Ok(report) = build(&mut host, &request()) else {
            return Err("the happy path failed".to_string());
        };
        for pair in report.partitions.windows(2) {
            assert_eq!(
                pair[0].offset_bytes + pair[0].len_bytes,
                pair[1].offset_bytes
            );
        }
        Ok(())
    }

    // -- joint behaviour with layout ---------------------------------------

    #[test]
    fn the_slot_numbers_layout_publishes_are_the_ones_the_trees_are_keyed_by() -> Result<(), String>
    {
        let mut host = FakeHost::new();
        let mut req = request();
        // Give slot B's boot partition a distinguishable file.
        let mut entries = boot_tree().entries().to_vec();
        entries.push(file("slot-b-marker"));
        let Ok(tree) = Tree::new(entries) else {
            return Err("refused".to_string());
        };
        req.trees.insert(Slot::B.boot_partition(), tree);
        assert!(build(&mut host, &req).is_ok());
        let Some((_, staged)) = host.staged.iter().find(|(r, _)| r == "/scratch/p2") else {
            return Err("slot B was not staged into p2".to_string());
        };
        assert!(staged.has("slot-b-marker"));
        Ok(())
    }

    #[test]
    fn the_content_of_a_system_slot_reaches_the_mkfs_for_that_slots_partition_number() {
        let mut host = FakeHost::new();
        assert!(build(&mut host, &request()).is_ok());
        // erofs is handed a staging directory; ext4 is handed a tar built from
        // one. Either way the path carries the partition number, and getting
        // that wrong would swap the A and B images.
        let sources: Vec<String> = host
            .args_for(&system_mkfs())
            .iter()
            .filter_map(|a| {
                a.iter()
                    .find(|arg| arg.starts_with("/scratch/p"))
                    .map(|arg| arg.rsplit('/').next().unwrap_or(arg).to_string())
            })
            .collect();
        let expect_a = if system_is_erofs() { "p3" } else { "p3.tar" };
        let expect_b = if system_is_erofs() { "p4" } else { "p4.tar" };
        assert_eq!(sources, [expect_a, expect_b]);
        assert_eq!(Slot::A.system_partition(), 3);
        assert_eq!(Slot::B.system_partition(), 4);
    }

    #[test]
    fn the_partition_layout_calls_state_is_the_one_that_gets_no_content() -> Result<(), String> {
        let mut host = FakeHost::new();
        assert!(build(&mut host, &request()).is_ok());
        let Some(args) = host
            .args_for("mkfs.ext4")
            .into_iter()
            .find(|a| a.contains(&s("state")))
        else {
            return Err("no mkfs.ext4 made a partition labelled `state`".to_string());
        };
        assert!(!args.contains(&s("-d")), "{args:?}");
        assert_eq!(placed_at(STATE_PARTITION).spec.label, "state");
        Ok(())
    }

    // -- against the real tools ---------------------------------------------
    //
    // Everything above this line proves a *decision*. Nothing above it proves
    // that `sgdisk` accepts `--partition-guid=`, that `mcopy` understands
    // `image@@offset`, or that two builds produce the same bytes. Only running
    // them does, and running them needs five packages installed, so these are
    // `#[ignore]`d and run deliberately:
    //
    //     cargo test -p rivet-image -- --ignored --nocapture
    //
    // `AGENTS.md` §4 warns that a suite which cannot be run rots silently. The
    // guard against that is that these assert on the *same* constants the
    // fast tests do, so a changed offset or GUID fails there first.

    fn real_boot_tree(slot: Slot) -> Tree {
        let autoboot = format!(
            "[all]\ntryboot_a_b=1\nboot_partition={}\n\n[tryboot]\nboot_partition={}\n",
            slot.boot_partition(),
            slot.other().boot_partition()
        );
        Tree::new(vec![
            Entry::File {
                path: s("config.txt"),
                mode: 0o644,
                contents: b"arm_64bit=1\nenable_uart=1\n".to_vec(),
            },
            Entry::File {
                path: s("tryboot.txt"),
                mode: 0o644,
                contents: b"arm_64bit=1\nenable_uart=1\n".to_vec(),
            },
            Entry::File {
                path: s("autoboot.txt"),
                mode: 0o644,
                contents: autoboot.into_bytes(),
            },
            Entry::File {
                path: s("cmdline.txt"),
                mode: 0o644,
                contents: format!(
                    "console=serial0,115200 root=/dev/mmcblk0p{} rootfstype={} ro rootwait \
                     init=/system/bin/rivet-initd\n",
                    slot.system_partition(),
                    SYSTEM_FSTYPE.as_str()
                )
                .into_bytes(),
            },
            Entry::File {
                path: s("overlays/README"),
                mode: 0o644,
                contents: b"placeholder\n".to_vec(),
            },
        ])
        .unwrap_or_default()
    }

    fn real_system_tree() -> Tree {
        Tree::new(vec![
            // ADR-018 §6: the tmpfs mount point must exist in the image,
            // because a mount point cannot be made on a read-only filesystem.
            Entry::Dir {
                path: s("ext"),
                mode: 0o755,
            },
            Entry::File {
                path: s("bin/rivet-initd"),
                mode: 0o755,
                contents: b"\x7fELF not really\n".to_vec(),
            },
            Entry::Symlink {
                path: s("bin/init"),
                target: s("rivet-initd"),
            },
        ])
        .unwrap_or_default()
    }

    fn real_request(path: &str) -> BuildRequest<'_> {
        let mut trees = BTreeMap::new();
        trees.insert(Slot::A.boot_partition(), real_boot_tree(Slot::A));
        trees.insert(Slot::B.boot_partition(), real_boot_tree(Slot::B));
        trees.insert(Slot::A.system_partition(), real_system_tree());
        trees.insert(Slot::B.system_partition(), real_system_tree());
        BuildRequest {
            image_path: path,
            device_mib: minimum_device_mib(),
            trees,
        }
    }

    fn shout(host: &mut CommandHost, program: &str, args: &[&str]) -> String {
        let owned: Vec<String> = args.iter().map(|a| s(a)).collect();
        match host.run(program, &owned, TOOL_ENV) {
            Ok(r) => format!("{}{}", r.stdout, r.stderr),
            Err(e) => format!("<{program} could not be run: {e}>"),
        }
    }

    #[test]
    #[ignore = "drives the real sgdisk/mkfs/mtools; run with --ignored"]
    fn a_real_image_is_partitioned_and_formatted_the_way_the_plan_says() -> Result<(), String> {
        let dir = std::env::temp_dir().join("rivet-image-real");
        let _ = std::fs::create_dir_all(&dir);
        // The basename comes off `IMAGE_PATH` so the name lives in one place.
        let file = IMAGE_PATH.rsplit('/').next().unwrap_or(IMAGE_PATH);
        let path = dir.join(file).to_string_lossy().into_owned();

        let mut host = CommandHost::new();
        let report = match build(&mut host, &real_request(&path)) {
            Ok(r) => r,
            Err(e) => return Err(format!("the build failed: {e}")),
        };

        println!(
            "=== sgdisk -p ===\n{}",
            shout(&mut host, "sgdisk", &["-p", &path])
        );
        println!("=== file ===\n{}", shout(&mut host, "file", &["-s", &path]));
        for p in &report.partitions {
            println!(
                "=== blkid p{} @ {} ===\n{}",
                p.number,
                p.offset_bytes,
                shout(
                    &mut host,
                    "blkid",
                    &["-p", "-O", &p.offset_bytes.to_string(), &path]
                )
            );
        }
        let boot_a = &report.partitions[0];
        println!(
            "=== mdir boot-a ===\n{}",
            shout(
                &mut host,
                "mdir",
                &[
                    "-i",
                    &format!("{path}@@{}", boot_a.offset_bytes),
                    "-/",
                    "::"
                ]
            )
        );

        // What the tools say, asserted rather than merely printed.
        let table = shout(&mut host, "sgdisk", &["-p", &path]);
        for label in ["boot-a", "boot-b", "system-a", "system-b", "state"] {
            if !table.contains(label) {
                return Err(format!("sgdisk -p does not mention {label}:\n{table}"));
            }
        }
        if !table.contains(DISK_GUID) {
            return Err(format!("the disk GUID is not the fixed one:\n{table}"));
        }
        // 0700 is sgdisk's short name for Microsoft basic data, which is what the
        // stock card's MBR type 0x0c maps to and the only FAT type this
        // bootloader was seen to accept (ADR-018 §9 item 1).
        if table.matches("0700").count() != 2 {
            return Err(format!("exactly two partitions should be 0700:\n{table}"));
        }

        for p in &report.partitions {
            let expected = format!("TYPE={}", p.fstype.as_str());
            let probed = shout(
                &mut host,
                "blkid",
                &[
                    "-p",
                    "-o",
                    "export",
                    "-O",
                    &p.offset_bytes.to_string(),
                    &path,
                ],
            );
            if !probed.contains(&expected) {
                return Err(format!(
                    "partition {} should be {expected}; blkid said:\n{probed}",
                    p.number
                ));
            }
        }

        let listing = shout(
            &mut host,
            "mdir",
            &[
                "-i",
                &format!("{path}@@{}", boot_a.offset_bytes),
                "-/",
                "::",
            ],
        );
        for required in REQUIRED_BOOT_FILES {
            // mdir prints `config   txt`, not `config.txt`.
            let stem = required.split('.').next().unwrap_or(required);
            if !listing.contains(stem) {
                return Err(format!("mdir does not list {required}:\n{listing}"));
            }
        }
        if !listing.contains("1980-01-01") {
            return Err(format!("FAT timestamps are not IMAGE_EPOCH:\n{listing}"));
        }
        Ok(())
    }

    #[test]
    #[ignore = "builds two real images and compares them; run with --ignored"]
    fn two_real_builds_agree_byte_for_byte_wherever_this_module_claims_they_do()
    -> Result<(), String> {
        let dir = std::env::temp_dir().join("rivet-image-det");
        let _ = std::fs::create_dir_all(&dir);
        let mut host = CommandHost::new();
        let mut reports = Vec::new();
        let mut paths = Vec::new();
        for n in ["one", "two"] {
            let path = dir.join(format!("{n}.img")).to_string_lossy().into_owned();
            match build(&mut host, &real_request(&path)) {
                Ok(r) => reports.push(r),
                Err(e) => return Err(format!("build {n} failed: {e}")),
            }
            paths.push(path);
        }

        let read_region = |path: &str, offset: u64, len: u64| -> Result<Vec<u8>, String> {
            use std::io::{Read, Seek, SeekFrom};
            let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
            f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; len as usize];
            f.read_exact(&mut buf).map_err(|e| e.to_string())?;
            Ok(buf)
        };

        // The GPT itself, which sits below the first partition.
        let a = read_region(&paths[0], 0, FIRST_USABLE_SECTOR * SECTOR_BYTES)?;
        let b = read_region(&paths[1], 0, FIRST_USABLE_SECTOR * SECTOR_BYTES)?;
        if a != b {
            return Err(s("the GPT differs between two builds"));
        }

        for p in &reports[0].partitions {
            let a = read_region(&paths[0], p.offset_bytes, p.len_bytes)?;
            let b = read_region(&paths[1], p.offset_bytes, p.len_bytes)?;
            let identical = a == b;
            println!(
                "{:9} {:>12} bytes  identical={identical}  claimed_reproducible={}",
                p.label,
                p.len_bytes,
                p.irreproducible_because.is_none()
            );
            if identical != p.irreproducible_because.is_none() {
                return Err(format!(
                    "{} was reported {} but is {}",
                    p.label,
                    if p.irreproducible_because.is_none() {
                        "reproducible"
                    } else {
                        "irreproducible"
                    },
                    if identical { "identical" } else { "different" }
                ));
            }
        }
        Ok(())
    }
}
