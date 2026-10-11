//! Item 2's second half: write the provisioning document to `/state/config`.
//!
//! `ADR-004` §6's sequence has three steps and PID 1 performed only the first:
//!
//! > 1. PID 1 mounts the boot partition and reads `rivet.toml`.
//! > 2. The passphrase is read, the PMK derived, and **the result written to the
//! >    persistent configuration**.
//! > 3. The credential is then removed from the provisioning file.
//!
//! [`crate::provision`] is step 1. This module is step 2, and step 3 is still
//! outstanding — see *What is not here* below.
//!
//! # Why this is PID 1's, which needed an argument and got one
//!
//! `ADR-017` §3.1.5 is "deliberately short, because everything on this list runs
//! in a process whose failure panics the kernel", and §3.1.7 was written as a
//! formal amendment for adding **one filesystem** to it. Adding to that list is
//! not a thing to do quietly, so the amendment for this is `ADR-017` §3.1.9,
//! dated 2026-08-23, and its argument is summarised here.
//!
//! **The journal argument does not carry this one, and that is worth saying
//! first**, because it is the argument §3.1.7 used and reaching for it again
//! would be the easy mistake. That argument is that whatever mounts `/state`
//! must sit *below* the journal, since the failure to mount `/state` is the one
//! event that cannot be journalled — journalling it is what has just failed
//! (`ADR-017` §8 item 2). It does not generalise to *writing a file* on a
//! `/state` that is already mounted: by then the journal has somewhere to go, so
//! a failed seed is perfectly journallable and the supervisor could report it.
//!
//! **What decides it is that step 1 and step 2 cannot be separated.** The
//! document exists only inside the process that read it. Moving step 2 above PID
//! 1 means one of two things, and both are worse than growing the list:
//!
//! * **A second process mounts the boot partition and parses the file again.**
//!   That is a second reader of the one file on the machine that holds a
//!   plaintext wireless passphrase (`ADR-004` §6), and a second thing that would
//!   have to agree about when step 3 removes it. `ADR-020` §3 puts "secrets leak
//!   into logs and errors" among the failures that will actually occur; halving
//!   the number of processes that ever hold one is the cheap side of that.
//! * **Or the document crosses a process boundary.** Through `argv`, where it
//!   would be world-readable in `/proc`; or through a pipe or a descriptor,
//!   which is machinery that does not exist and would have to be invented for
//!   one file. The obvious channel — a file both processes can read — is
//!   `/state/config` itself, which is what is being seeded.
//!
//! **And the cost to the process that must not fail is four syscalls.**
//! `crate::confirm` already does exactly this shape of work on the boot
//! partition: write a temporary file, `sync`, rename it over the target, `sync`
//! again. Nothing new is added to [`crate::kernel::Kernel`] except
//! `remove_file`, and the decisions all live in `rivet-store`, above a port,
//! where they are driven by tests rather than by a board.
//!
//! **The counter-precedent, stated rather than hidden.** `ADR-018` §10.3 refused
//! PID 1 the job of growing `/state` on first boot, in as many words: *"It must
//! not be PID 1's job. `ADR-017` §3.1.5 is a complete and deliberately short
//! list, and this is not on it."* That refusal stands and this does not weaken
//! it. Growing `/state` is a GPT rewrite and an `ext4` resize — a mechanism
//! ~~`ADR-018` §10.3 records as not even surveyed yet~~ — and it has no
//! dependency on anything PID 1 already holds. The list resists *machinery*, and
//! it resists work that could equally be done above it. This is neither.
//!
//! *Corrected 2026-08-24, in the direction `AGENTS.md` §7 warns about least
//! often — an absence that stopped being true.* **The survey was done**
//! (`ADR-018` §10.7) and the mechanism is built, in
//! `rivet_adapter_image::grow`. **The paragraph's conclusion is unchanged and
//! is now better supported than when it was written**: the survey chose the
//! kernel's own `EXT4_IOC_RESIZE_FS`, which requires `/state` to be **mounted**
//! — and mounting it is PID 1's item 1. So everything the operation needs
//! exists only after PID 1's first job is finished, which is precisely §3.1.8's
//! test for work that belongs above. `ADR-018` §10.7.3 rules it
//! `cmd/rivet-sysd`'s, and records that `ADR-017` §3.1.5's list does
//! **not** grow for it.
//!
//! # A failure here never blocks the boot confirmation
//!
//! Same ruling as the `/state` mount, and the same reason (`ADR-017` §3.1.7,
//! `ADR-018` §8): **a rollback cannot repair a `/state` that an update never
//! touches.** Withholding the confirmation would roll back to the previous
//! image, which would find the same full or unwritable card, and the machine
//! would alternate between two healthy images for ever without addressing the
//! fault.
//!
//! Note the asymmetry with [`crate::provision`], which *does* withhold the
//! confirmation for a document it could not parse. That fault is on the boot
//! partition, which an update **does** replace — so there, rolling back is a
//! remedy. Here it is not. The rule is not "configuration failures are fatal" or
//! "they are not"; it is `ADR-018` §5's, that a boot is withheld when the
//! rollback can fix what is wrong.
//!
//! # What is not here
//!
//! * **`ADR-004` §6 step 3 — removing the credential from the provisioning
//!   file — is outstanding.** It is not merely unimplemented: it has nothing to
//!   remove. `ADR-007` §7.3 keeps wireless credentials out of the configuration
//!   document entirely, putting the PMK in `ADR-020`'s encrypted store, so the
//!   schema declares no key a passphrase could be written under and a
//!   `rivet.toml` carrying one is refused at parse time as an unknown key.
//!   Step 3 becomes implementable when `ADR-020`'s store and a `Secret`-typed
//!   schema key exist, and not before.
//! * ~~**Nothing reads the stored document yet.** `cmd/rivet-netd` takes a
//!   `--document` path from whoever starts it, and `rivet-sysd` does not
//!   start it.~~ **Closed 2026-08-23, the same day**: `cmd/rivet-sysd`'s
//!   `netd` service definition starts it pointed at
//!   `/state/config/machine.toml`, via `ConfigStore::machine_path()`.

