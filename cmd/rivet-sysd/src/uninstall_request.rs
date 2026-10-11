//! **Taking up an uninstall request.** `rivet_core::uninstall`, ruled
//! 2026-08-25.
//!
//! `crate::deploy_request`'s twin, and deliberately its twin rather than a
//! generalisation of it: the two channels are read by the same process but mean
//! different things, are consumed at different moments, and their refusals name
//! different next actions. One poller parameterised by path would have to say
//! *"a request"* in every sentence, which is the vaguer wording `AGENTS.md` §5
//! exists to prevent.
//!
//! ```text
//!   rivet apply uninstall photo-index confirm
//!        │  writes                                    (cmd/rivet)
//!        ▼
//!   /state/uninstall-request     one line, 0600:
//!        │                       from=<client> [key=<label>] -- photo-index [purge]
//!        │  read on the next convergence pass         (this module)
//!        ▼
//!   Supervisor::begin_uninstall  →  disable, let ADR-017 §3.5 stop it,
//!                                    remove the tree when it is down
//! ```
//!
//! # The request is removed before it is acted on
//!
//! `crate::deploy_request`'s rule, and it matters more here. An uninstall that
//! somehow crashed this process **must not be retried for ever** — the second
//! attempt would find the application already gone and be a refusal, and the
//! third would too, and the machine would spend every convergence pass
//! reporting a removal it cannot perform. Removing the request first makes a
//! crash cost one uninstall rather than a permanent loop.
//!
//! # Not on the way down
//!
//! Same reason as an install: a `SIGTERM` has arrived and everything is
//! stopping, so `begin_uninstall`'s *stop it first* would race a shutdown that
//! is already stopping it. The request is on `/state` and the next boot finds
//! it, which is why it is there.

use rivet_core::uninstall;

use crate::journal::{Severity, Value};
use crate::kernel::{Errno, Kernel};
use crate::supervise::{Phase, Supervisor};

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

        let bytes = match k.file_size(uninstall::REQUEST_PATH) {
            Ok(bytes) => bytes,
            // The ordinary state of every machine that is not removing
            // anything. Silent, deliberately.
            Err(Errno::ENOENT) => {
                self.trouble_reported = false;
                return;
            }
            Err(e) => {
                self.report_trouble(k, sup, "could not be examined", &e.to_string());
                return;
            }
        };

        if bytes > uninstall::REQUEST_MAX_BYTES {
            self.refuse(
                k,
                sup,
                &format!(
                    "it is {bytes} bytes and a request may be at most {} — so it was NOT read \
                     into this process at all",
                    uninstall::REQUEST_MAX_BYTES
                ),
            );
            return;
        }

        let contents = match k.read_file(uninstall::REQUEST_PATH) {
            Ok(contents) => contents,
            Err(e) => {
                self.refuse(k, sup, &format!("it could not be read: {e}"));
                return;
            }
        };

        let request = match uninstall::parse_request(&contents) {
            Ok(request) => request,
            Err(problem) => {
                self.refuse(k, sup, &problem.why());
                return;
            }
        };

        // **Before anything is removed**, so a crash costs one uninstall rather
        // than a loop. See this module's documentation.
        if let Err(e) = k.remove_file(uninstall::REQUEST_PATH) {
            self.report_trouble(
                k,
                sup,
                "was read but could not be removed, so NOTHING WAS DONE — acting on a request \
                 that stays on the card is how one uninstall becomes every convergence pass",
                &e.to_string(),
            );
            return;
        }
        self.trouble_reported = false;

        sup.note(
            k,
            Severity::Info,
            "an uninstall was requested",
            vec![
                (
                    "application".to_string(),
                    Value::Str(request.name.to_string()),
                ),
                ("purge".to_string(), Value::Bool(request.purge)),
            ]
            .into_iter()
            // `ADR-028` §4.5's attribution, from the ONE place that spells it —
            // see `crate::deploy_request::attribution` for the defect that made
            // it shared. Read its own documentation before believing it:
            // anything running as uid 0 could have written the request itself.
            .chain(crate::deploy_request::attribution(&request.principal))
            .collect::<Vec<_>>(),
        );

        sup.begin_uninstall(k, request.name.as_str(), request.purge);
    }

    /// A request that is there and is not one. **Removed**, because a file that
    /// cannot be parsed will not parse next pass either.
    fn refuse(&mut self, k: &mut dyn Kernel, sup: &mut Supervisor, why: &str) {
        let removed = k.remove_file(uninstall::REQUEST_PATH).is_ok();
        sup.note(
            k,
            Severity::Error,
            "REFUSED the uninstall request, and NOTHING HAS BEEN REMOVED",
            vec![
                ("path".to_string(), Value::Str(uninstall::REQUEST_PATH.to_string())),
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
            &format!("the uninstall request at {} {what}", uninstall::REQUEST_PATH),
            vec![("detail".to_string(), Value::Str(detail.to_string()))],
        );
    }
}
