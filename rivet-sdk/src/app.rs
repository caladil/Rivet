//! The handed environment — `ADR-036` §3, rows 2 to 4.
//!
//! ~~Three variables~~ **four since 2026-09-04** and one parser. Every one of
//! them is a place a program can be wrong in a way that compiles: a hard-coded
//! `/state/app/<name>/data` is correct until the volume changes (`ADR-031`), and
//! a hard-coded peer socket path is correct until the supervisor moves it —
//! which `rivet-core` notes Aura is currently getting wrong, hardcoding
//! `/run/aura/kiosk.sock`.
//!
//! **The fourth is the sentence above happening.** The settings file was inside
//! `RIVET_APP_DATA` and an application could compose the path from what it
//! already had; it moved out (`ADR-038` §8.1), and every program that had
//! composed it would now be reading a file nothing writes. It is handed instead.

use std::path::PathBuf;

use crate::NotHanded;

/// Where this application's own installed files are.
///
/// `PLAN.md` §104.29.2. An application is installed at
/// `<volume>/app/<name>/current` with its assets written flat beside the
/// program. **The working directory is `/`, which is not it** — the case that
/// found this was Aura, where every font failed to open and the renderer
/// segfaulted a second later on a board that was otherwise entirely healthy.
pub const APP_DIR_ENV: &str = "RIVET_APP_DIR";

/// Where this application may write, and the only such place.
///
/// `ADR-015` §3.2A: `/state/app/<name>/data`, mode `0700`, owned by the
/// application's own uid. `/state/app/<name>` is its *tree* and is root-owned;
/// writing there fails.
pub const APP_DATA_ENV: &str = "RIVET_APP_DATA";

/// The sockets of other applications this one was granted.
///
/// Granted via `ADR-015` §5.3's `connects`. Absent when nothing was granted rather than empty — an
/// empty variable is a value a program can misread.
pub const CONNECT_ENV: &str = "RIVET_CONNECT";

/// **The settings file this application was handed** — `ADR-038` §8.
///
/// A FILE, not a directory, and it is **not** inside [`APP_DATA_ENV`]: it moved
/// out on 2026-09-04 so that the platform stops writing by path into a directory
/// an unprivileged uid owns (`ADR-038` §8.1). Nothing an application is handed
/// points at the directory it now lives in, which is why it is told rather than
/// composed.
///
/// Absent means **no settings**, never an error — `crate::settings` is the whole
/// of what a caller needs and never returns one.
pub const APP_SETTINGS_ENV: &str = "RIVET_APP_SETTINGS";

/// This application's own installed directory.
///
/// # Errors
///
/// [`NotHanded`] when the supervisor did not set it, which means this process
/// was not started by one.
pub fn dir() -> Result<PathBuf, NotHanded> {
    read(APP_DIR_ENV)
}

/// The one directory this application may write to.
///
/// # If you keep state here, read this — `ADR-036` §8.5
///
/// **Treat your own unparseable state as ABSENT.** Corrupt, missing, empty,
/// written by a version of you that no longer exists: **one path, and it is the
/// cold-start path.** Never exit, never panic, never refuse to start.
///
/// The reason is not tidiness. A Rivet's ordinary shutdown is somebody pulling
/// the plug, and a write caught by that leaves a file which is zero bytes or
/// truncated — not one that has lost a few seconds. An application that exits
/// rather than starting fresh is restarted by the supervisor, fails the same
/// way, and on a locked kiosk becomes `PLAN.md` §108's restart-loop fault
/// splash: a machine in a hallway that does not come back until somebody
/// arrives with a control-plane key.
///
/// **The expectation to design for is not "I lost no data". It is "it turned
/// back on".**
///
/// ```no_run
/// # #[derive(Default)] struct State;
/// # impl State { fn parse(_: &str) -> Option<Self> { None } }
/// let state = rivet_sdk::app::data()
///     .ok()
///     .map(|dir| dir.join("state.json"))
///     .and_then(|path| std::fs::read_to_string(path).ok())
///     .and_then(|text| State::parse(&text))
///     .unwrap_or_default(); // unreadable and unparseable are the same answer
/// ```
///
/// # What this deliberately does NOT offer
///
/// **A crash-safe write.** `ADR-036` §8.5 ruled against putting one here: it
/// addresses a narrower failure than the rule above and does not remove the
/// need for it, since a torn write is only one of the ways a state file goes
/// bad. If you are holding something that exists **nowhere else** — an offline
/// queue of work already accepted from a person — say so, because that is the
/// case §8.5.5 names as the one that would reopen it, and what it wants is
/// probably an append-only file with a discardable tail rather than this.
///
/// # Errors
///
/// [`NotHanded`], as [`dir`].
pub fn data() -> Result<PathBuf, NotHanded> {
    read(APP_DATA_ENV)
}

