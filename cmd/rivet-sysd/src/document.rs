//! Reading the desired-state document, and applying its `[services]` table to
//! the compiled-in definitions.
//!
//! `ADR-017` §4.1: *"The supervisor sees definitions ∪ desired state and
//! converges."* [`crate::definitions`] is the first half; this module is the
//! union.
//!
//! # This closes a claim that was repeated in four places
//!
//! ~~"Nothing reads the stored document yet."~~ — `ADR-007` §7.6.2,
//! `ADR-017` §3.1.9 and `cmd/rivet-netd`'s module documentation
//! all said it, and it was true until 2026-08-23. PID 1 seeds
//! `/state/config/machine.toml` on a first boot (`ADR-017` §3.1.9) and until
//! now nothing read it back. `crate::etc` was handed `Document::default()` with
//! a comment saying `rivet-store` did not exist; it does, and the comment is
//! corrected rather than the behaviour left alone.
//!
//! # Why the store's own filesystem adapter is not used
//!
//! `rivet-store` ships `StdFilesystem`, and using it here would put a
//! `std::fs` call in the middle of a process whose every other syscall goes
//! through [`Kernel`] — which is the seam that lets `ADR-017` §3.5's backoffs
//! and grace periods be tested in microseconds. A store read that could not be
//! driven from a test would be the one branch of the supervisor's start-up that
//! nothing exercises, and its interesting cases are exactly the ones a
//! developer's machine never reaches: a `/state` that did not mount, a card
//! returning `EIO`, a document a power cut truncated.
//!
//! So [`ReadOnlyStore`] adapts the `Kernel` port to the store's `Filesystem`
//! port, and **every write refuses**. `AGENTS.md` §3 — a stub refuses and never
//! fakes success. The supervisor has no business writing desired state: `rivet
//! apply` does that (`ADR-006` §6) and PID 1 seeds it (`ADR-017` §3.1.9), and a
//! supervisor that could write the document would be a third author of it.
//!
//! # One exception, and it is not desired state — ruled 2026-09-04
//!
//! **The last-good copy was never written on any running machine.** `load`
//! below asks the store to save it after a parse succeeds, and handed it this
//! adapter, whose `write` returns `EROFS` — so the fallback `PLAN.md` §104.19
//! designed has never existed, and every boot with a stored document journalled
//! a warning saying so. Found by reading during Wave 7a, confirmed on hardware
//! in `PLAN.md` §127.2 out of the board's own journal.
//!
//! `.rulings/` R-139 rules that the supervisor writes it, and the reason the
//! paragraph above does not forbid it is that **the last-good copy is not
//! desired state.** Nothing authors it, no operator edits it, and it is a
//! derived artifact of a parse this process just performed — which is exactly
//! the argument `crate::clock`'s `StateFs` already makes for writing the stored
//! clock through this same port.
//!
//! **So the exception is a whitelist of ONE path**, carried by
//! [`ReadOnlyStore::writable`]: the last-good file, the temporary it is
//! committed through, and the directory the rename lands in. Anything else —
//! the document itself above all — still refuses. A reader who wants to know
//! what the supervisor can write does not have to trace call sites; it is a
//! field on the adapter, and [`Writable::permits`] is the whole rule.
//!
//! # One fidelity loss, named rather than hidden
//!
//! [`Kernel::read_file`] returns a `String`, so a stored document that is not
//! UTF-8 arrives here as an unnamed I/O error rather than as
//! `StoreError::NotText { at }`, which is what `rivet-store`'s own reader
//! would have produced. The refusal is the same and the *reason* is less
//! precise. Widening the port to bytes for one diagnostic was judged the worse
//! trade; `cmd/rivet-netd` reads the same file independently and reports it
//! with the store's own precision.

use rivet_config::{Document, MachineMode};
use rivet_store::{ConfigStore, Errno as StoreErrno, Filesystem, StoreError};

use crate::journal::{Journal, Severity, Value};
use crate::kernel::{Errno, Kernel};
use crate::service::ServiceDef;
use crate::supervise::SELF_NAME;

/// **The one file this adapter may write**, and the two other paths that one
/// write touches.
///
/// `rivet_store::atomic::commit` is four syscalls — write the temporary, flush
/// it, rename it over the target, flush the directory — so permitting a single
/// logical write means naming three paths. Anything not in here refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Writable {
    /// The last-good copy itself.
    target: String,
    /// `target` plus the store's in-progress suffix.
    temp: String,
    /// The store root, which is what the post-rename flush syncs.
    dir: String,
}

impl Writable {
    /// The last-good copy of `store`, and nothing else.
    fn last_good_of(store: &ConfigStore) -> Self {
        let target = store.last_good_path();
        Self {
            temp: rivet_store::atomic::in_progress(&target),
            dir: store.root().to_string(),
            target,
        }
    }

    /// **The whole rule**, in one place so a reader does not have to assemble it
    /// from four method bodies.
    fn permits(&self, path: &str) -> bool {
        path == self.target || path == self.temp
    }
}

/// The store's `Filesystem` port over this crate's [`Kernel`]. Reads pass
/// through; writes refuse unless [`Self::writable`] names the path.
struct ReadOnlyStore<'k> {
    k: &'k mut dyn Kernel,
    /// `None` on every path but the last-good save — see this module's header.
    writable: Option<Writable>,
}

/// The two error vocabularies are separate types on purpose — `rivet-store`'s
/// constants are Linux's own, written down so that a process compiled on
/// Windows still reasons about the numbers a Rivet machine produces — and
/// this crate's come from `libc` on the target. On the target they are the same
/// numbers, so the conversion is the number itself and nothing else.
const fn to_store_errno(e: Errno) -> StoreErrno {
    StoreErrno(e.0)
}

impl Filesystem for ReadOnlyStore<'_> {
    fn read(&mut self, path: &str) -> Result<Vec<u8>, StoreErrno> {
        self.k
            .read_file(path)
            .map(String::into_bytes)
            .map_err(to_store_errno)
    }

    // Everything below refuses UNLESS `writable` names the path. `EROFS` is not
    // an approximation: the supervisor is not the author of desired state, so
    // for this process the store really is read-only, and reporting anything
    // else would invite a caller to believe a write had been attempted.
    //
    // The one exception is the last-good copy, which is not desired state —
    // see this module's header and `.rulings/` R-139. Each method asks
    // `Writable::permits` rather than carrying its own idea of the rule.

    // **No exception here, deliberately.** `atomic::commit` does not create
    // directories: the store root exists because PID 1 mounted `/state`, and a
    // supervisor that created it would be papering over a mount that did not
    // happen — which is a fault an operator has to see.
    fn create_dir_all(&mut self, _path: &str) -> Result<(), StoreErrno> {
        Err(StoreErrno::EROFS)
    }

    fn write(&mut self, path: &str, bytes: &[u8]) -> Result<(), StoreErrno> {
        if !self.writable.as_ref().is_some_and(|w| w.permits(path)) {
            return Err(StoreErrno::EROFS);
        }
        // **`write_file_nofollow`, not `write_file`.** Wave 2.1's fix — the
        // path is under `/state` and root must not be led through a symlink
        // into writing somewhere else. `0600`: this is the supervisor's own
        // copy of the document and nothing else reads it.
        self.k
            .write_file_nofollow(path, bytes, 0o600)
            .map_err(to_store_errno)
    }

    fn sync_file(&mut self, path: &str) -> Result<(), StoreErrno> {
        if !self.writable.as_ref().is_some_and(|w| w.permits(path)) {
            return Err(StoreErrno::EROFS);
        }
        self.k.sync_file(path).map_err(to_store_errno)
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), StoreErrno> {
        // **Both ends checked.** Permitting the source alone would let a rename
        // move the temporary over any path in the filesystem.
        let ok = self
            .writable
            .as_ref()
            .is_some_and(|w| from == w.temp && to == w.target);
        if !ok {
            return Err(StoreErrno::EROFS);
        }
        self.k.rename(from, to).map_err(to_store_errno)
    }

    fn sync_dir(&mut self, path: &str) -> Result<(), StoreErrno> {
        // The directory the commit lands in, which is not a path `permits`
        // covers — it names files.
        if !self.writable.as_ref().is_some_and(|w| path == w.dir) {
            return Err(StoreErrno::EROFS);
        }
        self.k.sync_dir(path).map_err(to_store_errno)
    }

    // **No exception here either.** `atomic::commit` cleans up its temporary by
    // renaming it away, not by unlinking, so nothing on the permitted path
    // needs this — and a supervisor that could unlink under `/state` is a
    // larger thing than the ruling asked for.
    fn remove_file(&mut self, _path: &str) -> Result<(), StoreErrno> {
        Err(StoreErrno::EROFS)
    }
}

