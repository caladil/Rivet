//! **The applications the card carried, installed on the first convergence** —
//! `ADR-050` §4.
//!
//! # What this closes
//!
//! `SPEC-05` §1 is the maintainer's: a device that has booted is expected to
//! have `fleetd` up and running just as `mcpd` is (.rulings/ R-303). `ADR-049`
//! put the per-machine credentials on the card and said so plainly — §5.1:
//! **none of the applications a fleet server runs is on the card.**
//! `cmd/rivet-image` and `cmd/rivet-initd` now get the payloads to
//! `/state/carried`; this is what turns them into installs.
//!
//! # It installs nothing itself, and that is the design
//!
//! It adopts `crate::definitions::deploy`, which is the same service instance
//! `crate::deploy_request` creates from an operator's `rivet apply install`. So
//! a carried payload and a deployed one are **the same artifact by the same
//! route**, which is `ADR-050` §4's whole argument for carrying payloads rather
//! than baking the programs into the image: `cargo rivet deploy` stays the way
//! to fix any of it afterwards.
//!
//! Nothing here verifies a signature. `ADR-029` §11 has one verifier and it is
//! `cmd/rivet-deploy`'s; the *second* check `ADR-050` §4 asks for happened on
//! the workstation, in `cmd/rivet-image`, against this card's own trust store.
//!
//! # One at a time, and the operator goes first
//!
//! One installer writes one inactive slot, so `crate::supervise::Supervisor`
//! refuses a second (`AdoptError::Busy`). This starts a payload only when no
//! install is running, and `crate::main` polls it **after**
//! `crate::deploy_request` — so on a pass where an operator has asked for
//! something, the operator's request is the one that runs.
//!
//! It is polled after `crate::installed` for a sharper reason. That watch fires
//! on the **edge** of a finished installer and re-runs discovery, which is what
//! delivers `ADR-049`'s staged credentials into the data directory the install
//! has just created. Adopting the next payload replaces the finished instance,
//! so doing it before `crate::installed` looked would skip that discovery pass
//! and leave an application installed, unknown to the supervisor, with its
//! credentials still in `/state/staging`.
//!
//! # `ADR-050` §10: a payload that fails does NOT stop this machine
//!
//! **Ruled 2026-09-07, in the direction the ADR leaned.** A carried payload
//! that will not install leaves the machine converging, running everything
//! else, and saying loudly what did not arrive.
//!
//! The alternative — refusing to converge — is the failure `ADR-050` §4 says
//! is survivable only *because* the control plane comes up regardless. A
//! machine that stopped converging over a bad payload would be a machine with
//! no `rivet-mcpd`, which is a machine **nobody can reach to fix it**, on
//! hardware with no shell. It also has no precedent here: `ADR-017` §3 has no
//! dependency graph, and every other failing service on a Rivet is reported and
//! retried rather than fatal.
//!
//! So a failed install:
//!
//! * **leaves the payload where it is**, under `/state/carried/<product>`, so
//!   an operator can `rivet apply install` it by hand and the next boot tries
//!   again;
//! * is recorded at `Error`, naming the product, the path and the fact that
//!   `rivet get install` holds the installer's own account of why;
//! * is **not retried on this boot** — see [`Watch::attempted`], because
//!   `ADR-017` §3's restart loop is an SD card destroyed 640 MiB at a time.
//!
//! # A payload is removed only when its install succeeded
//!
//! And the **record directory is never removed by anybody**, which is what
//! `cmd/rivet-initd` relies on to not copy the same payload off the card on
//! every boot. `rivet_core::carried` carries that argument; the halves are
//! split across two processes and the constant is in neither of them.

use rivet_core::carried;
use rivet_core::deploy;

use crate::definitions;
use crate::journal::{Severity, Value};
use crate::kernel::{Errno, Kernel};
use crate::supervise::{AdoptError, Phase, State, Supervisor};

