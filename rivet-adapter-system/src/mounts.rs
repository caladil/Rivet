//! The kernel's mount table, parsed.
//!
//! **Derived from `uutils`' `uucore::fsext` (MIT)** — specifically `MountInfo`
//! and `read_fs_list`'s Linux path, and above all the octal unescaping in
//! `unescape`, which is the part nobody derives from first principles and
//! which is wrong in most hand-written parsers. `ADR-005` §4.1.5 rules `uucore`
//! out as a dependency — 45 crates and a localization engine for four pure
//! functions — and §4.1.6 rules that where an algorithm is lifted instead, the
//! source file says so and names the module. This is that notice.
//!
//! **What was not taken:** `read_fs_list`'s `getmntent`/`setmntent` route, its
//! `/etc/mtab` fallbacks and its remote-filesystem handling. Rivet has one
//! kernel, no `libc` mount-table iterator behind a port, and no remote
//! filesystems; `/proc/self/mounts` is the only source and it always exists.
//!
//! # `/proc/self/mounts`, and not `/proc/self/mountinfo`
//!
//! `mountinfo` carries more — the mount id, the parent, the subtree root, the
//! propagation flags — and `df` reads it in preference. None of that answers
//! *how full is this filesystem*, and its optional-fields section is a genuine
//! parsing hazard (a variable number of fields terminated by a `-`, which is
//! where naive readers break). `mounts` has six fixed fields and is what
//! `df`'s own fallback reads.
//!
//! The one thing given up by that choice, stated so it is not rediscovered: with
//! `mounts` alone, a **bind mount** of a subtree looks exactly like a mount of
//! the whole filesystem, because the subtree root is a `mountinfo` field. Both
//! report the same usage — which is true, since they are the same filesystem —
//! so nothing this crate reports becomes wrong. Rivet makes no bind mounts
//! today (`rivet_core::mount::stage0_plan`).

use crate::port::{Refused, SystemMedium};

/// Where the kernel publishes what is mounted.
///
/// **Taken from `rivet-core` since 2026-08-28**, when `cmd/rivet-sysd` became a
/// third reader of the same path and `ADR-011` said to stop restating it. The
/// argument for `/proc/self/mounts` over `/proc/mounts` is there, with the rest
/// of the mount vocabulary.
pub const MOUNT_TABLE: &str = rivet_core::mount::MOUNT_TABLE;

/// One line of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    /// The first field: a device node, or a word like `tmpfs` for a filesystem
    /// with no backing device. **Unescaped**, and carried verbatim otherwise.
    pub source: String,
    /// The second field, unescaped.
    pub mount_point: String,
    /// The third field.
    pub fstype: String,
}

/// A line that is not a mount-table entry, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Malformed {
    /// 1-based, so it can be found in the file.
    pub line: usize,
    pub why: String,
}

/// Read the mount table.
///
/// **The whole file in one read**, because it is `procfs`: it has no size, and a
/// reader that made two calls would be describing two different machines.
pub fn read(medium: &mut dyn SystemMedium) -> Result<(Vec<MountEntry>, Vec<Malformed>), Refused> {
    let text = medium.read_file(MOUNT_TABLE)?;
    Ok(parse(&text))
}

/// Parse the table's text.
///
/// **A malformed line is reported, never skipped.** A mount table with a line
/// this parser does not understand is a machine with a filesystem the answer
/// does not mention, and an answer that quietly omits a filesystem says *there
/// is no such filesystem* — which `AGENTS.md` §5 forbids. Nothing has ever
/// produced one; that is exactly why it must not be silent when something does.
pub fn parse(text: &str) -> (Vec<MountEntry>, Vec<Malformed>) {
    let mut entries = Vec::new();
    let mut malformed = Vec::new();
    for (index, line) in text.lines().enumerate() {
        // A blank line is not a defect and not an entry. `procfs` does not
        // produce one; a trailing newline would give `lines()` nothing extra,
        // but a file read from elsewhere in a test might.
        if line.trim().is_empty() {
            continue;
        }
        // The fields are separated by single spaces and the kernel escapes any
        // space *inside* a field, so splitting on whitespace is correct — see
        // `unescape`. Six fields: source, target, type, options, dump, pass.
        let mut fields = line.split_ascii_whitespace();
        match (fields.next(), fields.next(), fields.next()) {
            (Some(source), Some(mount_point), Some(fstype)) => entries.push(MountEntry {
                source: unescape(source),
                mount_point: unescape(mount_point),
                fstype: fstype.to_string(),
            }),
            _ => malformed.push(Malformed {
                line: index + 1,
                why: format!(
                    "not a mount table entry — {MOUNT_TABLE} lines carry at least a source, a \
                     mount point and a type, and this has {}",
                    line.split_ascii_whitespace().count()
                ),
            }),
        }
    }
    (entries, malformed)
}

