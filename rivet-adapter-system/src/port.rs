//! **This crate's own I/O seam** — not a domain port, and the difference is
//! `AGENTS.md` §3.1's.
//!
//! > | | **Domain port** | **Adapter's own I/O seam** |
//! > | :-- | :-- | :-- |
//! > | Exists so that | the *domain* can express an operation it cannot perform
//! >   itself | an *adapter* can be tested without the platform it adapts |
//! > | Named in | `rivet-core` | the adapter crate |
//! > | Vocabulary | the domain's | `Errno`, mounts, sockets, file descriptors |
//!
//! [`SystemMedium`] is emphatically the second kind: it carries an errno and a
//! `statvfs(3)` answer field for field. `rivet-core` could not name it without
//! breaking its own rule, which is why the *domain* port —
//! `rivet_core::operation::Machine::read_filesystems` — speaks in mount
//! points, byte counts and block counts instead, and why this crate exists to
//! convert between the two.
//!
//! # This crate ships no implementor, and that is `ADR-014` §3
//!
//! The same posture `rivet-adapter-image`'s `port` module records, for the
//! same reason and with the same expiry condition. `statvfs(2)` has no `std`
//! equivalent, so a real implementor means taking `libc` here and standing a
//! second syscall seam beside the one `cmd/rivet` already owns —
//! `ADR-011`'s second authority, over the same syscalls. `cmd/rivet` supplies
//! the only implementor there is, in `src/syscall.rs`, where every `unsafe`
//! block in that crate already lives behind one wall (`ADR-009` §3.4).
//!
//! **The moment to revisit it** is the moment a process that is not
//! `cmd/rivet` needs to measure a filesystem. `ADR-022` §8 caps the journal at
//! 8 MiB and the supervisor enforces that cap by counting its own files rather
//! than by asking the card how much room is left; the day it asks, this question
//! is open again.

/// What the system said, in the two forms a caller acts on.
///
/// `ADR-006` §4.1's shape one level down: *"an agent branches on the code; a
/// human reads the string"*. Both halves are carried because both are needed and
/// neither can be derived from the other here — the number is stable across
/// locales and platforms, and the string is what the platform said, which
/// `AGENTS.md` §5 requires be reported rather than reconstructed from
/// documentation.
///
/// **Not `rivet_adapter_image::Errno`**, which is the same idea in a sibling
/// crate. `ADR-014` §3 forbids adapter → adapter, so reusing it is not
/// available; a shared one would have to go in `rivet-core`, which is a
/// decision about the core's vocabulary rather than a refactor. The duplication
/// is recorded here rather than resolved, and it is small: this type has no
/// table of names and no comparisons — it carries what it was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    /// The platform's error number, or `0` where the platform gave none.
    pub errno: i32,
    /// What the platform's own error string said.
    pub said: String,
}

impl Refused {
    pub fn new(errno: i32, said: impl Into<String>) -> Self {
        Self {
            errno,
            said: said.into(),
        }
    }
}

impl core::fmt::Display for Refused {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.errno == 0 {
            f.write_str(&self.said)
        } else {
            write!(f, "{} (errno {})", self.said, self.errno)
        }
    }
}

impl std::error::Error for Refused {}

/// One filesystem's usage, exactly as `statvfs(3)` reports it.
///
/// **No arithmetic has been done to these**, deliberately. The domain owns what
/// "used" means — `rivet_core::operation::Machine::read_filesystems` carries
/// the argument, and it is the one thing about `df` that is easy to get wrong —
/// so this seam hands over the counts and converts nothing.
///
/// **Derived from `uutils`' `uucore::fsext` (MIT), specifically `FsUsage`.**
/// `ADR-005` §4.1.5 rules that crate out as a dependency and §4.1.6 requires
/// this attribution where its algorithms are lifted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    /// The size of the block [`Self::blocks`] counts.
    ///
    /// **`f_frsize`, falling back to `f_bsize` when it is zero** — that fallback
    /// is `uucore::fsext`'s and it is the reason to read that module rather than
    /// the man page: the two fields are usually equal, are not always, and are
    /// zero on some filesystems. Using the wrong one scales every number a
    /// caller derives.
    pub block_bytes: u64,
    /// `f_blocks` — total blocks.
    pub blocks: u64,
    /// `f_bfree` — free blocks, **including the reserve**.
    pub blocks_free: u64,
    /// `f_bavail` — blocks available to an unprivileged caller.
    pub blocks_available: u64,
    /// `f_files` — total inodes, or `0` where the filesystem has no fixed
    /// number of them.
    pub inodes: u64,
    /// `f_ffree` — free inodes.
    pub inodes_free: u64,
    /// `f_flag & ST_RDONLY`.
    ///
    /// Read from the *filesystem's* flags rather than from the mount option
    /// string, because the two can differ: a card that has gone read-only
    /// mid-write is `ADR-018` §8's failure, and it changes this flag without
    /// rewriting anything an operator typed.
    pub read_only: bool,
}

/// The platform this crate adapts, as narrowly as it can be described.
///
/// Two methods, both read-only. **Nothing on this trait changes a machine**, and
/// that is a property rather than an accident: observed state is observed
/// (`ADR-014` §2 — *"procfs, sysfs, mounts — observed state"*), and an adapter
/// that could write would be one whose tests have to prove that it did not.
pub trait SystemMedium {
    /// Read a whole file as text.
    ///
    /// The one file this crate reads is [`crate::mounts::MOUNT_TABLE`], and it
    /// is a `procfs` file — so it has no size, cannot be `stat`ed usefully, and
    /// must be read in one pass. An implementor that seeked would get a
    /// different mount table between one read and the next.
    fn read_file(&mut self, path: &str) -> Result<String, Refused>;

    /// `statvfs(3)` on a path.
    ///
    /// Answers about **the filesystem the path is on**, not about the path, so
    /// a mount point is what a caller passes.
    fn statvfs(&mut self, path: &str) -> Result<Usage, Refused>;
}
