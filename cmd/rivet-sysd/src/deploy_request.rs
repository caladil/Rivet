//! The install request — the maintainer's 2026-08-24 ruling.
//!
//! `rivet apply install <digest> confirm` hands over the artifact and returns
//! before a byte is written, because an update writes 640 MiB and an operator's
//! connection must not own that write (the maintainer's 2026-08-24 ruling). What it
//! hands it *to* was the last unruled piece, and the ruling is a file:
//!
//! > The CLI writes `<payload path>` into a file under `/state`; each
//! > convergence pass checks for one and, if there is one, builds
//! > `definitions::deploy(path)` and runs it once, removing the request when it
//! > is started.
//!
//! **The file names a digest rather than a path since 2026-09-12** — see
//! `ADR-052` §7 below. The ruling's shape is untouched: a file, one convergence
//! pass, one install, the request removed before anything starts.
//!
//! This module is that check. It is the reading half of a channel whose writing
//! half is `cmd/rivet/src/deploy.rs`, whose format is
//! [`rivet_core::deploy::parse_request`], and whose one new idea is
//! [`crate::supervise::Supervisor::adopt`] — a service instance created at run
//! time, which `ADR-017` §3.2.2 is the amendment for.
//!
//! # It never touches `enabled`, and that was the trap
//!
//! `ADR-017` §6 makes `enabled` the operator's entire control surface. The
//! shortest implementation of this ruling is to compile the installer into
//! [`crate::definitions::system_services`] with `enabled: false` and have a
//! request flip it — and it is wrong, because it puts a second writer on the one
//! key the desired-state document owns. A request creates an **instance** no
//! document names. `Supervisor::adopt` carries the rest of that argument.
//!
//! # A request is attacker-reachable state, and this module is written that way
//!
//! `/state` is writable and survives reboots, so this file is the first thing in
//! Rivet that is both persistent and able to make the supervisor execute
//! something. Four decisions follow, and each of them is a test below:
//!
//! 1. **The size is read before the contents.** `rivet_core::deploy` caps a
//!    request at one digest plus a bounded principal; a `stat(2)` that reports
//!    more is a refusal, and
//!    the file is never opened. This process is PID 1's only child and it must
//!    not fail (`ADR-017` §3.1); allocating a card-sized `String` inside it is
//!    exactly the way it would.
//! 2. **The request is removed BEFORE anything is started**, not after. The
//!    ruling says *"removing the request when it is started"* and this is the
//!    order within that, chosen rather than assumed:
//!
//!    | | If power is lost between the two |
//!    | :-- | :-- |
//!    | remove, then adopt | the install did not happen; the operator asks again |
//!    | adopt, then remove | the install happens again on **every boot for ever** |
//!
//!    The second is not hypothetical on this machine: `/state` is where the
//!    request lives *because* it must survive a reboot, so a request that
//!    outlived its own install would be a permanent reinstall loop, and each
//!    iteration writes 640 MiB to an SD card. The cost of the safe direction is
//!    a lost request in a window of microseconds, which an operator can see —
//!    `rivet get install` reports nothing new — and repair by asking again.
//! 3. **A request that cannot be understood is removed too.** Leaving it would
//!    make every subsequent convergence pass re-read and re-refuse it, which is
//!    `ADR-017` §3.5's SD-card-destroying loop with a `stat(2)` instead of a
//!    `fork(2)`. What is *not* removed is a request this module could not even
//!    examine — see [`Watch::trouble_reported`], because a card that is failing
//!    to `stat` will fail to unlink too, and shouting about it once is the whole
//!    of what can be done.
//! 4. **The digest is not the payload.** Everything about the artifact the
//!    request names — signature, digests, product, architecture — is still
//!    refused by `cmd/rivet-deploy` under `ADR-029` §11's single-verifier rule.
//!    Writing a request buys the ability to *name* an artifact, not to install
//!    one. What it does not defend against is somebody who can already write
//!    `/state` as uid 0, and nothing here could: that is a machine whose
//!    `/state/app` they can write too (`ADR-015` §7). The boundary held is the
//!    unprivileged one — `cmd/rivet-image` creates `/state` with
//!    `root_owner=0:0`, so an application's own uid (`ADR-015` §5.1) cannot
//!    create a file in it.
//!
//! # A request names a DIGEST, since 2026-09-12 — `ADR-052` §7
//!
//! It named an absolute path until then, and [`resolve`] is the hop that
//! replaced it: the supervisor looks the digest up in this machine's own
//! artifact store and hands `cmd/rivet-deploy` the path it found. The installer
//! did not change and neither did [`crate::definitions::deploy`] — what changed
//! is that the path is now **derived** on this side of the channel rather than
//! **supplied** from the other side of it.
//!
//! **What that is worth, stated exactly.** It does not raise the boundary:
//! anything running as uid 0 can still write this file, and can write the store
//! as well. What it removes is the shape where the request file was a general
//! *"execute the installer against this path"* channel — the one thing on this
//! machine that was persistent, attacker-writable, and able to point a
//! privileged program at an arbitrary file. Now it can only point at something
//! a transfer already hashed into the store.
//!
//! # A request present at boot is honoured, and that is the ruling
//!
//! *"It survives a supervisor restart and a reboot"* is the property the
//! maintainer's 2026-08-24 install-request ruling chose option A **for**. So this runs before the first convergence as
//! well as inside the loop, and a machine that was powered off between the
//! request and the install performs it on the next boot. Item 2 above is what
//! keeps that *once* rather than *every time*.
//!
//! # Whose voice the records are in
//!
//! **The supervisor's own** (`crate::supervise::SELF_NAME`), never
//! `rivet-deploy`'s. `ADR-022` §5's correlation chain belongs to a running
//! install, and `rivet_core::operation::install`'s reader builds its chains by
//! filtering on the installer's service name — so a refusal emitted under that
//! name would make `rivet get install` report an install that never began,
//! unfinished, for ever. A request that was never turned into an install has no
//! install to belong to.
//!
//! **The consequence is a real gap and is not papered over:** `rivet get
//! install` cannot see a refused request. The refusal is in `rivet get log`,
//! under `rivet-sysd`, naming the request path and what was wrong with
//! it.
//!
//! # Who asked reaches a record here, and only here — added 2026-08-24
//!
//! `ADR-028` §4.5. A request now carries a `rivet_core::deploy::Principal`,
//! and this module writes it into the record it makes when it takes the request
//! up: [`PRINCIPAL_FIELD`] for an agent, and the principal's own sentence in the
//! message for a human.
//!
//! **Read this next part before assuming more.** Three limits, each of which is
//! a fact about the design rather than an omission waiting to be tidied:
//!
//! 1. **It is attribution, not authorisation.** Nothing here checks anything.
//!    Anything running as uid 0 can write `deploy::REQUEST_PATH` by hand and
//!    claim any principal it likes — the boundary is still *root on `/state`*,
//!    unchanged. What improves is the record: the journal can answer *"who asked
//!    for this install"*, where the only previous answer was *"somebody who was
//!    root"*. And the claim is **checkable**, because the origin token is the
//!    same key the control plane's own records name for the connection the
//!    request arrived on.
//! 2. **The principal stops here.** `definitions::deploy` hands
//!    `cmd/rivet-deploy` one argument — the payload path — so the records
//!    *inside* an install still carry `deploy::ACTOR`, which is the process
//!    rather than a person. That is honest, and it is why `rivet get install`
//!    cannot report who asked: its chain is the installer's records and this
//!    record is not one of them.
//! 3. **A request that names nobody is refused**, by
//!    `deploy::parse_request`, and lands in [`Watch::refuse`] like any other
//!    malformed file. That is the pre-2026-08-24 format and it is refused rather
//!    than installed unattributed — `ADR-028` §4.5 argues it.