/// What the supervisor found where the machine's configuration is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Desired {
    /// A document was read. Its `[services]` table is the operator's word.
    Stored(Document),
    /// Nothing is stored. **Not a fault** — it is every machine that was
    /// written without a `rivet.toml` on its boot partition, and every
    /// machine under QEMU, which has no boot partition at all (`ADR-026` §4).
    Unconfigured,
    /// Something is there and could not be read or understood.
    ///
    /// **The definitions still run.** See [`load`] for why that is a ruling
    /// rather than laziness.
    Unreadable { why: StoreError },
}

impl Desired {
    /// The document to project `/etc` from.
    ///
    /// `Document::default()` for the two cases with no document, which is
    /// `ADR-007` §3.4's layer 1 — the schema defaults — and the only layer that
    /// needs no store behind it.
    pub fn document(&self) -> Document {
        match self {
            Self::Stored(document) => document.clone(),
            Self::Unconfigured | Self::Unreadable { .. } => Document::default(),
        }
    }
}

/// Read the machine layer of the desired-state document (`ADR-007` §3.4).
///
/// # Why an unreadable document does not stop the supervisor
///
/// It is the same argument `ADR-018` §8 makes about `/state` and `ADR-017`
/// §3.1.7 reuses: **a machine that is reachable is one somebody can repair.**
/// Refusing to start anything would leave a headless board with no network and
/// no control plane, and the fault is on `/state`, which no rollback and no
/// update touches — so there is nothing the failure could escalate to that
/// would fix it.
///
/// What happens instead is that the definitions' own `enabled` stands, which
/// means `rivet-netd` and `rivet-mcpd` both run. That is **not** free of cost
/// and the cost is stated rather than buried: an operator who set
/// `enabled = false` on a service and whose document then became unreadable
/// gets that service started. The console says so in as many words, at
/// `Severity::Error`, naming the file and the reason.
///
/// `cmd/rivet-netd` will independently read the same file and refuse it with
/// `rivet-config`'s own diagnostic — a key, a line and a column — so the
/// operator gets two consistent accounts rather than one vague one.
pub fn load(k: &mut dyn Kernel, journal: &mut Journal) -> Desired {
    let store = ConfigStore::at_default();
    let path = store.machine_path();
    // Reborrowed rather than moved, because `k` is wanted again below to
    // journal what happened. The adapter lives only for the read.
    let outcome = {
        let mut fs = ReadOnlyStore { k: &mut *k, writable: None };
        store.load(&mut fs)
    };

    match outcome {
        Ok(Some(document)) => {
            note(
                k,
                journal,
                Severity::Info,
                "read the stored configuration",
                vec![
                    ("path".to_string(), Value::Str(path)),
                    (
                        "schema_version".to_string(),
                        Value::Int(i64::from(document.version().get())),
                    ),
                    (
                        "services_configured".to_string(),
                        Value::Int(count(document.services().len())),
                    ),
                    (
                        "interfaces_configured".to_string(),
                        Value::Int(count(document.network().len())),
                    ),
                ],
            );
            // **Keep a copy that is known to parse** — `PLAN.md` §104.19.
            // Written after the parse succeeded and before anything is done
            // with it, because the property being preserved is exactly
            // *readable by this image*, which is what the failing case lacked.
            //
            // **This is the ONE adapter in this module that may write** —
            // `.rulings/` R-139, ruled 2026-09-04 after `PLAN.md` §127.2 showed
            // on the board that it never had. `Writable::last_good_of` names
            // the three paths one atomic commit touches and nothing else, so
            // the document itself is as unwritable from here as it ever was.
            {
                let text = document.to_toml();
                let mut fs = ReadOnlyStore {
                    k: &mut *k,
                    writable: Some(Writable::last_good_of(&store)),
                };
                if let Err(why) = store.save_last_good(&mut fs, &text) {
                    note(
                        k,
                        journal,
                        Severity::Warn,
                        "the last-good copy of the configuration could not be written, so a \
                         document that later becomes unreadable will fall back to the schema \
                         defaults rather than to this one. Nothing about this boot is affected",
                        vec![("detail".to_string(), Value::Str(why.to_string()))],
                    );
                }
            }
            Desired::Stored(document)
        }
        Ok(None) => {
            note(
                k,
                journal,
                Severity::Warn,
                "no configuration is stored, so this machine has stated no desired state. The \
                 compiled-in service definitions run as they are, and rivet-netd will refuse \
                 the same absent document (ADR-007 §3.3, ADR-017 §3.1.9)",
                vec![("path".to_string(), Value::Str(path))],
            );
            Desired::Unconfigured
        }
        Err(why) => {
            // **The last-good copy first, and the defaults only after it** —
            // `PLAN.md` §104.19.
            //
            // The paragraph below has always ended *"a machine that can be
            // reached is one somebody can repair"*, and then fell back to
            // `Document::default()`, which carries no `[network]`. So the
            // sentence was true and the code did the opposite of it: an
            // unreadable document took the network away, the control plane
            // with it, and `ADR-035` has retired the second way in. A/B
            // could not undo it either — `/state` is the partition an update
            // never replaces.
            //
            // This is that sentence, followed through.
            let recovered = {
                let mut fs = ReadOnlyStore { k: &mut *k, writable: None };
                store.load_last_good(&mut fs)
            };
            if let Some(document) = recovered {
                note(
                    k,
                    journal,
                    Severity::Error,
                    "DEGRADED the stored configuration could not be read, so THE LAST COPY THAT \
                     PARSED is being used instead. This machine is running a PREVIOUS desired \
                     state: anything changed since that copy was written is not in effect, and \
                     the broken file is still on /state exactly as it was. It is used rather \
                     than the schema defaults because the defaults carry no network, and a \
                     machine with no network has no control plane and no way to be repaired \
                     (ADR-018 §8). Fix the file and reload",
                    vec![
                        ("path".to_string(), Value::Str(path.clone())),
                        ("detail".to_string(), Value::Str(why.to_string())),
                        (
                            "using".to_string(),
                            Value::Str(store.last_good_path()),
                        ),
                        // **What was recovered, in numbers.** An operator
                        // reading this needs to know whether the machine came
                        // back with a network, which is the whole point of the
                        // fallback and is not visible from the sentence.
                        (
                            "interfaces_configured".to_string(),
                            Value::Int(count(document.network().len())),
                        ),
                        (
                            "services_configured".to_string(),
                            Value::Int(count(document.services().len())),
                        ),
                    ],
                );
                return Desired::Stored(document);
            }
            note(
                k,
                journal,
                Severity::Error,
                "DEGRADED the stored configuration could not be read AND THERE IS NO LAST-GOOD \
                 COPY, so the operator's [services] table is not being honoured and every \
                 compiled-in definition runs with its own enabled value. THIS MACHINE MAY HAVE \
                 NO NETWORK: the schema defaults carry no [network] table, and without one \
                 there is no control plane to repair it over. The fault is on /state, which no \
                 update or rollback touches (ADR-018 §8)",
                vec![
                    ("path".to_string(), Value::Str(path)),
                    ("detail".to_string(), Value::Str(why.to_string())),
                ],
            );
            Desired::Unreadable { why }
        }
    }
}

