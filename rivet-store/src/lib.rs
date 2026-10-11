//! Rivet's persistence to `/state`. `ADR-014` §2.
//!
//! Two files, sharing one crash-safe write:
//!
//! * `/state/config/machine.toml` — the desired-state document ([`store`]).
//! * `/state/clock` — `ADR-021` §3's stored wall clock ([`clock`]), added
//!   2026-08-23. **`/state`, and deliberately not `/state/config`**: `ADR-008`
//!   §3.1 makes that directory the one file an operator edits with effect, and a
//!   clock stamp is machine-derived state rather than configuration.
//!
//! This is the crate `ADR-007` §7.6 and `ADR-004` §6.1 both named
//! as the one missing piece of the maintainer's ruling of 2026-08-23: a `[network]` table
//! could be written on the boot partition, read, validated and counted on the
//! console, and then **dropped**, because nothing persisted a document.
//!
//! ```
//! use rivet_config::Document;
//! use rivet_store::{ConfigStore, Seeding};
//! # use rivet_store::{Errno, Filesystem};
//! # struct Memory(std::collections::BTreeMap<String, Vec<u8>>);
//! # impl Filesystem for Memory {
//! #   fn create_dir_all(&mut self, _p: &str) -> Result<(), Errno> { Ok(()) }
//! #   fn read(&mut self, p: &str) -> Result<Vec<u8>, Errno> {
//! #       self.0.get(p).cloned().ok_or(Errno::ENOENT) }
//! #   fn write(&mut self, p: &str, b: &[u8]) -> Result<(), Errno> {
//! #       self.0.insert(p.to_owned(), b.to_vec()); Ok(()) }
//! #   fn sync_file(&mut self, _p: &str) -> Result<(), Errno> { Ok(()) }
//! #   fn rename(&mut self, f: &str, t: &str) -> Result<(), Errno> {
//! #       let b = self.0.remove(f).ok_or(Errno::ENOENT)?;
//! #       self.0.insert(t.to_owned(), b); Ok(()) }
//! #   fn sync_dir(&mut self, _p: &str) -> Result<(), Errno> { Ok(()) }
//! #   fn remove_file(&mut self, p: &str) -> Result<(), Errno> {
//! #       self.0.remove(p).map(|_| ()).ok_or(Errno::ENOENT) }
//! # }
//! # let mut fs = Memory(Default::default());
//! let store = ConfigStore::at("/state/config");
//! let offered = Document::parse("version = 1\n[network.eth0]\nmethod = \"dhcp\"\n")?;
//!
//! // First boot: nothing is stored, so the provisioning document becomes the
//! // stored one. `ADR-004` §6 step 2.
//! assert_eq!(store.seed(&mut fs, Some(&offered)), Seeding::Seeded);
//! assert_eq!(store.load(&mut fs)?, Some(offered.clone()));
//!
//! // Every boot after: the provisioning file is still on the boot partition
//! // (`ADR-004` §6.1 removes the credential and keeps the address), and it is
//! // NOT applied over a machine that is already configured.
//! assert_eq!(store.seed(&mut fs, Some(&offered)), Seeding::AlreadyConfigured);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # The shape of this crate
//!
//! | Module | Holds |
//! | :-- | :-- |
//! | [`port`] | the syscall seam: the [`Filesystem`] port, [`Errno`], and `StdFilesystem` |
//! | `atomic` | the crash-safe write — four calls whose order is the guarantee, shared by both stores |
//! | [`store`] | the desired-state document: what is stored, and when it is seeded |
//! | [`clock`] | `ADR-021` §3's stored wall clock, and the **floor** that restores it |
//! | `testfs` | a filesystem in a `BTreeMap`, under `cfg(test)` |
//!
//! The split is `AGENTS.md` §3's, and here it earns itself on one list: **the
//! states this code has to be right about are states a working machine is never
//! in.** `/state` unmounted, `/state` full, `ENOSPC` arriving at `fsync` rather
//! than at `write`, a document half-written by a power cut, a card returning
//! `EIO`. None is reachable from a developer's machine and most are not
//! reachable from a board without damaging it, so every one of them is driven
//! through the port by a test instead.
//!
//! # What this crate does not do
//!
//! Said plainly, because `AGENTS.md` §3 forbids a stub that looks further along
//! than it is:
//!
//! * **Only `ADR-007` §3.4's machine layer exists.** Image defaults and the
//!   fleet layer have no store, and **nothing merges layers** — there is no
//!   precedence engine and no per-value provenance. `ADR-007` §5 puts both at
//!   Stage 2–3. See [`store`] for what that costs today.
//! * **It does not remove the credential from the provisioning file.**
//!   `ADR-004` §6 step 3 is a write to the FAT boot partition, which
//!   `cmd/rivet-initd` mounts and unmounts, and it is not this crate's
//!   filesystem. It also has nothing to remove yet: `ADR-007` §7.3 keeps
//!   wireless credentials out of this document entirely — they belong to
//!   `ADR-020`'s encrypted store — so the schema has no field a passphrase could
//!   be read into. **Step 3 is therefore outstanding**, and not yet ruled.
//! * **It holds no secrets and has no `Secret` type.** `ADR-020` §5 puts the
//!   material in `/state/secrets/`, root-owned and mode 0700, referenced by name
//!   from the document. This crate writes `/state/config` and nothing else.
//! * **It does not correct the clock and takes no time protocol.**
//!   [`clock`] persists and restores; `ADR-021` §4's SNTP client is unbuilt and
//!   no crate for it has been taken. So no Rivet machine has ever had a
//!   *correct* clock — only a floor, and a floor seeded from a machine that
//!   started at the epoch is still the epoch. `Origin::Synchronised` exists in
//!   the on-disk format and **nothing writes it**, which [`clock::Origin`] says
//!   at length.
//! * **It does not set a mode on what it writes.** `ADR-018` §4 says
//!   "`/state/config` is root-owned" and gets that from the process's umask and
//!   from PID 1 running as root; nothing here calls `chmod`, because the port
//!   has no such call and adding one for a permission the record does not
//!   specify would be inventing a requirement.
//!
//! # What has not run on hardware
//!
//! **Nothing in this crate has.** `AGENTS.md` §10.5 and §6 apply with full
//! force: every test above the port runs against a fake, and `StdFilesystem`'s
//! own tests run against a temporary directory on a development host or under
//! WSL. That an `ext4` on `/dev/mmcblk0p5` accepts these writes, that the Pi's
//! card honours the cache flush `sync_file` issues, and that a real power cut
//! leaves what [`store::ConfigStore::save`] says it leaves are all in
//! `ADR-026` §4's right-hand column and are untouched.

#![forbid(unsafe_code)]

/// **The crash-safe write, and it is public because a THIRD caller arrived.**
///
/// It was `pub(crate)` while the two callers were both in this crate. On
/// 2026-09-01 the fleet register moved onto a Rivet machine (`ADR-034` §2.2.1),
/// which put a fourth file on `/state` that a power cut can catch mid-write —
/// and the module's own argument for existing applies unchanged to a caller
/// outside it: *"the sequence is four calls whose order is the entire
/// guarantee, and a second copy is a second place for a step to be dropped in a
/// refactor with nothing failing until somebody pulls the power out."*
pub mod atomic;
pub mod clock;
pub mod port;
pub mod store;
#[cfg(test)]
mod testfs;

pub use clock::{CLOCK_FILE, ClockStore, Floor, Origin, Stamp};
pub use port::{Errno, Filesystem};
#[cfg(unix)]
pub use port::StdFilesystem;
pub use store::{
    ConfigStore, IN_PROGRESS_SUFFIX, MACHINE_LAYER_FILE, Operation, Seeding, StoreError,
};