use rivet_core::deploy;

use crate::definitions;
use crate::journal::{Severity, Value};
use crate::kernel::{Errno, Kernel};
use crate::supervise::{AdoptError, Phase, Supervisor};

/// The field naming who a request said asked for the install.
///
/// `ADR-022` §3.2's structured fields, and the name is `principal` because that
/// is `ADR-028` §4's word — **which is exactly why the message beside it says,
/// every time, that this is attribution and not authority.** A field called
/// `principal` in a security-shaped record is the kind of thing a later reader
/// takes to mean something was checked.
const PRINCIPAL_FIELD: &str = "principal";

/// The structured half of a principal, for a record.
///
/// **Separate from the sentence, and both are written.** `ADR-006` §4.1: an
/// agent branches on the field, a human reads the string. The field is a token
/// the control plane's own records can be matched against with `==`, which is
/// the thing a reader following a chain of custody actually does.
/// **Shared, since 2026-09-06, and that is the point.** `crate::uninstall_request`
/// and `crate::repo_delete_request` had each spelled this themselves and both
/// had drifted to `Principal::render_fields()` — the WIRE form, which prefixes
/// `from=` because that is the field name in the request FILE. A board recorded
/// an install as `pinned-<hex>` and an uninstall beside it as `from=pinned-<hex>`,
/// so a reader matching on one spelling silently missed the other. One function,
/// three callers, `ADR-011` §3.
pub(crate) fn attribution(principal: &rivet_core::deploy::Principal) -> Vec<(String, Value)> {
    vec![(
        PRINCIPAL_FIELD.to_string(),
        Value::Str(principal.origin().as_token().to_string()),
    )]
}

/// **Where the digest a request names actually is** — `ADR-052` §7.
///
/// `/state/artifacts/objects/<digest>/` holds exactly one file, named by
/// `ADR-027`, and this is the `readdir` that finds it. The path it returns is
/// what `crate::definitions::deploy` puts on the installer's argument vector,
/// which is the one place in this program that a path and a digest meet.
///
/// # Three failures, and they are three sentences rather than one
///
/// A supervisor that answered *"could not resolve it"* would leave an operator
/// unable to tell a machine that never received the artifact from a machine
/// whose card is failing, and those are opposite actions. So:
///
/// * **not there** — the ordinary case, and the one an operator causes by
///   asking for a digest this machine was never sent;
/// * **unreadable** — a fault, and the journal already has the `errno`;
/// * **not exactly one file** — something wrote into the store by hand, since
///   an object arrives by renaming a complete directory into place. Nothing
///   here picks one.
///
/// **Nothing is opened and nothing is hashed.** Re-hashing 700 MB on a Pi to
/// re-answer what the transfer already answered would cost minutes inside PID
/// 1's only child, and a second reader of the container is what `ADR-029` §11
/// forbids. The artifact is verified by `cmd/rivet-deploy`, which is about to
/// run.
fn resolve(k: &mut dyn Kernel, digest: &rivet_core::artifact::Digest) -> Result<String, String> {
    let store = rivet_core::artifact::Store::on_this_machine();
    let dir = store.object_dir(digest);
    let entries = match k.list_dir_entries(&dir) {
        Ok(entries) => entries,
        Err(Errno::ENOENT) => {
            return Err(format!(
                "this machine's artifact store does not hold {digest}. An install names a digest \
                 and the supervisor resolves it under {dir}; there is no form of a request that \
                 names a path (ADR-052 §7). Send the artifact to this machine first — `cargo \
                 rivet deploy` transfers it into the store and asks for the install by the \
                 digest it sent"
            ));
        }
        Err(e) => {
            return Err(format!(
                "{dir} could not be read ({e}), so whether this machine holds {digest} is \
                 UNKNOWN. That is not the same as not holding it: a failing or read-only /state \
                 answers this way"
            ));
        }
    };

    let mut files = entries.into_iter().filter(|name| name != "." && name != "..");
    match (files.next(), files.next()) {
        (Some(name), None) => Ok(format!("{dir}/{name}")),
        _ => Err(format!(
            "{dir} is in the store and does not hold exactly one artifact, so which file \
             {digest} names is UNKNOWN. An object arrives by renaming a complete directory into \
             place, so this one was written by something other than a transfer. Nothing here \
             will pick one"
        )),
    }
}

/// The state one convergence loop keeps about the request file.
///
/// **It holds latches and nothing else.** Everything that decides anything is
/// read from the filesystem on the pass that needs it (`AGENTS.md` §5 — the
/// kernel is the authority, never a cached verdict); what is remembered here is
/// only *what has already been said out loud*, because the alternative to a
/// latch is the same sentence on every wakeup, which hides the rest of the
/// journal.
#[derive(Debug, Default)]
pub struct Watch {
    /// Whether the "this file cannot be examined" report has been made.
    ///
    /// Distinct from every other refusal in this module because it is the one
    /// where **nothing was removed**: an `EIO` on `stat(2)` is a card that will
    /// also refuse to unlink, so the file stays and this branch is reached again
    /// on every pass. Reported once per episode, and re-armed when the fault
    /// clears — a fault that comes back is news again.
    trouble_reported: bool,
    /// Whether "an install is already running" has been said for the request
    /// that is currently waiting.
    ///
    /// Also once, and for a sharper reason: a running install emits output
    /// records, each of which wakes the loop and runs a convergence pass, so
    /// this branch would otherwise interleave one line of noise with every line
    /// of the install it is describing.
    busy_reported: bool,
}

