//! The mount table plus `statvfs(3)`, as the domain's
//! [`StorageRead`].
//!
//! This is the whole of the crate's job: two platform facts joined, converted
//! into `rivet-core`'s vocabulary, and handed inward. **No arithmetic happens
//! here** — `rivet_core::operation::storage` owns what "used" means, and the
//! port's own documentation carries why that seam is where it is.
//!
//! # Every entry is measured, and every failure is carried
//!
//! A `statvfs` that refuses does not fail the read. `/proc/sys/fs/binfmt_misc`
//! is the ordinary example — an autofs-backed mount that answers `ENOENT` when
//! nothing has triggered it — and a machine that reported *no filesystems* every
//! time one of those was mounted would be useless. So each refusal becomes an
//! [`UnreadableFilesystem`] and
//! the rest of the table is still answered.
//!
//! What is **not** carried on is a mount table that cannot be read at all. That
//! is a machine without `/proc`, which is a machine PID 1 failed to set up
//! (`rivet_core::mount::stage0_plan`), and answering "no filesystems" there
//! would report a state that is not true (`AGENTS.md` §3).

use rivet_core::operation::{MountedFilesystem, StorageRead, UnreadableFilesystem};

use crate::mounts::{self, MOUNT_TABLE};
use crate::port::{Refused, SystemMedium};

