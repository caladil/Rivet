//! **Removing an installed application.** `ADR-015` §7's other direction,
//! ruled 2026-08-25.
//!
//! # Why this is the supervisor's and not the installer's
//!
//! `cmd/rivet-deploy` exists because *an update writes 640 MiB and a caller's
//! connection must not own that write* (the maintainer's 2026-08-24 ruling). An uninstall
//! writes nothing and takes milliseconds, so that argument does not reach it.
//!
//! **A different one does: an application may be running, and only the
//! supervisor can stop a service.** Nothing else on the machine may — `apply`'s
//! subjects stop the *machine*, never one service, and `Supervisor::adopt`
//! refuses to replace a running one. So a remover that was not the supervisor
//! would either delete the tree from under a live process or need a way to ask
//! the supervisor to stop it, which is this channel with extra steps.
//!
//! **And the supervisor already has the stop.** `decide` moves a `Started`
//! service whose `enabled` has gone false into `Stopping`, with the deadline and
//! the `SIGKILL` escalation `ADR-017` §3.5 already rules. An uninstall is
//! therefore *disable, wait for the existing machinery, then remove* — no new
//! way to stop anything, which is the whole reason this shape was chosen.
//!
//! # `data` survives by default, and that is a ruling
//!
//! Ruled 2026-08-25 by the maintainer (`.rulings/` R-362): data is not removed
//! by default, and the operator must be able to opt in to removing everything.
//!
//! So [`Request::purge`] is `false` unless the operator typed [`PURGE`], and a
//! plain uninstall leaves `/state/app/<name>/data` exactly where it is. The
//! reason is `ADR-015` §3.2's `state = "private"`: that directory is the
//! *operator's*, not the program's, and removing a program is not consent to
//! discard what it wrote. The answer names the path that was kept, so nobody has
//! to guess whether it is still there.
//!
//! **The cost is stated rather than hidden**: data outlives its application,
//! and nothing reaps it. `purge` is the word that does, and it has to be typed.

use crate::service::ServiceName;

/// Where the request is written. Read and removed by `cmd/rivet-sysd`.
///
/// **A second file rather than a field on `crate::deploy`'s.** That one is
/// consumed by `cmd/rivet-deploy`, a different process with a different
/// authority; one file adopted by two readers is how a request ends up
/// half-handled by each.
pub const REQUEST_PATH: &str = "/state/uninstall-request";

/// The temporary half of the write-sync-rename that produces it.
pub const REQUEST_TEMP_PATH: &str = "/state/uninstall-request.new";

/// `0600`, as `crate::deploy::REQUEST_MODE` is and for the same reason: it
/// names a principal, and nothing that is not root has any business reading who
/// asked for what.
pub const REQUEST_MODE: u32 = 0o600;

/// The word that also removes the application's data.
///
/// **Typed, never defaulted, and never abbreviated.** It sits where
/// `crate::deploy`'s confirmation does — a literal word an operator has to
/// produce — because `ADR-012` §7.1's argument for a confirmation is exactly
/// this one: the irreversible thing must be said out loud.
pub const PURGE: &str = "purge";

/// The largest request this build will read. A name and a principal; there is
/// no path in it.
pub const REQUEST_MAX_BYTES: u64 = 1024;

/// What an operator asked to be removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The application's name, already checked against [`ServiceName`]'s rules
    /// — so it is a single path component and cannot traverse.
    pub name: ServiceName,
    /// Whether `data` goes too. `false` unless [`PURGE`] was typed.
    pub purge: bool,
    /// Who the writing process said asked. `crate::deploy::Principal`'s first
    /// paragraph applies unchanged: attribution, not authority.
    pub principal: crate::deploy::Principal,
}

/// Why a request on disk is not one.
///
/// Closed, per `ADR-006` §6.4, and every variant is a thing a *reader* of the
/// file can conclude without knowing what the writer meant.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RequestProblem {
    Empty,
    TooLarge { bytes: usize },
    MoreThanOneLine,
    ControlCharacter { at: usize },
    /// The name is not one `ADR-017` §3.1.3 permits — which is also what stops
    /// a request naming `../../system`.
    UnusableName { name: String },
    /// A word after the name that is not [`PURGE`].
    UnknownWord { word: String },
    /// The principal half did not parse. Carries `crate::deploy`'s own account
    /// rather than a second wording of it.
    Principal { why: String },
}

impl RequestProblem {
    /// One sentence, for the journal.
    pub fn why(&self) -> String {
        match self {
            Self::Empty => "it is empty".to_string(),
            Self::TooLarge { bytes } => format!(
                "it is {bytes} bytes and a request may be at most {REQUEST_MAX_BYTES}"
            ),
            Self::MoreThanOneLine => "it has more than one line".to_string(),
            Self::ControlCharacter { at } => {
                format!("it carries a control character at byte {at}")
            }
            Self::UnusableName { name } => format!(
                "{name:?} is not a usable application name, so no directory under \
                 /state/app could correspond to it"
            ),
            Self::UnknownWord { word } => format!(
                "there is a {word:?} after the name and this build knows only `{PURGE}`"
            ),
            Self::Principal { why } => why.clone(),
        }
    }
}

/// Render a request. The inverse of [`parse_request`], and the only writer.
pub fn render_request(
    name: &ServiceName,
    purge: bool,
    principal: &crate::deploy::Principal,
) -> String {
    let tail = if purge {
        format!("{name} {PURGE}")
    } else {
        name.to_string()
    };
    // The space before the marker is load-bearing: `split_principal` reads the
    // line a whitespace token at a time, so `from=…--` is one token and a
    // refusal rather than a field and a marker.
    format!(
        "{} {} {tail}\n",
        principal.render_fields(),
        crate::deploy::PAYLOAD_MARKER
    )
}