impl Watch {
    pub const fn new() -> Self {
        Self {
            trouble_reported: false,
            busy_reported: false,
        }
    }

    /// One pass: look for a request, and turn it into a service instance.
    ///
    /// Called immediately before each [`Supervisor::converge`], so that a
    /// request found on this pass is started by this pass.
    pub fn poll(&mut self, k: &mut dyn Kernel, sup: &mut Supervisor) {
        // **Not on the way down.** A `SIGTERM` has arrived, every service is
        // being stopped, and starting a 640 MiB write into that would be a
        // machine that cannot be rebooted. The request is left where it is: it
        // is on `/state`, so the next boot finds it, which is the whole reason
        // it is there.
        if sup.phase() != Phase::Converging {
            return;
        }

        let bytes = match k.file_size(deploy::REQUEST_PATH) {
            Ok(bytes) => bytes,
            // The ordinary state of every machine that is not installing
            // anything. Silent, deliberately: a line per convergence pass saying
            // that nobody asked for an install is the noise that hides the
            // pass where somebody did.
            Err(Errno::ENOENT) => {
                // No request, so any episode of trouble about one is over.
                self.trouble_reported = false;
                return;
            }
            Err(e) => {
                self.report_trouble(k, sup, "could not be examined", e);
                return;
            }
        };

        if bytes > deploy::REQUEST_MAX_BYTES {
            self.refuse(
                k,
                sup,
                format!(
                    "it is {bytes} bytes and a request may be at most {} — so it was NOT read \
                     into this process at all",
                    deploy::REQUEST_MAX_BYTES
                ),
            );
            return;
        }

        let contents = match k.read_file(deploy::REQUEST_PATH) {
            Ok(contents) => contents,
            // Includes a file that is not UTF-8, which is what a truncated
            // write or a deliberately hostile one looks like from here.
            Err(e) => {
                self.refuse(k, sup, format!("it could not be read: {e}"));
                return;
            }
        };

        let (digest, principal) = match deploy::parse_request(&contents) {
            Ok(request) => (request.digest, request.principal),
            Err(problem) => {
                self.refuse(k, sup, problem.why());
                return;
            }
        };

        // **The digest becomes a path here and nowhere else** — `ADR-052` §7.
        // A request names an object in this machine's own artifact store; what
        // the installer is handed is still a path, because that is what
        // `crate::definitions::deploy` and `cmd/rivet-deploy` take and neither
        // needed to change. The resolution is a `readdir` on a directory whose
        // name came out of a checked digest, so nothing a request says can
        // point it anywhere else.
        let payload = match resolve(k, &digest) {
            Ok(path) => path,
            Err(why) => {
                self.refuse(k, sup, why);
                return;
            }
        };

        // **Removed before anything is started**, and the request is gone from
        // the card before the definition exists. See this module's second
        // decision: the other order reinstalls on every boot for ever.
        if !self.consume(k, sup, &payload) {
            return;
        }

        match sup.adopt(definitions::deploy(&payload)) {
            Ok(()) => {
                let mut fields = vec![
                    ("payload".to_string(), Value::Str(payload)),
                    // **Both, and not one.** The path is where the installer
                    // will read from and the digest is what the operator asked
                    // for; a record with only the first cannot be matched
                    // against the request that caused it, and a record with
                    // only the second cannot be matched against the
                    // installer's own records, which name the file.
                    ("digest".to_string(), Value::Str(digest.to_string())),
                    (
                        "svc_name".to_string(),
                        Value::Str(deploy::SERVICE_NAME.to_string()),
                    ),
                ];
                fields.extend(attribution(&principal));
                sup.note(
                    k,
                    Severity::Info,
                    &format!(
                        "an install was requested and the installer is now a service of this \
                         supervisor. NOTHING IS INSTALLED YET: it starts on this convergence \
                         pass, writes into the INACTIVE slot, and the machine does not reboot \
                         (ADR-029 §9.1). `rivet get install` is the account of what happens \
                         next. WHO ASKED: {}",
                        principal.describe()
                    ),
                    fields,
                );
                self.busy_reported = false;
            }
            Err(AdoptError::Busy { name }) => {
                // Reachable only through a race this design does not otherwise
                // produce — the request was consumed above, so a second one
                // cannot exist while this install runs unless somebody wrote it
                // by hand between the two passes. Said once, and the request is
                // already gone, so the operator must ask again when the running
                // install ends.
                if !self.busy_reported {
                    self.busy_reported = true;
                    let mut fields = vec![
                        ("payload".to_string(), Value::Str(payload)),
                        ("svc_name".to_string(), Value::Str(name)),
                    ];
                    // **The discarded request is attributed too.** Whoever asked
                    // is the person who has to ask again, and a record of a
                    // request that was thrown away is exactly the one somebody
                    // goes looking for.
                    fields.extend(attribution(&principal));
                    sup.note(
                        k,
                        Severity::Warn,
                        &format!(
                            "an install was requested while one was already running, and the \
                             SECOND REQUEST HAS BEEN DISCARDED. One installer writes one inactive \
                             slot, so two at once would be two writers on the same partition. Ask \
                             again once `rivet get install` reports the running one finished. \
                             WHO ASKED: {}",
                            principal.describe()
                        ),
                        fields,
                    );
                }
            }
            Err(e @ AdoptError::Definition(_)) => {
                // Not reachable from any payload — every field
                // `definitions::deploy` fills is a constant and a test in that
                // module pins that it validates. Reported rather than ignored
                // because a definition that stopped validating is a build
                // defect, and the machine is the only place it would show.
                let mut fields = vec![
                    ("payload".to_string(), Value::Str(payload)),
                    ("detail".to_string(), Value::Str(e.to_string())),
                ];
                fields.extend(attribution(&principal));
                sup.note(
                    k,
                    Severity::Error,
                    "REFUSED the installer's own definition is not one this supervisor can run, \
                     so the install did not start and the request has already been removed. This \
                     is a defect in this build rather than anything about the payload",
                    fields,
                );
            }
        }
    }