/// The settings file this application was handed — [`APP_SETTINGS_ENV`].
///
/// Most callers want [`crate::settings::get`] rather than this; it is public
/// because an application that wants to know *whether it was handed one at all*
/// can otherwise not tell that apart from an empty file.
///
/// # Errors
///
/// [`NotHanded`], as [`dir`].
pub fn settings_file() -> Result<PathBuf, NotHanded> {
    read(APP_SETTINGS_ENV)
}

fn read(variable: &'static str) -> Result<PathBuf, NotHanded> {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .ok_or(NotHanded { variable })
}

/// One socket belonging to another application, granted to this one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// The application that provides it.
    pub owner: String,
    /// The endpoint's declared name — the `api` in `provides = ["socket:api"]`.
    pub name: String,
    /// Where to connect. **Told rather than assumed**, so the layout can change
    /// without every consumer changing with it.
    pub path: PathBuf,
}

/// Every endpoint this application was granted, in the order it was told them.
///
/// Empty when nothing was granted, which is not an error: `connects` is a
/// request (`ADR-015` §3.3) and an ungranted one is an ordinary state of the
/// machine.
///
/// # The format, and what a malformed entry does
///
/// `owner:name=path`, space-separated. An entry missing either separator is
/// **skipped rather than guessed at** — half a path is not a better answer than
/// no path, and a caller that finds its endpoint absent fails naming it.
#[must_use]
pub fn endpoints() -> Vec<Endpoint> {
    let Some(raw) = std::env::var_os(CONNECT_ENV) else {
        return Vec::new();
    };
    parse_endpoints(&raw.to_string_lossy())
}

/// The parser, separated so it is testable without an environment.
#[must_use]
pub fn parse_endpoints(raw: &str) -> Vec<Endpoint> {
    raw.split_whitespace()
        .filter_map(|entry| {
            let (identity, path) = entry.split_once('=')?;
            let (owner, name) = identity.split_once(':')?;
            if owner.is_empty() || name.is_empty() || path.is_empty() {
                return None;
            }
            Some(Endpoint {
                owner: owner.to_string(),
                name: name.to_string(),
                path: PathBuf::from(path),
            })
        })
        .collect()
}

/// The endpoint named `name` from `owner`, if it was granted.
#[must_use]
pub fn endpoint(owner: &str, name: &str) -> Option<Endpoint> {
    endpoints()
        .into_iter()
        .find(|e| e.owner == owner && e.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_granted_endpoint_is_parsed_into_its_three_parts() {
        let parsed = parse_endpoints("aura:kiosk=/run/rivet/connect/aura/kiosk");
        assert_eq!(
            parsed,
            vec![Endpoint {
                owner: "aura".to_string(),
                name: "kiosk".to_string(),
                path: PathBuf::from("/run/rivet/connect/aura/kiosk"),
            }]
        );
    }

    #[test]
    fn several_endpoints_keep_the_order_they_were_given_in() {
        let parsed = parse_endpoints("a:one=/one b:two=/two c:three=/three");
        let names: Vec<_> = parsed.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["one", "two", "three"]);
    }

    /// Half a path is not a better answer than no path — see the doc comment.
    #[test]
    fn a_malformed_entry_is_skipped_rather_than_guessed_at() {
        for bad in [
            "no-separators",
            "owner-only=/path",
            "owner:name",
            ":name=/path",
            "owner:=/path",
            "owner:name=",
        ] {
            assert!(
                parse_endpoints(bad).is_empty(),
                "{bad:?} should not have produced an endpoint"
            );
        }
    }

    #[test]
    fn one_bad_entry_does_not_discard_the_good_ones_beside_it() {
        let parsed = parse_endpoints("good:one=/one rubbish bad:=/x good:two=/two");
        let names: Vec<_> = parsed.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["one", "two"]);
    }

    #[test]
    fn nothing_granted_is_an_empty_list_and_not_an_error() {
        assert!(parse_endpoints("").is_empty());
    }

    /// The variable's absence is reported by name, because the message is what
    /// tells an author they are running somewhere that made no promise.
    #[test]
    fn a_missing_variable_names_itself_and_says_what_it_means() {
        let why = NotHanded {
            variable: APP_DATA_ENV,
        };
        let text = why.to_string();
        assert!(text.contains("RIVET_APP_DATA"));
        assert!(text.contains("ADR-036"));
    }
}
