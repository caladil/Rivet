//! Loading and saving the machine layer of the desired-state document.
//!
//! Everything here is a decision, and every one of them is reachable from a test
//! with no `/state` and no board — see [`crate::port`] for the seam that makes
//! that true.
//!
//! # Which layer this is
//!
//! `ADR-007` §3.4 stacks four layers with strict precedence:
//!
//! | Layer | Source | Persisted | Here |
//! | :-- | :-- | :-- | :-- |
//! | 1. Schema defaults | the Rust types | n/a | `rivet_config::Document::default()`, and it needs no store because it *is* the types |
//! | 2. Image defaults | baked into the image | yes | **not built** |
//! | 3. Fleet | shared across the operator's machines | yes | **not built** |
//! | 4. Machine | this specific machine | yes | **this module** |
//!
//! **The machine layer alone, and nothing is stubbed to look like the others.**
//! `ADR-007` §5 puts layering with per-value provenance at Stage 2–3 and says
//! why deferring it is cheap: "layers can be added if values are typed", which
//! is the property `rivet-config` exists to establish. A fleet layer that
//! parsed a file nobody writes and merged nothing would be `AGENTS.md` §3's
//! stub that fakes success — it would answer "what is the effective value" with
//! a number that had been through no precedence at all.
//!
//! What that costs, stated rather than buried: `rivet config get X --explain`
//! (`ADR-007` §3.4) cannot say `overrode: image default` yet, because there is
//! no image default to override. One layer's value is trivially its own
//! provenance, and this module does not pretend to more.
//!
//! # What is on the card
//!
//! ```text
//! /state/config/machine.toml       the machine layer  (ADR-007 §3.4 layer 4)
//! /state/config/machine.toml.new   a save in progress (see `ConfigStore::save`)
//! ```
//!
//! **The file name is derived, not quoted.** `ADR-008` §3.1 rules the
//! *directory* — "Rivet's own configuration lives at `/state/config`" — and
//! `ADR-017` §3.1.7 confirms it is a directory by listing it beside
//! `/state/home` and `/state/ext`. Nothing in the record names a file inside it.
//! `machine.toml` is chosen so that §3.4's other persisted layers have obvious
//! neighbours (`image.toml`, `fleet.toml`) rather than needing this one renamed
//! when they arrive — which `AGENTS.md` §2 rule 2's spirit argues for, and which
//! costs nothing today.
//!
//! The `.new` suffix follows `cmd/rivet-initd`'s `confirm` module, which
//! already writes `autoboot.txt.new` and renames it over `autoboot.txt`. One
//! convention for one operation.

use rivet_config::Document;
use rivet_core::state::CONFIG_DIR;

use crate::port::{Errno, Filesystem};

/// The machine layer's file name inside [`CONFIG_DIR`]. See the module docs for
/// why it is named rather than being the directory's only file.
pub const MACHINE_LAYER_FILE: &str = "machine.toml";

/// What a save in progress is called. Derived from the target rather than a
/// second constant, so the two cannot drift apart; see [`ConfigStore::save`].
pub const IN_PROGRESS_SUFFIX: &str = ".new";

/// Which step of a store operation a failure came from.
///
/// Named rather than left to a message string, so that a caller can act on the
/// difference — `AGENTS.md` §5 and `ADR-006` §8 item 4, a typed error rather
/// than prose to match on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Creating `/state/config`.
    CreateDirectory,
    /// Reading the stored document.
    Read,
    /// Writing the new document to its temporary path.
    Write,
    /// Flushing the new document to the medium.
    SyncFile,
    /// Renaming the temporary path over the stored one — **the commit**.
    Rename,
    /// Flushing the directory, so the rename survives a power cut.
    SyncDirectory,
}

impl Operation {
    /// What the step was trying to do, for a message an operator reads at a
    /// serial console.
    ///
    /// **Reworded 2026-08-23, from a defect a real run showed.** These said
    /// "the configuration" until [`crate::clock`] began sharing them, and a
    /// board then reported *"cannot write the new configuration at
    /// /state/clock.new"* — which names the wrong file's purpose to the one
    /// person who has to act on it. One shared vocabulary cannot be specific to
    /// two files without lying about one of them, so the step is named and the
    /// **path** says which file it is; every message these appear in carries it.
    pub const fn describe(self) -> &'static str {
        match self {
            Self::CreateDirectory => "create the directory it goes in",
            Self::Read => "read the stored file",
            Self::Write => "write the new contents",
            Self::SyncFile => "flush the new contents to the card",
            Self::Rename => "commit the new contents",
            Self::SyncDirectory => "flush the directory entry to the card",
        }
    }
}

/// Every way the store can refuse.
///
/// Closed on purpose (`ADR-006` §6.4). **No variant carries the document's
/// content**, for the reason [`rivet_config::ConfigError`]'s own module gives
/// at length: the provisioning document this store is seeded from is read from a
/// file that also holds a wireless passphrase (`ADR-004` §6), and a diagnostic
/// that quotes the bytes it failed on is a secret in a log (`ADR-020` §4,
/// `AGENTS.md` §8.3). `Refused` carries a `ConfigError`'s *message*, which is
/// built from field names and positions and is structurally unable to carry a
/// value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreError {
    /// The filesystem refused.
    Io {
        /// The path, which is composed from [`CONFIG_DIR`] rather than taken
        /// from a document.
        path: String,
        /// Which step.
        op: Operation,
        /// What the platform said.
        errno: Errno,
    },

    /// The stored bytes are not UTF-8, so there is no text to parse.
    ///
    /// `ADR-018` §8's damaged `/state`, arriving in the form a half-erased flash
    /// block actually takes. The byte offset is carried and the bytes are not.
    NotText {
        path: String,
        /// Where the first invalid sequence starts, in bytes from the beginning
        /// of the file.
        at: usize,
    },

    /// The stored text is not a document this build accepts.
    ///
    /// Distinct from [`StoreError::NotText`] because the repair is different: a
    /// document from a newer schema version is this, and so is a file something
    /// else wrote into `/state/config`.
    Refused {
        path: String,
        /// [`rivet_config::ConfigError`]'s own words. See the type
        /// documentation for why this cannot carry the document.
        detail: String,
    },

    /// The document is in place and was not flushed to the medium.
    ///
    /// **Not the same as a failed save**, and the distinction is the whole
    /// reason it is its own variant: a reader will see the new document, and a
    /// power cut in the next few seconds may take it away again. Reporting this
    /// as a failure would tell an operator that nothing happened, which is
    /// false.
    NotDurable { path: String, errno: Errno },
}

