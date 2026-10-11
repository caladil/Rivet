//! **Taking up a library uninstall request.** `rivet_core::uninstall_library`.
//!
//! `crate::uninstall_request`'s twin, and deliberately its twin rather than a
//! generalisation of it, for that module's reason: the two channels mean
//! different things, are refused for different reasons, and their journal lines
//! name different next actions.
//!
//! ```text
//!   rivet apply uninstall-library aura-background-pack confirm
//!        │  writes                                       (cmd/rivet)
//!        ▼
//!   /state/uninstall-library-request    one line, 0600:
//!        │                              from=<client> [key=<label>] -- <selector>
//!        │  read on the next convergence pass            (this module)
//!        ▼
//!   rivet_core::uninstall_library::survey   →  refuse, or set each build aside
//!   crate::installed::rediscover            →  every library path recomposed
//! ```
//!
//! # The supervisor decides again, and its answer governs
//!
//! The command that wrote the request made the same decision through the same
//! function, against the machine as it stood then. An application can be
//! installed between the two, naming a build the request selects, so the
//! closure is read again here before anything is removed — and a request this
//! refuses removes nothing at all, not the builds that happened to be safe.
//!
//! # The request is removed before it is acted on
//!
//! `crate::uninstall_request`'s rule and its reason: a removal that somehow
//! crashed this process must not be retried on every convergence pass for
//! ever. A crash costs one request, never a loop.
//!
//! # Not on the way down
//!
//! A `SIGTERM` has arrived and everything is stopping, so a removal and a
//! rediscovery would race a shutdown already under way. The request is on
//! `/state` and the next boot finds it, which is why it is there.
//!
//! # Nothing running is restarted
//!
//! `crate::installed`'s header gives the reason: a running process keeps the
//! environment it was exec'd with, and stopping or restarting it is the
//! operator's decision, through `apply service-restart`. So every running
//! application that had a removed build on its library path, or joins one by
//! class, is named in the journal with the fact that it takes the change at its
//! next start.

use rivet_core::package::install::{library_dir, library_object_dir};
use rivet_core::package::library::Reference;
use rivet_core::service::BUILT_IN_VOLUME;
use rivet_core::uninstall_library::{self, Removal, Selector};

use crate::journal::{Severity, Value};
use crate::kernel::{Errno, Kernel};
use crate::service::Origin;
use crate::supervise::{Phase, SELF_NAME, State, Supervisor};

/// Reads the request path once per convergence pass.
#[derive(Default)]
pub struct Watch {
    /// Whether an episode of trouble about the request file has been reported.
    /// Once is information; once per pass is noise.
    trouble_reported: bool,
}

impl Watch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn poll(&mut self, k: &mut dyn Kernel, sup: &mut Supervisor) {
        if sup.phase() != Phase::Converging {
            return;
        }

        let bytes = match k.file_size(uninstall_library::REQUEST_PATH) {
            Ok(bytes) => bytes,
            // The ordinary state of every machine that is not removing a
            // library. Silent, deliberately.
            Err(Errno::ENOENT) => {
                self.trouble_reported = false;
                return;
            }
            Err(e) => {
                self.report_trouble(k, sup, "could not be examined", &e.to_string());
                return;
            }
        };

        if bytes > uninstall_library::REQUEST_MAX_BYTES {
            self.refuse(
                k,
                sup,
                &format!(
                    "it is {bytes} bytes and a request may be at most {} — so it was NOT read \
                     into this process at all",
                    uninstall_library::REQUEST_MAX_BYTES
                ),
            );
            return;
        }

        let contents = match k.read_file(uninstall_library::REQUEST_PATH) {
            Ok(contents) => contents,
            Err(e) => {
                self.refuse(k, sup, &format!("it could not be read: {e}"));
                return;
            }
        };

        let request = match uninstall_library::parse_request(&contents) {
            Ok(request) => request,
            Err(problem) => {
                self.refuse(k, sup, &problem.why());
                return;
            }
        };

