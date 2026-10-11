//! `rivet apply uninstall-library <selector> [confirm]` — remove installed
//! library builds on purpose.
//!
//! `crate::uninstall_library` carries the channel, the selector and the
//! decision; this module is the operation. A selector is a **product name**,
//! which selects every build of that product under `/state/lib`, or a **full
//! reference**, which selects exactly one.
//!
//! # It delivers a request and returns, as `apply uninstall` does
//!
//! The supervisor is the authority for what may be removed: it reads every
//! installed application's declared closure again before it removes anything,
//! and only it can recompose the library set every service is given without a
//! reboot. So `requested: true` means *the request was written*, and
//! `rivet get log` is where the removal reports, on records whose `svc` is
//! `rivet-sysd`.
//!
//! # The plan is the machine's answer, not this command's guess
//!
//! The plan names every build the selector matches and every installed
//! application that joins one of them by class, as the machine found them when
//! asked — [`super::machine::Deployer::can_request_library_uninstall`]. An
//! application that DECLARES one of them refuses the plan outright, for the
//! reason `crate::uninstall_library` gives.

use super::machine::{Board, MachineError};
use super::plan::{Change, Effect, Plan, TakesEffect, resolve};
use super::{ErrorCode, Field, OperationError, Verb};
use crate::package::install::library_dir;
use crate::service::BUILT_IN_VOLUME;
use crate::uninstall_library::{Removal, Selector};
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "uninstall-library";

/// The literal `ADR-012` §7 requires as the last word — `apply`'s one.
const CONFIRMATION: &str = super::apply::CONFIRMATION;

/// What this subject's result carries.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was applied. Always `uninstall-library` in this build.",
    },
    Field {
        name: "selector",
        description: "The product name or full library reference named, carried back verbatim.",
    },
    Field {
        name: "requested",
        description: "Always true in a result. It means the request was DELIVERED, not that \
                      anything has been removed (ADR-006 §4).",
    },
    Field {
        name: "removed",
        description: "Always false. NOTHING IS GONE when this answers: the supervisor checks \
                      every installed application's declared closure again and removes on its \
                      next convergence pass.",
    },
    Field {
        name: "builds",
        description: "Every installed build the selector matched when the request was written, \
                      by full reference: what the supervisor removes if nothing has changed by \
                      then.",
    },
    Field {
        name: "accepted-by",
        description: "Every installed application that joins one of those builds by its class \
                      (its manifest's `accepts-class`), as `<application> accepts <class> from \
                      <reference>`. NOT a refusal: that application LOSES the addon at its next \
                      start, and a running one keeps the library it was started with until then.",
    },
    Field {
        name: "class-unknown",
        description: "Every one of those builds whose own manifest could not be read, so which \
                      applications join it by class is UNKNOWN. Empty on an ordinary machine.",
    },
    Field {
        name: "next",
        description: "Where the outcome appears, and what to watch.",
    },
];

/// Everything an operator must be told when the request is delivered.
fn answer(selector: &Selector, removal: &Removal) -> Vec<(&'static str, Value)> {
    vec![
        ("subject", Value::str(SUBJECT)),
        ("selector", Value::str(selector.as_str())),
        ("requested", Value::Bool(true)),
        ("removed", Value::Bool(false)),
        (
            "builds",
            Value::strings(removal.builds.iter().map(ToString::to_string)),
        ),
        (
            "accepted-by",
            Value::strings(removal.acceptors.iter().map(|a| {
                format!("{} accepts {} from {}", a.application, a.class, a.reference)
            })),
        ),
        (
            "class-unknown",
            Value::strings(
                removal
                    .unread
                    .iter()
                    .map(|u| format!("{}: {}", u.reference, u.detail)),
            ),
        ),
        (
            "next",
            Value::str(
                "the request was DELIVERED and nothing has been removed yet. The supervisor \
                 reads every installed application's declared closure again, removes each build \
                 by name, and recomposes every service's library path without a reboot. Watch it \
                 with `rivet get log` and select the records whose `svc` is `rivet-sysd`. A \
                 running application is NOT restarted: one that joins a removed build keeps it \
                 until its next start, and `rivet apply service-restart <name>` gives it the \
                 change now.",
            ),
        ),
    ]
}