/// Read one back.
///
/// **The name is validated here and not by the caller**, because this is the
/// boundary a hostile file crosses: `ServiceName` refuses anything that is not
/// a single well-formed component, which is what stops `../../..` reaching a
/// directory removal.
pub fn parse_request(contents: &str) -> Result<Request, RequestProblem> {
    let cap = usize::try_from(REQUEST_MAX_BYTES).unwrap_or(usize::MAX);
    if contents.len() > cap {
        return Err(RequestProblem::TooLarge {
            bytes: contents.len(),
        });
    }
    let body = contents
        .strip_suffix('\n')
        .map_or(contents, |s| s.strip_suffix('\r').unwrap_or(s));
    if body.is_empty() {
        return Err(RequestProblem::Empty);
    }
    if body.contains('\n') || body.contains('\r') {
        return Err(RequestProblem::MoreThanOneLine);
    }
    if let Some(at) = body.bytes().position(|b| b < 0x20 || b == 0x7f) {
        return Err(RequestProblem::ControlCharacter { at });
    }

    let (principal, rest) = crate::deploy::split_principal(body)
        .map_err(|problem| RequestProblem::Principal { why: problem.why() })?;

    let mut words = rest.split_whitespace();
    let Some(name) = words.next() else {
        return Err(RequestProblem::Empty);
    };
    let name = ServiceName::new(name).ok_or_else(|| RequestProblem::UnusableName {
        name: name.to_string(),
    })?;

    let mut purge = false;
    for word in words {
        if word == PURGE {
            purge = true;
        } else {
            return Err(RequestProblem::UnknownWord {
                word: word.to_string(),
            });
        }
    }

    Ok(Request {
        name,
        purge,
        principal,
    })
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::deploy::Principal;

    fn principal() -> Principal {
        Principal::no_session()
    }

    /// A valid principal and the marker — everything a request line needs
    /// before its name. Written once, because a test that hand-rolls it tests
    /// the principal parser by accident.
    fn prefix() -> String {
        format!(
            "{}{} {}",
            crate::deploy::FROM_FIELD,
            crate::deploy::NO_SESSION_TOKEN,
            crate::deploy::PAYLOAD_MARKER
        )
    }

    fn name(text: &str) -> ServiceName {
        ServiceName::new(text).expect("a usable name")
    }

    /// Round trip, both ways, because the writer and the reader are in one
    /// module precisely so they cannot drift.
    #[test]
    fn a_rendered_request_parses_back_to_what_produced_it() {
        for purge in [false, true] {
            let text = render_request(&name("photo-index"), purge, &principal());
            let back = parse_request(&text).expect("it parses");
            assert_eq!(back.name.as_str(), "photo-index");
            assert_eq!(back.purge, purge, "{text}");
        }
    }

    /// **`purge` is off unless it is typed.** The maintainer's ruling of
    /// 2026-08-25, as the one assertion that would catch a default flipping.
    #[test]
    fn data_is_kept_unless_the_word_is_present() {
        let text = render_request(&name("photo-index"), false, &principal());
        assert!(!text.contains(PURGE), "{text}");
        assert!(!parse_request(&text).expect("parses").purge);
    }

    /// **A traversal cannot reach the remover.** The name is a `ServiceName`,
    /// which is a single component, so this is refused at the boundary rather
    /// than by whatever joins it to a path later.
    #[test]
    fn a_name_that_could_escape_state_app_is_refused() {
        for hostile in ["..", "../..", "a/b", "/etc", ".hidden", ""] {
            let line = format!("{} {hostile}\n", prefix());
            let got = parse_request(&line);
            assert!(
                matches!(
                    got,
                    Err(RequestProblem::UnusableName { .. } | RequestProblem::Empty)
                ),
                "{hostile:?} was accepted: {got:?}"
            );
        }
    }

    /// A word that is not `purge` is a refusal, never something ignored — an
    /// operator who typed `purgee` meant to remove data and must not be told
    /// the uninstall succeeded as though they had not asked.
    #[test]
    fn an_unknown_word_after_the_name_is_refused_rather_than_ignored() {
        let line = format!("{} photo-index purgee\n", prefix());
        assert!(matches!(
            parse_request(&line),
            Err(RequestProblem::UnknownWord { .. })
        ));
    }

    /// The same three malformed-file refusals `crate::deploy` has, because the
    /// file is attacker-reachable state on `/state` in exactly the same way.
    #[test]
    fn a_malformed_file_is_refused_rather_than_guessed_at() {
        assert!(matches!(parse_request(""), Err(RequestProblem::Empty)));
        // **Each of these is well-formed up to the one thing being tested.**
        // A line that was ALSO missing its principal would be refused for that
        // instead, and the assertion would pass while proving nothing about the
        // check it names.
        let good = prefix();
        assert!(matches!(
            parse_request(&format!("{good} a\n{good} b\n")),
            Err(RequestProblem::MoreThanOneLine)
        ));
        assert!(matches!(
            parse_request(&format!("{good} a\u{7}b\n")),
            Err(RequestProblem::ControlCharacter { .. })
        ));
        let huge = format!("{good} {}\n", "a".repeat(REQUEST_MAX_BYTES as usize));
        assert!(matches!(
            parse_request(&huge),
            Err(RequestProblem::TooLarge { .. })
        ));
    }
}