/// Apply the document's `[services]` table to the compiled-in definitions.
///
/// This function is the union `ADR-017` §4.1 names — *"the supervisor sees
/// definitions ∪ desired state and converges"*.
///
/// # `[services]`: `enabled` and the two caps
///
/// The surface is `enabled`, `memory` and `cpu` (`critical` is a fourth key,
/// argued separately below), and the two caps were each argued to a ruling
/// on the identical footing (the split-by-direction ruling, 2026-08-24 for
/// `memory`, amended 2026-09-09 for `cpu`):
///
/// * **`listen` is not the operator's**, so this function never sees one:
///   `rivet-config` refuses it at parse time. `ADR-017` §3.2's pre-created
///   socket and §3.4's `LISTEN_FDS` handoff are not built, and what says so is
///   `crate::applications`, over the *manifest's* `listen`.
/// * **`memory` and `cpu` are applied**, and the argument against applying
///   them was that a cap reported and not enforced is worse than no cap. That
///   reasoning is right and its conclusion was too strong: `ADR-024` §8
///   found the memory controller absent **on the board**, and the same section then
///   found the same kernel offering it under an emulator. So whether a cap
///   can be enforced is a question about the running machine, not about this
///   build — and the honest shape is to ask the kernel and report what it
///   said (`AGENTS.md` §5), which is `crate::supervise`'s `apply_cap` and
///   `apply_cpu_cap`. A machine that cannot enforce one still says so, once
///   per service, in as many words.
///
/// `memory` and `cpu` are **caps** rather than values: `rivet_core::service::
/// effective_memory` and `effective_cpu` each take the lower of the operator's
/// and what the service's manifest requested. A system service has no
/// manifest, so an operator's cap on `rivet-mcpd` stands alone — which is
/// `ADR-007` §7's grant, unchanged by either ruling.
///
/// Anything not honoured is still said out loud rather than dropped:
/// `ADR-007` §3.6 — the system never accepts an input it will discard.
///
/// A service the document names and this image does not define is **not** an
/// error here. `rivet-config`'s `services` module argues it at length: the
/// document is an input artifact authored anywhere (`ADR-007` §3.6.3), a fleet
/// layer applies one document to several machines, and writing configuration
/// for a service you are about to install is the normal case. It is reported,
/// because the same module's third point puts the refusal "one layer up, where
/// the information is" — and this is that layer.
pub fn apply_desired_state(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    defs: &mut [ServiceDef],
    desired: &Desired,
) {
    apply_on(k, journal, defs, desired, Occasion::Boot);
}

/// **When the document is being applied**, and it decides what may change.
///
/// `ADR-032` §3.2.1, ruled 2026-08-26. The two occasions are not two policies
/// for the sake of caution — they are two genuinely different situations:
///
/// | | [`Self::Boot`] | [`Self::Reload`] |
/// | :-- | :-- | :-- |
/// | Who is watching | a console, on a machine that has not started services yet | nobody; the operator may be on the control-plane session this could close |
/// | What undoes a mistake | the next reset, and `ADR-018` §5.1's rollback under it | **nothing** — the machine is up, and up is where it stays |
///
/// So a reload withholds [`crate::definitions::REACHABILITY_SET`], and says so.
/// Everything else it applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Occasion {
    /// `crate::main`, before the supervisor exists. Everything applies.
    Boot,
    /// A `SIGHUP` on a running machine. See the type's documentation.
    Reload,
}

impl Occasion {
    /// Whether this occasion may change `enabled` for `name`.
    fn may_stop(self, name: &str) -> bool {
        match self {
            Self::Boot => true,
            Self::Reload => !crate::definitions::keeps_the_machine_reachable(name),
        }
    }
}

/// **What this machine is for**, read out of the document it was given.
///
/// # `mode` decides; `[kiosk]` is only its configuration — `.rulings/` R-164, R-165
///
/// A document's `mode` key, where it names one, is the switch: `headless` runs
/// no kiosk even when a `[kiosk]` table is present, and `kiosk` runs the one
/// the table describes. **The table's presence never means a mode is active**,
/// and nothing here clears one mode's table when `mode` names another — a
/// headless machine may carry a `[kiosk]` table it does not act on, and that is
/// not a contradiction (R-165 leaves room for a future `[desktop]` table beside
/// it on the identical footing).
///
/// A document with no `mode` behaves exactly as it always has: a `[kiosk]`
/// table makes it a kiosk, and no table makes it headless. That is what keeps
/// every document written before this key existed — `rivet-01`'s included —
/// meaning what it always meant.
///
/// `mode = "kiosk"` with no `[kiosk]` table is **tolerated and reported, not
/// refused**: [`rivet_core::operation::status::KioskMode::kiosk_unconfigured`]
/// still answers `kiosk` — that is the fact worth surfacing — while behaving
/// exactly as a machine with no kiosk configured does today.
///
/// # This is the one place that reads it, and that is the point
///
/// `rivet-core` cannot: its `[dependencies]` table is empty by rule
/// (`AGENTS.md` §10.1), so it has no TOML parser and could only have
/// string-matched — the same defect one crate further in. This process already
/// parses the document, already reports document-derived facts through
/// `get status` (`ServiceStatus::enabled` is one), and is therefore the shape
/// that exists rather than a new one.
///
/// # The answers, and why `Unreadable` is never `headless`
///
/// | [`Desired`] | `document.mode()` | `document.kiosk()` | mode | why |
/// | :-- | :-- | :-- | :-- | :-- |
/// | [`Desired::Stored`] | `Headless` | either | `headless` | the switch says so; the table, if any, is not acted on |
/// | [`Desired::Stored`] | `Kiosk` | `Some` | `kiosk` | the switch says so, configured |
/// | [`Desired::Stored`] | `Kiosk` | `None` | `kiosk`, unconfigured | asked for and not configured — reported, not refused |
/// | [`Desired::Stored`] | `None` | `Some` | `kiosk` | today's fallback: the table alone makes it a kiosk |
/// | [`Desired::Stored`] | `None` | `None` | `headless` | today's fallback: no table, no switch |
/// | [`Desired::Unconfigured`] | — | — | `headless` | there is no document at all. `ADR-026` §4's QEMU machines are all of these |
/// | [`Desired::Unreadable`] | — | — | `unknown`, with the store's own sentence | something is there and could not be read, so nothing has been established. **`headless` here would be the defect this function exists to end** |
///
/// One reader, called from both ends of the wire that carries this fact: this
/// function is what `crate::supervise::Supervisor::kiosk_offer` (the launcher's
/// `offer` answer) and `get status` both report, so the two can never disagree
/// about what this document means.
pub fn kiosk_mode(desired: &Desired) -> rivet_core::operation::status::KioskMode {
    use rivet_core::operation::status::KioskMode;
    // `locked` and `auto-run` are carried as the table wrote them. Neither is
    // validated here: `auto-run` names an application a LAUNCHER starts, so a
    // name this process cannot resolve is the launcher's to report — on the
    // screen, where somebody is standing (`PLAN.md` §108.2).
    fn configured(kiosk: &rivet_config::kiosk::Kiosk) -> rivet_core::operation::status::KioskMode {
        rivet_core::operation::status::KioskMode::kiosk(
            kiosk.locked(),
            kiosk.auto_run().map(std::string::ToString::to_string),
        )
    }

    match desired {
        Desired::Stored(document) => match document.mode() {
            Some(MachineMode::Headless) => KioskMode::headless(),
            Some(MachineMode::Kiosk) => document
                .kiosk()
                .map_or_else(KioskMode::kiosk_unconfigured, configured),
            // No `mode` at all: today's fallback, unchanged by R-164.
            None => document.kiosk().map_or_else(KioskMode::headless, configured),
        },
        Desired::Unconfigured => KioskMode::headless(),
        Desired::Unreadable { why } => KioskMode::unknown(format!(
            "this machine's desired-state document could not be read, so its `mode` and \
             `[kiosk]` table were never seen: {why}. THIS IS NOT EVIDENCE THAT THE MACHINE IS \
             HEADLESS"
        )),
    }
}