/// The state one convergence loop keeps about the carried payloads.
#[derive(Debug, Default)]
pub struct Watch {
    /// Products this boot has already started an install for.
    ///
    /// **Once per boot, not once per machine.** A payload that fails is left on
    /// `/state` deliberately — an operator can install it by hand and the next
    /// boot tries again — so something has to stop this pass finding it again
    /// on the next wakeup and starting a 640 MiB write in a loop. That is
    /// `ADR-017` §3's hazard, and a set held in memory is the cheapest thing
    /// that ends it without writing a second kind of state to the card.
    ///
    /// It also holds products whose payload is already gone, so a machine that
    /// has installed everything it carried costs one `list_dir` per pass and
    /// nothing else.
    attempted: Vec<String>,
    /// The product whose install this watch is waiting on.
    ///
    /// **The instance is then found by the PAYLOAD PATH on its argument
    /// vector, not by a correlation.** That was the first shape of this and it
    /// was wrong: `crate::supervise::Managed::corr` is *"re-minted for each
    /// attempt"*, so the correlation recorded at `adopt` is not the one the
    /// instance carries once `converge` has started it — and every settle
    /// looked for an instance that no longer existed, leaving an installed
    /// payload on the card and a failed one unreported. The argument vector is
    /// what the supervisor was told to install and does not change under it,
    /// and it is also what tells this watch's install apart from an operator's.
    in_flight: Option<String>,
    /// Whether "the carried directory cannot be read" has been said.
    trouble_reported: bool,
}