    /// Remove the request, and say whether an install may now be started.
    ///
    /// `false` means **do not start anything**. A request that could not be
    /// removed is one that will be found again on the next pass and on the next
    /// boot, so starting an install from it is starting an install that repeats
    /// for ever.
    fn consume(&mut self, k: &mut dyn Kernel, sup: &mut Supervisor, payload: &str) -> bool {
        match k.remove_file(deploy::REQUEST_PATH) {
            // The episode is over: the file is gone, so a card that starts
            // failing again is news again.
            Ok(()) => self.trouble_reported = false,
            // Somebody removed it between the read and here. Nothing was asked
            // for any more; do nothing, and say nothing, because a retracted
            // request is not a fault.
            Err(Errno::ENOENT) => {
                self.trouble_reported = false;
                return false;
            }
            Err(e) => {
                self.report_trouble(k, sup, "could not be removed", e);
                return false;
            }
        }

        // **The removal is flushed before the install starts.** A power cut
        // after the unlink but before the metadata reaches the card would
        // resurrect the request, and the machine would install the same payload
        // again on the next boot — the exact failure the ordering above exists
        // to prevent, arriving through the page cache instead of through the
        // code. Reported and not fatal: the request *is* gone from this
        // running system either way.
        if let Err(e) = k.sync_dir(rivet_core::state::STATE_MOUNT_POINT) {
            sup.note(
                k,
                Severity::Warn,
                "the install request was removed but the removal was not flushed to the card, so \
                 a power cut in the next moments could leave it there and install the same \
                 payload again on the next boot",
                vec![
                    (
                        "path".to_string(),
                        Value::Str(deploy::REQUEST_PATH.to_string()),
                    ),
                    ("payload".to_string(), Value::Str(payload.to_string())),
                    ("detail".to_string(), Value::Str(e.to_string())),
                ],
            );
        }
        true
    }

    /// A request that is a file but is not a request: remove it, and say why.
    ///
    /// **Removed**, because the alternative is re-reading and re-refusing it on
    /// every convergence pass for the life of the machine.
    fn refuse(&mut self, k: &mut dyn Kernel, sup: &mut Supervisor, why: String) {
        let removed = k.remove_file(deploy::REQUEST_PATH);
        if removed.is_ok() {
            self.trouble_reported = false;
        }
        sup.note(
            k,
            Severity::Error,
            "REFUSED an install request this machine will not act on, and NOTHING HAS BEEN \
             INSTALLED. A request is one line saying WHO ASKED and then the BLAKE3 DIGEST of an \
             artifact this machine already holds, written by `rivet apply install <digest> \
             confirm` — `problem` says which of those this file failed. Nothing is installed \
             without a request that names somebody (ADR-028 §4.5) and nothing is installed that \
             is not already in this machine's artifact store (ADR-052 §7). The request has been \
             removed",
            vec![
                (
                    "path".to_string(),
                    Value::Str(deploy::REQUEST_PATH.to_string()),
                ),
                ("problem".to_string(), Value::Str(why)),
                (
                    "removed".to_string(),
                    Value::Bool(matches!(removed, Ok(()) | Err(Errno::ENOENT))),
                ),
            ],
        );
        self.busy_reported = false;
    }

