//! **Work in progress, kept between sittings.**
//!
//! Asked for after the first run (.rulings/ R-346): a `save` that keeps a copy of
//! the configuration being worked on, a question on exit when there is unsaved
//! work, and an offer on resume to load a saved configuration.
//!
//! # What is saved is ANSWERS, not a document
//!
//! A provisioning document has to be valid. **Work in progress does not** —
//! that is what makes it work in progress, and refusing to save a half-finished
//! configuration would lose exactly the sitting somebody wanted to keep.
//!
//! So this writes the answers, and says so in the file itself. A person who
//! finds one of these on their disk must not mistake it for something they can
//! put on a card, and the first line tells them.
//!
//! # It is a different file from the thing it becomes
//!
//! `crate::compose` turns answers into a document, and that document is the
//! artefact. This is a notebook.

use crate::field::Field;

/// The first bytes of a notebook, which is how [`saved`] tells one from a
/// composed document sitting in the same directory.
///
/// **The first line of [`HEADER`], and it must stay that way.** A separate
/// literal would be a second spelling of one fact (`ADR-011` §3), so the
/// assertion below holds them together.
const NOTEBOOK_MARK: &str = "# rivet-provision — WORK IN PROGRESS";

/// The header, so a file found later explains itself.
const HEADER: &str = "\
# rivet-provision — WORK IN PROGRESS, not a provisioning document.
#
# These are answers part-way through being given. They have not been validated
# as a whole and may not be a usable configuration. `rivet-provision` reads this
# back to carry on; nothing else should read it at all.
#
# The document this becomes is what goes on a card.
";

/// Where these live.
///
/// Beside the other things this maintainer's tools keep — `~/.rivet/keys`,
/// `~/.rivet/identity` — because a person looking for one will look there.
///
/// **`crate::rivet_root()`, not a second `HOME`/`USERPROFILE` read** —
/// `main.rs`'s own `RIVET_AT` section: this used to read the environment
/// directly, which is exactly how a scratch `cargo rivet provision --at DIR`
/// run ended up reading and offering to resume from the maintainer's real
/// saved notebooks.
#[must_use]
pub fn directory() -> Option<std::path::PathBuf> {
    Some(crate::rivet_root()?.join("provision"))
}

/// A hostname reduced to something that is safely a filename.
pub(crate) fn safe_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    if safe.is_empty() { "unnamed".to_owned() } else { safe }
}

/// The file a set of answers belongs in, named for what it is provisioning.
#[must_use]
pub fn path_for(name: &str) -> Option<std::path::PathBuf> {
    Some(directory()?.join(format!("{}.toml", safe_name(name))))
}

/// The file the **composed document** belongs in — the thing that goes on a
/// card.
///
/// # Why it is not `<host>.toml`
///
/// **Two different artefacts cannot share one name.** [`path_for`] is the
/// notebook and this is the document, and a maintainer's `~/.rivet/provision`
/// was found holding both kinds under the same scheme — `rivet-0c3a8.toml` a
/// set of answers, `rivet-01.toml` a document written by hand. Had `w` written
/// `<host>.toml`, pressing `s` and then `w` in one sitting would have destroyed
/// the first with the second and reported success both times.
///
/// `.provisioning.toml` cannot collide: [`safe_name`] keeps a hostname to
/// letters, digits and hyphens, so no answer file can ever be named this.
#[must_use]
pub fn document_path_for(name: &str) -> Option<std::path::PathBuf> {
    Some(directory()?.join(format!("{}{DOCUMENT_SUFFIX}", safe_name(name))))
}

/// What marks a file in this directory as a composed document.
pub const DOCUMENT_SUFFIX: &str = ".provisioning.toml";

/// The file the **staging bundle** belongs in — `ADR-049`, the material the
/// card carries for this machine's applications.
///
/// A third artefact and therefore a third suffix, on
/// [`document_path_for`]'s reasoning exactly: two different things cannot share
/// one name, and this one holds private keys, so a collision that destroyed it
/// would be found on a booted board rather than here.
#[must_use]
pub fn staging_path_for(name: &str) -> Option<std::path::PathBuf> {
    Some(directory()?.join(format!("{}{STAGING_SUFFIX}", safe_name(name))))
}

/// What marks a file in this directory as a staging bundle.
pub const STAGING_SUFFIX: &str = ".staged.txt";

/// **The staging bundle that belongs beside `document`**, wherever the
/// document lives — what `rivet-provision stage` writes and what `cargo rivet
/// provision` and `write-image` look for.
///
/// A document named on this module's own convention, `<stem>.provisioning.toml`,
/// has `<stem>.staged.txt` beside it — the pairing `w` writes. Any other name
/// has `<hostname>.staged.txt` beside it, the hostname reduced by
/// [`safe_name`]. `cmd/cargo-rivet`'s `staging_path_for_document` states the
/// same rule; the two must agree or the bundle this writes is one that tool
/// does not find.
#[must_use]
pub fn staging_beside(document: &std::path::Path, hostname: &str) -> std::path::PathBuf {
    let stem = document
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(DOCUMENT_SUFFIX))
        .filter(|stem| !stem.is_empty())
        .map_or_else(|| safe_name(hostname), str::to_owned);
    document.with_file_name(format!("{stem}{STAGING_SUFFIX}"))
}

/// Render the answers.
#[must_use]
pub fn render(fields: &[Field]) -> String {
    let mut out = String::from(HEADER);
    out.push_str("\n[answers]\n");
    for f in fields {
        // Escaped by hand rather than by a TOML writer: the values here are a
        // hostname and three addresses, and a dependency for that would be a
        // dependency for four strings.
        let value = f.value.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!("{} = \"{value}\"\n", f.label.to_lowercase()));
    }
    out
}