impl Watch {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            attempted: Vec::new(),
            in_flight: None,
            trouble_reported: false,
        }
    }

    /// One pass: finish the last install, then start the next.
    pub fn poll(&mut self, k: &mut dyn Kernel, sup: &mut Supervisor) {
        // **Not on the way down**, on `crate::deploy_request`'s argument: a
        // `SIGTERM` has arrived and starting a long write into it is a machine
        // that cannot be rebooted. The payloads are on `/state` and the next
        // boot finds them, which is what putting them there was for.
        if sup.phase() != Phase::Converging {
            return;
        }

        self.settle(k, sup);

        // One installer writes one inactive slot. An install that is running —
        // this watch's or an operator's — owns the instance until it finishes.
        if sup
            .services()
            .iter()
            .any(|m| m.def.name == deploy::SERVICE_NAME && !finished(&m.state))
        {
            return;
        }

        let Some(product) = self.next(k, sup) else {
            return;
        };
        self.start(k, sup, &product);
    }

    /// Deal with the install this watch started, if it has finished.
    fn settle(&mut self, k: &mut dyn Kernel, sup: &mut Supervisor) {
        let Some(product) = self.in_flight.clone() else {
            return;
        };
        let path = carried::payload_path(&product);
        let Some(installer) = sup.services().iter().find(|m| {
            m.def.name == deploy::SERVICE_NAME
                && m.def.program.args.first().map(String::as_str) == Some(path.as_str())
        }) else {
            // The instance this watch started has been replaced — by an
            // operator's request, which `crate::deploy_request` adopts without
            // asking anybody. Its outcome is unknowable from here, so the
            // payload is LEFT and the product is not retried this boot: the
            // safe direction, since the alternative is deleting a payload whose
            // install may never have happened.
            self.in_flight = None;
            return;
        };
        let state = installer.state.clone();
        if !finished(&state) {
            return;
        }
        self.in_flight = None;

        // **The filesystem decides, not the exit code**, and that is
        // `AGENTS.md` §5 rather than a workaround.
        //
        // It looks like the exit status should answer this, and it cannot: a
        // `RestartPolicy::Never` instance is `State::Done` whichever way it
        // exited — `Supervisor::reap` sets `Done` for any exit a policy does
        // not want restarted — so `State::Failed` is unreachable for the
        // installer and a watch that read the state would delete the payload of
        // every failed install. That was this module's first shape and its
        // tests caught it.
        //
        // The question this actually wants answered is *is it installed*, and
        // the manifest under `current` is what `crate::applications::discover`
        // reads to decide the same thing. So the payload is removed only when
        // the thing it was for is on the machine.
        if !self.is_installed(k, &product) {
            // **`ADR-050` §10, and the machine keeps going.** Nothing is
            // removed, nothing is retried this boot, and everything else on
            // this machine converges as if this had not happened.
            sup.note(
                k,
                Severity::Error,
                "DEGRADED an application this CARD CARRIED did not install, and this machine has \
                 come up without it. Everything else is running and the control plane is up, \
                 which is why this is a diagnosis rather than a lost card (ADR-050 §4). The \
                 payload has been LEFT at `path` so `rivet apply install <path> confirm` can \
                 retry it, and the next boot tries again by itself. `rivet get install` is the \
                 installer's own account of what was wrong with it",
                vec![
                    ("application".to_string(), Value::Str(product)),
                    ("path".to_string(), Value::Str(path)),
                    ("carried".to_string(), Value::Bool(true)),
                ],
            );
            return;
        }

        // Installed. The payload has done its job and is several megabytes of a
        // card; the RECORD DIRECTORY around it stays, because that is what
        // stops PID 1 copying the same payload off the boot partition on the
        // next boot (`rivet_core::carried`).
        let removed = k.remove_file(&path);
        sup.note(
            k,
            Severity::Info,
            "an application this card carried has been installed, and the supervisor has \
             re-examined what is on the machine. Whether it RUNS is the document's answer, not \
             this one (ADR-017 §6) — and if the card also staged credentials for it, they were \
             delivered by the discovery pass this install triggered (ADR-049 §5)",
            vec![
                ("application".to_string(), Value::Str(product)),
                ("path".to_string(), Value::Str(path)),
                (
                    "payload_removed".to_string(),
                    Value::Bool(matches!(removed, Ok(()) | Err(Errno::ENOENT))),
                ),
            ],
        );
    }

    /// **Is this application on the machine?** The manifest under `current` is
    /// the same file `crate::applications::discover` reads to decide what is
    /// installed, so this is one authority rather than a second opinion.
    ///
    /// A `/state` that will not answer reads as **not installed**, which keeps
    /// the payload — the safe direction, since the other one deletes the only
    /// copy of a program on the strength of a card that is failing.
    fn is_installed(&self, k: &mut dyn Kernel, product: &str) -> bool {
        let Some(name) = rivet_core::service::ServiceName::new(product) else {
            // Unreachable: `rivet_core::carried::product_of` refuses anything
            // `ServiceName` refuses, and PID 1 named this directory through it.
            return false;
        };
        let at = rivet_core::service::manifest_path(rivet_core::service::BUILT_IN_VOLUME, &name);
        k.file_exists(&at).unwrap_or(false)
    }

    /// The next product with a payload waiting, or nothing.
    fn next(&mut self, k: &mut dyn Kernel, sup: &mut Supervisor) -> Option<String> {
        let entries = match k.list_dir(carried::CARRIED_DIR) {
            Ok(entries) => entries,
            // **The ordinary state of every machine that carried nothing.**
            // Silent, deliberately: a line per convergence pass saying that
            // this card carried no application is the noise that hides the pass
            // where one did.
            Err(Errno::ENOENT) => {
                self.trouble_reported = false;
                return None;
            }
            Err(e) => {
                self.report_trouble(k, sup, e);
                return None;
            }
        };
        self.trouble_reported = false;

        // Sorted, so the order is the same on every boot of every card. It is
        // not a dependency order and cannot be: `ADR-017` §3 has no graph,
        // and an application that starts before something it needs retries.
        let mut products: Vec<String> = entries;
        products.sort();
        for product in products {
            if self.attempted.iter().any(|a| a == &product) {
                continue;
            }
            match k.file_exists(&carried::payload_path(&product)) {
                // Carried and already dealt with on an earlier boot: the record
                // directory outlived its payload. Latched so the `file_exists`
                // is one call per boot rather than one per pass.
                Ok(false) => self.attempted.push(product),
                Ok(true) => return Some(product),
                Err(e) => {
                    self.report_trouble(k, sup, e);
                    return None;
                }
            }
        }
        None
    }

    /// Ask the installer for one payload.
    fn start(&mut self, k: &mut dyn Kernel, sup: &mut Supervisor, product: &str) {
        // **Marked attempted before it is started**, which is
        // `crate::deploy_request`'s remove-before-adopt discipline in the form
        // this channel can have: an adopt that fails must not be retried on the
        // next wakeup, because the reason it failed will still be true.
        self.attempted.push(product.to_string());
        let path = carried::payload_path(product);

        match sup.adopt(definitions::deploy(&path)) {
            Ok(()) => {
                self.in_flight = Some(product.to_string());
                sup.note(
                    k,
                    Severity::Info,
                    "an application THIS CARD CARRIED is being installed, and nobody asked for \
                     it — the card did (ADR-050 §4). NOTHING IS INSTALLED YET: the installer \
                     starts on this convergence pass and `rivet get install` is the account of \
                     what happens next. The payload was verified on the workstation against \
                     this image's own trust store before the card was written, and is verified \
                     again here by cmd/rivet-deploy",
                    vec![
                        ("application".to_string(), Value::Str(product.to_string())),
                        ("payload".to_string(), Value::Str(path)),
                        (
                            "svc_name".to_string(),
                            Value::Str(deploy::SERVICE_NAME.to_string()),
                        ),
                        ("carried".to_string(), Value::Bool(true)),
                    ],
                );
            }
            Err(e) => {
                // `Busy` is unreachable — the caller checked — and a definition
                // fault is a defect in this build rather than anything about
                // the payload. Reported and never fatal: `ADR-050` §10.
                let busy = matches!(e, AdoptError::Busy { .. });
                sup.note(
                    k,
                    Severity::Error,
                    "DEGRADED an application this card carried could not be handed to the \
                     installer, so this machine has come up without it and everything else is \
                     running. The payload has been LEFT where it is; `rivet apply install \
                     <payload> confirm` retries it and so does the next boot (ADR-050 §10)",
                    vec![
                        ("application".to_string(), Value::Str(product.to_string())),
                        ("payload".to_string(), Value::Str(path)),
                        ("detail".to_string(), Value::Str(e.to_string())),
                        ("busy".to_string(), Value::Bool(busy)),
                    ],
                );
            }
        }
    }

    /// A card that will not answer about `/state/carried`.
    ///
    /// Once per episode, on `crate::deploy_request::Watch::report_trouble`'s
    /// argument: nothing here can clear the fault, so the branch is reached on
    /// every pass and a latch is all that stands between it and a journal made
    /// of one sentence. Re-armed when the fault clears, because a fault that
    /// comes back is news again.
    fn report_trouble(&mut self, k: &mut dyn Kernel, sup: &mut Supervisor, e: Errno) {
        if self.trouble_reported {
            return;
        }
        self.trouble_reported = true;
        sup.note(
            k,
            Severity::Error,
            "DEGRADED this card carried applications and they cannot be read off /state, so any \
             application it was built to bring with it is NOT installed and nothing here can \
             clear that. A failing or read-only /state is the usual cause, and it stops nothing \
             else this supervisor does",
            vec![
                (
                    "path".to_string(),
                    Value::Str(carried::CARRIED_DIR.to_string()),
                ),
                ("errno".to_string(), Value::Int(i64::from(e.0))),
                ("detail".to_string(), Value::Str(e.to_string())),
            ],
        );
    }
}