use rivet_config::Document;
use rivet_store::{ConfigStore, Errno as StoreErrno, Filesystem, Seeding};

use crate::kernel::{Errno, Kernel};
use crate::provision::Provisioning;

/// `rivet-store`'s filesystem port, over PID 1's syscall seam.
///
/// **Two ports rather than one, on purpose.** `rivet-store` has to be usable
/// by `cmd/rivet` and by anything else that reads a document, none of which
/// has PID 1's `Kernel`; and PID 1's `Kernel` must not grow a dependency on a
/// crate's trait. So the store declares what it needs and this adapts.
///
/// # `fsync` becomes `sync(2)`, which is stronger and not weaker
///
/// [`Kernel`] has no per-file or per-directory flush and is not gaining one.
/// It has `sync`, which is `sync(2)` — writeback of *every* filesystem, waited
/// on — and that is a superset of what
/// [`Filesystem::sync_file`] and
/// `sync_dir` ask for. `crate::confirm` already commits `autoboot.txt` this way.
///
/// Two consequences, stated rather than left to be discovered:
///
/// * It is a bigger hammer than `fsync(2)`, and here it costs nothing: the seed
///   happens once, early in a boot, when almost nothing else is dirty.
/// * `sync(2)` cannot fail, so **PID 1 can never report
///   [`rivet_store::StoreError::NotDurable`]**. That variant is reachable from
///   a caller using `rivet_store::StdFilesystem`, and not from here.
struct KernelFs<'a> {
    k: &'a mut dyn Kernel,
}

/// PID 1's errnos come from `libc` on the target and the store's constants are
/// Linux's own; on `aarch64-unknown-linux-musl` they are the same numbers, which
/// is what makes this a renaming rather than a translation.
const fn errno(e: Errno) -> StoreErrno {
    StoreErrno(e.0)
}