/// Read answers back into the fields they came from.
///
/// **Unknown keys are ignored rather than refused.** This is a notebook, and a
/// notebook written by an older build should still be readable by a newer one —
/// the opposite rule from `ADR-007` §3.6.5, which governs documents a machine
/// acts on. Nothing here reaches a machine.
pub fn apply(text: &str, fields: &mut [Field], check: impl Fn(&str) -> fn(&str) -> Result<(), String>) {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((key, raw)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = raw.trim().trim_matches('"').replace("\\\"", "\"");
        if let Some(f) = fields.iter_mut().find(|f| f.label.to_lowercase() == key) {
            let checker = check(f.label);
            f.set(&value, checker);
        }
    }
}

/// Every saved set of answers, newest first.
///
/// **A composed document is not offered here**, and that is a fix rather than a
/// nicety: this directory holds both kinds, and `resume` once listed
/// `rivet-01.toml` — an actual provisioning document — as a set of answers to
/// carry on from. [`apply`] would then have read four of its keys, ignored the
/// rest, and produced a half-populated form indistinguishable from a real one.
///
/// The test is the header [`render`] writes, not the filename, so a document
/// somebody hand-placed under any name is excluded too.
#[must_use]
pub fn saved() -> Vec<std::path::PathBuf> {
    let Some(dir) = directory() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .filter(|p| {
            std::fs::read_to_string(p).is_ok_and(|text| text.starts_with(NOTEBOOK_MARK))
        })
        .filter_map(|p| {
            let when = p.metadata().and_then(|m| m.modified()).ok()?;
            Some((when, p))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.into_iter().map(|(_, p)| p).collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;
    use crate::field;

    fn checker(_: &str) -> fn(&str) -> Result<(), String> {
        field::anything
    }

    /// **A saved file says what it is.** Somebody will find one of these in six
    /// months and needs to know it is not a card's worth of configuration.
    #[test]
    fn the_file_declares_that_it_is_not_a_document() {
        let fields = vec![Field::text("Hostname", "rivet-1", "")];
        let text = render(&fields);
        assert!(text.contains("WORK IN PROGRESS"), "{text}");
        assert!(text.contains("not a provisioning document"), "{text}");
    }

    /// Answers survive the round trip, which is the whole feature.
    #[test]
    fn answers_come_back() {
        let mut fields = vec![
            Field::text("Hostname", "rivet-1", ""),
            Field::text("Gateway", "", ""),
        ];
        fields[0].set("lounge", field::anything);
        fields[1].set("192.168.50.1", field::anything);
        let text = render(&fields);

        let mut fresh = vec![
            Field::text("Hostname", "rivet-1", ""),
            Field::text("Gateway", "", ""),
        ];
        apply(&text, &mut fresh, checker);
        assert_eq!(fresh[0].value, "lounge");
        assert_eq!(fresh[1].value, "192.168.50.1");
    }

    /// **A half-finished answer is saved too.** Refusing to keep an invalid
    /// value would lose the sitting somebody wanted to keep.
    #[test]
    fn an_invalid_answer_is_still_saved() {
        let mut fields = vec![Field::text("Gateway", "", "")];
        fields[0].set("560.10.22.384", field::ipv4);
        assert!(fields[0].error.is_some());
        let text = render(&fields);
        assert!(text.contains("560.10.22.384"), "{text}");
    }

    /// A key this build does not know is skipped, not fatal — a notebook from
    /// an older version still opens.
    #[test]
    fn an_unknown_key_is_ignored() {
        let mut fields = vec![Field::text("Hostname", "rivet-1", "")];
        apply("[answers]\nhostname = \"a\"\nvanished = \"b\"\n", &mut fields, checker);
        assert_eq!(fields[0].value, "a");
    }

    /// **The marker is the header's own first line.** Two literals saying one
    /// thing is how `saved()` silently stops recognising a notebook.
    #[test]
    fn the_notebook_marker_is_the_header_it_claims_to_be() {
        assert!(HEADER.starts_with(NOTEBOOK_MARK), "{HEADER}");
        assert!(render(&[]).starts_with(NOTEBOOK_MARK));
    }

    /// **A notebook and a document cannot land on one path.** Both are named
    /// after the same hostname and both live in `~/.rivet/provision`, so on one
    /// path pressing `s` then `w` would have the second overwrite the first,
    /// reporting success both times.
    #[test]
    fn the_answers_and_the_document_are_different_files() {
        let answers = path_for("rivet-01").expect("a home");
        let document = document_path_for("rivet-01").expect("a home");
        assert_ne!(answers, document);
        assert_eq!(answers.parent(), document.parent(), "both live beside each other");
        assert!(
            document.to_string_lossy().ends_with(DOCUMENT_SUFFIX),
            "{}",
            document.display()
        );
        // The sanitiser cannot produce a dot, so no hostname can ever compose
        // an answers file that collides with a document.
        assert!(!answers.to_string_lossy().ends_with(DOCUMENT_SUFFIX));
        assert_eq!(safe_name("rivet.01.provisioning"), "rivet-01-provisioning");
    }

    /// A name that is not a filename becomes one.
    #[test]
    fn a_hostile_name_becomes_a_filename() {
        let p = path_for("../../etc/passwd").expect("a home");
        let name = p.file_name().expect("a name").to_string_lossy().to_string();
        assert!(!name.contains(".."), "{name}");
        assert!(!name.contains('/'), "{name}");
    }
}