        // **Before anything is removed**, so a crash costs one request rather
        // than a loop. See this module's documentation.
        if let Err(e) = k.remove_file(uninstall_library::REQUEST_PATH) {
            self.report_trouble(
                k,
                sup,
                "was read but could not be removed, so NOTHING WAS DONE — acting on a request \
                 that stays on the card is how one removal becomes every convergence pass",
                &e.to_string(),
            );
            return;
        }
        self.trouble_reported = false;

        sup.note(
            k,
            Severity::Info,
            "a library uninstall was requested",
            vec![(
                "selector".to_string(),
                Value::Str(request.selector.to_string()),
            )]
            .into_iter()
            // `ADR-028` §4's attribution, from the one place that spells it.
            // Attribution, not authority: anything running as uid 0 could have
            // written the request itself.
            .chain(crate::deploy_request::attribution(&request.principal))
            .collect::<Vec<_>>(),
        );

        remove(k, sup, &request.selector);
    }

    /// A request that is there and is not one. **Removed**, because a file that
    /// cannot be parsed will not parse next pass either.
    fn refuse(&mut self, k: &mut dyn Kernel, sup: &mut Supervisor, why: &str) {
        let removed = k.remove_file(uninstall_library::REQUEST_PATH).is_ok();
        sup.note(
            k,
            Severity::Error,
            "REFUSED the library uninstall request, and NOTHING HAS BEEN REMOVED",
            vec![
                (
                    "path".to_string(),
                    Value::Str(uninstall_library::REQUEST_PATH.to_string()),
                ),
                ("detail".to_string(), Value::Str(why.to_string())),
                // Said out loud, because a refusal that left the file behind
                // repeats every pass and an operator needs to know which.
                ("cleared".to_string(), Value::Bool(removed)),
            ],
        );
        self.trouble_reported = true;
    }

    /// Trouble with the file itself rather than with its contents. Reported
    /// once per episode.
    fn report_trouble(&mut self, k: &mut dyn Kernel, sup: &mut Supervisor, what: &str, detail: &str) {
        if self.trouble_reported {
            return;
        }
        self.trouble_reported = true;
        sup.note(
            k,
            Severity::Warn,
            &format!(
                "the library uninstall request at {} {what}",
                uninstall_library::REQUEST_PATH
            ),
            vec![("detail".to_string(), Value::Str(detail.to_string()))],
        );
    }
}