impl StoreError {
    /// The likely cause, where the errno has one worth naming.
    ///
    /// Two of these are the failures a Rivet machine will actually produce,
    /// and neither is obvious from the errno alone at 2am on a serial console.
    pub fn likely_cause(&self) -> Option<&'static str> {
        let errno = match self {
            Self::Io { errno, .. } | Self::NotDurable { errno, .. } => *errno,
            _ => return None,
        };
        Some(match errno {
            // **TWO causes, and naming only the first sent a reader to a
            // healthy subsystem.** Corrected 2026-09-04 after `PLAN.md` §127.2
            // read this message off a running board where `/state` was mounted
            // perfectly well: the `EROFS` came from `cmd/rivet-sysd`'s own
            // `Filesystem` adapter refusing by policy, and the sentence sent
            // whoever read it to pid 1's mount records, which were fine.
            //
            // This function is called `likely_cause` and the wording now
            // matches that: it offers, and it says how to tell the two apart,
            // rather than asserting the one that is usually right.
            Errno::EROFS => {
                "the filesystem refused the write. Either /state is not mounted — the mount \
                 point is then a directory on the read-only root, and PID 1's mount phase \
                 will have said why (ADR-018 §3) — or the caller was handed a Filesystem \
                 that refuses writes by policy, which is what cmd/rivet-sysd does for \
                 everything but the last-good copy. `rivet get storage` says which: if \
                 /state is listed as mounted, it is the second"
            }
            Errno::ENOSPC | Errno::EDQUOT => {
                "/state is full. It is the only writable filesystem on this machine and every \
                 write goes there (ADR-018 §4, §7)"
            }
            Errno::EIO => "the card returned an I/O error. This is hardware (ADR-018 §7)",
            _ => return None,
        })
    }
}

impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Io { path, op, errno } => {
                write!(f, "cannot {} at {path}: {errno}", op.describe())?;
            }
            Self::NotText { path, at } => write!(
                f,
                "{path} is not UTF-8 text; the first invalid byte sequence is at offset {at}"
            )?,
            Self::Refused { path, detail } => write!(f, "{path} is not a valid document: {detail}")?,
            Self::NotDurable { path, errno } => write!(
                f,
                "{path} was written and committed, and the directory entry could not be \
                 flushed to the card: {errno}. A power cut now may lose it"
            )?,
        }
        if let Some(cause) = self.likely_cause() {
            write!(f, " — {cause}")?;
        }
        Ok(())
    }
}

impl std::error::Error for StoreError {}

/// What became of first-boot seeding.
///
/// Five outcomes, and the two that are neither success nor failure are the
/// interesting ones: a machine that was already configured and a machine that
/// was offered nothing are both entirely normal, and neither is an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seeding {
    /// The boot partition offered no document. Every boot after the first, and
    /// every boot of a card written without one.
    NothingOffered,
    /// A document is already stored. The offered one is **not** applied — see
    /// [`ConfigStore::seed`] for why that is a ruling rather than caution.
    AlreadyConfigured,
    /// The offered document became the stored one.
    Seeded,
    /// Something is at the stored path and could not be read or understood, so
    /// whether to overwrite it is a question the record has not answered.
    Refused { why: StoreError },
    /// Nothing was stored and the write did not succeed.
    Failed { why: StoreError },
}

impl Seeding {
    /// One line for the console.
    ///
    /// **By count and by path, never by value.** The document this seeds from is
    /// read from the file that carries a wireless passphrase (`ADR-004` §6), and
    /// a console is a log — the same rule `cmd/rivet-initd`'s `provision`
    /// module applies to its own summary.
    pub fn summary(&self) -> String {
        match self {
            Self::NothingOffered => {
                "no provisioning document was offered; the stored configuration is unchanged"
                    .to_owned()
            }
            // It names the path and stops there. **It does not tell the
            // operator to run `rivet`**, which is `ADR-006`'s CLI and does not
            // exist yet — a console line is read now, and pointing at a binary
            // that is not on the machine is the kind of small dishonesty
            // `AGENTS.md` §6 is about. The canonical document's own header does
            // name the tools, and that is different: `ADR-007` §3.6.1 item 2
            // asks for exactly that, in a file whose reader is looking at the
            // design rather than at a boot.
            Self::AlreadyConfigured => format!(
                "this machine is already configured at {}; the provisioning document was read \
                 and NOT applied (ADR-011 — the stored document is the authority once it exists)",
                format_args!("{CONFIG_DIR}/{MACHINE_LAYER_FILE}")
            ),
            Self::Seeded => "first boot: the provisioning document is now the stored \
                             configuration (ADR-004 §6 step 2)"
                .to_owned(),
            Self::Refused { why } => format!(
                "the stored configuration could not be read and was NOT replaced: {why}. When \
                 re-provisioning is correct is open — ADR-018 §9 item 3"
            ),
            Self::Failed { why } => {
                format!("the provisioning document could NOT be stored: {why}")
            }
        }
    }

    /// Whether the machine ended this without the configuration it was given.
    ///
    /// [`Seeding::AlreadyConfigured`] is deliberately **not** a fault: it is the
    /// steady state of every boot after the first.
    pub const fn is_fault(&self) -> bool {
        matches!(self, Self::Refused { .. } | Self::Failed { .. })
    }
}

/// The machine layer of the desired-state document, on a filesystem.
///
/// Holds a directory and nothing else — no handle, no cache. `AGENTS.md` §5's
/// *never cache the authority's verdicts* is about the kernel, and the same
/// argument applies one level up: a store that held the last document it read
/// would be a second answer to "what is this machine configured to be", and the
/// stale one would be the one that got displayed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigStore {
    root: String,
}

impl ConfigStore {
    /// The store at `/state/config` — `ADR-008` §3.1, through
    /// [`rivet_core::state::CONFIG_DIR`] rather than a second spelling.
    pub fn at_default() -> Self {
        Self::at(CONFIG_DIR)
    }

    /// A store rooted anywhere, which is what the tests use and what an
    /// eventual `rivet --root` would use. **A path is not a policy**, so it is
    /// not a second authority for anything (the same reasoning that gave
    /// `cmd/rivet-netd` a `--document` in place of `--address`).
    pub fn at(root: &str) -> Self {
        Self {
            root: root.trim_end_matches('/').to_owned(),
        }
    }

