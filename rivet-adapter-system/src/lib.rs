//! **Rivet's observed-state adapter** — the mount table and how full each
//! filesystem is.
//!
//! `ADR-014` §2 names this crate and says what it holds:
//!
//! > `rivet-adapter-system   procfs, sysfs, mounts — observed state`
//!
//! and §5 says when it comes into existence:
//!
//! > the rest are created as the stage that needs them arrives — **the layout is
//! > the destination, not a scaffolding requirement before the first boot.**
//!
//! **What needed it, 2026-08-24:** `rivet get storage` (`ADR-005` §4.1.3), the
//! first operation in the catalogue that reads a fact off the *running* machine
//! rather than off a card or out of a const table. `ADR-006` §8.2's table
//! recorded the gap in as many words — *"anything over observed state: the
//! kernel is the authority (`AGENTS.md` §5) and `ADR-014` §2's
//! `rivet-adapter-system` does not exist"* — and this closes it for one fact.
//!
//! # The shape, which is `rivet-adapter-image`'s
//!
//! ```text
//! rivet_core::operation::Machine     the DOMAIN port — mount points, byte and block counts
//!            ▲  cmd/rivet's PiMachine
//!            │
//! rivet_adapter_system::read         this crate — joins two platform facts, converts nothing
//!            ▲  port::SystemMedium     this crate's own I/O seam — a file and statvfs(3)
//!            │
//! cmd/rivet/src/syscall.rs           the libc wall (ADR-009 §3.4)
//! ```
//!
//! Both of `AGENTS.md` §3.1's seams are in that picture and this crate owns the
//! lower one. [`port`] carries the argument for which is which, and for why
//! **this crate ships no implementor** of its own.
//!
//! # `uucore` was read and is not linked
//!
//! `ADR-005` §4.1.5 rules `uutils`' shared crate out as a dependency: the most
//! minimal ask available pulls 45 crates including the Mozilla Fluent
//! localization engine, non-optionally, and no feature flag removes it. §4.1.6
//! rules what to do instead — *"take the algorithms, with attribution … where
//! Rivet's implementation is derived from `uutils`, the source file says so
//! and names the module"*.
//!
//! Two files here carry that notice, and between them they are the whole of what
//! was taken: [`mounts`] for the mount table's octal escaping, and [`port`]'s
//! `Usage` for `statvfs`'s field semantics. The third piece —`df`'s arithmetic
//! over those fields — is in `rivet_core::operation::storage`, because it is
//! the domain's.

#![forbid(unsafe_code)]

pub mod filesystems;
pub mod mounts;
pub mod port;

pub use filesystems::read;
pub use mounts::{MOUNT_TABLE, MountEntry};
pub use port::{Refused, SystemMedium, Usage};