/// **Decide again, remove, and recompose** — the whole of a request once it
/// has been taken off the card.
fn remove(k: &mut dyn Kernel, sup: &mut Supervisor, selector: &Selector) {
    let volume = BUILT_IN_VOLUME;
    let removal = match uninstall_library::survey(
        &mut crate::retention::Card(&mut *k),
        volume,
        selector,
    ) {
        Ok(removal) => removal,
        Err(refusal) => {
            sup.note(
                k,
                Severity::Error,
                "REFUSED the library uninstall request, and NOTHING HAS BEEN REMOVED",
                vec![
                    ("selector".to_string(), Value::Str(selector.to_string())),
                    ("detail".to_string(), Value::Str(refusal.why())),
                ],
            );
            return;
        }
    };

    // Read BEFORE the refresh, so it names the environment each running
    // application was started with rather than the one it will next be given.
    let affected = running_and_affected(sup, volume, &removal);

    let mut removed: Vec<&Reference> = Vec::new();
    let mut kept: Vec<&Reference> = Vec::new();
    for reference in &removal.builds {
        let path = library_dir(volume, reference);
        // **A removal of a path built from a reference.** What makes it safe is
        // retention's argument: the name went through `Reference::parse` when
        // the library root was listed, which refuses anything that is not one
        // path component below `<volume>/lib`. Renamed aside and deleted by a
        // helper (`ADR-017` §3), so the build is gone from the library root
        // when this reports it — which is all the rediscovery below needs.
        match crate::reclaim::set_aside(k, SELF_NAME, &path) {
            Ok(done) => {
                let mut fields = vec![
                    ("reference".to_string(), Value::Str(reference.to_string())),
                    ("path".to_string(), Value::Str(path)),
                    ("selector".to_string(), Value::Str(selector.to_string())),
                ];
                if let Some(e) = done.unforked {
                    fields.push((
                        "deletion".to_string(),
                        Value::Str(format!(
                            "the helper to delete what was set aside could not start ({e}); the \
                             next start deletes it"
                        )),
                    ));
                }
                sup.note(
                    k,
                    Severity::Info,
                    "an installed library build was removed, as an operator asked, so it is no \
                     longer installed and no application is given it from its next start",
                    fields,
                );
                removed.push(reference);
            }
            Err(e) => {
                sup.note(
                    k,
                    Severity::Error,
                    "FAILED to remove an installed library build an operator asked to remove, so \
                     it is STILL INSTALLED and still given to every application that joins it",
                    vec![
                        ("reference".to_string(), Value::Str(reference.to_string())),
                        ("path".to_string(), Value::Str(path)),
                        ("detail".to_string(), Value::Str(e.to_string())),
                    ],
                );
                kept.push(reference);
            }
        }
    }

    // **The library set every service is given, recomposed without a
    // reboot** — the install edge's own rediscovery, which recomposes
    // `RIVET_ADDONS` and `LD_LIBRARY_PATH` from what is on the card now.
    // Only when something was removed: otherwise nothing changed.
    if !removed.is_empty() {
        crate::installed::rediscover(k, sup);
    }

    let list = |refs: &[&Reference]| {
        if refs.is_empty() {
            "none".to_string()
        } else {
            refs.iter()
                .map(|r| r.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    let (severity, msg) = match (removed.is_empty(), kept.is_empty()) {
        (_, true) => (
            Severity::Info,
            "the library uninstall request is done: every build it selected was removed and the \
             library path of every service was recomposed without a reboot",
        ),
        (true, false) => (
            Severity::Error,
            "the library uninstall request removed NOTHING: no build it selected could be \
             removed, so every one of them is still installed",
        ),
        (false, false) => (
            Severity::Error,
            "the library uninstall request was only PARTLY done: some builds it selected were \
             removed and the rest are STILL INSTALLED. The library path of every service was \
             recomposed from what is left",
        ),
    };
    sup.note(
        k,
        severity,
        msg,
        vec![
            ("selector".to_string(), Value::Str(selector.to_string())),
            ("removed".to_string(), Value::Str(list(&removed))),
            ("not_removed".to_string(), Value::Str(list(&kept))),
        ],
    );

    for (application, reference) in affected {
        if !removed.contains(&&reference) {
            continue;
        }
        sup.note(
            k,
            Severity::Info,
            "a RUNNING application had a removed library build on its library path, or joins it \
             by class. NOTHING WAS RESTARTED: a running process keeps the environment it was \
             exec'd with, so it takes the change at its next start — a crash restart, `rivet \
             apply service-restart`, or the next boot",
            vec![
                ("application".to_string(), Value::Str(application)),
                ("reference".to_string(), Value::Str(reference.to_string())),
            ],
        );
    }
}

/// Every running application, and each selected build it has on its library
/// path or joins by class.
fn running_and_affected(sup: &Supervisor, volume: &str, removal: &Removal) -> Vec<(String, Reference)> {
    let mut out = Vec::new();
    for managed in sup.services() {
        if managed.def.origin != Origin::Application
            || !matches!(managed.state, State::Started { .. } | State::Adopted { .. })
        {
            continue;
        }
        let path: Vec<&str> = managed
            .def
            .program
            .env
            .iter()
            .filter(|(key, _)| key == "LD_LIBRARY_PATH")
            .flat_map(|(_, value)| value.split(':'))
            .collect();
        for reference in &removal.builds {
            let object_dir = library_object_dir(volume, reference);
            let on_path = path.iter().any(|dir| *dir == object_dir);
            let joins = removal
                .acceptors
                .iter()
                .any(|a| a.application == managed.def.name && a.reference == *reference);
            if on_path || joins {
                out.push((managed.def.name.clone(), reference.clone()));
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "uninstall_library_request_tests.rs"]
mod tests;
