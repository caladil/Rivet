//! The composition root's storage half: one adapter, one seam, and no decisions.
//!
//! ```text
//! rivet_core::operation::Machine        the DOMAIN port — mount points, byte and block counts
//!            ▲  PiMachine (crate::boot)
//!            │
//! rivet_adapter_system::read            the mount table joined to statvfs(3)
//!            ▲  SystemMedium (here)       the adapter's own I/O seam
//!            │
//! crate::syscall::statvfs  +  std::fs     the libc wall (ADR-009 §3.4)
//! ```
//!
//! `crate::journal` is the same shape for `get log` and this is deliberately its
//! sibling: an operation that reads the world gets an adapter module of its own,
//! and `crate::boot`'s `PiMachine` calls into it rather than growing a third
//! seam. What `PiMachine` holds — a card and pid 1 — is untouched by this, which
//! matters for the reason its own documentation gives: a stop that had to read a
//! filesystem would be a machine that cannot be stopped once its card is dying.
//!
//! # What this module decides, and it is exactly one thing
//!
//! **That `statvfs(2)` is how a filesystem is measured, and `std::fs` is how a
//! `procfs` file is read.** Everything else — which file, how its lines parse,
//! what "used" means, which filesystem holds `/state` — is above this line, in
//! `rivet-adapter-system` and `rivet-core` respectively.
//!
//! # It refuses on a platform that is not Linux, and `get log` does not
//!
//! The two read-only operations differ here and the difference is real rather
//! than an inconsistency. `crate::journal` is `read_to_string` over absolute
//! paths, so a Windows host answers *"there is no journal at /state/log"*, which
//! is **true about that host**. There is no such answer for storage: a Windows
//! machine has filesystems and this build cannot see them, so the honest report
//! is a refusal naming the platform (`AGENTS.md` §3 — *"stubs refuse; they never
//! fake success"*). Reporting "no filesystems" would state something false about
//! a machine that plainly has some.
//!
//! The consequence for the development loop is stated rather than discovered:
//! **`rivet get storage` cannot be exercised end to end on the Windows host**.
//! It runs under WSL, in the emulator, and on a board.

use rivet_core::operation::{MachineError, StorageRead};

/// Read every mounted filesystem, as the domain's port wants it.
#[cfg(target_os = "linux")]
pub fn read() -> Result<StorageRead, MachineError> {
    rivet_adapter_system::read(&mut SyscallSystem).map_err(|refused| {
        MachineError::Refused(format!(
            "this machine's filesystems could not be read: {refused}. \
             {} is the kernel's own mount table and PID 1 mounts /proc \
             (rivet_core::mount); a machine without it is one whose mount phase failed.",
            rivet_adapter_system::MOUNT_TABLE,
        ))
    })
}

/// The same, on a platform this build cannot read.
///
/// **A refusal, never an empty answer** — see this module's documentation.
#[cfg(not(target_os = "linux"))]
pub fn read() -> Result<StorageRead, MachineError> {
    Err(refusal(std::env::consts::OS))
}

/// Why a build that is not Linux cannot answer.
///
/// A separate sentence from `crate::boot::Detached`'s two, because it is a
/// separate reason: an operator told about `mount(2)` and a device tree after
/// asking how full a disk is would go looking for the wrong thing.
#[cfg(any(test, not(target_os = "linux")))]
pub fn refusal(platform: &str) -> MachineError {
    MachineError::Refused(format!(
        "this rivet was built for {platform}, and measuring a filesystem needs Linux's \
         {} and statvfs(3). NOTHING WAS MEASURED — this is not a machine with no filesystems, \
         it is a build that cannot see the ones it has. {ask}",
        rivet_adapter_system::MOUNT_TABLE,
        ask = rivet_core::control::ASK_A_BOARD,
    ))
}

/// `rivet_adapter_system::SystemMedium` over Linux.
///
/// Stateless: it holds nothing, because every fact it deals in is read at the
/// point of use (`AGENTS.md` §5 — *"the kernel is the authority for system
/// state; never cache its verdicts"*, and free space is the most perishable
/// verdict there is).
#[cfg(target_os = "linux")]
pub struct SyscallSystem;

#[cfg(target_os = "linux")]
impl rivet_adapter_system::SystemMedium for SyscallSystem {
    fn read_file(&mut self, path: &str) -> Result<String, rivet_adapter_system::Refused> {
        std::fs::read_to_string(path).map_err(|e| {
            rivet_adapter_system::Refused::new(crate::syscall::errno_of(&e), e.to_string())
        })
    }