/// Undo the kernel's escaping of a mount-table field.
///
/// **Derived from `uutils`' `uucore::fsext`.** The kernel's `mangle()` writes a
/// backslash followed by **exactly three octal digits** for the four characters
/// that would otherwise break the format — space, tab, newline and backslash
/// itself — so `/mnt/my card` appears as `/mnt/my\040card`.
///
/// Three properties, each of which a naive implementation gets wrong:
///
/// 1. **Exactly three digits.** `\0401` is a space followed by a `1`, not
///    whatever `\0401` parses as.
/// 2. **A backslash that does not begin a valid escape is a literal
///    backslash.** The kernel would not produce one, and a parser that dropped
///    it would silently rename a path (`AGENTS.md` §5 — refuse rather than
///    guess; here the guess is *keep what was there*, which is the only reading
///    that cannot lose information).
/// 3. **Only bytes below `0x80`.** Higher escapes are not produced by
///    `mangle()`, and treating one as a code point would produce a path that is
///    not the path — so anything that does not fit in a byte is left alone.
fn unescape(field: &str) -> String {
    if !field.contains('\\') {
        return field.to_string();
    }
    let mut out = String::with_capacity(field.len());
    let mut rest = field;
    while let Some(at) = rest.find('\\') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        match octal(after) {
            Some(byte) => {
                out.push(byte);
                rest = &after[3..];
            }
            None => {
                out.push('\\');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The character three octal digits name, or `None` if the next three bytes are
/// not three octal digits naming an ASCII character.
fn octal(after: &str) -> Option<char> {
    let digits = after.get(..3)?;
    if !digits.bytes().all(|b| b.is_ascii_digit() && b < b'8') {
        return None;
    }
    let value = u32::from_str_radix(digits, 8).ok()?;
    // `mangle()` escapes four ASCII characters and nothing else. A value above
    // 0x7f would be one byte of a multi-byte sequence, and turning it into a
    // `char` would produce a different path rather than the one on the machine.
    if value > 0x7f {
        return None;
    }
    char::from_u32(value)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **Read off a running Rivet machine**, 2026-08-24 — `open
    /// /proc/self/mounts` in the guest `tools/qemu/run.sh` boots, with
    /// nothing edited but the trailing newline. `AGENTS.md` §4 wants fixtures
    /// labelled by provenance, and this one is worth the label: it is what PID 1
    /// and the supervisor between them actually produce, in the order they
    /// produce it.
    ///
    /// It is the **emulator's** and not a board's, and the difference is the
    /// whole of `ADR-026` §4's right-hand column. Two things about it would be
    /// different on a card and both are visible here:
    ///
    /// * **`/` is `rootfs`**, the initramfs. On a card it is `erofs` on
    ///   `/dev/mmcblk0p3` or `p4` (`ADR-018` §4).
    /// * **There is no `/state` line at all.** There is no `/dev/mmcblk0` under
    ///   `-M virt`, so PID 1's mount refuses and `/state` is a directory on the
    ///   initramfs — which is exactly why
    ///   `rivet_core::operation::storage`'s `holds` puts all four named
    ///   locations on the `/` row in this loop.
    ///
    /// The `/etc` line is `ro` and that is not the emulator: `ADR-007` §3.5's
    /// runtime projection mounts a read-only tmpfs over it on a card too.
    const READ_OFF_THE_EMULATOR: &str = "\
rootfs / rootfs rw,size=998576k,nr_inodes=249644 0 0
proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0
devtmpfs /dev devtmpfs rw,nosuid,noexec,relatime,size=998576k,nr_inodes=249644,mode=755 0 0
sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0
cgroup2 /sys/fs/cgroup cgroup2 rw,nosuid,nodev,noexec,relatime 0 0
devpts /dev/pts devpts rw,nosuid,noexec,relatime,gid=5,mode=620,ptmxmode=000 0 0
tmpfs /dev/shm tmpfs rw,nosuid,nodev,noexec,relatime 0 0
tmpfs /run tmpfs rw,nosuid,nodev,relatime,mode=755 0 0
tmpfs /tmp tmpfs rw,nosuid,nodev,noexec,relatime,size=65536k 0 0
tmpfs /etc tmpfs ro,nosuid,nodev,noexec,relatime,size=1024k,mode=755 0 0
";

    /// Every line of a real machine's table parses, and the parser agrees with
    /// what the machine said rather than with its own author.
    #[test]
    fn a_real_machines_mount_table_parses_to_every_filesystem_it_had() {
        let (entries, malformed) = parse(READ_OFF_THE_EMULATOR);
        assert!(malformed.is_empty(), "{malformed:?}");
        assert_eq!(entries.len(), 10);

        let points: Vec<&str> = entries.iter().map(|e| e.mount_point.as_str()).collect();
        assert_eq!(
            points,
            [
                "/",
                "/proc",
                "/dev",
                "/sys",
                "/sys/fs/cgroup",
                "/dev/pts",
                "/dev/shm",
                "/run",
                "/tmp",
                "/etc",
            ],
            "order is the table's, and rivet_core::operation::storage depends on it"
        );
        assert_eq!(entries[0].source, "rootfs");
        assert_eq!(entries[0].fstype, "rootfs");
        assert_eq!(entries[4].fstype, "cgroup2");
        // A nested mount point is an ordinary entry here: `/sys/fs/cgroup`
        // inside `/sys` is what makes the longest-match rule in the core's
        // `holder` do work rather than being a formality.
        assert_eq!(entries[4].mount_point, "/sys/fs/cgroup");
        // The options field is deliberately not carried: nothing in the answer
        // is derived from it, and `read_only` comes from the kernel's own
        // filesystem flags instead (`port::Usage`). The `/etc` line is the case
        // that proves it matters — its options say `ro` AND its flags do, and
        // only one of the two is read.
        assert_eq!(entries[9].mount_point, "/etc");
        assert_eq!(entries[9].fstype, "tmpfs");
    }

    /// **The `/state` line is not there, and that is the emulator.** Asserted as
    /// an absence so that a future capture taken from a *board* fails this test
    /// and has to change the fixture's provenance note with it — `ADR-026` §4's
    /// distinction is exactly the kind that goes quiet otherwise.
    #[test]
    fn the_emulators_table_has_no_state_partition_because_there_is_no_card() {
        let (entries, _) = parse(READ_OFF_THE_EMULATOR);
        assert!(
            !entries.iter().any(|e| e.mount_point == "/state"),
            "this fixture is labelled as the emulator's and contains a /state mount"
        );
        assert!(
            !entries.iter().any(|e| e.source.starts_with("/dev/mmcblk")),
            "this fixture is labelled as the emulator's and names a card"
        );
    }

    /// The line a card produces and the emulator cannot: `/state` as its own
    /// `ext4` partition, and the root as the slot's `erofs` (`ADR-018` §4).
    #[test]
    fn a_cards_own_two_filesystems_parse_the_way_the_layout_describes_them() {
        let (entries, malformed) = parse(
            "/dev/mmcblk0p3 / erofs ro,relatime 0 0\n\
             /dev/mmcblk0p5 /state ext4 rw,noatime,nosuid,nodev 0 0\n",
        );
        assert!(malformed.is_empty(), "{malformed:?}");
        assert_eq!(entries[0].fstype, "erofs");
        assert_eq!(entries[1].source, "/dev/mmcblk0p5");
        assert_eq!(entries[1].mount_point, "/state");
        assert_eq!(entries[1].fstype, "ext4");
    }

    /// **The `uutils`-derived escaping, which is the reason to have read that
    /// module.** A space inside a path is `\040`, and a parser that split on
    /// whitespace without unescaping would report a mount point that is not the
    /// one on the machine — or, worse, read the rest of the path as the type.
    #[test]
    fn a_space_in_a_path_is_unescaped_rather_than_splitting_the_line() {
        let (entries, malformed) = parse("/dev/sdb1 /mnt/my\\040card vfat rw 0 0\n");
        assert!(malformed.is_empty(), "{malformed:?}");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].mount_point, "/mnt/my card");
        assert_eq!(entries[0].fstype, "vfat", "the type must not have shifted");
    }

    /// All four characters `mangle()` escapes, in both fields it escapes them
    /// in.
    #[test]
    fn every_character_the_kernel_escapes_comes_back_as_itself() {
        assert_eq!(unescape("a\\040b"), "a b");
        assert_eq!(unescape("a\\011b"), "a\tb");
        assert_eq!(unescape("a\\012b"), "a\nb");
        assert_eq!(unescape("a\\134b"), "a\\b");
        assert_eq!(unescape("/plain/path"), "/plain/path");
        assert_eq!(unescape(""), "");
    }

    /// **Exactly three digits.** `\0401` is a space and a `1`; a greedy reader
    /// gets a different character and loses one.
    #[test]
    fn an_escape_is_exactly_three_digits_long() {
        assert_eq!(unescape("\\0401"), " 1");
        assert_eq!(unescape("\\040\\040"), "  ");
    }

    /// A backslash that begins nothing valid is kept. The kernel does not
    /// produce one, and dropping it would silently rename a path — the reading
    /// that loses information is the one to refuse.
    #[test]
    fn a_backslash_that_is_not_an_escape_survives_as_itself() {
        for (given, want) in [
            ("a\\b", "a\\b"),
            ("a\\", "a\\"),
            ("a\\99b", "a\\99b"),
            ("a\\08b", "a\\08b"),
            ("a\\12", "a\\12"),
        ] {
            assert_eq!(unescape(given), want, "{given}");
        }
    }

    /// Above `0x7f` is left alone: `mangle()` never writes one, and reading it
    /// as a code point would produce a path that is not the path.
    #[test]
    fn an_escape_above_ascii_is_left_alone_rather_than_becoming_a_code_point() {
        assert_eq!(unescape("a\\200b"), "a\\200b");
        assert_eq!(unescape("a\\377b"), "a\\377b");
    }

    /// **A line that is not an entry is reported, not skipped.** An answer that
    /// quietly omitted a filesystem would say there is no such filesystem.
    #[test]
    fn a_line_that_is_not_an_entry_is_reported_with_its_number() {
        let (entries, malformed) =
            parse("rootfs / rootfs rw 0 0\ngarbage\ntmpfs /run tmpfs rw 0 0\n");
        assert_eq!(entries.len(), 2);
        assert_eq!(malformed.len(), 1);
        assert_eq!(malformed[0].line, 2);
        assert!(malformed[0].why.contains(MOUNT_TABLE), "{:?}", malformed[0]);
    }

    /// A blank line is neither an entry nor a defect.
    #[test]
    fn a_blank_line_is_neither_an_entry_nor_a_complaint() {
        let (entries, malformed) = parse("\nrootfs / rootfs rw 0 0\n\n");
        assert_eq!(entries.len(), 1);
        assert!(malformed.is_empty(), "{malformed:?}");
    }

    /// An empty table is an empty answer rather than a failure. It is not a
    /// state Linux can be in, which is why the *caller* says where it looked —
    /// so "nothing was mounted" can be told from "I read the wrong file".
    #[test]
    fn an_empty_table_parses_to_nothing_and_complains_about_nothing() {
        assert_eq!(parse(""), (Vec::new(), Vec::new()));
    }

    /// The path names the namespace-aware form, which is the one that stays
    /// correct if Rivet ever gains a second mount namespace.
    #[test]
    fn the_table_read_is_the_namespace_aware_one() {
        assert_eq!(MOUNT_TABLE, "/proc/self/mounts");
    }
}