/// Perform it. `arguments[0]` is the subject.
pub(super) fn perform(arguments: &[&str], board: &mut dyn Board) -> Result<Value, OperationError> {
    let Some(word) = arguments.get(1).copied() else {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{apply} {SUBJECT}` needs the library to remove, as the word after \
                 `{SUBJECT}`: a PRODUCT NAME, which removes every build of that product, or a \
                 FULL REFERENCE `<name>-<version>-<64 hex>`, which removes exactly one — `rivet \
                 {apply} {SUBJECT} <product|reference> {CONFIRMATION}`. `rivet get file \
                 /state/lib` lists what is installed.",
                apply = Verb::Apply,
            ),
        ));
    };

    // The confirmation alone is a forgotten selector — `apply uninstall`'s
    // precedent, and the failure it prevents is the same: a delivered request
    // for a command that named nothing.
    if word == CONFIRMATION {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            word,
            format!(
                "`{word}` is a keyword of this command and cannot be the library's name, so this \
                 names nothing to remove. The library goes first: `rivet {apply} {SUBJECT} \
                 <product|reference> {CONFIRMATION}`.",
                apply = Verb::Apply,
            ),
        ));
    }

    // Every word after the selector is accounted for, and the confirmation is
    // the only one there is — a misspelling is refused rather than read as
    // absence (ADR-012 §7).
    let mut confirmed = false;
    for extra in arguments.iter().skip(2).copied() {
        if extra != CONFIRMATION {
            return Err(OperationError::about(
                ErrorCode::Arguments,
                extra,
                format!(
                    "`{extra}` is not a word this command takes. After the library comes \
                     `{CONFIRMATION}` and nothing else, and a misspelling of it is REFUSED \
                     rather than read as absent (ADR-012 §7)."
                ),
            ));
        }
        confirmed = true;
    }

    // **Checked before the machine is touched**, so a plan and a confirmed run
    // refuse an impossible selector identically, and nothing that could
    // traverse reaches a path.
    let selector = Selector::parse(word)
        .map_err(|problem| OperationError::about(ErrorCode::Arguments, word, problem.why()))?;

    resolve(RequestLibraryUninstall { selector }, confirmed, board)
}

/// **The change `apply uninstall-library` is**, as `ADR-006` §6 defines one.
struct RequestLibraryUninstall {
    selector: Selector,
}

impl Change for RequestLibraryUninstall {
    /// Nothing: the machine's account is asked again when the request is
    /// written, because the card is the authority and is asked at the point of
    /// use (`AGENTS.md` §5).
    type Prepared = ();
    type Applied = Value;

    fn plan(&self, board: &mut dyn Board) -> Result<(Plan, ()), OperationError> {
        let removal = board
            .can_request_library_uninstall(&self.selector)
            .map_err(refused)?;

        let mut plan = Plan::new(
            format!("{SUBJECT} {}", self.selector),
            "removes installed libraries from this machine",
        );
        for build in &removal.builds {
            let mut effect = Effect::remove(library_dir(BUILT_IN_VOLUME, build))
                .was(format!("build `{build}` of the library `{}`", build.product()));
            let joined: Vec<String> = removal
                .acceptors
                .iter()
                .filter(|a| a.reference == *build)
                .map(|a| format!("`{}` (class `{}`)", a.application, a.class))
                .collect();
            if !joined.is_empty() {
                effect = effect.noting(format!(
                    "joined by class by {} — each LOSES this addon at its next start; this is \
                     reported, not refused",
                    joined.join(", ")
                ));
            } else if removal.unread.iter().any(|u| u.reference == *build) {
                effect = effect.noting(
                    "its own manifest could not be read, so which applications join it by class \
                     is UNKNOWN",
                );
            }
            plan = plan.doing(effect);
        }

        Ok((
            plan.doing(
                Effect::request(
                    "the supervisor",
                    "read every installed application's declared closure again, remove these \
                     builds, and recompose every service's library path without a reboot",
                )
                .noting(
                    "the supervisor REFUSES the whole request, removing nothing, if an installed \
                     application names one of these builds in its `library` closure by then — \
                     and this command does not wait for it",
                ),
            )
            .leaving(
                "every running application: NOTHING IS RESTARTED. One that joins a removed build \
                 keeps the library it was started with until its next start, and `rivet apply \
                 service-restart <name>` gives it the change now",
            )
            .leaving(format!(
                "this machine's artifact store at {}: a .rvt of one of these builds that it \
                 still holds can be installed again by its digest",
                crate::state::ARTIFACTS
            ))
            .leaving("NOTHING HAS BEEN REMOVED — the request is delivered only if this is confirmed")
            .taking_effect(TakesEffect::Immediately)
            .confirmed_by(&[SUBJECT, self.selector.as_str()]),
            (),
        ))
    }

    fn apply(self, (): (), board: &mut dyn Board) -> Result<Value, OperationError> {
        let removal = board
            .request_library_uninstall(&self.selector)
            .map_err(refused)?;
        Ok(Value::object(answer(&self.selector, &removal)))
    }
}

/// The one place a [`MachineError`] becomes this operation's refusal — the same
/// mapping every other subject uses, and nothing is reworded on the way through.
fn refused(error: MachineError) -> OperationError {
    let code = match error {
        MachineError::Unsafe(_) => ErrorCode::Unsafe,
        MachineError::Refused(_) => ErrorCode::MachineRefused,
    };
    OperationError::about(code, SUBJECT, error.why())
}

#[cfg(test)]
#[path = "uninstall_library_tests.rs"]
mod tests;