/// **Which services hold the two display roles on this machine** — the
/// launcher and the compositor, as the document names them in `[kiosk]`.
///
/// A manifest can only *ask* for a role: `connects = ["rivet-sysd:kiosk"]` asks
/// to be the launcher and `provides = ["socket:wayland"]` offers to drive the
/// screen. The document decides which service may answer, so a second
/// application making the same declaration takes nothing from the one the
/// machine is meant to run. Both a name and the declaration are required, so
/// naming a service that does not ask for a role gives it nothing either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayAuthority {
    /// The one service that may be given the kiosk socket, and the screen when
    /// no compositor is enabled.
    pub launcher: String,
    /// The one service that may arbitrate the screen as its compositor.
    pub compositor: String,
}

impl Default for DisplayAuthority {
    /// **The image's own launcher and compositor**, which is what every
    /// machine ran before the document could name them. A document that names
    /// neither, or no document at all, keeps that behaviour.
    fn default() -> Self {
        Self {
            launcher: crate::definitions::LAUNCHER_NAME.to_string(),
            compositor: crate::definitions::DISPLAYD_NAME.to_string(),
        }
    }
}

/// The [`DisplayAuthority`] this document names, each role falling back to the
/// image's own service where the document is silent.
///
/// Read whatever `mode` says: the roles matter only where a launcher or a
/// compositor is enabled, which the document decides separately, and reading
/// them conditionally would give one document two meanings. A document that
/// could not be read names nothing, and the image's own services keep the
/// roles — the same answer a machine with no document gets.
#[must_use]
pub fn display_authority(desired: &Desired) -> DisplayAuthority {
    let mut authority = DisplayAuthority::default();
    if let Desired::Stored(document) = desired
        && let Some(kiosk) = document.kiosk()
    {
        if let Some(name) = kiosk.launcher() {
            authority.launcher = name.to_string();
        }
        if let Some(name) = kiosk.compositor() {
            authority.compositor = name.to_string();
        }
    }
    authority
}

/// [`apply_desired_state`] with the occasion said out loud. See [`Occasion`].
/// **Which of the offered display applications actually runs** — `[kiosk]
/// auto-run`, ruled 2026-08-31 (.rulings/ R-321): several kiosk applications can
/// be installed and enabled at once, only one of them is the auto launch, and
/// enabled means being on the launcher's list of apps.
///
/// So `enabled` stopped meaning RUNS for an application that declares a display
/// and started meaning OFFERED. This is the half that decides the other thing:
/// among the enabled display declarers, the one `[kiosk] auto-run` names keeps
/// its `enabled` and the rest have it cleared.
///
/// # Why here rather than in `crate::applications`
///
/// Because `enabled` does not exist yet when a definition is discovered — an
/// application starts `enabled: false` and the document overrides it, above.
/// `ADR-015` §6.2's arbitration ran at discovery and therefore could not see
/// it, which is why a service that was `enabled = false` and idle still held
/// the display and refused every later one (`PLAN.md` §104.17.3). Deciding
/// after the document has spoken is the fix and it is the only place the
/// question is answerable.
/// ~~`[kiosk] auto-run`, applied by the supervisor.~~ **REMOVED 2026-09-01.**
///
/// The maintainer corrected the language and the code followed: the supervisor
/// does not autorun the launcher, and the autorun language belongs to the
/// launcher, which might autostart a kiosk application (.rulings/ R-344).
///
/// `auto-run` names the kiosk application a LAUNCHER starts instead of showing
/// its menu. It is not this process's key, so this process does not validate
/// it: a launcher that cannot honour it can say so **on the screen**, with a
/// code, which is worth more than a journal line nobody is standing next to
/// (`PLAN.md` §108.2).