    fn statvfs(
        &mut self,
        path: &str,
    ) -> Result<rivet_adapter_system::Usage, rivet_adapter_system::Refused> {
        // The one conversion this module performs, and it is a copy: the wall in
        // `crate::syscall` means `libc::statvfs` cannot travel, so its fields
        // are restated in a struct with no `libc` in it and restated again here
        // in the port's. That is what `ADR-009` §3.4 costs, and it is what it
        // buys — nothing above this line can name a `c_ulong`.
        crate::syscall::statvfs(path)
            .map(|s| rivet_adapter_system::Usage {
                block_bytes: s.block_bytes,
                blocks: s.blocks,
                blocks_free: s.blocks_free,
                blocks_available: s.blocks_available,
                inodes: s.inodes,
                inodes_free: s.inodes_free,
                read_only: s.read_only,
            })
            .map_err(|errno| {
                rivet_adapter_system::Refused::new(
                    errno,
                    std::io::Error::from_raw_os_error(errno).to_string(),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// The refusal a build that cannot measure anything gives must say **that
    /// nothing was measured**, not merely that something went wrong — an
    /// operator who read "could not be read" would reasonably wonder whether a
    /// partial answer had been produced.
    #[test]
    fn a_build_that_cannot_measure_says_so_and_names_the_platform() {
        let refusal = refusal("windows");
        assert!(refusal.why().contains("windows"), "{refusal}");
        assert!(refusal.why().contains("NOTHING WAS MEASURED"), "{refusal}");
        assert!(refusal.why().contains("statvfs"), "{refusal}");
        // And it names the file it would have read, so the refusal is
        // diagnosable rather than a shrug.
        assert!(
            refusal.why().contains(rivet_adapter_system::MOUNT_TABLE),
            "{refusal}"
        );
    }

    /// **The real thing, on whatever host is running the suite.**
    ///
    /// Not a fake: `rivet-adapter-system`'s tests drive one, and repeating
    /// them here would test the fake. What this asserts is the half a fake
    /// cannot — that the syscall seam is wired to a kernel that answers, and
    /// that a machine which obviously has filesystems reports some.
    ///
    /// **What it does NOT settle** (`AGENTS.md` §6): the numbers. The host
    /// running this is a development machine or WSL, not a card; `ADR-026` §4
    /// puts `ADR-018`'s partition layout on real media squarely in the board's
    /// column. What is proven is that the call returns and that its answer is
    /// shaped like a machine.
    #[cfg(target_os = "linux")]
    #[test]
    fn on_linux_the_real_seam_reports_the_filesystems_this_host_actually_has() {
        let read = read().expect("a Linux host has a mount table");
        assert_eq!(read.mount_table, rivet_adapter_system::MOUNT_TABLE);
        assert!(
            !read.filesystems.is_empty(),
            "a running Linux reported no filesystems at all: {read:?}"
        );
        // Every Linux has a root, and it must be measurable.
        let root = read
            .filesystems
            .iter()
            .find(|f| f.mount_point == "/")
            .expect("a running Linux has a root filesystem");
        assert!(!root.fstype.is_empty(), "{root:?}");
        assert!(root.block_bytes > 0, "the block size is zero: {root:?}");
        // The BLOCK counts are consistent with each other, which is the cheap
        // check that the fields were not shuffled on the way through the wall:
        // free blocks cannot exceed the total, and available cannot exceed free.
        for fs in &read.filesystems {
            assert!(
                fs.blocks_free <= fs.blocks,
                "more free blocks than blocks: {fs:?}"
            );
            assert!(
                fs.blocks_available <= fs.blocks_free,
                "more available than free — the reserve is negative: {fs:?}"
            );
        }
        // **The INODE counts are deliberately not checked the same way, and
        // that is a finding rather than a gap.** This assertion existed and
        // failed on the first host it was run against, 2026-08-24: a `9p`
        // filesystem reported 999 inodes and 1 000 000 free. `statvfs` is not
        // a contract a filesystem driver has to honour, so an invariant
        // asserted here would be asserting something about other people's
        // drivers rather than about this code. What Rivet does about it is
        // `rivet_core::operation::storage`'s — it reports both numbers and
        // derives no third — and that is where the regression test for it
        // lives.
    }

    /// A path that is not there refuses with an errno rather than panicking, and
    /// the message carries what the system said rather than a reconstruction of
    /// it (`AGENTS.md` §5).
    #[cfg(target_os = "linux")]
    #[test]
    fn measuring_a_path_that_is_not_there_reports_what_the_system_said() {
        use rivet_adapter_system::SystemMedium as _;
        let got = SyscallSystem.statvfs("/rivet-no-such-path");
        match got {
            Err(refused) => {
                assert_ne!(refused.errno, 0, "no errno was reported: {refused}");
                assert!(!refused.said.is_empty(), "no message: {refused}");
            }
            Ok(usage) => panic!("a path that is not there was measured: {usage:?}"),
        }
    }
}