/// Read every mounted filesystem and how full it is.
pub fn read(medium: &mut dyn SystemMedium) -> Result<StorageRead, Refused> {
    let (entries, malformed) = mounts::read(medium)?;

    // The table's own defects first, then the filesystems that would not be
    // measured. Deterministic rather than meaningful: an operator reading
    // `unreadable-detail` wants the structural problem before the individual
    // ones, because a garbled table explains the rest and the rest do not
    // explain it.
    let mut unreadable: Vec<UnreadableFilesystem> = malformed
        .into_iter()
        .map(|m| UnreadableFilesystem {
            mount_point: format!("{MOUNT_TABLE} line {}", m.line),
            why: m.why,
        })
        .collect();

    let mut filesystems = Vec::with_capacity(entries.len());
    for entry in entries {
        match medium.statvfs(&entry.mount_point) {
            Ok(usage) => filesystems.push(MountedFilesystem {
                source: entry.source,
                mount_point: entry.mount_point,
                fstype: entry.fstype,
                read_only: usage.read_only,
                block_bytes: usage.block_bytes,
                blocks: usage.blocks,
                blocks_free: usage.blocks_free,
                blocks_available: usage.blocks_available,
                inodes: usage.inodes,
                inodes_free: usage.inodes_free,
            }),
            Err(refused) => unreadable.push(UnreadableFilesystem {
                // The type is carried into the message because it is the thing
                // that usually explains the refusal — `autofs` and `nsfs`
                // answer for reasons that have nothing to do with the card.
                mount_point: entry.mount_point,
                why: format!("{} ({}): {refused}", entry.source, entry.fstype),
            }),
        }
    }

    Ok(StorageRead {
        filesystems,
        unreadable,
        mount_table: MOUNT_TABLE.to_string(),
    })
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::port::Usage;
    use std::collections::BTreeMap;

    /// A machine in two maps: one file and one answer per mount point.
    struct FakeSystem {
        table: Result<String, Refused>,
        usage: BTreeMap<String, Result<Usage, Refused>>,
        statvfs_calls: Vec<String>,
    }

    impl FakeSystem {
        fn with(table: &str) -> Self {
            Self {
                table: Ok(table.to_string()),
                usage: BTreeMap::new(),
                statvfs_calls: Vec::new(),
            }
        }

        fn answering(mut self, at: &str, usage: Usage) -> Self {
            self.usage.insert(at.to_string(), Ok(usage));
            self
        }

        fn refusing_at(mut self, at: &str, refused: Refused) -> Self {
            self.usage.insert(at.to_string(), Err(refused));
            self
        }
    }

    impl SystemMedium for FakeSystem {
        fn read_file(&mut self, path: &str) -> Result<String, Refused> {
            assert_eq!(path, MOUNT_TABLE, "this crate reads one file");
            self.table.clone()
        }

        fn statvfs(&mut self, path: &str) -> Result<Usage, Refused> {
            self.statvfs_calls.push(path.to_string());
            self.usage
                .get(path)
                .cloned()
                .unwrap_or_else(|| Err(Refused::new(2, "No such file or directory")))
        }
    }

    fn usage(blocks: u64, free: u64, available: u64) -> Usage {
        Usage {
            block_bytes: 4096,
            blocks,
            blocks_free: free,
            blocks_available: available,
            inodes: 128,
            inodes_free: 120,
            read_only: false,
        }
    }

    const TWO_LINES: &str = "/dev/mmcblk0p3 / erofs ro,relatime 0 0\n\
                             /dev/mmcblk0p5 /state ext4 rw,noatime 0 0\n";

    /// The whole of what this module does, end to end over the seam.
    #[test]
    fn every_entry_is_measured_and_carried_into_the_domains_vocabulary() {
        let mut medium = FakeSystem::with(TWO_LINES)
            .answering("/", usage(1000, 0, 0))
            .answering("/state", usage(2000, 1800, 1700));
        let read = read(&mut medium).expect("a readable table");

        assert_eq!(read.mount_table, MOUNT_TABLE);
        assert!(read.unreadable.is_empty(), "{:?}", read.unreadable);
        assert_eq!(read.filesystems.len(), 2);

        let state = &read.filesystems[1];
        assert_eq!(state.source, "/dev/mmcblk0p5");
        assert_eq!(state.mount_point, "/state");
        assert_eq!(state.fstype, "ext4");
        assert_eq!(state.blocks, 2000);
        assert_eq!(state.blocks_free, 1800);
        assert_eq!(state.blocks_available, 1700);
        assert_eq!(state.block_bytes, 4096);
        assert_eq!(state.inodes, 128);
        assert_eq!(state.inodes_free, 120);
        assert!(!state.read_only);
    }

    /// **Nothing is converted on the way through.** The counts the platform gave
    /// are the counts the domain gets — the arithmetic is
    /// `rivet_core::operation::storage`'s, and an adapter that pre-computed
    /// `used` would be a second answer to the one question `df` gets wrong.
    #[test]
    fn no_arithmetic_happens_on_this_side_of_the_seam() {
        let mut medium =
            FakeSystem::with("tmpfs /run tmpfs rw 0 0\n").answering("/run", usage(1000, 900, 850));
        let read = read(&mut medium).expect("a readable table");
        let fs = &read.filesystems[0];
        // 100 blocks are in use and 50 are the reserve. If this side had done
        // the subtraction, one of the three would be missing.
        assert_eq!(
            (fs.blocks, fs.blocks_free, fs.blocks_available),
            (1000, 900, 850)
        );
    }

    /// **Order is the table's**, and that is a contract rather than an
    /// incidental: `rivet_core::operation::storage` resolves a path to the
    /// LAST filesystem whose mount point contains it, because a later mount
    /// shadows an earlier one. Sorting here would make it name the shadowed one.
    #[test]
    fn the_filesystems_come_back_in_the_order_the_table_listed_them() {
        let mut medium = FakeSystem::with(
            "tmpfs /run tmpfs rw 0 0\n\
             /dev/mmcblk0p3 / erofs ro 0 0\n\
             /dev/mmcblk0p5 /state ext4 rw 0 0\n",
        )
        .answering("/run", usage(10, 10, 10))
        .answering("/", usage(10, 10, 10))
        .answering("/state", usage(10, 10, 10));
        let read = read(&mut medium).expect("a readable table");
        let points: Vec<&str> = read
            .filesystems
            .iter()
            .map(|f| f.mount_point.as_str())
            .collect();
        assert_eq!(points, ["/run", "/", "/state"]);
    }

    /// **A filesystem that will not answer does not fail the read**, and is not
    /// dropped either. `/proc/sys/fs/binfmt_misc` is the ordinary case and it
    /// must not cost an operator the answer about `/state`.
    #[test]
    fn one_filesystem_that_refuses_is_reported_and_the_rest_are_still_answered() {
        let mut medium = FakeSystem::with(
            "/dev/mmcblk0p5 /state ext4 rw 0 0\n\
             binfmt_misc /proc/sys/fs/binfmt_misc autofs rw 0 0\n",
        )
        .answering("/state", usage(2000, 1800, 1700))
        .refusing_at(
            "/proc/sys/fs/binfmt_misc",
            Refused::new(2, "No such file or directory"),
        );
        let read = read(&mut medium).expect("a readable table");

        assert_eq!(read.filesystems.len(), 1);
        assert_eq!(read.filesystems[0].mount_point, "/state");
        assert_eq!(read.unreadable.len(), 1);
        assert_eq!(read.unreadable[0].mount_point, "/proc/sys/fs/binfmt_misc");
        // What the system said, and what it was — the type is usually the
        // explanation.
        assert!(
            read.unreadable[0].why.contains("autofs"),
            "{:?}",
            read.unreadable[0]
        );
        assert!(
            read.unreadable[0].why.contains("errno 2"),
            "{:?}",
            read.unreadable[0]
        );
    }

    /// A line that is not an entry is carried through as an unreadable one,
    /// naming the file and the line so it can be found.
    #[test]
    fn a_malformed_table_line_reaches_the_answer_rather_than_vanishing() {
        let mut medium = FakeSystem::with("garbage\n/dev/mmcblk0p5 /state ext4 rw 0 0\n")
            .answering("/state", usage(10, 10, 10));
        let read = read(&mut medium).expect("a readable table");
        assert_eq!(read.filesystems.len(), 1);
        assert_eq!(read.unreadable.len(), 1);
        assert!(
            read.unreadable[0].mount_point.contains("line 1"),
            "{:?}",
            read.unreadable[0]
        );
        assert!(
            read.unreadable[0].mount_point.contains(MOUNT_TABLE),
            "{:?}",
            read.unreadable[0]
        );
    }

    /// **A mount table that cannot be read at all is a refusal**, never an empty
    /// answer. A machine with no `/proc` is one PID 1 failed to set up, and
    /// reporting "no filesystems" there would be a stub faking success
    /// (`AGENTS.md` §3).
    #[test]
    fn a_table_that_cannot_be_read_is_a_refusal_rather_than_a_machine_with_nothing_mounted() {
        let mut medium = FakeSystem::with("");
        medium.table = Err(Refused::new(2, "No such file or directory"));
        match read(&mut medium) {
            Err(refused) => {
                assert_eq!(refused.errno, 2);
                assert!(refused.to_string().contains("errno 2"), "{refused}");
            }
            Ok(read) => panic!("an unreadable /proc answered {read:?}"),
        }
        assert!(
            medium.statvfs_calls.is_empty(),
            "a failed table read measured something anyway"
        );
    }

    /// The kernel's own read-only flag reaches the domain. `/system` is
    /// `erofs` and read-only by format (`ADR-018` §3); a `/state` that has gone
    /// read-only mid-write is `ADR-018` §8's dying card, and the two must be
    /// distinguishable from each other by the mount point rather than by the
    /// flag being absent.
    #[test]
    fn the_kernels_read_only_flag_is_carried_rather_than_inferred_from_the_options() {
        let mut usage = usage(1000, 0, 0);
        usage.read_only = true;
        // Note the options field says `rw` and the flag says otherwise: the flag
        // is the authority, which is the whole reason it is read from statvfs.
        let mut medium =
            FakeSystem::with("/dev/mmcblk0p3 / erofs rw,relatime 0 0\n").answering("/", usage);
        let read = read(&mut medium).expect("a readable table");
        assert!(read.filesystems[0].read_only);
    }

    /// Each mount point is measured **once**, and by the path the table gave.
    #[test]
    fn each_mount_point_is_measured_once_and_by_its_own_path() {
        let mut medium = FakeSystem::with(TWO_LINES)
            .answering("/", usage(10, 10, 10))
            .answering("/state", usage(10, 10, 10));
        let _ = read(&mut medium).expect("a readable table");
        assert_eq!(medium.statvfs_calls, ["/", "/state"]);
    }
}