impl Filesystem for KernelFs<'_> {
    fn create_dir_all(&mut self, path: &str) -> Result<(), StoreErrno> {
        self.k.mkdir_p(path).map_err(errno)
    }

    fn read(&mut self, path: &str) -> Result<Vec<u8>, StoreErrno> {
        self.k.read_bytes(path).map_err(errno)
    }

    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), StoreErrno> {
        self.k.write_file(path, bytes).map_err(errno)
    }

    fn sync_file(&mut self, _path: &str) -> Result<(), StoreErrno> {
        self.k.sync();
        Ok(())
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), StoreErrno> {
        self.k.rename(from, to).map_err(errno)
    }

    fn sync_dir(&mut self, _path: &str) -> Result<(), StoreErrno> {
        self.k.sync();
        Ok(())
    }

    fn remove_file(&mut self, path: &str) -> Result<(), StoreErrno> {
        self.k.remove_file(path).map_err(errno)
    }
}

/// Which document, if any, the boot partition offered.
///
/// Only [`Provisioning::Loaded`] carries one. A refused document is deliberately
/// **not** offered: `crate::provision` has already withheld the boot
/// confirmation for it, and storing something the machine refused to understand
/// would be the discarded-input failure `ADR-007` §3.6.5 forbids, running
/// backwards.
fn offered(provisioning: &Provisioning) -> Option<&Document> {
    match provisioning {
        Provisioning::Loaded(document) => Some(document),
        Provisioning::Absent { .. }
        | Provisioning::Unreadable { .. }
        | Provisioning::Refused { .. } => None,
    }
}