    /// A card that will not answer about the request file.
    ///
    /// Once per episode. Nothing is removed here — the removal would fail the
    /// same way — so this branch is reached again on every pass until the fault
    /// clears, and a latch is the only thing between it and a journal made
    /// entirely of this sentence.
    fn report_trouble(&mut self, k: &mut dyn Kernel, sup: &mut Supervisor, what: &str, e: Errno) {
        if self.trouble_reported {
            return;
        }
        self.trouble_reported = true;
        sup.note(
            k,
            Severity::Error,
            &format!(
                "DEGRADED there is a file where an install request goes and it {what}, so an \
                 install asked for on this machine may never start and nothing here can clear \
                 it. A failing or read-only /state is the usual cause, and it stops nothing else \
                 this supervisor does"
            ),
            vec![
                (
                    "path".to_string(),
                    Value::Str(deploy::REQUEST_PATH.to_string()),
                ),
                ("errno".to_string(), Value::Int(i64::from(e.0))),
                ("detail".to_string(), Value::Str(e.to_string())),
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::journal::Journal;
    use crate::kernel::Observation;
    use crate::service::Limits;
    use crate::supervise::{self, State};
    use crate::testkernel::{FakeKernel, Script};

    /// A supervisor with no services at all, which is what the request is being
    /// asserted about: everything below is about the set **growing**.
    fn supervisor() -> Supervisor {
        let journal = Journal::new("boot".to_string(), 7, supervise::ACTOR.to_string());
        Supervisor::new(Vec::new(), Limits::default(), journal)
            .unwrap_or_else(|e| unreachable!("{e}"))
    }

    fn kernel() -> FakeKernel {
        let mut k = FakeKernel::new();
        k.observations = Script::always(Observation::Empty);
        k
    }

    /// The caller every test below asks from, unless it is testing the
    /// principal.
    fn over_the_control_plane() -> deploy::Principal {
        deploy::Principal::from_environment(Some(A_PIN))
    }

    /// A pinned key, spelled out once.
    const A_PIN: &str = "5108de6435c6a6e501f8905629c908f13ff8a348193d55172f4572b966131107";

    /// **A caller is recorded by the key the control plane pinned it with.**
    ///
    /// This test exists because the signature it calls has changed under it
    /// before and nothing noticed: `rivetos-sysd` does not build on the Windows
    /// development host (`libc::clock_gettime` and friends), so `cargo test`
    /// there never compiles this file, and an argument once arrived with
    /// `PLAN.md` §81.3 without this call site being updated. The card build
    /// compiles the BINARY and not its tests, so a green build proves nothing
    /// about it either.
    #[test]
    fn a_caller_is_recorded_by_the_key_the_control_plane_pinned_it_with() {
        let principal = deploy::Principal::from_environment(Some(A_PIN));
        assert_eq!(principal.origin().pinned_key(), Some(A_PIN));
        assert_eq!(
            principal.origin().as_token(),
            format!("{}{A_PIN}", deploy::PINNED_PREFIX)
        );
    }

    /// Put whatever bytes on the fake card, request-shaped or not.
    ///
    /// **Both maps**, because `file_size` and `read_file` are separate questions
    /// on a real filesystem and the fake keeps them separate for the same
    /// reason: this module asks the first before it asks the second, and a
    /// helper that could not express "a file whose size and contents disagree"
    /// could not test the cap at all.
    fn write_raw(k: &mut FakeKernel, contents: &str) {
        k.files_mut().insert(
            deploy::REQUEST_PATH.to_string(),
            Script::always(Ok(contents.to_string())),
        );
        k.sizes_mut().insert(
            deploy::REQUEST_PATH.to_string(),
            u64::try_from(contents.len()).unwrap_or(u64::MAX),
        );
    }

    /// This machine's store, which is where `resolve` looks. A test here
    /// redirects nothing: the filesystem is `FakeKernel`'s map, so the real
    /// constant is a key in a `BTreeMap` rather than a directory on a disk.
    fn store() -> rivet_core::artifact::Store {
        rivet_core::artifact::Store::on_this_machine()
    }

    /// A digest, from a word, for a test that needs two that differ.
    fn digest_of(word: &str) -> rivet_core::artifact::Digest {
        let padded = format!("{word:0<64}").replace(|c: char| !c.is_ascii_hexdigit(), "0");
        rivet_core::artifact::Digest::new(&padded[..64].to_ascii_lowercase())
            .unwrap_or_else(|| unreachable!("{padded} should be a digest"))
    }

    /// The digest most tests below ask for.
    fn a_digest() -> rivet_core::artifact::Digest {
        digest_of("abc")
    }

    /// The artifact name the store holds it under.
    const ARTIFACT: &str = "photo-index-0.1.0-aarch64-application.rvt";

    /// **Put an artifact in this machine's store**, which is what makes a
    /// request installable at all since `ADR-052` §7.
    fn store_holds(k: &mut FakeKernel, digest: &rivet_core::artifact::Digest, name: &str) {
        k.dir_entries
            .insert(store().object_dir(digest), vec![name.to_string()]);
    }

    /// Put a well-formed request for `digest` on the fake card.
    ///
    /// **Rendered by `rivet_core::deploy`'s own writer**, never spelled out
    /// here: a second spelling of the format in this crate's tests is exactly
    /// the drift the one-writer-one-reader rule exists to prevent, and it would
    /// keep passing while the real CLI wrote something else.
    fn write_request(k: &mut FakeKernel, digest: &rivet_core::artifact::Digest) {
        write_raw(k, &deploy::render_request(digest, &over_the_control_plane()));
    }

    /// A machine holding one artifact, with a request for it waiting — the
    /// ordinary state this module is about.
    fn asked_for(k: &mut FakeKernel, digest: &rivet_core::artifact::Digest) {
        store_holds(k, digest, ARTIFACT);
        write_request(k, digest);
    }

    /// Where the installer will be told to read from, for that digest.
    fn object_path(digest: &rivet_core::artifact::Digest) -> String {
        store().object_path(digest, ARTIFACT)
    }

    fn errors(k: &FakeKernel) -> Vec<String> {
        k.records
            .iter()
            .filter(|r| r.sev == Severity::Error)
            .map(|r| r.msg.clone())
            .collect()
    }

    // -- the ordinary machine ------------------------------------------------

    /// **A machine nobody has asked anything of does nothing and says nothing.**
    /// This runs before every convergence pass, so a line here is a line on
    /// every wakeup of every board for ever.
    #[test]
    fn no_request_is_silent_and_creates_no_service() {
        let mut k = kernel();
        let mut sup = supervisor();
        let mut watch = Watch::new();
        for _ in 0..5 {
            watch.poll(&mut k, &mut sup);
        }
        assert!(sup.services().is_empty());
        assert!(k.records.is_empty(), "{:?}", k.records);
        assert!(k.removed.is_empty(), "nothing should have been removed");
    }

    // -- the whole of the ruling, from the outside ---------------------------

    /// **The channel, end to end within this process.** A request naming a
    /// payload becomes a `rivet-deploy` service instance carrying that exact
    /// path on its argument vector, and the next convergence starts it.
    #[test]
    fn a_request_becomes_a_running_installer_carrying_the_payload_it_named() {
        let mut k = kernel();
        let mut sup = supervisor();
        asked_for(&mut k, &a_digest());

        Watch::new().poll(&mut k, &mut sup);
        assert_eq!(
            sup.state_of(deploy::SERVICE_NAME),
            Some(&State::Idle),
            "the request did not create an instance"
        );

        sup.converge(&mut k);
        let Some((_, program)) = k
            .spawned
            .iter()
            .find(|(n, _)| n == deploy::SERVICE_NAME)
        else {
            unreachable!("the installer was never spawned: {:?}", k.spawned)
        };
        assert_eq!(program.path, deploy::PROGRAM_PATH);
        // **The installer is handed the path the STORE resolved to**, which is
        // the whole of `ADR-052` §7 end to end within this process: the request
        // named a digest and nothing but the store decided where that is.
        assert_eq!(program.args, vec![object_path(&a_digest())]);
        assert!(
            program.args[0].starts_with(&store().objects_dir()),
            "the installer was pointed outside the store: {:?}",
            program.args
        );
    }

    /// **A request for something this machine does not hold installs nothing**,
    /// and the refusal says so rather than starting an installer against a path
    /// that is not there. `ADR-052` §7's other half.
    #[test]
    fn a_request_for_a_digest_the_store_does_not_hold_starts_nothing() {
        let mut k = kernel();
        let mut sup = supervisor();
        // The request is written; the store is left empty.
        write_request(&mut k, &a_digest());

        Watch::new().poll(&mut k, &mut sup);
        assert!(sup.services().is_empty(), "an install started with no artifact");
        assert_eq!(
            k.removed,
            vec![deploy::REQUEST_PATH.to_string()],
            "the request was left to be refused again on every pass"
        );
        let Some(record) = k.records.last() else {
            unreachable!("nothing was recorded")
        };
        assert!(record.to_jsonl().contains("does not hold"), "{}", record.to_jsonl());
        assert!(record.to_jsonl().contains("ADR-052"), "{}", record.to_jsonl());
    }

    /// **An unreadable store is not an absent one.** A failing card and a
    /// machine that was never sent the artifact are opposite things to do next,
    /// so they are two sentences.
    #[test]
    fn a_store_that_cannot_be_read_says_unknown_rather_than_not_held() {
        let mut k = kernel();
        let mut sup = supervisor();
        write_request(&mut k, &a_digest());
        k.dir_errors_mut().insert(store().object_dir(&a_digest()), Errno::EIO);

        Watch::new().poll(&mut k, &mut sup);
        assert!(sup.services().is_empty());
        let Some(record) = k.records.last() else {
            unreachable!("nothing was recorded")
        };
        assert!(record.to_jsonl().contains("UNKNOWN"), "{}", record.to_jsonl());
    }

    /// **An object directory holding more than one file is refused**, because
    /// *"install whichever the filesystem listed first"* is not a rule anybody
    /// could rely on. An object arrives by renaming a complete directory, so
    /// this state means something else wrote into the store.
    #[test]
    fn a_store_entry_that_is_not_exactly_one_artifact_is_refused_rather_than_chosen() {
        for entries in [Vec::new(), vec!["a.rvt".to_string(), "b.rvt".to_string()]] {
            let mut k = kernel();
            let mut sup = supervisor();
            write_request(&mut k, &a_digest());
            k.dir_entries.insert(store().object_dir(&a_digest()), entries.clone());

            Watch::new().poll(&mut k, &mut sup);
            assert!(sup.services().is_empty(), "{entries:?} started an install");
            let Some(record) = k.records.last() else {
                unreachable!("nothing was recorded for {entries:?}")
            };
            assert!(
                record.to_jsonl().contains("exactly one artifact"),
                "{entries:?}: {}",
                record.to_jsonl()
            );
        }
    }

    // -- who asked, ADR-028 §4.5 ---------------------------------------------

    /// **One spelling, and the wire form is not it** — the regression this test
    /// exists for was found on a board on 2026-09-06.
    ///
    /// `Principal::render_fields()` renders the REQUEST FILE's line, so it
    /// prefixes `from=`. `crate::uninstall_request` and
    /// `crate::repo_delete_request` had each reached for it, and a board's
    /// journal carried an install as `pinned-<hex>` beside an uninstall as
    /// `from=pinned-<hex>`. **A reader matching `principal == "pinned-…"` — the
    /// thing anyone following a chain of custody actually does — silently
    /// missed every uninstall.**
    ///
    /// All three now call [`attribution`], so this asserts the property that
    /// makes that worth doing: the field carries the TOKEN, and never the
    /// field name the wire form puts in front of it.
    #[test]
    fn the_attribution_field_carries_the_token_and_not_the_wire_prefix() {
        let principal = rivet_core::deploy::Principal::from_environment(Some(A_PIN));
        let fields = attribution(&principal);

        assert_eq!(fields.len(), 1, "attribution is one field: {fields:?}");
        let (name, value) = &fields[0];
        assert_eq!(name, "principal");
        let Value::Str(recorded) = value else {
            unreachable!("the principal is a string: {value:?}")
        };

        assert_eq!(recorded, &format!("{}{A_PIN}", deploy::PINNED_PREFIX));
        assert!(
            !recorded.contains(deploy::FROM_FIELD),
            "the wire prefix reached a journal field already called `principal`: {recorded}"
        );
        // And the wire form really does differ, so this test is testing
        // something rather than asserting two spellings of one string.
        assert_ne!(&principal.render_fields(), recorded);
    }

    /// **The record says who asked**, in both readings: a field an agent matches
    /// on and a sentence a human reads.
    #[test]
    fn taking_up_a_request_records_the_caller_that_asked_for_it() {
        let mut k = kernel();
        let mut sup = supervisor();
        asked_for(&mut k, &a_digest());
        Watch::new().poll(&mut k, &mut sup);

        let Some(record) = k.records.last() else {
            unreachable!("nothing was recorded")
        };
        let line = record.to_jsonl();
        // The token is the key the control plane pinned the caller by, which is
        // what makes it matchable against the control plane's own record of the
        // same connection. That is the whole of the chain of custody.
        assert!(
            line.contains(&format!(r#""principal":"{}{A_PIN}""#, deploy::PINNED_PREFIX)),
            "{line}"
        );
        assert!(record.msg.contains("WHO ASKED"), "{}", record.msg);
    }

    /// **And it says, every time, that this is not authority.** A field called
    /// `principal` in a record about installing software is exactly the kind of
    /// thing a later reader takes to mean something was checked, and nothing
    /// here checks anything.
    #[test]
    fn every_record_naming_a_principal_says_it_is_not_authorisation() {
        let mut k = kernel();
        let mut sup = supervisor();
        asked_for(&mut k, &a_digest());
        Watch::new().poll(&mut k, &mut sup);

        let named: Vec<&String> = k
            .records
            .iter()
            .filter(|r| r.to_jsonl().contains(r#""principal":"#))
            .map(|r| &r.msg)
            .collect();
        assert!(!named.is_empty(), "nothing named a principal at all");
        for msg in named {
            assert!(
                msg.contains("ATTRIBUTION AND NOT AUTHORISATION"),
                "a record named a principal without saying what that means: {msg}"
            );
        }
    }

    /// A request from a process whose environment **named no caller** is taken
    /// up like any other and the record says exactly what was observed — never
    /// that somebody was at the console.
    #[test]
    fn a_request_with_no_named_caller_installs_and_claims_no_console() {
        let mut k = kernel();
        let mut sup = supervisor();
        store_holds(&mut k, &a_digest(), ARTIFACT);
        write_raw(
            &mut k,
            &deploy::render_request(&a_digest(), &deploy::Principal::no_session()),
        );
        Watch::new().poll(&mut k, &mut sup);

        assert_eq!(sup.state_of(deploy::SERVICE_NAME), Some(&State::Idle));
        let Some(record) = k.records.last() else {
            unreachable!("nothing was recorded")
        };
        assert!(
            record.to_jsonl().contains(r#""principal":"no-session""#),
            "{}",
            record.to_jsonl()
        );
    }

    /// **The pre-2026-08-24 format starts nothing**, and the refusal says what
    /// to do about it rather than only that the file was wrong.
    #[test]
    fn a_bare_path_is_refused_and_the_refusal_names_the_command_that_writes_one() {
        let mut k = kernel();
        let mut sup = supervisor();
        write_raw(&mut k, "/state/update.rvt\n");
        Watch::new().poll(&mut k, &mut sup);

        assert!(sup.services().is_empty(), "an unattributed install started");
        assert_eq!(k.removed, vec![deploy::REQUEST_PATH.to_string()]);
        let Some(record) = k.records.last() else {
            unreachable!("nothing was recorded")
        };
        assert!(
            record.to_jsonl().contains("rivet apply install"),
            "{}",
            record.to_jsonl()
        );
    }

    /// **The trap the ruling named.** Nothing in this path writes `enabled` on
    /// anything, and the instance it creates is not one the desired-state
    /// document names or could name — `ADR-017` §6 is untouched.
    #[test]
    fn a_request_creates_an_instance_rather_than_enabling_a_compiled_in_definition() {
        assert!(
            !definitions::system_services()
                .iter()
                .any(|d| d.name == deploy::SERVICE_NAME),
            "the installer is compiled into the service list, so a request COULD flip a flag on \
             it — which is the shape ADR-017 §6 forbids"
        );

        let mut k = kernel();
        let mut sup = supervisor();
        asked_for(&mut k, &a_digest());
        Watch::new().poll(&mut k, &mut sup);

        assert_eq!(sup.services().len(), 1, "{:?}", sup.services());
        assert!(
            k.writes
                .iter()
                .all(|w| w.path != rivet_core::state::CONFIG_DIR),
            "the document was written to"
        );
    }

    /// **The request is gone before the installer starts**, which is the
    /// ordering the whole safety argument rests on: the other direction
    /// reinstalls on every boot for ever.
    #[test]
    fn the_request_is_removed_before_anything_is_started() {
        let mut k = kernel();
        let mut sup = supervisor();
        asked_for(&mut k, &a_digest());
        Watch::new().poll(&mut k, &mut sup);

        assert_eq!(k.removed, vec![deploy::REQUEST_PATH.to_string()]);
        assert!(
            k.spawned.is_empty(),
            "the installer was spawned by the poll rather than by the convergence"
        );
        assert!(
            k.dir_syncs
                .contains(&rivet_core::state::STATE_MOUNT_POINT.to_string()),
            "the removal was not flushed: {:?}",
            k.dir_syncs
        );
    }

    /// And it happens **once**. A second pass over a card whose request was
    /// consumed finds nothing, so an installer that has been started is not
    /// started again on the next wakeup.
    #[test]
    fn a_consumed_request_is_not_found_again() {
        let mut k = kernel();
        let mut sup = supervisor();
        asked_for(&mut k, &a_digest());
        let mut watch = Watch::new();

        watch.poll(&mut k, &mut sup);
        sup.converge(&mut k);
        for _ in 0..4 {
            watch.poll(&mut k, &mut sup);
            sup.converge(&mut k);
        }
        assert_eq!(
            k.spawned
                .iter()
                .filter(|(n, _)| n == deploy::SERVICE_NAME)
                .count(),
            1,
            "the installer ran more than once for one request"
        );
    }

    /// A request that could not be removed **starts nothing**, and that is the
    /// same argument one notch further: a card that cannot unlink is a card
    /// that would hand this request back on every boot.
    #[test]
    fn a_request_that_cannot_be_removed_starts_no_install() {
        let mut k = kernel();
        let mut sup = supervisor();
        asked_for(&mut k, &a_digest());
        k.remove_file_results = Script::always(Err(Errno::EROFS));

        Watch::new().poll(&mut k, &mut sup);
        assert!(sup.services().is_empty(), "an install was set up anyway");
        assert!(
            !errors(&k).is_empty(),
            "a request that could not be consumed was silently dropped"
        );
    }

    // -- hostile and broken files -------------------------------------------

    /// **An enormous request is refused without being read.** The cap exists
    /// because this process is PID 1's only child and must not fail; allocating
    /// a card-sized `String` in it is how it would.
    #[test]
    fn an_enormous_request_is_refused_without_being_read() {
        let mut k = kernel();
        let mut sup = supervisor();
        // A size that says one thing and contents that say another, which is
        // what a file being written while this runs looks like — and the only
        // way to prove the size is what was believed.
        k.files_mut().insert(
            deploy::REQUEST_PATH.to_string(),
            Script::always(Ok("/state/x.rvt\n".to_string())),
        );
        k.sizes_mut()
            .insert(deploy::REQUEST_PATH.to_string(), 64 * 1024 * 1024);

        Watch::new().poll(&mut k, &mut sup);
        assert!(
            !k.reads.contains(&deploy::REQUEST_PATH.to_string()),
            "an oversized request was read into this process: {:?}",
            k.reads
        );
        assert!(sup.services().is_empty());
        assert_eq!(k.removed, vec![deploy::REQUEST_PATH.to_string()]);
        assert_eq!(errors(&k).len(), 1, "{:?}", errors(&k));
    }

    /// Every shape of malformed request is refused, **removed**, and reported —
    /// and none of them creates a service.
    #[test]
    fn a_file_that_is_not_a_request_is_refused_and_removed_rather_than_retried() {
        for contents in [
            "",
            "\n",
            // **The format from before 2026-09-12**, which named a path.
            // Refused by the parser rather than resolved — `ADR-052` §7.
            "from=no-session -- /state/update.rvt\n",
            "from=no-session -- relative/path.rvt\n",
            "from=no-session -- /state/a.rvt\nfrom=no-session -- /state/b.rvt\n",
            "from=no-session -- /state/\0.rvt\n",
            // **The format from before the principal**, which is one bare path.
            // Refused rather than installed unattributed — `ADR-028` §4.5.
            "/state/x.rvt\n",
            // And a principal this build will not repeat.
            "from=nobody -- /state/x.rvt\n",
            "from=1.2.3.4:5 who=root -- /state/x.rvt\n",
        ] {
            let mut k = kernel();
            let mut sup = supervisor();
            write_raw(&mut k, contents);
            let mut watch = Watch::new();

            watch.poll(&mut k, &mut sup);
            assert!(sup.services().is_empty(), "{contents:?} started an install");
            assert_eq!(
                k.removed,
                vec![deploy::REQUEST_PATH.to_string()],
                "{contents:?} was left on the card to be refused again"
            );
            assert_eq!(errors(&k).len(), 1, "{contents:?}: {:?}", errors(&k));

            // ...and the refusal says which problem it was, rather than only
            // that there was one.
            let Some(record) = k.records.last() else {
                unreachable!("nothing was recorded for {contents:?}")
            };
            assert!(
                record.to_jsonl().contains("\"problem\":"),
                "{contents:?}: {}",
                record.to_jsonl()
            );
        }
    }

    /// **A refusal is said once per file, not once per pass.** The file is gone
    /// after the first, so the machine cannot spend its journal on it — which is
    /// `ADR-017` §3.5's loop hazard in a cheaper costume.
    #[test]
    fn a_malformed_request_is_refused_once_rather_than_on_every_pass() {
        let mut k = kernel();
        let mut sup = supervisor();
        write_raw(&mut k, "not-a-path\n");
        let mut watch = Watch::new();
        for _ in 0..10 {
            watch.poll(&mut k, &mut sup);
        }
        assert_eq!(errors(&k).len(), 1, "{:?}", errors(&k));
    }

    /// A card that will not say whether there is a request is reported **once
    /// per episode**, nothing is removed, and the supervisor carries on. A latch
    /// is the only thing between this branch and a journal made entirely of one
    /// sentence — the file cannot be cleared, so the branch is reached again on
    /// every pass.
    #[test]
    fn a_card_that_will_not_answer_is_reported_once_and_stops_nothing() {
        let mut k = kernel();
        let mut sup = supervisor();
        k.size_errors_mut()
            .insert(deploy::REQUEST_PATH.to_string(), Errno::EIO);
        let mut watch = Watch::new();
        for _ in 0..6 {
            watch.poll(&mut k, &mut sup);
        }
        assert_eq!(errors(&k).len(), 1, "{:?}", errors(&k));
        assert!(k.removed.is_empty(), "a card that cannot stat can unlink?");
        assert!(sup.services().is_empty());

        // **And it is said again when the fault comes back.** A latch that never
        // re-arms turns the second episode into silence, which is the failure
        // mode of every "report once" flag.
        k.size_errors_mut().remove(deploy::REQUEST_PATH);
        watch.poll(&mut k, &mut sup);
        k.size_errors_mut()
            .insert(deploy::REQUEST_PATH.to_string(), Errno::EIO);
        watch.poll(&mut k, &mut sup);
        assert_eq!(errors(&k).len(), 2, "{:?}", errors(&k));
    }

    /// A request that could not be **removed** is the other half of a failing
    /// card, and it is reported once for the same reason.
    #[test]
    fn a_request_that_cannot_be_unlinked_is_reported_once() {
        let mut k = kernel();
        let mut sup = supervisor();
        asked_for(&mut k, &a_digest());
        k.remove_file_results = Script::always(Err(Errno::EIO));
        let mut watch = Watch::new();
        for _ in 0..6 {
            watch.poll(&mut k, &mut sup);
        }
        assert!(sup.services().is_empty());
        assert_eq!(errors(&k).len(), 1, "{:?}", errors(&k));
    }

    // -- the boundaries with the rest of the supervisor ----------------------

    /// **Not on the way down.** A `SIGTERM` has arrived and every service is
    /// being stopped; starting a 640 MiB write into that would be a machine that
    /// cannot be rebooted. The request is left for the next boot, which is what
    /// putting it on `/state` was for.
    #[test]
    fn a_request_is_not_taken_up_while_the_machine_is_shutting_down() {
        let mut k = kernel();
        let mut sup = supervisor();
        asked_for(&mut k, &a_digest());
        sup.begin_shutdown(&mut k, crate::kernel::Terminating::PowerOff);

        Watch::new().poll(&mut k, &mut sup);
        assert!(sup.services().is_empty());
        assert!(
            k.removed.is_empty(),
            "the request was consumed on the way down and would be lost"
        );
    }

    /// A second request arriving while an install runs is **discarded and said
    /// so**, rather than replacing the definition of a process that is halfway
    /// through writing a partition.
    #[test]
    fn a_second_request_during_an_install_is_refused_rather_than_replacing_it() {
        let mut k = kernel();
        let mut sup = supervisor();
        asked_for(&mut k, &digest_of("f1"));
        let mut watch = Watch::new();
        watch.poll(&mut k, &mut sup);
        sup.converge(&mut k);
        assert!(matches!(
            sup.state_of(deploy::SERVICE_NAME),
            Some(State::Started { .. })
        ));

        asked_for(&mut k, &digest_of("52"));
        watch.poll(&mut k, &mut sup);
        watch.poll(&mut k, &mut sup);

        let warned: Vec<_> = k
            .records
            .iter()
            .filter(|r| r.sev == Severity::Warn && r.msg.contains("SECOND REQUEST"))
            .collect();
        assert_eq!(warned.len(), 1, "said {} times", warned.len());
        // The running install still carries the payload it was started with.
        let Some((_, program)) = k
            .spawned
            .iter()
            .find(|(n, _)| n == deploy::SERVICE_NAME)
        else {
            unreachable!("nothing was spawned")
        };
        assert_eq!(program.args, vec![object_path(&digest_of("f1"))]);
        assert_eq!(
            k.spawned
                .iter()
                .filter(|(n, _)| n == deploy::SERVICE_NAME)
                .count(),
            1
        );
    }

    /// And once the first install has **finished**, a second request is a second
    /// install — a run-once service that has run is replaced, not restarted.
    #[test]
    fn a_request_after_an_install_finished_installs_the_new_payload() {
        let mut k = kernel();
        let mut sup = supervisor();
        asked_for(&mut k, &digest_of("f1"));
        let mut watch = Watch::new();
        watch.poll(&mut k, &mut sup);
        sup.converge(&mut k);

        // The installer exits, and `RestartPolicy::Never` puts it in `Done`.
        let Some(State::Started { pid, .. }) = sup.state_of(deploy::SERVICE_NAME).cloned() else {
            unreachable!("the installer is not running")
        };
        k.exits
            .push((pid, crate::kernel::ExitStatus::Exited(0)));
        sup.reap(&mut k);
        sup.converge(&mut k);
        assert_eq!(sup.state_of(deploy::SERVICE_NAME), Some(&State::Done));

        asked_for(&mut k, &digest_of("52"));
        watch.poll(&mut k, &mut sup);
        sup.converge(&mut k);

        let payloads: Vec<Vec<String>> = k
            .spawned
            .iter()
            .filter(|(n, _)| n == deploy::SERVICE_NAME)
            .map(|(_, p)| p.args.clone())
            .collect();
        assert_eq!(
            payloads,
            vec![
                vec![object_path(&digest_of("f1"))],
                vec![object_path(&digest_of("52"))],
            ]
        );
        assert_eq!(sup.services().len(), 1, "the set grew a duplicate entry");
    }
}