    /// The directory this store lives in.
    pub fn root(&self) -> &str {
        &self.root
    }

    /// Where the machine layer is stored.
    pub fn machine_path(&self) -> String {
        format!("{}/{MACHINE_LAYER_FILE}", self.root)
    }

    /// **The last document that PARSED**, kept beside the real one.
    ///
    /// `PLAN.md` §104.19, written after a document stranded a board. The chain
    /// was: an unreadable document falls back to the schema defaults, the
    /// defaults carry no `[network]`, and a machine with no network has no
    /// control plane and no second way in. `ADR-018`'s A/B cannot undo it —
    /// `/state` is the one partition an update never replaces, so a rollback
    /// leaves the same bad document and an OLDER image is more likely to refuse
    /// it than a newer one.
    ///
    /// **`cmd/rivet-sysd`'s own comment already knew the rule** and did not
    /// follow it through: *"a machine that is reachable is one somebody can
    /// repair"*. This is the copy that keeps it reachable.
    ///
    /// # What is copied, and when
    ///
    /// The bytes that just parsed, at the moment they parsed, on the image that
    /// parsed them. That is deliberately a weaker claim than *"the document
    /// that worked"*: it does not promise the machine converged, only that this
    /// file is readable by something that ran. **That is the property the
    /// failure needed** — the board was not stranded by a document that meant
    /// the wrong thing, but by one that could not be read at all.
    ///
    /// # Why not the provisioning document on the boot partition
    ///
    /// It is the other candidate and it is worse in two ways: it is per-slot so
    /// a rollback changes it, and it is the FACTORY document — restoring it
    /// would silently undo every change an operator has made since. It is also
    /// absent under QEMU (`ADR-026` §4), which is where this is tested. A
    /// machine with neither still falls back to the defaults, as before.
    pub fn last_good_path(&self) -> String {
        format!("{}/{MACHINE_LAYER_FILE}.last-good", self.root)
    }

    /// Where a save in progress is written before it is committed.
    ///
    /// Inside `self.root` on purpose: `rename(2)` is atomic only within one
    /// filesystem, so a temp under `/tmp` or `/run` would make the commit a
    /// copy — which is exactly the non-atomic write this whole module exists to
    /// avoid.
    pub fn in_progress_path(&self) -> String {
        crate::atomic::in_progress(&self.machine_path())
    }

    /// Read the stored document.
    ///
    /// `Ok(None)` means **nothing is stored**, which is a complete answer and
    /// not a failure: it is the state of every machine before its first save,
    /// and it is what [`Self::seed`] tests for. Anything else that went wrong is
    /// an error, because `AGENTS.md` §5 forbids a fallback to another
    /// interpretation — a machine that read `EIO` and reported "unconfigured"
    /// would then be *re-provisioned over a card fault*.
    ///
    /// **The in-progress path is never read.** A `machine.toml.new` left by a
    /// power cut names bytes that were never committed, and adopting them would
    /// accept an input nothing ever asked for. It is not read, not deleted, and
    /// overwritten by the next [`Self::save`] — the recovery costs no code at
    /// all, which is why there is none.
    /// Read [`Self::last_good_path`], or `Ok(None)` if there is none.
    ///
    /// **A last-good copy that will not parse is `Ok(None)`, not an error.** It
    /// is a fallback, and a fallback that can itself fail loudly is one more
    /// thing between a machine and coming back up. The caller is left to fall
    /// through to the defaults exactly as it did before this existed.
    pub fn load_last_good(&self, fs: &mut dyn Filesystem) -> Option<Document> {
        let bytes = fs.read(&self.last_good_path()).ok()?;
        let text = core::str::from_utf8(&bytes).ok()?;
        Document::parse(text).ok()
    }

    /// Copy the bytes that just parsed to [`Self::last_good_path`].
    ///
    /// **Best effort, and a failure is returned rather than ignored** so the
    /// caller can say so — but nothing about the boot depends on it. Written
    /// through [`crate::atomic::commit`], the same four calls [`Self::save`]
    /// uses, because a torn last-good is worse than none: it is the thing read
    /// when everything else has already gone wrong.
    ///
    /// # It was a write and a rename, and the comment claimed more than that
    ///
    /// **Corrected 2026-09-04**, for the audit of 2026-09-03. This function
    /// wrote the temp and renamed it and did **neither `fsync`** — while the
    /// sentence above it said it was *"written through the same atomic rename
    /// the real document uses"*, which a reader had no reason to doubt. Two
    /// separate defects, and the comment is the worse of them:
    ///
    /// * Without the flush **before** the rename, a power cut can leave the
    ///   last-good file present, the right length, and full of zeroes — the
    ///   `ext4` failure [`Self::save`] documents, and the one that looks like
    ///   corruption rather than like a lost write. This file exists to be read
    ///   *after* a crash, so the crash window is not an edge case for it: it is
    ///   the case.
    /// * Without the flush **after** the rename, the commit itself can be lost.
    /// * And a wrong comment beside correct-looking code is what stops the next
    ///   reader from checking. `AGENTS.md` §2 rule 4.
    pub fn save_last_good(&self, fs: &mut dyn Filesystem, text: &str) -> Result<(), StoreError> {
        let path = self.last_good_path();
        let temporary = crate::atomic::in_progress(&path);
        crate::atomic::commit(fs, &self.root, &path, &temporary, text.as_bytes())
    }

    pub fn load(&self, fs: &mut dyn Filesystem) -> Result<Option<Document>, StoreError> {
        let path = self.machine_path();
        let bytes = match fs.read(&path) {
            Ok(bytes) => bytes,
            Err(Errno::ENOENT) => return Ok(None),
            Err(errno) => {
                return Err(StoreError::Io {
                    path,
                    op: Operation::Read,
                    errno,
                });
            }
        };

        let text = match core::str::from_utf8(&bytes) {
            Ok(text) => text,
            Err(e) => {
                return Err(StoreError::NotText {
                    path,
                    at: e.valid_up_to(),
                });
            }
        };

        match Document::parse(text) {
            Ok(document) => Ok(Some(document)),
            Err(e) => Err(StoreError::Refused {
                path,
                detail: e.to_string(),
            }),
        }
    }