pub fn apply_on(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    defs: &mut [ServiceDef],
    desired: &Desired,
    occasion: Occasion,
) {
    let Desired::Stored(document) = desired else {
        return;
    };

    for service in document.services() {
        let name = service.name().as_str();
        let Some(def) = defs.iter_mut().find(|d| d.name == name) else {
            // **A declaration with nothing behind it is two different facts,
            // and the `enabled` value is what separates them** — `ADR-017` §6.
            //
            // `enabled = false` is a line waiting for an install, which is the
            // ordinary case this branch was written for. `enabled = true` is an
            // ORPHANED DECLARATION: the operator is asking this machine to run
            // a program the machine does not have, so a convergence that says
            // only *nothing is started for it* has reported the harmless half
            // of a real divergence. `apply uninstall` leaves exactly this
            // behind, by design, because the document is what was asked for
            // rather than a mirror of what is installed.
            let (severity, msg) = if service.enabled() {
                (
                    Severity::Warn,
                    "ORPHANED DECLARATION: the document asks for this service to RUN and nothing \
                     on this machine provides it, so it will never start and no failure will be \
                     reported anywhere else. The document is kept rather than rewritten — it is \
                     what the operator asked for, not a mirror of what is installed (ADR-017 \
                     §6) — so this resolves either by installing the application or by removing \
                     the table with `rivet apply edit /state/config/machine.toml`. An uninstall \
                     leaves this behind deliberately and says so in its answer",
                )
            } else {
                // Unchanged, severity included. This is the line a fleet
                // document produces on every machine that has not been given
                // the application yet, and lowering it is a separate question
                // from telling the two cases apart.
                (
                    Severity::Warn,
                    "the document configures a service this image does not define; it is kept \
                     rather than refused (ADR-007 §3.6.3 — a document is authored anywhere, for \
                     a machine that may not have the service yet) and nothing is started for it",
                )
            };
            note(
                k,
                journal,
                severity,
                msg,
                vec![
                    ("svc_name".to_string(), Value::Str(name.to_string())),
                    ("enabled".to_string(), Value::Bool(service.enabled())),
                ],
            );
            continue;
        };

        if def.enabled != service.enabled() {
            let was = def.enabled;
            if occasion.may_stop(name) {
                def.enabled = service.enabled();
                note(
                    k,
                    journal,
                    Severity::Info,
                    "the document overrides the definition's enabled value (ADR-017 §6 — enabled \
                     is configuration, and no operation writes it)",
                    vec![
                        ("svc_name".to_string(), Value::Str(name.to_string())),
                        ("definition".to_string(), Value::Bool(was)),
                        ("document".to_string(), Value::Bool(def.enabled)),
                    ],
                );
            } else {
                // Withheld, never discarded — `ADR-007` §3.6, and the operator
                // is told what to do instead rather than left to conclude the
                // line was ignored.
                note(
                    k,
                    journal,
                    Severity::Warn,
                    "a RELOAD does not change this service's enabled value, because stopping it \
                     would take away the means of undoing that (ADR-032 §3.2.1, ADR-018 §8). The \
                     document's answer stands and is applied at the next boot; `rivet apply \
                     restart confirm` is how to take it now",
                    vec![
                        ("svc_name".to_string(), Value::Str(name.to_string())),
                        ("running".to_string(), Value::Bool(was)),
                        ("document".to_string(), Value::Bool(service.enabled())),
                    ],
                );
            }
        }

        // **`critical` applies on BOTH occasions, immediately** — `ADR-045`
        // §4.3, and it is one of the few keys for which that is unambiguous.
        //
        // Unlike `enabled` there is nothing for a reload to withhold: this key
        // starts nothing and stops nothing, so `ADR-032` §3.2.1's reachability
        // carve-out — the whole reason `enabled` cannot answer — does not
        // arise. And unlike `memory` there is nothing already written into a
        // kernel object that would disagree with it: the fold is recomputed on
        // every `get status`, so the next answer after this line already uses
        // the new value. `rivet-config`'s schema publishes `immediately` and
        // this is what makes that true.
        if def.critical != service.critical() {
            let was = def.critical;
            def.critical = service.critical();
            note(
                k,
                journal,
                Severity::Info,
                "the document decides whether this service's health moves the MACHINE's health \
                 (ADR-045 §4). It starts and stops nothing; what changes is whether a degraded \
                 or unanswering instance of it makes `get status` report a degraded machine",
                vec![
                    ("svc_name".to_string(), Value::Str(name.to_string())),
                    ("definition".to_string(), Value::Bool(was)),
                    ("document".to_string(), Value::Bool(def.critical)),
                ],
            );
        }

        // **`memory` is a boot-time answer on both occasions**, and that is not
        // an oversight. `crate::supervise` writes the cap into a cgroup at the
        // spawn that creates it (`ADR-017` §3.1.3), so a reload that moved the
        // number on the definition would leave the RUNNING process at the old
        // cap while every record said otherwise — which is `ADR-034` §5.3's
        // hazard exactly: a change indistinguishable from success until the next
        // boot. The schema says `on-next-restart` for this key and means it.
        if occasion == Occasion::Boot {
            apply_cap(k, journal, defs, name, service.memory());
            // `apply_cap`'s footing exactly, for CPU — `ADR-007` §7 and
            // `ADR-015` §3's 2026-09-09 amendment.
            apply_cpu_cap(k, journal, defs, name, service.cpu());
        } else {
            // Compared against what WOULD apply rather than against the raw
            // key, so a cap the manifest already beats is silent — the operator
            // has changed nothing that could take effect either way.
            let would = rivet_core::service::effective_application_memory(def.requested_memory, def.default_memory, service.memory());
            if would != def.memory {
                note(
                    k,
                    journal,
                    Severity::Warn,
                    // **Corrected 2026-08-29, by the change that made it
                    // false.** It said *"It applies the next time this service
                    // starts"*, which was true only because nothing could start
                    // a service without a reboot. `ADR-017` §6.3.2's
                    // `apply service-restart` can — and it would write the OLD
                    // cap, because a reload does not touch `def.memory` at all
                    // (`apply_cap` runs on `Occasion::Boot` only) and
                    // `Supervisor::start` writes what the definition holds. An
                    // operator who edited a cap, reloaded, and restarted the
                    // service would have been told the new number was on and
                    // been wrong — `ADR-034` §5.3's hazard, produced by the
                    // sentence rather than by the code.
                    "a RELOAD does not change a memory cap: it is written into the cgroup when \
                     the service starts (ADR-017 §3.1.3), so moving it here would report a limit \
                     the running process does not have. IT IS TAKEN AT THE NEXT BOOT, not at the \
                     next start of this service — `rivet apply service-restart` would write the \
                     cap this definition already holds, which is the old one. `rivet apply \
                     restart confirm` is how to take it now",
                    vec![
                        ("svc_name".to_string(), Value::Str(name.to_string())),
                        (
                            "document".to_string(),
                            Value::Str(
                                would
                                    .limit
                                    .map_or_else(|| "none".to_string(), |l| l.to_string()),
                            ),
                        ),
                    ],
                );
            }
            // `memory`'s reload-warning footing exactly, for CPU.
            let cpu_would = rivet_core::service::effective_cpu(def.requested_cpu, service.cpu());
            if cpu_would != def.cpu {
                note(
                    k,
                    journal,
                    Severity::Warn,
                    "a RELOAD does not change a cpu cap: it is written into the cgroup when the \
                     service starts (ADR-017 §3), so moving it here would report a limit the \
                     running process does not have. IT IS TAKEN AT THE NEXT BOOT, not at the next \
                     start of this service — `rivet apply service-restart` would write the cap \
                     this definition already holds, which is the old one. `rivet apply restart \
                     confirm` is how to take it now",
                    vec![
                        ("svc_name".to_string(), Value::Str(name.to_string())),
                        (
                            "document".to_string(),
                            Value::Str(
                                cpu_would
                                    .limit
                                    .map_or_else(|| "none".to_string(), |l| l.to_string()),
                            ),
                        ),
                    ],
                );
            }
        }
    }

    // **Last, and after every `enabled` above has been settled.** The gate
    // reads the value the document just wrote, so it cannot run earlier.
}

/// **The operator's cap meets the manifest's request, and the lower wins.**
///
/// Ruled by the maintainer, 2026-08-24. The rule itself is
/// `rivet_core::service::effective_memory`, in the domain, with its own tests;
/// this is where the document's half arrives and where the outcome is said out
/// loud.
///
/// # Why it reports which side won, always
///
/// `ADR-011` part 2 — *the system says which one, at the moment the answer is
/// needed* — is the half that makes part 1 worth anything. An operator who
/// wrote `memory = "512M"` for an application that requested `128M` gets a
/// service capped at `128M`, and if the record said only *"capped at 128M"*
/// they would reasonably conclude their line had been ignored. It has not: a cap
/// can only tighten, and `manifest-lower` is the record saying so.
///
/// # What this does not do
///
/// **It does not write anything to a cgroup.** The number lands on the
/// definition; `crate::supervise` applies it immediately after the spawn that
/// creates the cgroup to apply it to. That split is `ADR-017` §3.1.3's — a
/// service's cgroup is a thing that exists once there is a process in it.
fn apply_cap(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    defs: &mut [ServiceDef],
    name: &str,
    cap: Option<rivet_core::service::MemoryLimit>,
) {
    let Some(def) = defs.iter_mut().find(|d| d.name == name) else {
        return;
    };
    let effective = rivet_core::service::effective_application_memory(def.requested_memory, def.default_memory, cap);
    if effective == def.memory {
        return;
    }
    def.memory = effective;

    let limit = effective
        .limit
        .map_or_else(|| "none".to_string(), |l| l.to_string());
    note(
        k,
        journal,
        Severity::Info,
        if effective.source.operator_tightened() {
            "the document caps this service's memory (ADR-007 §7.2; a cap runs the other way \
             from the signature, so it is the operator's — ruled 2026-08-24). It is written to \
             the cgroup at the next start, and reported there if the kernel refuses it"
        } else if effective.source == rivet_core::service::MemorySource::Default {
            "neither this application's manifest nor the document names a memory limit, so it \
             runs under the default one, half of the machine's memory (ADR-015 §3)"
        } else {
            "the document's memory cap is looser than what this service's own manifest asked \
             for, so the manifest's request stands: a cap can only tighten, never raise a value \
             the signature covers (ADR-015 §4.1, ruled 2026-08-24)"
        },
        vec![
            ("svc_name".to_string(), Value::Str(name.to_string())),
            ("memory".to_string(), Value::Str(limit)),
            (
                "memory_source".to_string(),
                Value::Str(effective.source.as_str().to_string()),
            ),
        ],
    );
}

/// [`apply_cap`]'s footing exactly, for CPU — `ADR-007` §7 and `ADR-015`
/// §3's 2026-09-09 amendment gave `cpu` the identical split `memory`
/// already had, and this is that ruling's other half arriving. See
/// [`apply_cap`]'s own documentation for the argument; this function does not
/// repeat it.
fn apply_cpu_cap(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    defs: &mut [ServiceDef],
    name: &str,
    cap: Option<rivet_core::service::CpuLimit>,
) {
    let Some(def) = defs.iter_mut().find(|d| d.name == name) else {
        return;
    };
    let effective = rivet_core::service::effective_cpu(def.requested_cpu, cap);
    if effective == def.cpu {
        return;
    }
    def.cpu = effective;

    let limit = effective
        .limit
        .map_or_else(|| "none".to_string(), |l| l.to_string());
    note(
        k,
        journal,
        Severity::Info,
        if effective.source.operator_tightened() {
            "the document caps this service's cpu (ADR-007 §7; a cap runs the other way from \
             the signature, so it is the operator's — ruled 2026-09-09, on memory's 2026-08-24 \
             footing). It is written to the cgroup at the next start, and reported there if the \
             kernel refuses it"
        } else {
            "the document's cpu cap is looser than what this service's own manifest asked for, \
             so the manifest's request stands: a cap can only tighten, never raise a value the \
             signature covers (ADR-015 §4)"
        },
        vec![
            ("svc_name".to_string(), Value::Str(name.to_string())),
            ("cpu".to_string(), Value::Str(limit)),
            (
                "cpu_source".to_string(),
                Value::Str(effective.source.as_str().to_string()),
            ),
        ],
    );
}