/// Whether an installer instance has stopped running.
///
/// The same pair `crate::installed` watches, and it is deliberately **only**
/// about having stopped. A `RestartPolicy::Never` instance is `Done` whichever
/// way it exited — `Supervisor::reap` sets `Done` for any exit a policy does
/// not want restarted — so this says *the installer is no longer running* and
/// nothing at all about whether it installed anything. [`Watch::settle`] asks
/// the filesystem that.
const fn finished(state: &State) -> bool {
    matches!(state, State::Done | State::Failed)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::journal::Journal;
    use crate::kernel::{ExitStatus, Observation};
    use crate::service::Limits;
    use crate::supervise::{self, State};
    use crate::testkernel::{FakeKernel, Script};

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

    /// A machine whose card carried these products, each with its payload still
    /// waiting — which is what PID 1 leaves behind on a first boot.
    fn a_card_that_carried(k: &mut FakeKernel, products: &[&str]) {
        k.dirs_mut().insert(
            carried::CARRIED_DIR.to_string(),
            products.iter().map(|p| (*p).to_string()).collect(),
        );
        for product in products {
            k.exists_mut().insert(
                carried::payload_path(product),
                Script::always(Ok(true)),
            );
        }
    }

    /// Run the installer this watch started to completion, and hand back the
    /// exit it should be told about.
    ///
    /// **Through `converge` and `reap`, never by assigning a state.** The edge
    /// this module acts on is one the supervisor produces, so a fixture that
    /// wrote `State::Done` into the table would be testing an edge no machine
    /// makes — `crate::installed`'s own fixture carries the same note.
    fn finish_the_installer(k: &mut FakeKernel, sup: &mut Supervisor, installed: Option<&str>) {
        sup.converge(k);
        let Some(State::Started { pid, .. }) = sup.state_of(deploy::SERVICE_NAME).cloned() else {
            unreachable!(
                "the installer did not start: {:?}",
                sup.state_of(deploy::SERVICE_NAME)
            )
        };
        // **What a successful install actually leaves behind**, and it is what
        // this watch reads: the manifest under `current`, which is the same
        // file `crate::applications::discover` reads. The exit code is
        // deliberately not the signal — a `Never` service is `Done` whichever
        // way it exited, so a fixture that varied only the status would test a
        // distinction the supervisor does not make.
        if let Some(product) = installed {
            let name = rivet_core::service::ServiceName::new(product).expect("a fixture name");
            k.exists_mut().insert(
                rivet_core::service::manifest_path(rivet_core::service::BUILT_IN_VOLUME, &name),
                Script::always(Ok(true)),
            );
        }
        k.exits.push((pid, ExitStatus::Exited(0)));
        sup.reap(k);
        sup.converge(k);
    }

    fn spawned_payloads(k: &FakeKernel) -> Vec<String> {
        k.spawned
            .iter()
            .filter(|(n, _)| n == deploy::SERVICE_NAME)
            .filter_map(|(_, p)| p.args.first().cloned())
            .collect()
    }

    fn errors(k: &FakeKernel) -> Vec<String> {
        k.records
            .iter()
            .filter(|r| r.sev == Severity::Error)
            .map(|r| r.msg.clone())
            .collect()
    }

    // -- the ordinary machine ------------------------------------------------

    /// **A card that carried nothing does nothing and says nothing.** This runs
    /// before every convergence, so a line here is a line on every wakeup of
    /// every board that was not built with `--application`.
    #[test]
    fn a_card_that_carried_nothing_is_silent_and_starts_no_installer() {
        let mut k = kernel();
        let mut sup = supervisor();
        let mut watch = Watch::new();
        for _ in 0..5 {
            watch.poll(&mut k, &mut sup);
            sup.converge(&mut k);
        }
        assert!(sup.services().is_empty(), "{:?}", sup.services());
        assert!(k.records.is_empty(), "{:?}", k.records);
        assert!(k.removed.is_empty(), "{:?}", k.removed);
    }

    // -- the whole of ADR-050 §4, from the outside ---------------------------

    /// **The sentence `SPEC-05` §1 asks for, as far as this process can make
    /// it true.** A card carrying `rivet-fleetd` installs it with nobody asking
    /// — and it goes through `cmd/rivet-deploy` carrying the payload path, the
    /// same route `rivet apply install` takes.
    #[test]
    fn a_carried_payload_becomes_an_install_nobody_asked_for() {
        let mut k = kernel();
        let mut sup = supervisor();
        a_card_that_carried(&mut k, &["rivet-fleetd"]);

        let mut watch = Watch::new();
        watch.poll(&mut k, &mut sup);
        assert_eq!(sup.state_of(deploy::SERVICE_NAME), Some(&State::Idle));

        sup.converge(&mut k);
        assert_eq!(
            spawned_payloads(&k),
            vec![carried::payload_path("rivet-fleetd")],
            "the installer was not given the carried payload"
        );
    }

    /// **One at a time, in a stable order, and every one of them.** A fleet
    /// server's card carries two; one installer writes one slot, so they go one
    /// per finished install rather than two at once.
    #[test]
    fn two_carried_payloads_are_installed_one_after_the_other() {
        let mut k = kernel();
        let mut sup = supervisor();
        a_card_that_carried(&mut k, &["rivet-proxyd", "rivet-fleetd"]);
        let mut watch = Watch::new();

        watch.poll(&mut k, &mut sup);
        finish_the_installer(&mut k, &mut sup, Some("rivet-fleetd"));
        // A second payload must not be adopted while the first is running: the
        // poll above started one and this one runs before the exit is reaped in
        // a real loop, so assert the count rather than only the order.
        watch.poll(&mut k, &mut sup);
        finish_the_installer(&mut k, &mut sup, Some("rivet-proxyd"));

        assert_eq!(
            spawned_payloads(&k),
            vec![
                carried::payload_path("rivet-fleetd"),
                carried::payload_path("rivet-proxyd"),
            ],
            "sorted order, one at a time"
        );
    }

    /// **A payload that installed is removed and its record directory is
    /// not.** That pairing is the whole of the idempotence — `cmd/rivet-initd`
    /// skips a payload whose record directory exists, so removing the directory
    /// here would have the card copy it again on the next boot and install it
    /// again, for ever.
    #[test]
    fn an_installed_payload_is_removed_and_its_record_directory_is_kept() {
        let mut k = kernel();
        let mut sup = supervisor();
        a_card_that_carried(&mut k, &["rivet-fleetd"]);
        let mut watch = Watch::new();

        watch.poll(&mut k, &mut sup);
        finish_the_installer(&mut k, &mut sup, Some("rivet-fleetd"));
        watch.poll(&mut k, &mut sup);

        assert_eq!(
            k.removed,
            vec![carried::payload_path("rivet-fleetd")],
            "the payload was not removed after a successful install"
        );
        assert!(
            k.removed_paths().is_empty(),
            "the record directory was removed, so the next boot will carry this payload again: \
             {:?}",
            k.removed_paths()
        );
    }

    /// And a machine that has installed everything it carried starts nothing on
    /// the next boot: the record directory is there and the payload is not.
    #[test]
    fn a_card_whose_payloads_were_all_installed_starts_nothing_on_a_later_boot() {
        let mut k = kernel();
        let mut sup = supervisor();
        k.dirs_mut().insert(
            carried::CARRIED_DIR.to_string(),
            vec!["rivet-fleetd".to_string()],
        );
        k.exists_mut().insert(
            carried::payload_path("rivet-fleetd"),
            Script::always(Ok(false)),
        );

        let mut watch = Watch::new();
        for _ in 0..4 {
            watch.poll(&mut k, &mut sup);
            sup.converge(&mut k);
        }
        assert!(sup.services().is_empty(), "{:?}", sup.services());
        assert!(k.records.is_empty(), "{:?}", k.records);
    }

    // -- ADR-050 §10, which is the ruling this module carries ----------------

    /// **The open question, decided.** A payload that will not install leaves
    /// the machine converging: everything else runs, the payload is LEFT for a
    /// retry, and the failure is an `Error` record naming the application.
    ///
    /// The alternative — refusing to converge — would take `rivet-mcpd` down
    /// with it, and `ADR-050` §4's whole argument for accepting this risk is
    /// that the control plane comes up regardless.
    #[test]
    fn a_payload_that_will_not_install_leaves_the_machine_running_and_says_so() {
        let mut k = kernel();
        let mut sup = supervisor();
        a_card_that_carried(&mut k, &["rivet-fleetd"]);
        let mut watch = Watch::new();

        watch.poll(&mut k, &mut sup);
        finish_the_installer(&mut k, &mut sup, None);
        watch.poll(&mut k, &mut sup);

        assert!(
            k.removed.is_empty(),
            "a payload that failed to install was deleted: {:?}",
            k.removed
        );
        let failures = errors(&k);
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(
            failures[0].contains("DEGRADED") && failures[0].contains("come up without it"),
            "the refusal does not say the machine is still up: {}",
            failures[0]
        );
        // And the machine is still converging: nothing about this stopped the
        // supervisor.
        assert_eq!(sup.phase(), Phase::Converging);
    }

    /// **And it is not retried on this boot.** `ADR-017` §3's restart loop is
    /// an SD card destroyed 640 MiB at a time, and a payload that failed once
    /// will fail the same way in four seconds.
    #[test]
    fn a_failed_payload_is_not_retried_for_the_rest_of_this_boot() {
        let mut k = kernel();
        let mut sup = supervisor();
        a_card_that_carried(&mut k, &["rivet-fleetd"]);
        let mut watch = Watch::new();

        watch.poll(&mut k, &mut sup);
        finish_the_installer(&mut k, &mut sup, None);
        for _ in 0..6 {
            watch.poll(&mut k, &mut sup);
            sup.converge(&mut k);
        }
        assert_eq!(
            spawned_payloads(&k).len(),
            1,
            "the installer ran more than once for one payload: {:?}",
            spawned_payloads(&k)
        );
        assert_eq!(errors(&k).len(), 1, "{:?}", errors(&k));
    }

    /// **One bad payload does not stop the good one beside it.** A fleet
    /// server's card carries two, and a machine that came up with neither
    /// because the first was broken would be the dependency graph `ADR-017`
    /// §3.2 does not have.
    #[test]
    fn a_payload_that_fails_does_not_stop_the_next_one_being_installed() {
        let mut k = kernel();
        let mut sup = supervisor();
        a_card_that_carried(&mut k, &["rivet-fleetd", "rivet-proxyd"]);
        let mut watch = Watch::new();

        watch.poll(&mut k, &mut sup);
        finish_the_installer(&mut k, &mut sup, None);
        watch.poll(&mut k, &mut sup);
        finish_the_installer(&mut k, &mut sup, Some("rivet-proxyd"));
        watch.poll(&mut k, &mut sup);

        assert_eq!(
            spawned_payloads(&k),
            vec![
                carried::payload_path("rivet-fleetd"),
                carried::payload_path("rivet-proxyd"),
            ]
        );
        assert_eq!(
            k.removed,
            vec![carried::payload_path("rivet-proxyd")],
            "the wrong payload was removed"
        );
    }

    // -- the boundaries with the rest of the supervisor ----------------------

    /// **Not on the way down**, on `crate::deploy_request`'s argument: starting
    /// a long write into a shutdown is a machine that cannot be rebooted. The
    /// payloads are on `/state` and the next boot finds them.
    #[test]
    fn nothing_is_installed_while_the_machine_is_shutting_down() {
        let mut k = kernel();
        let mut sup = supervisor();
        a_card_that_carried(&mut k, &["rivet-fleetd"]);
        sup.begin_shutdown(&mut k, crate::kernel::Terminating::PowerOff);

        Watch::new().poll(&mut k, &mut sup);
        assert!(sup.services().is_empty());
        assert!(k.removed.is_empty(), "{:?}", k.removed);
    }

    /// **An operator's install goes first, and this waits for it.** One
    /// installer writes one inactive slot, so a carried payload adopted on top
    /// of a running install would be two writers on one partition.
    #[test]
    fn a_carried_payload_waits_while_an_operators_install_is_running() {
        let mut k = kernel();
        let mut sup = supervisor();
        a_card_that_carried(&mut k, &["rivet-fleetd"]);

        // The operator's, through the channel that creates one.
        sup.adopt(definitions::deploy("/state/theirs.rvt"))
            .expect("the installer is adoptable");
        sup.converge(&mut k);

        Watch::new().poll(&mut k, &mut sup);
        assert_eq!(
            spawned_payloads(&k),
            vec!["/state/theirs.rvt".to_string()],
            "a carried payload displaced a running install"
        );
    }

    /// A `/state` that will not answer is reported **once per episode**,
    /// nothing is installed, and the supervisor carries on — `ADR-050` §10 in
    /// its other shape.
    #[test]
    fn a_state_that_will_not_answer_is_reported_once_and_stops_nothing() {
        let mut k = kernel();
        let mut sup = supervisor();
        k.dir_errors_mut()
            .insert(carried::CARRIED_DIR.to_string(), Errno::EIO);
        let mut watch = Watch::new();
        for _ in 0..6 {
            watch.poll(&mut k, &mut sup);
            sup.converge(&mut k);
        }
        assert_eq!(errors(&k).len(), 1, "{:?}", errors(&k));
        assert!(sup.services().is_empty());
        assert_eq!(sup.phase(), Phase::Converging);
    }
}