    /// Write the document, so that a power cut leaves either the old one or the
    /// new one and never a mixture.
    ///
    /// # The sequence, and what each step buys
    ///
    /// ```text
    /// create_dir_all(/state/config)      the directory is created by what writes into it
    /// write(machine.toml.new)            the new document, nowhere anything reads
    /// sync_file(machine.toml.new)        its bytes reach the card
    /// rename(machine.toml.new -> ...)    THE COMMIT — atomic, by rename(2)
    /// sync_dir(/state/config)            the directory entry reaches the card
    /// ```
    ///
    /// **The last four are [`crate::atomic::commit`]** since 2026-08-23, when
    /// [`crate::clock`] arrived needing the identical sequence for `/state/clock`.
    /// The call order is unchanged and its own test still pins it; only the
    /// `create_dir_all` stayed here, because it is a fact about *this* store
    /// rather than part of the crash-safety argument.
    ///
    /// # What this guarantees
    ///
    /// * **A reader never sees a partial document.** `rename(2)` within one
    ///   filesystem replaces the directory entry in one step, so a concurrent
    ///   reader gets the whole old file or the whole new one.
    /// * **A crash never leaves a partial document at the stored path.** The
    ///   only path that is ever partially written is the `.new` one, which
    ///   nothing reads.
    /// * **The new bytes are on the card before anything points at them**, which
    ///   is what the `sync_file` before the rename is for. Without it a crash
    ///   can produce a file that is present, the right length, and full of
    ///   zeroes — the classic `ext4` failure, and the one that looks like data
    ///   corruption rather than like a lost write.
    /// * **The commit survives a power cut**, which is what the `sync_dir`
    ///   after the rename is for.
    ///
    /// # What it does not guarantee, stated because `ADR-020` §2's honesty rule
    /// applies to durability as much as to secrecy
    ///
    /// * **Nothing here can make an SD card tell the truth.** `fsync(2)` issues
    ///   a cache flush; a card that acknowledges the flush without honouring it
    ///   defeats every step above, and consumer cards are known to. `ADR-018`
    ///   §2 accepts working *with* the card's limitations, and this is one of
    ///   them: the guarantee is against a crash, not against a lying device.
    /// * **It does not protect a card that corrupts unrelated blocks** when
    ///   power is lost mid-erase, which flash can do. That is `ADR-018` §8's
    ///   damaged `/state` and its remedy is re-provisioning, not this.
    /// * **It is not a transaction across files.** One document is written
    ///   atomically; when `ADR-007` §3.4's other layers arrive, saving two of
    ///   them will be two commits and the machine can be interrupted between.
    /// * **`ext4`'s `data=ordered` default is assumed** and not verified from
    ///   the running kernel. `ADR-018` §7 chose `ext4` and named no mount data;
    ///   a `/state` mounted `data=writeback` would weaken the ordering the first
    ///   bullet depends on.
    pub fn save(&self, fs: &mut dyn Filesystem, document: &Document) -> Result<(), StoreError> {
        let path = self.machine_path();
        let temp = self.in_progress_path();

        // `ADR-017` §3.1.7: `/state/config` is created by whatever writes into
        // it, because `mkdir` is idempotent and a directory list in PID 1 is
        // creep. This is that writer.
        if let Err(errno) = fs.create_dir_all(&self.root) {
            return Err(StoreError::Io {
                path: self.root.clone(),
                op: Operation::CreateDirectory,
                errno,
            });
        }

        let bytes = document.to_toml();

        // The four calls whose ORDER is the guarantee, shared with
        // `crate::clock` rather than written twice. `crate::atomic` carries the
        // step-by-step reasoning and the cleanup rule.
        crate::atomic::commit(fs, &self.root, &path, &temp, bytes.as_bytes())
    }