/// Item 2's second half, end to end.
///
/// Returns the outcome so that a caller can report on it; PID 1's own loop does
/// not read it, because a seeding fault changes nothing about what PID 1 will
/// certify (see the module documentation).
pub fn seed_configuration(k: &mut dyn Kernel, provisioning: &Provisioning) -> Seeding {
    let Some(document) = offered(provisioning) else {
        // Nothing was offered, so there is nothing to say and no reason to touch
        // `/state` at all. The commonest boot on a running machine takes this
        // branch, and a line on every one of them would teach the reader of a
        // serial console to skip the console.
        return Seeding::NothingOffered;
    };

    let store = ConfigStore::at_default();
    k.say(&format!(
        "rivet-initd: storing the provisioning document at {} (ADR-004 §6 step 2)",
        store.machine_path()
    ));

    // The document is cloned out of the borrow so the kernel can be lent to the
    // filesystem adapter. It is a handful of small strings.
    let document = document.clone();
    let outcome = {
        let mut fs = KernelFs { k };
        store.seed(&mut fs, Some(&document))
    };

    k.say(&format!("  {}", outcome.summary()));
    if outcome.is_fault() {
        k.say("  the boot is still confirmed: an A/B rollback cannot repair /state, which an");
        k.say("  update never touches (ADR-018 §8). The card wants inspecting.");
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkernel::{FakeKernel, Script};
    use rivet_store::{MACHINE_LAYER_FILE, Operation, StoreError};

    /// A passphrase-shaped value, so the console can be checked for leakage.
    /// `AGENTS.md` §8.3 — a test fixture, not a real secret.
    const NOT_A_REAL_SECRET: &str = "hunter2-correct-horse-battery";

    const STORED: &str = "/state/config/machine.toml";
    const IN_PROGRESS: &str = "/state/config/machine.toml.new";

    /// **This crate denies `panic!`, `unwrap` and `expect` in its tests too**
    /// (see `main.rs`), because everything here compiles into the process whose
    /// panic is a kernel panic. So a fixture that will not parse becomes a
    /// `Refused` rather than an assertion — and every caller asserts on the
    /// outcome it expects, so a broken fixture fails the test loudly instead of
    /// passing quietly.
    fn loaded(src: &str) -> Provisioning {
        match Document::parse(src) {
            Ok(document) => Provisioning::Loaded(document),
            Err(e) => Provisioning::Refused {
                detail: e.to_string(),
            },
        }
    }

    fn provisioned() -> Provisioning {
        loaded(
            "version = 1\n\
             [network.eth0]\nmethod = \"static\"\naddress = \"192.168.1.50/24\"\n\
             gateway = \"192.168.1.1\"\n",
        )
    }

    /// A `FakeKernel` whose `/state/config` holds nothing — a first boot.
    fn first_boot() -> FakeKernel {
        FakeKernel::new()
    }

    // ---- what is offered ---------------------------------------------------

    #[test]
    fn only_a_document_that_was_understood_is_offered_to_the_store() {
        assert!(offered(&provisioned()).is_some());
        for other in [
            Provisioning::Absent { why: "none".into() },
            Provisioning::Unreadable { errno: Errno::EIO },
            Provisioning::Refused {
                detail: "no".into(),
            },
        ] {
            assert!(offered(&other).is_none(), "{other:?}");
        }
    }

    /// The commonest boot on a running machine, and it must be silent and must
    /// not touch `/state`.
    #[test]
    fn no_provisioning_document_means_no_write_and_no_console_line() {
        let mut k = first_boot();
        let got = seed_configuration(&mut k, &Provisioning::Absent { why: "none".into() });
        assert_eq!(got, Seeding::NothingOffered);
        assert!(k.lines.is_empty(), "{:?}", k.lines);
        assert!(k.write_calls.is_empty());
        assert!(k.mkdir_calls.is_empty());
    }

    // ---- the first boot ----------------------------------------------------

    /// `ADR-004` §6 step 2, at last: the sequence PID 1 actually performs.
    #[test]
    fn a_first_boot_writes_syncs_renames_and_syncs_again() {
        let mut k = first_boot();
        let got = seed_configuration(&mut k, &provisioned());
        assert_eq!(got, Seeding::Seeded);

        assert_eq!(k.mkdir_calls, vec!["/state/config".to_string()]);
        let written = k.write_calls.first().map(|(p, _)| p.clone());
        assert_eq!(written, Some(IN_PROGRESS.to_string()));
        assert_eq!(
            k.rename_calls,
            vec![(IN_PROGRESS.to_string(), STORED.to_string())]
        );
        // Once for the file, once for the directory entry. `crate::confirm`
        // syncs twice around its rename for the same reason.
        assert_eq!(k.sync_calls, 2, "once before the rename and once after");
        assert!(
            k.remove_calls.is_empty(),
            "nothing failed, so nothing is cleaned up"
        );
    }

    /// The bytes that reach the card are `ADR-007` §3.1's canonical form of the
    /// document that was read, and they parse back to it.
    #[test]
    fn what_is_written_is_the_canonical_document_and_parses_back_to_it() {
        let mut k = first_boot();
        let provisioning = provisioned();
        let _ = seed_configuration(&mut k, &provisioning);
        let bytes = k
            .write_calls
            .first()
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        let text = String::from_utf8(bytes).unwrap_or_default();
        let back = Document::parse(&text);
        assert_eq!(
            back.as_ref().ok(),
            offered(&provisioning),
            "what PID 1 wrote does not read back as what it was given: {text:?} gave {back:?}"
        );
    }

    #[test]
    fn the_console_says_where_the_document_went_and_that_it_is_the_first_boot() {
        let mut k = first_boot();
        let _ = seed_configuration(&mut k, &provisioned());
        let console = k.lines.join("\n");
        assert!(console.contains(MACHINE_LAYER_FILE), "{console}");
        assert!(console.contains("first boot"), "{console}");
    }

    /// The count-not-value rule the console has everywhere else. The document
    /// comes from the file that carries a wireless passphrase (`ADR-004` §6) and
    /// a console is a log (`ADR-020` §4).
    #[test]
    fn the_console_never_carries_the_documents_values() {
        let mut k = first_boot();
        let _ = seed_configuration(&mut k, &provisioned());
        let console = k.lines.join("\n");
        assert!(!console.contains("192.168.1.50"), "{console}");
        assert!(!console.contains(NOT_A_REAL_SECRET), "{console}");
    }

    // ---- every boot after --------------------------------------------------

    /// The whole point of `ConfigStore::seed`: `ADR-004` §6.1 leaves the
    /// `[network]` table on the boot partition for ever, so applying it on every
    /// boot would make that file authoritative and discard what the operator has
    /// since set (`ADR-011` §3).
    #[test]
    fn a_machine_that_is_already_configured_is_not_overwritten() {
        let mut k = first_boot();
        k.files_mut().insert(
            STORED.to_string(),
            Script::always(Ok(
                "version = 1\n[network.eth0]\nmethod = \"dhcp\"\n".to_string()
            )),
        );
        let got = seed_configuration(&mut k, &provisioned());
        assert_eq!(got, Seeding::AlreadyConfigured);
        assert!(
            k.write_calls.is_empty(),
            "nothing may be written: {:?}",
            k.write_calls
        );
        assert!(k.rename_calls.is_empty());
        assert!(!got.is_fault(), "the steady state is not a fault");
        assert!(
            k.lines.iter().any(|l| l.contains("NOT applied")),
            "the console must say it was read and not applied: {:?}",
            k.lines
        );
    }

    // ---- the failure paths -------------------------------------------------

    /// A `/state` that never mounted: the mount point is a directory on the
    /// read-only root, so `mkdir` is refused with `EROFS` (`ADR-018` §3).
    #[test]
    fn an_unmounted_state_fails_the_seed_loudly_and_still_confirms_the_boot() {
        let mut k = first_boot();
        k.mkdir_results = Script::always(Err(Errno::EROFS));
        let got = seed_configuration(&mut k, &provisioned());
        assert!(
            matches!(
                &got,
                Seeding::Failed {
                    why: StoreError::Io {
                        op: Operation::CreateDirectory,
                        ..
                    }
                }
            ),
            "{got:?}"
        );
        assert!(got.is_fault());
        let console = k.lines.join("\n");
        assert!(console.contains("/state is not mounted"), "{console}");
        assert!(console.contains("still confirmed"), "{console}");
        assert!(console.contains("ADR-018 §8"), "{console}");
    }

    /// A full card. The temporary file is cleared, because on a full `/state` it
    /// is the thing occupying the space.
    #[test]
    fn a_full_state_fails_the_seed_and_clears_the_file_that_is_filling_it() {
        let mut k = first_boot();
        k.write_results = Script::always(Err(Errno::ENOSPC));
        let got = seed_configuration(&mut k, &provisioned());
        assert!(got.is_fault());
        assert!(
            got.summary().contains("/state is full"),
            "{}",
            got.summary()
        );
        assert_eq!(k.remove_calls, vec![IN_PROGRESS.to_string()]);
        assert!(k.rename_calls.is_empty(), "nothing may be committed");
    }

    /// A card returning `EIO` on the read. `AGENTS.md` §5 — this must not be
    /// mistaken for "unconfigured", because the machine would then be
    /// re-provisioned over a card fault.
    #[test]
    fn a_store_that_cannot_be_read_is_refused_rather_than_written_over() {
        let mut k = first_boot();
        k.blobs_mut()
            .insert(STORED.to_string(), Script::always(Err(Errno::EIO)));
        let got = seed_configuration(&mut k, &provisioned());
        assert!(matches!(&got, Seeding::Refused { .. }), "{got:?}");
        assert!(k.write_calls.is_empty());
        assert!(
            got.summary().contains("ADR-018 §9 item 3"),
            "the refusal must name the open item: {}",
            got.summary()
        );
    }

    /// A commit refused at the rename leaves whatever was there before. It must
    /// be reported and it must not stop the boot.
    #[test]
    fn a_refused_commit_is_reported_and_survived() {
        let mut k = first_boot();
        k.rename_results = Script::always(Err(Errno::EIO));
        let got = seed_configuration(&mut k, &provisioned());
        assert!(
            matches!(
                &got,
                Seeding::Failed {
                    why: StoreError::Io {
                        op: Operation::Rename,
                        ..
                    }
                }
            ),
            "{got:?}"
        );
        assert!(k.lines.iter().any(|l| l.contains("still confirmed")));
    }

    /// Every failure at once. PID 1 must reach the end of this function whatever
    /// the card does — the same standing rule as every other phase.
    #[test]
    fn every_failure_is_survived_and_none_of_them_propagates() {
        let mut k = first_boot();
        k.mkdir_results = Script::always(Err(Errno::EROFS));
        k.write_results = Script::always(Err(Errno::EROFS));
        k.rename_results = Script::always(Err(Errno::EROFS));
        k.remove_results = Script::always(Err(Errno::EROFS));
        let got = seed_configuration(&mut k, &provisioned());
        assert!(got.is_fault());
        assert!(!k.lines.is_empty(), "it must say something");
    }
}
