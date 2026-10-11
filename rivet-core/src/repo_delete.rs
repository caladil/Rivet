//! **Destroying a repository** — the one operation in this system that loses
//! data on purpose.
//!
//! `crate::repo`'s projection creates and moves and *never* removes, which is
//! right and which leaves a hole: an operator who undeclares a repository has no
//! way to get the bytes back off the disk. This is that way, and it is a
//! separate, confirmed operation because `ADR-012` §7.1 says the irreversible
//! thing has to be said out loud.
//!
//! # It is the supervisor's, for `crate::uninstall`'s reason
//!
//! `cmd/rivet` writes a request to `/state` and returns; `cmd/rivet-sysd`
//! performs it. Not because a removal is slow — it is not — but because
//! **the supervisor owns that disk.** `crate::repos` converges the repository
//! root on every boot and on every reload; a second process removing a
//! directory underneath it would be two authorities over one tree, which is
//! `ADR-011` in the place it costs a repository.
//!
//! # Two interlocks, and each stops a different accident
//!
//! 1. **A declared repository is refused.** If `[repos]` still names it, the
//!    next convergence recreates it — so the delete would appear to fail, and
//!    an operator would reasonably conclude the command is broken and reach for
//!    `rm -rf`. Undeclare first (`rivet-fleet remove repo`), then delete. Ruled
//!    2026-08-26.
//! 2. **The identifier must match.** The request carries the id the operator
//!    expects to destroy, and the machine compares it to the one in the
//!    directory. A name is a label and labels get reused; an operator who
//!    undeclared `notes`, created a new `notes`, and then ran a delete he had
//!    typed ten minutes earlier would otherwise destroy the wrong one. This is
//!    the same reason `ADR-025` §4.1 exists at all, arriving at the one moment
//!    where getting it wrong is unrecoverable.
//!
//! # What is NOT here
//!
//! **No `purge` distinction**, unlike `crate::uninstall`. An application has a
//! program and separately has data, and removing one is not consent to remove
//! the other. A repository **is** data — there is no half of it that could
//! survive and no half worth keeping — so a flag offering to keep something
//! would be offering nothing.

use crate::identifier;
use crate::repo::RepoName;

/// Where a request waits. `/state`, so it survives a reboot between the asking
/// and the doing — `crate::uninstall::REQUEST_PATH`'s reason exactly.
pub const REQUEST_PATH: &str = "/state/repo-delete-request";

/// Written here and renamed into place, so a supervisor never reads half of one.
pub const REQUEST_TEMP_PATH: &str = "/state/repo-delete-request.new";

/// Readable by root and nobody else. It names a thing about to be destroyed.
pub const REQUEST_MODE: u32 = 0o600;

/// The largest request this build will read. A name, an identifier and a
/// principal; there is no path in it.
pub const REQUEST_MAX_BYTES: u64 = 1024;

/// What an operator asked to be destroyed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The repository's name, already checked — so it is a single path
    /// component and cannot traverse.
    pub name: RepoName,
    /// **The identifier the operator expects to destroy.**
    ///
    /// The machine compares this to the one in the directory and refuses if
    /// they differ. See the module header: a name is a label, labels get
    /// reused, and this is the one operation where being wrong is permanent.
    pub id: String,
    /// Who the writing process said asked. `crate::deploy::Principal`'s first
    /// paragraph applies unchanged: attribution, not authority.
    pub principal: crate::deploy::Principal,
}

/// Why a request on disk is not one.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RequestProblem {
    Empty,
    TooLarge { bytes: usize },
    MoreThanOneLine,
    ControlCharacter { at: usize },
    /// Not a name the schema permits — which is also what stops a request
    /// naming `../../system`.
    UnusableName { name: String },
    /// No identifier after the name. **Refused rather than defaulted**: a
    /// request that named a repository and not which one is a request this
    /// build will not guess at.
    NoIdentifier,
    /// The identifier is not one.
    UnusableIdentifier { id: String },
    /// A word after the identifier. There are none in this build.
    UnknownWord { word: String },
    /// The principal half did not parse.
    Principal { why: String },
}

impl RequestProblem {
    /// One sentence, for the journal.
    #[must_use]
    pub fn why(&self) -> String {
        match self {
            Self::Empty => "it is empty".to_string(),
            Self::TooLarge { bytes } => {
                format!("it is {bytes} bytes and a request may be at most {REQUEST_MAX_BYTES}")
            }
            Self::MoreThanOneLine => "it has more than one line".to_string(),
            Self::ControlCharacter { at } => {
                format!("it carries a control character at byte {at}")
            }
            Self::UnusableName { name } => format!(
                "{name:?} is not a usable repository name, so no directory under {} could \
                 correspond to it",
                crate::git::REPOSITORY_ROOT
            ),
            Self::NoIdentifier => format!(
                "it names a repository and not WHICH repository. A delete carries the \
                 identifier the operator expects to destroy, because a name is a label and \
                 labels get reused - {}",
                identifier::RULE
            ),
            Self::UnusableIdentifier { id } => {
                format!("{id:?} is not an identifier; {}", identifier::RULE)
            }
            Self::UnknownWord { word } => format!(
                "there is a {word:?} after the identifier and this build takes no words there"
            ),
            Self::Principal { why } => why.clone(),
        }
    }
}