    /// First boot: make the provisioning document the stored one, **if and only
    /// if nothing is stored yet.**
    ///
    /// `ADR-004` §6 step 2 — *"the result written to the persistent
    /// configuration"* — is this function, and §6.1 generalised it from the
    /// wireless credential to the `[network]` table on 2026-08-23.
    ///
    /// # Why it never overwrites, which is a ruling rather than caution
    ///
    /// `ADR-004` §6.1 is explicit that an address, unlike a passphrase, is
    /// **not** removed from the provisioning file after the first boot: "a
    /// machine whose provisioning file still describes the network it is on is a
    /// machine that can be re-imaged onto the same network without being told
    /// twice." So the file is still there on the second boot, and on the
    /// hundredth.
    ///
    /// A seed that applied it every time would make the boot partition
    /// authoritative for configuration — which is `ADR-011` §3's second
    /// authority — and would silently discard everything the operator had since
    /// set through the tools, which is `ADR-007` §3.6.5's discarded input. Both
    /// are refused by doing nothing, loudly: [`Seeding::AlreadyConfigured`] says
    /// on the console that the document was read and not applied.
    ///
    /// # Why a stored document it cannot read is refused rather than replaced
    ///
    /// That is `ADR-018` §8's recovery path — "if `/state/config` is unreadable,
    /// PID 1 can re-run provisioning and reach a machine that is
    /// network-reachable and re-configurable, rather than one that is bricked" —
    /// and the same section says it is "not designed here … it needs care around
    /// when re-provisioning is correct versus when it would silently discard a
    /// machine's configuration". `ADR-018` §9 item 3 carries it as **open**.
    ///
    /// An agent deciding it here would be answering a question the maintainer
    /// has reserved, in the direction that destroys data. So the store refuses
    /// and says which open item it is refusing under (`AGENTS.md` §1 — a
    /// decision with no obviously correct answer stops and asks).
    pub fn seed(&self, fs: &mut dyn Filesystem, offered: Option<&Document>) -> Seeding {
        let Some(document) = offered else {
            // Nothing to do, and the store is not touched at all — a read here
            // would put an `/state` access on every boot for no answer.
            return Seeding::NothingOffered;
        };

        match self.load(fs) {
            Ok(Some(_)) => Seeding::AlreadyConfigured,
            Ok(None) => match self.save(fs, document) {
                Ok(()) => Seeding::Seeded,
                Err(why) => Seeding::Failed { why },
            },
            Err(why) => Seeding::Refused { why },
        }
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::testfs::{Call, FakeFs, Script};

    /// A passphrase-shaped value, so output can be checked for leakage.
    /// `AGENTS.md` §8.3 — a test fixture, not a real secret.
    const NOT_A_REAL_SECRET: &str = "hunter2-correct-horse-battery";

    const PROVISIONED: &str = "version = 1\n\
                               [network.eth0]\nmethod = \"static\"\naddress = \"192.168.1.50/24\"\n\
                               gateway = \"192.168.1.1\"\n\
                               [network.wlan0]\nmethod = \"dhcp\"\n";

    fn store() -> ConfigStore {
        ConfigStore::at("/state/config")
    }

    fn document(src: &str) -> Document {
        Document::parse(src).expect("a valid fixture")
    }

    // ---- the paths ---------------------------------------------------------

    #[test]
    fn the_default_store_is_where_adr_008_3_1_put_it() {
        let s = ConfigStore::at_default();
        assert_eq!(s.root(), CONFIG_DIR);
        assert_eq!(s.machine_path(), "/state/config/machine.toml");
    }

    /// `rename(2)` is atomic only within one filesystem, so the temp has to be a
    /// sibling of the target. A temp under `/run` or `/tmp` would make the
    /// commit a copy, which is the non-atomic write this module exists to avoid.
    #[test]
    fn the_in_progress_path_is_a_sibling_of_the_stored_one() {
        let s = store();
        let temp = s.in_progress_path();
        assert!(temp.starts_with(&format!("{}/", s.root())), "{temp}");
        assert_eq!(temp.rfind('/'), s.machine_path().rfind('/'));
        assert_ne!(temp, s.machine_path());
    }

    #[test]
    fn a_trailing_slash_on_the_root_does_not_double_up_in_the_paths() {
        assert_eq!(
            ConfigStore::at("/state/config/").machine_path(),
            ConfigStore::at("/state/config").machine_path()
        );
    }

    // ---- load --------------------------------------------------------------

    /// The branch every first boot takes. It must be `Ok(None)` and not an
    /// error, or seeding would refuse on the one boot it exists for.
    #[test]
    fn nothing_stored_is_a_complete_answer_and_not_a_failure() {
        let mut fs = FakeFs::mounted();
        assert_eq!(store().load(&mut fs), Ok(None));
    }

    #[test]
    fn a_stored_document_loads_back_as_the_value_that_was_saved() {
        let mut fs = FakeFs::mounted();
        let doc = document(PROVISIONED);
        assert_eq!(store().save(&mut fs, &doc), Ok(()));
        assert_eq!(store().load(&mut fs), Ok(Some(doc)));
    }

    /// `AGENTS.md` §5: refuse rather than guess. An `EIO` reported as
    /// "unconfigured" would re-provision the machine over a card fault.
    #[test]
    fn an_unreadable_store_is_an_error_and_never_mistaken_for_an_empty_one() {
        let mut fs = FakeFs::mounted();
        fs.read_results = Some(Script::always(Err(Errno::EIO)));
        assert_eq!(
            store().load(&mut fs),
            Err(StoreError::Io {
                path: "/state/config/machine.toml".to_owned(),
                op: Operation::Read,
                errno: Errno::EIO,
            })
        );
    }

    /// `ADR-018` §8's damaged `/state`, in the shape a half-erased flash block
    /// actually takes.
    #[test]
    fn bytes_that_are_not_utf8_are_refused_with_an_offset_and_not_with_the_bytes() {
        let mut fs = FakeFs::mounted();
        fs.given("/state/config/machine.toml", b"version = 1\n\xff\xfe");
        let err = store().load(&mut fs).expect_err("not text");
        assert_eq!(
            err,
            StoreError::NotText {
                path: "/state/config/machine.toml".to_owned(),
                at: 12,
            }
        );
        assert!(err.to_string().contains("offset 12"), "{err}");
    }

    #[test]
    fn a_stored_document_the_schema_refuses_is_reported_as_such() {
        let mut fs = FakeFs::mounted();
        fs.given("/state/config/machine.toml", b"verison = 1\n");
        let err = store().load(&mut fs).expect_err("a misspelled key");
        assert!(
            matches!(&err, StoreError::Refused { detail, .. } if detail.contains("verison")),
            "{err:?}"
        );
    }

    /// A power cut between the write and the rename leaves a `.new` naming bytes
    /// nothing ever committed. They must not be adopted — and the cheapest way
    /// to guarantee that is never to look at the path, which is what this
    /// asserts.
    #[test]
    fn a_temp_left_by_a_crash_is_never_read_and_never_becomes_the_document() {
        let mut fs = FakeFs::mounted();
        fs.given("/state/config/machine.toml.new", PROVISIONED.as_bytes());
        assert_eq!(store().load(&mut fs), Ok(None));
        assert!(
            !fs.calls.iter().any(|c| matches!(c, Call::Read(p) if p.ends_with(".new"))),
            "the in-progress path was read: {:?}",
            fs.calls
        );
    }

    /// ...and the next save overwrites it, so the leftover is self-healing and
    /// needs no cleanup code at all.
    #[test]
    fn the_next_save_overwrites_a_temp_left_by_a_crash() {
        let mut fs = FakeFs::mounted();
        fs.given("/state/config/machine.toml.new", b"rubbish from a power cut");
        assert_eq!(store().save(&mut fs, &document(PROVISIONED)), Ok(()));
        assert_eq!(
            fs.paths(),
            vec!["/state/config/machine.toml"],
            "the rename must consume the temp, leaving one file"
        );
    }

    // ---- save: the sequence ------------------------------------------------

    /// The crash-safety argument, as an assertion on the call order rather than
    /// as a paragraph. Every step is load-bearing and the doc comment says what
    /// each one buys; this is what stops one being dropped in a refactor.
    #[test]
    fn a_save_creates_writes_syncs_renames_and_syncs_the_directory_in_that_order() {
        let mut fs = FakeFs::mounted();
        let doc = document(PROVISIONED);
        let bytes = doc.to_toml().len();
        assert_eq!(store().save(&mut fs, &doc), Ok(()));
        assert_eq!(
            fs.calls,
            vec![
                Call::CreateDirAll("/state/config".to_owned()),
                Call::Write("/state/config/machine.toml.new".to_owned(), bytes),
                Call::SyncFile("/state/config/machine.toml.new".to_owned()),
                Call::Rename(
                    "/state/config/machine.toml.new".to_owned(),
                    "/state/config/machine.toml".to_owned()
                ),
                Call::SyncDir("/state/config".to_owned()),
            ]
        );
    }

    /// The bytes the file ends up holding are `ADR-007` §3.1's canonical form
    /// and nothing else — no header the store added, no trailing junk.
    #[test]
    fn what_lands_on_the_card_is_exactly_the_canonical_document() {
        let mut fs = FakeFs::mounted();
        let doc = document(PROVISIONED);
        assert_eq!(store().save(&mut fs, &doc), Ok(()));
        assert_eq!(
            fs.contents("/state/config/machine.toml"),
            Some(doc.to_toml().as_bytes())
        );
    }

    /// `ADR-007` §3.1, at the level where it is actually observable: two
    /// machines in the same state produce the same file, byte for byte.
    #[test]
    fn two_machines_in_the_same_state_write_byte_identical_files() {
        // The same state, arrived at from two differently written documents.
        let a = document(PROVISIONED);
        let b = document(
            "version = 1\n\
             [network.wlan0]\nmethod = \"dhcp\"\n\
             # a comment nobody else wrote\n\
             [network.eth0]\ngateway = \"192.168.1.1\"\nmethod = \"static\"\n\
             address = \"192.168.1.50/24\"\n",
        );
        assert_eq!(a, b, "the fixtures must be the same state to prove anything");

        let mut one = FakeFs::mounted();
        let mut other = FakeFs::mounted();
        assert_eq!(store().save(&mut one, &a), Ok(()));
        assert_eq!(store().save(&mut other, &b), Ok(()));
        assert_eq!(
            one.contents("/state/config/machine.toml"),
            other.contents("/state/config/machine.toml")
        );
    }

    /// Saving twice must converge, not accumulate. `ADR-015` §7's re-run rule,
    /// and the reason the temp is a fixed name rather than a random one.
    #[test]
    fn saving_the_same_document_twice_leaves_one_file_and_the_same_bytes() {
        let mut fs = FakeFs::mounted();
        let doc = document(PROVISIONED);
        assert_eq!(store().save(&mut fs, &doc), Ok(()));
        let first = fs.contents("/state/config/machine.toml").map(<[u8]>::to_vec);
        assert_eq!(store().save(&mut fs, &doc), Ok(()));
        assert_eq!(fs.paths(), vec!["/state/config/machine.toml"]);
        assert_eq!(fs.contents("/state/config/machine.toml").map(<[u8]>::to_vec), first);
    }

    /// **B02, through the real persistence path.** The serialiser's own tests
    /// prove `to_toml()` escapes correctly; this proves `save` actually PUTS
    /// those bytes on the card, by re-parsing what `FakeFs` was actually given
    /// rather than trusting `document.to_toml()`'s return value a second time.
    /// A quoted application name, a dotted and a spaced setting key, and an
    /// `auto-run` value carrying a quote and a backslash are all accepted
    /// input `crate::applications`/`Kiosk` place no character restriction on.
    #[test]
    fn a_document_with_escaping_needs_persists_and_reparses_unchanged() {
        // The value TOML must escape to carry: a quote and a backslash.
        // Written with ordinary Rust escapes so the fixture reads as what it
        // means, then escaped for TOML independently of the code under test —
        // the same argument `rivet_config::serialise`'s own fixture helpers
        // make for not calling the renderer to build the renderer's own test.
        let auto_run = "weird\"app\\name";
        let escaped_auto_run = auto_run.replace('\\', "\\\\").replace('"', "\\\"");

        let mut fs = FakeFs::mounted();
        let doc = document(&format!(
            "version = 1\n\
             [kiosk]\nauto-run = \"{escaped_auto_run}\"\n\
             [applications.\"my.app\"]\n\"unit.name\" = \"a\"\n\"a b\" = \"c\"\n"
        ));
        assert_eq!(store().save(&mut fs, &doc), Ok(()));

        let bytes = fs
            .contents("/state/config/machine.toml")
            .expect("the document was persisted");
        let text = core::str::from_utf8(bytes).expect("the persisted bytes are text");
        let reparsed = Document::parse(text)
            .unwrap_or_else(|e| panic!("the persisted bytes will not parse: {e}\n{text}"));
        assert_eq!(reparsed, doc, "{text}");
    }

    // ---- the last-good copy ------------------------------------------------

    /// The machine the copy is actually made on: the document has just been
    /// read out of `/state/config`, so the directory is there and
    /// `save_last_good` creates nothing. Arranged with `given`, which makes the
    /// parent directories **without recording a call** — the sequence the tests
    /// below assert on has to be the store's alone.
    fn configured() -> FakeFs {
        let mut fs = FakeFs::mounted();
        fs.given("/state/config/machine.toml", PROVISIONED.as_bytes());
        fs
    }

    /// **The regression test for the audit of 2026-09-03.** `save_last_good`
    /// wrote the temp and renamed it and did neither `fsync`, while its own
    /// comment said it went through *"the same atomic rename the real document
    /// uses"*. The claim is now checked rather than asserted in prose: the same
    /// four calls in the same order as
    /// [`a_save_creates_writes_syncs_renames_and_syncs_the_directory_in_that_order`],
    /// minus the `create_dir_all` that is `save`'s alone.
    #[test]
    fn the_last_good_copy_is_written_with_both_flushes_in_the_order_that_is_the_guarantee() {
        let mut fs = configured();
        assert_eq!(store().save_last_good(&mut fs, PROVISIONED), Ok(()));
        assert_eq!(
            fs.calls,
            vec![
                Call::Write(
                    "/state/config/machine.toml.last-good.new".to_owned(),
                    PROVISIONED.len()
                ),
                Call::SyncFile("/state/config/machine.toml.last-good.new".to_owned()),
                Call::Rename(
                    "/state/config/machine.toml.last-good.new".to_owned(),
                    "/state/config/machine.toml.last-good".to_owned()
                ),
                Call::SyncDir("/state/config".to_owned()),
            ]
        );
        assert_eq!(
            fs.contents("/state/config/machine.toml.last-good"),
            Some(PROVISIONED.as_bytes())
        );
    }

    /// The `ext4` case, and the one this file exists for: `ENOSPC` arrives at
    /// the flush with the write having already succeeded. Without the flush
    /// before the rename, a crash here leaves a last-good copy that is present,
    /// the right length and full of zeroes — read *after* everything else has
    /// gone wrong, which is the only time it is read at all.
    #[test]
    fn a_last_good_copy_whose_bytes_did_not_reach_the_card_is_not_committed() {
        let mut fs = configured();
        fs.sync_file_results = Some(Script::always(Err(Errno::ENOSPC)));
        let err = store()
            .save_last_good(&mut fs, PROVISIONED)
            .expect_err("the flush failed");
        assert!(
            matches!(&err, StoreError::Io { op: Operation::SyncFile, .. }),
            "{err:?}"
        );
        assert!(!fs.exists("/state/config/machine.toml.last-good"));
        assert!(!fs.exists("/state/config/machine.toml.last-good.new"));
    }

    /// Past the rename the copy is in place, so a directory flush that fails is
    /// a durability failure and not a failed save — the same distinction
    /// [`crate::atomic`] draws for the document itself.
    #[test]
    fn a_last_good_copy_whose_name_did_not_reach_the_card_is_still_in_place() {
        let mut fs = configured();
        fs.sync_dir_results = Some(Script::always(Err(Errno::EIO)));
        let err = store()
            .save_last_good(&mut fs, PROVISIONED)
            .expect_err("the directory flush failed");
        assert!(matches!(&err, StoreError::NotDurable { .. }), "{err:?}");
        assert_eq!(
            fs.contents("/state/config/machine.toml.last-good"),
            Some(PROVISIONED.as_bytes())
        );
    }

    /// It reads back as what was written, through the store's own reader — the
    /// property the whole mechanism exists for.
    #[test]
    fn a_saved_last_good_copy_loads_back_as_the_document_it_was_made_from() {
        let mut fs = configured();
        assert_eq!(store().save_last_good(&mut fs, PROVISIONED), Ok(()));
        assert_eq!(store().load_last_good(&mut fs), Some(document(PROVISIONED)));
    }

    // ---- save: every failure path ------------------------------------------

    /// A `/state` that never mounted. The mount point is a directory on the
    /// read-only root, so this is what the operator's machine actually
    /// produces — and the message has to name the cause, because `EROFS` alone
    /// sends them looking at permissions.
    #[test]
    fn an_unmounted_state_refuses_the_save_and_the_message_names_the_cause() {
        let mut fs = FakeFs::unmounted();
        let err = store()
            .save(&mut fs, &document(PROVISIONED))
            .expect_err("nothing can be written");
        assert_eq!(
            err,
            StoreError::Io {
                path: "/state/config".to_owned(),
                op: Operation::CreateDirectory,
                errno: Errno::EROFS,
            }
        );
        let rendered = err.to_string();
        // **Both causes, because naming one was the defect.** A board read on
        // 2026-09-04 (`PLAN.md` §127.2) produced this message while `/state`
        // was mounted and healthy — the refusal came from a policy adapter —
        // and the old wording sent the reader to pid 1's mount records.
        assert!(rendered.contains("/state is not mounted"), "{rendered}");
        assert!(
            rendered.contains("refuses writes by policy"),
            "the second cause is missing, so this message names one and asserts it: {rendered}"
        );
        assert!(
            rendered.contains("rivet get storage"),
            "a reader is told the two are possible and not how to tell them apart: {rendered}"
        );
        assert!(rendered.contains("ADR-018"), "{rendered}");
    }

    /// A full card, reported at `write(2)`.
    #[test]
    fn a_full_state_refuses_at_the_write_and_leaves_nothing_behind() {
        let mut fs = FakeFs::mounted();
        fs.write_results = Some(Script::always(Err(Errno::ENOSPC)));
        let err = store()
            .save(&mut fs, &document(PROVISIONED))
            .expect_err("no space");
        assert!(
            matches!(&err, StoreError::Io { op: Operation::Write, errno: Errno::ENOSPC, .. }),
            "{err:?}"
        );
        assert!(err.to_string().contains("/state is full"), "{err}");
        assert!(!fs.exists("/state/config/machine.toml"));
        // The temp was cleared, because on a full filesystem it is the thing
        // occupying the space.
        assert!(fs.calls.contains(&Call::RemoveFile("/state/config/machine.toml.new".to_owned())));
    }

    /// **The `ext4` case, and the reason `sync_file` is its own port call.**
    /// Delayed allocation defers the block allocation past `write(2)`, so a full
    /// filesystem is reported at `fsync` with the write having already
    /// succeeded. A store that flushed inside its write would report success
    /// here.
    #[test]
    fn a_full_state_discovered_at_fsync_is_a_failure_and_not_a_success() {
        let mut fs = FakeFs::mounted();
        fs.sync_file_results = Some(Script::always(Err(Errno::ENOSPC)));
        let err = store()
            .save(&mut fs, &document(PROVISIONED))
            .expect_err("no space, discovered late");
        assert!(
            matches!(&err, StoreError::Io { op: Operation::SyncFile, errno: Errno::ENOSPC, .. }),
            "{err:?}"
        );
        // Nothing was committed, and the temp was cleared.
        assert!(!fs.exists("/state/config/machine.toml"));
        assert!(!fs.exists("/state/config/machine.toml.new"));
    }

    /// A rename that fails leaves the previous document in place, which is the
    /// safe direction — and it must be reported rather than swallowed.
    #[test]
    fn a_refused_commit_leaves_the_previous_document_untouched() {
        let mut fs = FakeFs::mounted();
        let first = document("version = 1\n[network.eth0]\nmethod = \"dhcp\"\n");
        assert_eq!(store().save(&mut fs, &first), Ok(()));
        let before = fs.contents("/state/config/machine.toml").map(<[u8]>::to_vec);

        fs.rename_results = Some(Script::always(Err(Errno::EIO)));
        let err = store()
            .save(&mut fs, &document(PROVISIONED))
            .expect_err("the commit was refused");
        assert!(
            matches!(&err, StoreError::Io { op: Operation::Rename, errno: Errno::EIO, .. }),
            "{err:?}"
        );
        assert_eq!(fs.contents("/state/config/machine.toml").map(<[u8]>::to_vec), before);
    }

    /// Past the commit the document *is* in place, so a failure to flush the
    /// directory is a durability failure and must not be reported as a failed
    /// save. Telling an operator "nothing happened" when something did is the
    /// dishonesty `ADR-007` §3.6 is about, arriving from the other direction.
    #[test]
    fn a_directory_flush_that_fails_says_the_document_is_in_place_anyway() {
        let mut fs = FakeFs::mounted();
        fs.sync_dir_results = Some(Script::always(Err(Errno::EIO)));
        let doc = document(PROVISIONED);
        let err = store().save(&mut fs, &doc).expect_err("the flush failed");
        assert!(matches!(&err, StoreError::NotDurable { .. }), "{err:?}");
        // The distinction that matters: it is there.
        assert_eq!(
            fs.contents("/state/config/machine.toml"),
            Some(doc.to_toml().as_bytes())
        );
        let rendered = err.to_string();
        assert!(rendered.contains("committed"), "{rendered}");
        assert!(rendered.contains("power cut"), "{rendered}");
    }

    /// `/state/config` existing as a *file* is not a state anything should
    /// produce, and if it ever happens the save must refuse rather than write
    /// nothing and report success.
    #[test]
    fn a_configuration_directory_that_is_a_file_is_refused_rather_than_ignored() {
        let mut fs = FakeFs::mounted();
        fs.given("/state/config", b"not a directory");
        let err = store()
            .save(&mut fs, &document(PROVISIONED))
            .expect_err("a file is in the way");
        assert!(
            matches!(&err, StoreError::Io { op: Operation::CreateDirectory, .. }),
            "{err:?}"
        );
    }

    /// The cleanup after a failed write is best-effort by design: on a machine
    /// where even the removal fails, the *original* failure is what the operator
    /// needs, and losing it behind a second one would be worse.
    #[test]
    fn a_cleanup_that_also_fails_does_not_replace_the_failure_that_matters() {
        let mut fs = FakeFs::mounted();
        fs.write_results = Some(Script::always(Err(Errno::ENOSPC)));
        fs.remove_file_results = Some(Script::always(Err(Errno::EIO)));
        let err = store()
            .save(&mut fs, &document(PROVISIONED))
            .expect_err("no space");
        assert!(
            matches!(&err, StoreError::Io { op: Operation::Write, errno: Errno::ENOSPC, .. }),
            "the reported failure must be the write, not the cleanup: {err:?}"
        );
    }

    // ---- seeding -----------------------------------------------------------

    #[test]
    fn nothing_offered_touches_the_store_not_at_all() {
        let mut fs = FakeFs::mounted();
        assert_eq!(store().seed(&mut fs, None), Seeding::NothingOffered);
        assert!(fs.calls.is_empty(), "{:?}", fs.calls);
    }

    /// The first boot, end to end: a document arrives from the boot partition
    /// and becomes the stored one. `ADR-004` §6 step 2.
    #[test]
    fn a_first_boot_stores_the_provisioning_document_and_reads_it_back() {
        let mut fs = FakeFs::mounted();
        let offered = document(PROVISIONED);
        assert_eq!(store().seed(&mut fs, Some(&offered)), Seeding::Seeded);
        assert_eq!(store().load(&mut fs), Ok(Some(offered)));
    }

    /// **The ruling this function exists to make.** `ADR-004` §6.1 leaves the
    /// `[network]` table on the boot partition for ever, so a seed that applied
    /// it every boot would make that file authoritative and discard whatever the
    /// operator had since set (`ADR-011` §3, `ADR-007` §3.6.5).
    #[test]
    fn a_second_boot_does_not_apply_the_provisioning_document_over_a_configured_machine() {
        let mut fs = FakeFs::mounted();
        let configured = document("version = 1\n[network.eth0]\nmethod = \"static\"\naddress = \"10.9.9.9/24\"\n");
        assert_eq!(store().save(&mut fs, &configured), Ok(()));

        let offered = document(PROVISIONED);
        assert_eq!(store().seed(&mut fs, Some(&offered)), Seeding::AlreadyConfigured);
        assert_eq!(store().load(&mut fs), Ok(Some(configured)));
        // ...and it is not a fault. It is what every boot after the first does.
        assert!(!Seeding::AlreadyConfigured.is_fault());
    }

    /// `ADR-018` §9 item 3 is open, and the direction an agent could guess in is
    /// the one that destroys the machine's configuration. So it refuses, and the
    /// message says which open item it is refusing under.
    #[test]
    fn a_store_that_cannot_be_read_is_refused_rather_than_re_provisioned_over() {
        for damage in [b"\xff\xff".as_slice(), b"version = 9999\n".as_slice()] {
            let mut fs = FakeFs::mounted();
            fs.given("/state/config/machine.toml", damage);
            let before = fs.contents("/state/config/machine.toml").map(<[u8]>::to_vec);

            let got = store().seed(&mut fs, Some(&document(PROVISIONED)));
            assert!(matches!(got, Seeding::Refused { .. }), "{got:?}");
            assert!(got.is_fault());
            assert!(got.summary().contains("ADR-018 §9 item 3"), "{}", got.summary());
            // Nothing was overwritten.
            assert_eq!(fs.contents("/state/config/machine.toml").map(<[u8]>::to_vec), before);
        }
    }

    #[test]
    fn a_seed_onto_an_unmounted_state_fails_and_says_which_step_and_why() {
        let mut fs = FakeFs::unmounted();
        let got = store().seed(&mut fs, Some(&document(PROVISIONED)));
        assert!(matches!(got, Seeding::Failed { .. }), "{got:?}");
        assert!(got.is_fault());
        assert!(got.summary().contains("/state is not mounted"), "{}", got.summary());
    }

    // ---- what must never be said -------------------------------------------

    /// `ADR-020` §4 and `AGENTS.md` §8.3, at the reporting end. Every summary
    /// and every error message is built from paths, field names, positions and
    /// `&'static str`, and none of them can carry a value out of a document.
    #[test]
    fn no_outcome_can_put_a_documents_own_content_on_the_console() {
        let mut fs = FakeFs::mounted();
        // A file that is not a document, whose content is passphrase-shaped.
        fs.given(
            "/state/config/machine.toml",
            format!("psk = \"{NOT_A_REAL_SECRET}\"\n").as_bytes(),
        );
        let got = store().seed(&mut fs, Some(&document(PROVISIONED)));
        let said = got.summary();
        assert!(!said.contains(NOT_A_REAL_SECRET), "{said}");
        // ...and it still names the key, or the operator cannot fix it.
        assert!(said.contains("psk"), "{said}");
    }

    /// A saved document cannot carry a secret either, because the schema
    /// declares no field that holds one — `ADR-007` §7.3 puts the wireless
    /// passphrase in `ADR-020`'s encrypted store and not in this document.
    #[test]
    fn a_stored_document_has_no_field_a_credential_could_live_in() {
        let mut fs = FakeFs::mounted();
        assert_eq!(store().save(&mut fs, &document(PROVISIONED)), Ok(()));
        let written = String::from_utf8(
            fs.contents("/state/config/machine.toml")
                .expect("just written")
                .to_vec(),
        )
        .expect("canonical output is UTF-8");
        for key in ["psk", "passphrase", "password", "secret", "key"] {
            assert!(!written.contains(key), "{key:?} appears in\n{written}");
        }
    }
}
