//! **What this application was handed** — `ADR-038` §8.
//!
//! An application on a Rivet had **no configuration surface at all** until
//! 2026-09-02: no `env` key in a manifest, no argv, and nothing an operator
//! could write into `RIVET_APP_DATA` over the control plane. `SPEC-04` §11 (2)
//! recorded it as the gap, and `aura-renderer` carried its background and its
//! brightness as compile-time constants with a comment saying *"change this
//! line and rebuild"*. This is the other end of the fix.
//!
//! # The shape, in one sentence each
//!
//! 1. The application **declares** which settings it understands, in its
//!    manifest, inside its signed `.rvt` (`ADR-038` §3).
//! 2. An operator **sets** them in the machine document, under
//!    `[applications.<name>]` (§4).
//! 3. The **supervisor** matches one against the other and writes what it can
//!    deliver into ~~this application's data directory~~ **a root-owned file in
//!    this application's tree** (§6, §8.1), reporting what it could not (§4.4).
//! 4. This module reads it, at the path it was handed.
//!
//! # Absent and unreadable are the same answer
//!
//! `ADR-036` §8.5.2's rule, and `ADR-038` §8 states it again for exactly this
//! call: **an application that finds no settings must behave as though every
//! value were unset.** Not as though it had failed — a machine nobody has
//! configured is the ordinary machine, and the file is absent on every one of
//! them.
//!
//! So nothing here returns an error. [`all`] gives an empty set for a file that
//! is missing, empty, unreadable or nonsense, and [`get`] gives `None` — which
//! is what a caller does with an unset value anyway.
//!
//! # The values are TEXT, and that is a first version rather than a design
//!
//! `ADR-038` §10.2 accepted text-only *unwillingly*. An application that wants
//! a number parses one and decides for itself what a bad one means, because it
//! is the only thing that knows whether `brightness = "banana"` should be
//! ignored, clamped or complained about. [`get_parsed`] is the convenience, and
//! it deliberately gives back `None` rather than a parse error for the same
//! reason: to this module a value it cannot use and a value that is not there
//! are one answer.
//!
//! # Re-declared, not imported — `ADR-036` §8.3
//!
//! An application is a foreign program and must not link the operating system,
//! so the parser below restates `rivet_core::settings::parse` rather than
//! importing it. `tests/agreement.rs` is what buys the right to restate: it
//! asserts that what the supervisor writes is exactly what this reads, for
//! every shape either side can produce.

use std::collections::BTreeMap;

/// The file's name — `rivet_core::settings::FILE`.
///
/// ~~Inside the directory `RIVET_APP_DATA` names.~~ **Not since 2026-09-04**:
/// it is beside `current`, root-owned, and this application is TOLD where by
/// [`crate::app::APP_SETTINGS_ENV`] rather than composing the path. `ADR-038`
/// §8.1 has the reason, and it is about the platform not writing into a
/// directory an unprivileged uid owns rather than about this file.
///
/// The name is still agreed with `rivet-core` — `tests/agreement.rs` — because
/// the supervisor writing a file this never opens is exactly the failure that
/// test exists for.
pub const FILE: &str = "settings";

/// **Everything this application was handed**, keyed by setting name.
///
/// Empty when nothing was — see the module note on why that is not an error.
#[must_use]
pub fn all() -> BTreeMap<String, String> {
    let Ok(path) = crate::app::settings_file() else {
        return BTreeMap::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    parse(&text)
}

/// One setting, or `None` if it was not set.
#[must_use]
pub fn get(key: &str) -> Option<String> {
    all().remove(key)
}

/// One setting, parsed — `None` when it is unset **or** unusable.
///
/// ```no_run
/// let brightness: u8 = rivet_sdk::settings::get_parsed("brightness").unwrap_or(60);
/// ```
///
/// **The fallback is the caller's**, and that is the point: this module cannot
/// know whether a bad value should be ignored, clamped or complained about, and
/// an application that wants to complain still can — [`get`] gives it the text.
#[must_use]
pub fn get_parsed<T: std::str::FromStr>(key: &str) -> Option<T> {
    get(key)?.parse().ok()
}

/// Read the file's text — `rivet_core::settings::parse`, restated.
///
/// `#` comments and blank lines are skipped; a line with no `=` is skipped, and
/// so is one whose key is not a setting name. A half-written file is better
/// read partially than not at all, and this reads something another program
/// wrote.
#[must_use]
pub fn parse(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !is_setting_name(key) {
            continue;
        }
        out.insert(key.to_string(), value.to_string());
    }
    out
}

/// Whether `text` is a name a manifest could have declared —
/// `rivet_core::package::application::is_setting_name`, restated.
#[must_use]
pub fn is_setting_name(text: &str) -> bool {
    !text.is_empty()
        && text.starts_with(|c: char| c.is_ascii_lowercase())
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    #[test]
    fn a_file_the_supervisor_wrote_reads_back_as_its_settings() {
        let text = "# Generated by Rivet from the [applications.<name>] table\n\
                    background=digital-rain\n\
                    brightness=60\n";
        let read = parse(text);
        assert_eq!(read.get("brightness").map(String::as_str), Some("60"));
        assert_eq!(read.get("background").map(String::as_str), Some("digital-rain"));
        assert_eq!(read.len(), 2, "the header was read as a setting");
    }

    /// **Nothing here fails.** A machine nobody configured is the ordinary
    /// machine, and every one of them has no file.
    #[test]
    fn nothing_readable_is_an_empty_set_rather_than_an_error() {
        for junk in ["", "\n\n", "# only a header\n", "garbage with no equals\n", "\u{0}"] {
            assert!(parse(junk).is_empty(), "{junk:?}");
        }
    }

    /// A value may contain `=` and spaces; the split is at the first `=` and a
    /// key cannot contain one.
    #[test]
    fn a_value_may_contain_an_equals_sign() {
        assert_eq!(
            parse("background=a=b c\n").get("background").map(String::as_str),
            Some("a=b c")
        );
    }

    /// A key that no manifest could have declared is skipped, because it did
    /// not come from a declaration and nothing will ask for it.
    #[test]
    fn a_line_whose_key_is_not_a_setting_name_is_skipped() {
        let text = "Brightness=1\nwith space=1\n2leading=1\nbrightness=60\n";
        let read = parse(text);
        assert_eq!(read.len(), 1);
        assert_eq!(read.get("brightness").map(String::as_str), Some("60"));
    }
}