/// Render a request. The inverse of [`parse_request`], and the only writer.
#[must_use]
pub fn render_request(
    name: &RepoName,
    id: &str,
    principal: &crate::deploy::Principal,
) -> String {
    // The space before the marker is load-bearing, as it is for an uninstall:
    // `split_principal` reads a whitespace token at a time.
    format!(
        "{} {} {name} {id}\n",
        principal.render_fields(),
        crate::deploy::PAYLOAD_MARKER
    )
}

/// Read one back.
///
/// **The name is validated here and not by the caller**, because this is the
/// boundary a hostile file crosses: `RepoName` refuses anything that is not a
/// single well-formed component, which is what stops `../../..` reaching a
/// directory removal.
///
/// # Errors
///
/// [`RequestProblem`], every variant of which is something a *reader* of the
/// file can conclude without knowing what the writer meant.
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
    let name = RepoName::new(name).ok_or_else(|| RequestProblem::UnusableName {
        name: name.to_string(),
    })?;

    let Some(id) = words.next() else {
        return Err(RequestProblem::NoIdentifier);
    };
    if !identifier::is_identifier(id) {
        return Err(RequestProblem::UnusableIdentifier { id: id.to_string() });
    }

    if let Some(word) = words.next() {
        return Err(RequestProblem::UnknownWord {
            word: word.to_string(),
        });
    }

    Ok(Request {
        name,
        id: id.to_string(),
        principal,
    })
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    const ID: &str = "4f2c9a01bd7e3865";

    fn principal() -> crate::deploy::Principal {
        crate::deploy::Principal::no_session()
    }

    fn name(text: &str) -> RepoName {
        RepoName::new(text).expect("a valid name")
    }

    /// **The round trip is the contract**, and it proves agreement rather than
    /// format (`AGENTS.md` §4): one writer, one reader, and the reader is the
    /// only thing that ever sees this file.
    #[test]
    fn a_request_written_is_the_request_read() {
        let rendered = render_request(&name("example-repo"), ID, &principal());
        let parsed = parse_request(&rendered).expect("it parses");
        assert_eq!(parsed.name.as_str(), "example-repo");
        assert_eq!(parsed.id, ID);
    }

    /// **A request without an identifier is refused, never defaulted.** A name
    /// is a label; labels get reused; and this is the one operation where being
    /// wrong is permanent.
    #[test]
    fn a_request_that_names_a_repository_and_not_which_one_is_refused() {
        let bare = format!(
            "{} {} example-repo\n",
            principal().render_fields(),
            crate::deploy::PAYLOAD_MARKER
        );
        assert_eq!(parse_request(&bare), Err(RequestProblem::NoIdentifier));
        assert!(RequestProblem::NoIdentifier.why().contains("labels get reused"));
    }

    /// **The boundary a hostile file crosses.** A name that could climb never
    /// becomes a `RepoName`, so it never reaches a path.
    #[test]
    fn a_name_that_could_traverse_never_becomes_a_request() {
        for bad in ["../../system", "..", "a/b", "/etc", "-rf"] {
            let line = format!(
                "{} {} {bad} {ID}\n",
                principal().render_fields(),
                crate::deploy::PAYLOAD_MARKER
            );
            let problem = parse_request(&line).expect_err(bad);
            assert!(
                matches!(problem, RequestProblem::UnusableName { .. }),
                "{bad}: {problem:?}"
            );
        }
    }

    #[test]
    fn an_identifier_that_is_not_one_is_refused() {
        for bad in ["4f2c", "NOTHEXATALL0000", "example-repo"] {
            let line = format!(
                "{} {} example-repo {bad}\n",
                principal().render_fields(),
                crate::deploy::PAYLOAD_MARKER
            );
            assert!(
                matches!(
                    parse_request(&line),
                    Err(RequestProblem::UnusableIdentifier { .. })
                ),
                "{bad}"
            );
        }
    }

    /// A word this build does not know is refused rather than ignored — the
    /// misspelling of a future flag must not read as its absence.
    #[test]
    fn a_word_this_build_does_not_know_is_refused_rather_than_ignored() {
        let line = format!(
            "{} {} example-repo {ID} purge\n",
            principal().render_fields(),
            crate::deploy::PAYLOAD_MARKER
        );
        assert!(matches!(
            parse_request(&line),
            Err(RequestProblem::UnknownWord { .. })
        ));
    }

    #[test]
    fn a_file_that_is_not_a_request_is_refused_by_name() {
        assert_eq!(parse_request(""), Err(RequestProblem::Empty));
        assert_eq!(parse_request("\n"), Err(RequestProblem::Empty));
        assert!(matches!(
            parse_request(&"x".repeat(2000)),
            Err(RequestProblem::TooLarge { .. })
        ));
    }

    /// **There is no `purge`, and that is a decision.** A repository *is* data;
    /// a flag offering to keep something would be offering nothing.
    #[test]
    fn there_is_no_half_of_a_repository_to_keep() {
        let source = include_str!("repo_delete.rs");
        let body = source.split("mod tests").next().expect("a body");
        assert!(!body.contains("pub const PURGE"));
    }
}