/// Say something in the supervisor's own voice, before there is a `Supervisor`
/// to say it through. The same helper `crate::etc` keeps, for the same reason.
fn note(
    k: &mut dyn Kernel,
    journal: &mut Journal,
    sev: Severity,
    msg: &str,
    fields: Vec<(String, Value)>,
) {
    let corr = journal.next_correlation();
    journal.emit(k, sev, SELF_NAME, &corr, msg, fields);
}

fn count(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    //
    // **Added 2026-08-24**, and it should have been here from the start: this
    // crate sets `expect_used`, `panic` and `unwrap_used` to `deny` rather than
    // `warn` (`AGENTS.md` §10.5), so the first `expect` written in this module
    // was six `cargo clippy --all-targets` errors rather than six warnings. The
    // module had none until then only because every fixture happened to be
    // infallible.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::definitions;
    use crate::testkernel::{FakeKernel, Script};

    fn journal() -> Journal {
        Journal::new("b".to_string(), 1, crate::supervise::ACTOR.to_string())
    }

    fn stored(k: &mut FakeKernel, text: &str) {
        let path = ConfigStore::at_default().machine_path();
        k.files_mut()
            .insert(path, Script::always(Ok(text.to_string())));
    }

    #[test]
    fn a_machine_with_nothing_stored_is_unconfigured_and_not_a_fault() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        assert_eq!(load(&mut k, &mut j), Desired::Unconfigured);
        assert!(
            k.records
                .iter()
                .any(|r| r.msg.starts_with("no configuration is stored")),
            "an unconfigured machine must say so"
        );
    }

    #[test]
    fn a_stored_document_is_read_and_its_version_reported() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        stored(&mut k, "version = 1\n[network.eth0]\nmethod = \"dhcp\"\n");
        let desired = load(&mut k, &mut j);
        assert!(matches!(desired, Desired::Stored(_)));
        assert_eq!(desired.document().network().len(), 1);
    }

    /// The branch that decides whether a damaged `/state` bricks the machine.
    #[test]
    fn a_document_that_will_not_parse_degrades_loudly_and_keeps_the_definitions() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        stored(&mut k, "version = 1\nverison = 2\n");
        let desired = load(&mut k, &mut j);
        assert!(matches!(desired, Desired::Unreadable { .. }));
        assert!(
            k.records
                .iter()
                .any(|r| r.sev == Severity::Error && r.msg.starts_with("DEGRADED")),
        );
        // And the definitions are untouched, which is the half that matters.
        //
        // ~~`defs.iter().all(|d| d.enabled)`~~ — **rewritten 2026-08-23**, when
        // `rivet-timed` arrived with `enabled: false` and turned an assertion
        // about *untouched* into an assertion that every service is on. The two
        // agreed only for as long as every definition happened to start
        // enabled; comparing against the pristine list says what was meant.
        let mut defs = definitions::system_services();
        apply_desired_state(&mut k, &mut j, &mut defs, &desired);
        assert_eq!(defs, definitions::system_services());
    }

    /// A card returning `EIO` must not be read as "this machine is
    /// unconfigured", or the next thing to touch it would re-provision over a
    /// card fault. `rivet-store`'s `load` makes that distinction and this
    /// checks it survives the port adapter.
    #[test]
    fn an_io_error_is_not_mistaken_for_an_unconfigured_machine() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        let path = ConfigStore::at_default().machine_path();
        k.files_mut().insert(path, Script::always(Err(Errno::EIO)));
        assert!(matches!(load(&mut k, &mut j), Desired::Unreadable { .. }));
    }

    #[test]
    fn the_documents_enabled_value_wins_over_the_definitions() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        stored(
            &mut k,
            "version = 1\n[services.rivet-timed]\nenabled = false\n",
        );
        let desired = load(&mut k, &mut j);
        let mut defs = definitions::system_services();
        apply_desired_state(&mut k, &mut j, &mut defs, &desired);

        let timed = defs
            .iter()
            .find(|d| d.name == definitions::TIMED_NAME)
            .map(|d| d.enabled);
        assert_eq!(
            timed,
            Some(false),
            "ADR-017 §6 — enabled is configuration"
        );
        let netd = defs
            .iter()
            .find(|d| d.name == definitions::NETD_NAME)
            .map(|d| d.enabled);
        assert_eq!(
            netd,
            Some(true),
            "a service the document is silent about is untouched"
        );
    }

    #[test]
    fn a_service_the_image_does_not_define_is_reported_rather_than_refused() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        stored(
            &mut k,
            "version = 1\n[services.nothing-here]\nenabled = false\n",
        );
        let desired = load(&mut k, &mut j);
        let mut defs = definitions::system_services();
        apply_desired_state(&mut k, &mut j, &mut defs, &desired);
        assert!(k.records.iter().any(|r| {
            r.sev == Severity::Warn
                && r.msg
                    .starts_with("the document configures a service this image does not define")
        }));
    }

    /// **An enabled declaration with nothing behind it is not the same fact**,
    /// and reporting it as *nothing is started for it* said the harmless half.
    /// `ADR-017` §6: `apply uninstall` removes the tree and leaves the table, so
    /// this is the shape a machine is left in after one — the operator asking
    /// for a program the machine does not have.
    #[test]
    fn an_enabled_declaration_with_no_artifact_is_reported_as_an_orphan() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        stored(
            &mut k,
            "version = 1\n[services.nothing-here]\nenabled = true\n",
        );
        let desired = load(&mut k, &mut j);
        let mut defs = definitions::system_services();
        apply_desired_state(&mut k, &mut j, &mut defs, &desired);

        let orphan = k
            .records
            .iter()
            .find(|r| r.msg.starts_with("ORPHANED DECLARATION"))
            .unwrap_or_else(|| {
                panic!(
                    "an enabled service with no artifact was not reported as an orphan: {:?}",
                    k.records.iter().map(|r| &r.msg).collect::<Vec<_>>()
                )
            });
        assert_eq!(orphan.sev, Severity::Warn);
        assert!(
            orphan.msg.contains("rivet apply edit"),
            "the orphan report does not name what clears it: {}",
            orphan.msg
        );
    }

    /// ~~`ADR-007` §3.6 — the system never accepts an input it will discard.
    /// Both of these are discarded today, so both have to be said.~~
    ///
    /// **Rewritten 2026-08-24.** Neither is discarded any more and the two
    /// stopped being discarded for opposite reasons, which is the ruling in one
    /// test: `listen` is not the operator's at all now, so
    /// `rivet-config` refuses it and this function never sees one; `memory`
    /// **is** the operator's, so it is applied rather than warned about.
    #[test]
    fn the_operators_cap_reaches_the_definition_rather_than_being_warned_about() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        stored(
            &mut k,
            "version = 1\n[services.rivet-timed]\nenabled = true\nmemory = \"64M\"\n",
        );
        let desired = load(&mut k, &mut j);
        let mut defs = definitions::system_services();
        apply_desired_state(&mut k, &mut j, &mut defs, &desired);

        let timed = defs
            .iter()
            .find(|d| d.name == definitions::TIMED_NAME)
            .expect("a system service");
        assert_eq!(
            timed.memory.limit.map(rivet_core::service::MemoryLimit::bytes),
            Some(64 * 1024 * 1024)
        );
        // A system service has no manifest, so the operator's cap stands alone.
        assert_eq!(
            timed.memory.source,
            rivet_core::service::MemorySource::Document
        );
        assert!(
            k.records.iter().any(|r| r.msg.contains("caps this service's memory")),
            "{:?}",
            k.records
        );
        // And nothing else was touched by it.
        let netd = defs
            .iter()
            .find(|d| d.name == definitions::NETD_NAME)
            .expect("a system service");
        assert_eq!(netd.memory, rivet_core::service::EffectiveMemory::UNSTATED);
    }

    /// The other half, and the one an operator will get wrong: a cap **above**
    /// what the artifact asked for does not raise it. Raising it would be the
    /// document rewriting a value the signature covers (`ADR-015` §4.1).
    #[test]
    fn an_operators_cap_above_the_manifests_request_leaves_the_request_standing() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        stored(
            &mut k,
            "version = 1\n[services.photo-index]\nenabled = true\nmemory = \"512M\"\n",
        );
        let desired = load(&mut k, &mut j);

        // An application definition, as `crate::applications` builds one.
        let manifest = rivet_core::package::application::Manifest::new(
            rivet_core::package::application::ProgramKind::Daemon,
        )
        .with_memory(Some(
            rivet_core::package::application::Memory::parse("128M").expect("valid"),
        ));
        let name = rivet_core::service::ServiceName::new("photo-index").expect("valid");
        let mut defs = vec![crate::applications::definition(
            rivet_core::service::BUILT_IN_VOLUME,
            &name,
            &manifest,
            &[],
        )];

        apply_desired_state(&mut k, &mut j, &mut defs, &desired);

        assert_eq!(
            defs[0].memory.limit.map(rivet_core::service::MemoryLimit::bytes),
            Some(128 * 1024 * 1024),
            "the operator's higher cap raised a signed request"
        );
        assert_eq!(
            defs[0].memory.source,
            rivet_core::service::MemorySource::ManifestLower
        );
        assert!(
            k.records.iter().any(|r| r.msg.contains("looser than")),
            "an operator whose cap did nothing was not told: {:?}",
            k.records
        );
    }

    /// **Where the manifest asks for nothing, the operator's cap replaces the
    /// default limit in either direction** — above it as readily as below.
    #[test]
    fn an_operators_cap_replaces_the_default_limit_above_and_below() {
        for (cap, bytes) in [("900M", 900_u64), ("64M", 64)] {
            let mut k = FakeKernel::new();
            let mut j = journal();
            stored(
                &mut k,
                &format!("version = 1\n[services.photo-index]\nenabled = true\nmemory = \"{cap}\"\n"),
            );
            let desired = load(&mut k, &mut j);
            let manifest = rivet_core::package::application::Manifest::new(
                rivet_core::package::application::ProgramKind::Daemon,
            );
            let name = rivet_core::service::ServiceName::new("photo-index").expect("valid");
            let mut def = crate::applications::definition(
                rivet_core::service::BUILT_IN_VOLUME,
                &name,
                &manifest,
                &[],
            );
            // As discovery leaves it on a machine with 1 GiB: half of that.
            let default = rivet_core::service::default_application_memory(1024 * 1024 * 1024);
            def.default_memory = default;
            def.memory = rivet_core::service::effective_application_memory(None, default, None);
            let mut defs = vec![def];

            apply_desired_state(&mut k, &mut j, &mut defs, &desired);

            assert_eq!(
                defs[0].memory.limit.map(rivet_core::service::MemoryLimit::bytes),
                Some(bytes * 1024 * 1024),
                "the document's {cap} did not replace the default"
            );
            assert_eq!(defs[0].memory.source, rivet_core::service::MemorySource::Document);
        }
    }

    /// And the direction that makes it a cap at all: lower wins.
    #[test]
    fn an_operators_cap_below_the_manifests_request_tightens_it() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        stored(
            &mut k,
            "version = 1\n[services.photo-index]\nenabled = true\nmemory = \"32M\"\n",
        );
        let desired = load(&mut k, &mut j);
        let manifest = rivet_core::package::application::Manifest::new(
            rivet_core::package::application::ProgramKind::Daemon,
        )
        .with_memory(Some(
            rivet_core::package::application::Memory::parse("128M").expect("valid"),
        ));
        let name = rivet_core::service::ServiceName::new("photo-index").expect("valid");
        let mut defs = vec![crate::applications::definition(
            rivet_core::service::BUILT_IN_VOLUME,
            &name,
            &manifest,
            &[],
        )];

        apply_desired_state(&mut k, &mut j, &mut defs, &desired);

        assert_eq!(
            defs[0].memory.limit.map(rivet_core::service::MemoryLimit::bytes),
            Some(32 * 1024 * 1024)
        );
        assert_eq!(
            defs[0].memory.source,
            rivet_core::service::MemorySource::DocumentLower
        );
    }

    /// The supervisor is not an author of desired state. If it ever becomes
    /// one, this is the test that has to be deleted on purpose.
    #[test]
    fn the_store_adapter_refuses_every_write() {
        let mut k = FakeKernel::new();
        let mut fs = ReadOnlyStore { k: &mut k, writable: None };
        assert_eq!(fs.create_dir_all("/state/config"), Err(StoreErrno::EROFS));
        assert_eq!(fs.write("/state/config/x", b"x"), Err(StoreErrno::EROFS));
        assert_eq!(fs.sync_file("/state/config/x"), Err(StoreErrno::EROFS));
        assert_eq!(fs.rename("/a", "/b"), Err(StoreErrno::EROFS));
        assert_eq!(fs.sync_dir("/state/config"), Err(StoreErrno::EROFS));
        assert_eq!(fs.remove_file("/state/config/x"), Err(StoreErrno::EROFS));
    }

    /// **The exception is exactly one file, and the document is not it.**
    ///
    /// `.rulings/` R-139 lets the supervisor write the last-good copy because
    /// that copy is not desired state. The risk in granting it is that the
    /// grant is wider than the argument — so this pins the boundary from both
    /// sides on the SAME adapter instance: the three paths one atomic commit
    /// touches are permitted, and the machine's own document, which sits in the
    /// same directory, still refuses.
    #[test]
    fn the_write_exception_reaches_the_last_good_copy_and_nothing_else() {
        let store = ConfigStore::at_default();
        let allowed = Writable::last_good_of(&store);
        let mut k = FakeKernel::new();
        let mut fs = ReadOnlyStore {
            k: &mut k,
            writable: Some(allowed.clone()),
        };

        // Permitted: the temporary, the target, and the directory the rename
        // commits in.
        assert_eq!(fs.write(&allowed.temp, b"x = 1\n"), Ok(()));
        assert_eq!(fs.sync_file(&allowed.temp), Ok(()));
        assert_eq!(fs.rename(&allowed.temp, &allowed.target), Ok(()));
        assert_eq!(fs.sync_dir(&allowed.dir), Ok(()));

        // Refused: the desired-state document itself, which lives beside it.
        let doc = store.machine_path();
        assert_ne!(doc, allowed.target, "the fixture would prove nothing");
        assert_eq!(
            fs.write(&doc, b"version = 1\n"),
            Err(StoreErrno::EROFS),
            "the supervisor wrote the machine's document, which is the one thing this \
             adapter exists to prevent"
        );

        // Refused: a rename whose SOURCE is permitted but whose target is not.
        // Checking one end would let the temporary be moved anywhere.
        assert_eq!(
            fs.rename(&allowed.temp, "/state/config/machine.toml"),
            Err(StoreErrno::EROFS)
        );
        // And directory creation and unlinking stay refused outright.
        assert_eq!(fs.create_dir_all(&allowed.dir), Err(StoreErrno::EROFS));
        assert_eq!(fs.remove_file(&allowed.target), Err(StoreErrno::EROFS));
    }

    /// **And it actually happens on the path a boot takes.**
    ///
    /// The defect was never that the write was wrong — it was that nothing
    /// reached it, for months, while a warning said so every boot. So this
    /// drives `load` exactly as the supervisor does and then asks the kernel
    /// whether the file is there. Asserting the syscalls rather than the
    /// adapter is the point: a unit test of `ReadOnlyStore` would have passed
    /// throughout the period when this never worked.
    #[test]
    fn a_document_that_parses_leaves_a_last_good_copy_behind() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        stored(&mut k, "version = 1\n");

        let desired = load(&mut k, &mut j);
        assert!(matches!(desired, Desired::Stored(_)), "the fixture must parse");

        let target = ConfigStore::at_default().last_good_path();
        assert!(
            k.live_files().contains(&target),
            "no last-good copy is on the card. The supervisor asked for one and the adapter \
             refused it, which is the state PLAN.md §127.2 found on the board. Files present: \
             {:?}",
            k.live_files()
        );
        // **And the temporary is gone**, which is the difference between a
        // commit and a half-written file the next boot would read.
        let temp = rivet_store::atomic::in_progress(&target);
        assert!(
            !k.live_files().contains(&temp),
            "the in-progress copy survived, so the rename did not happen"
        );
        // The write went through the `O_NOFOLLOW` path — Wave 2.1's guard, on
        // a file under `/state` that root writes.
        assert!(
            k.nofollow_writes.iter().any(|p| p == &temp),
            "the last-good copy was written through the symlink-following path"
        );
    }

    /// The two `Errno` types are separate on purpose and the numbers must not
    /// be reinterpreted on the way across.
    #[test]
    fn the_errno_conversion_carries_the_number_and_nothing_else() {
        assert_eq!(to_store_errno(Errno::ENOENT), StoreErrno(Errno::ENOENT.0));
        assert_eq!(to_store_errno(Errno::EIO).0, Errno::EIO.0);
    }

    // -- the document names the launcher and the compositor ------------------

    /// **A document that names neither role keeps today's machine**: the
    /// image's own launcher and compositor. Every document written before the
    /// keys existed is one of these, and so is a machine with no document.
    #[test]
    fn a_document_that_names_no_role_gives_both_to_the_images_own() {
        let images_own = DisplayAuthority {
            launcher: "rivet-launcher".to_string(),
            compositor: "rivet-displayd".to_string(),
        };
        assert_eq!(DisplayAuthority::default(), images_own);

        let mut k = FakeKernel::new();
        let mut j = journal();
        stored(&mut k, "version = 1\n[kiosk]\nauto-run = \"aura-renderer\"\n");
        assert_eq!(display_authority(&load(&mut k, &mut j)), images_own);

        assert_eq!(display_authority(&Desired::Unconfigured), images_own);

        let mut k = FakeKernel::new();
        let mut j = journal();
        stored(&mut k, "version = 1\nverison = 2\n");
        let unreadable = load(&mut k, &mut j);
        assert!(matches!(unreadable, Desired::Unreadable { .. }));
        assert_eq!(display_authority(&unreadable), images_own);
    }

    /// Each role the document names replaces the default for that role only.
    #[test]
    fn a_named_role_replaces_only_its_own_default() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        stored(&mut k, "version = 1\n[kiosk]\ncompositor = \"cosmic-comp\"\n");
        let authority = display_authority(&load(&mut k, &mut j));
        assert_eq!(authority.compositor, "cosmic-comp");
        assert_eq!(authority.launcher, "rivet-launcher");
    }

    // -- `mode` is the switch, `.rulings/` R-164 and R-165 -------------------

    /// `mode = headless` with a `[kiosk]` table: `headless`, and the table is
    /// untouched. Break the `Some(MachineMode::Headless) => KioskMode::headless()`
    /// arm and this fails.
    #[test]
    fn headless_mode_runs_no_kiosk_even_with_a_table() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        stored(
            &mut k,
            "version = 1\nmode = \"headless\"\n[kiosk]\nauto-run = \"aura-renderer\"\n\
             locked = true\n",
        );
        let desired = load(&mut k, &mut j);
        let mode = kiosk_mode(&desired);
        assert!(!mode.is_kiosk(), "headless must win over a present table");
        assert_eq!(mode.mode, rivet_core::operation::status::MODE_HEADLESS);
        assert!(!mode.locked);
        assert!(mode.auto_run.is_none());
        // The table itself is still in the document — nothing clears it.
        assert_eq!(
            desired.document().kiosk().and_then(|k| k.auto_run()),
            Some("aura-renderer")
        );
    }

    /// `mode = kiosk` with a `[kiosk]` table: `kiosk`, with `locked`/`auto-run`
    /// carried exactly as the table wrote them.
    #[test]
    fn kiosk_mode_with_a_table_reads_locked_and_auto_run_from_it() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        stored(
            &mut k,
            "version = 1\nmode = \"kiosk\"\n[kiosk]\nauto-run = \"aura-renderer\"\n\
             locked = true\n",
        );
        let desired = load(&mut k, &mut j);
        let mode = kiosk_mode(&desired);
        assert!(mode.is_kiosk());
        assert!(mode.locked);
        assert_eq!(mode.auto_run.as_deref(), Some("aura-renderer"));
        assert!(mode.why_unconfigured.is_none());
    }

    /// `mode = kiosk` with no `[kiosk]` table: reported as asked-for-but-not-
    /// configured, never refused. `load` must still succeed.
    #[test]
    fn kiosk_mode_with_no_table_is_reported_not_refused() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        stored(&mut k, "version = 1\nmode = \"kiosk\"\n");
        let desired = load(&mut k, &mut j);
        assert!(matches!(desired, Desired::Stored(_)), "not a refusal");
        let mode = kiosk_mode(&desired);
        assert!(mode.is_kiosk(), "asked for is still reported");
        assert!(!mode.locked);
        assert!(mode.auto_run.is_none());
        assert!(
            mode.why_unconfigured
                .as_deref()
                .is_some_and(|why| why.contains("no [kiosk] table")),
            "{:?}",
            mode.why_unconfigured
        );
    }

    /// No `mode` at all, with a `[kiosk]` table: today's behaviour, unchanged.
    /// No `mode` and no table: headless.
    #[test]
    fn no_mode_falls_back_to_the_tables_presence_exactly_as_today() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        stored(&mut k, "version = 1\n[kiosk]\nauto-run = \"aura-renderer\"\n");
        let with_table = kiosk_mode(&load(&mut k, &mut j));
        assert!(with_table.is_kiosk());
        assert_eq!(with_table.auto_run.as_deref(), Some("aura-renderer"));

        let mut k2 = FakeKernel::new();
        let mut j2 = journal();
        stored(&mut k2, "version = 1\n");
        let without_table = kiosk_mode(&load(&mut k2, &mut j2));
        assert!(!without_table.is_kiosk());
    }

    /// **A document shaped like `rivet-01`'s today** (no `mode`, `[kiosk]`
    /// `auto-run = "aura-renderer"`, unlocked) must answer exactly as it does
    /// today. This is the regression test R-164 exists not to cause.
    #[test]
    fn a_document_shaped_like_rivet_01_answers_exactly_as_today() {
        let mut k = FakeKernel::new();
        let mut j = journal();
        stored(
            &mut k,
            "version = 1\nhostname = \"rivet-01\"\n[services.aura-renderer]\n\
             enabled = true\n\n[kiosk]\nauto-run = \"aura-renderer\"\n",
        );
        let mode = kiosk_mode(&load(&mut k, &mut j));
        assert!(mode.is_kiosk());
        assert!(!mode.locked);
        assert_eq!(mode.auto_run.as_deref(), Some("aura-renderer"));
        assert!(mode.why_unconfigured.is_none());
        assert!(mode.why_unknown.is_none());
    }
}
