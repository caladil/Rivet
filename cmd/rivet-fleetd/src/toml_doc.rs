//! **A small TOML reader and writer for one machine's desired-state
//! document** — ported from `web/config.js`'s `parseDocument`/
//! `serializeDocument`, R39.
//!
//! # Why this exists, when `rivet-config` already parses TOML
//!
//! `rivet_config::Document::parse` decides whether a document is VALID and
//! turns it into typed values; it throws the text away in the process. This
//! module does the opposite: it keeps every byte, and its only two jobs are
//! (1) round-trip that text unchanged when nothing is edited and (2) find
//! every `[applications.*]` key whose NAME looks like a credential, before a
//! commit is even attempted (`crate::commit`). Neither job needs a value
//! decoded — the JS half needs `displayValue`/`tomlLiteral` because a browser
//! form has to SHOW a value; this module never does, so it keeps only the
//! raw line each entry came from.
//!
//! # Why it was never proven before R39
//!
//! `web/config.js`'s reader and writer had run only against documents ITS OWN
//! writer produced — `AGENTS.md` §4's "a test that round-trips a reader
//! against its own writer proves the two agree with each other and nothing
//! about the format." `rivet-01`'s real document (the fixture both modules'
//! tests use) was never fed through it before Save started composing a
//! commit from what this reads. This module is that proof, in a place
//! `cargo test` can run it — R39's brief, "why Save was unsafe."
//!
//! # The bug the naive port has, and why it is fixed here rather than copied
//!
//! A literal transliteration of `serializeDocument` — `parts.join("\n") +
//! "\n"` — appends a trailing newline UNCONDITIONALLY, whether or not the
//! original document had one. `rivet-01`'s own document (the fixture below)
//! does not — its header says it is "canonically serialised" and it still
//! has no final newline, because every `apply edit --append` that ever ran
//! against it left its own formatting behind (`ADR-044` §4). A round trip
//! through the naive port would therefore add one byte on EVERY save, for
//! ever — not a parse failure, just a document that keeps changing shape
//! under an editor that touched nothing (`ADR-044` §4's whole point: the
//! repository is authoritative for VALUES, not for BYTES, and a git history
//! that gained a line on every no-op save would defeat that). [`Document`]
//! therefore records whether the SOURCE text ended in a newline and
//! [`serialize`] reproduces exactly that, not a fixed convention.

use std::collections::HashMap;

/// One line kept from the source text. `Kv` is a key this module recognises
/// as `NAME = value` at the start of a line (a plain `TOML` bare key —
/// `rivet-config`'s own grammar); everything else — comments, blank lines, a
/// key this reader's narrower grammar does not recognise — is `Other`, kept
/// so it survives a round trip untouched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Kv { key: String, raw: String },
    Other { raw: String },
}

/// One `[section]`'s lines. `header` is the literal text of its own header
/// line (`Some("[services.rivet-mdnsd]")`), or `None` for the document's
/// top-level section, which has no header line of its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Section {
    pub header: Option<String>,
    pub entries: Vec<Entry>,
}

/// A document, read into sections in the order they first appear.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// Section paths — `""` for the top level, `"services.rivet-mdnsd"` for
    /// a table, `"applications.tempmon-rivet"` for a named one — in the
    /// order the text first introduced them.
    pub order: Vec<String>,
    pub sections: HashMap<String, Section>,
    /// Whether the SOURCE text ended in `\n`. See the module documentation
    /// for why this is tracked rather than assumed.
    pub trailing_newline: bool,
}

/// Parse `text` into a [`Document`]. Never fails — a line this reader does
/// not recognise as a header or a key becomes [`Entry::Other`], which
/// [`serialize`] writes back byte for byte. This module is not a validator;
/// `rivet_config::Document::parse` is (`crate::code`'s own module
/// documentation: the schema guard runs on the fleet server before anything
/// is sent).
#[must_use]
pub fn parse(text: &str) -> Document {
    let (lines, trailing_newline) = split_lines(text);

    let mut sections: HashMap<String, Section> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    ensure(&mut sections, &mut order, "");
    let mut current = String::new();

    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        let trimmed = line.trim();
        if let Some(path) = header_path(trimmed) {
            ensure(&mut sections, &mut order, &path);
            current = path;
            i += 1;
            continue;
        }
        if let Some((key, value)) = kv_key_value(line) {
            let mut raw = line.clone();
            // A triple-quoted string may span lines. Consume forward to the
            // line that closes it.
            if value.starts_with("\"\"\"") && !closes_triple_quote(&value) {
                let mut collected = vec![line.clone()];
                i += 1;
                while i < lines.len() && !lines[i].contains("\"\"\"") {
                    collected.push(lines[i].clone());
                    i += 1;
                }
                if i < lines.len() {
                    collected.push(lines[i].clone());
                }
                raw = collected.join("\n");
            }
            section_mut(&mut sections, &current).entries.push(Entry::Kv { key, raw });
            i += 1;
            continue;
        }
        section_mut(&mut sections, &current).entries.push(Entry::Other { raw: line.clone() });
        i += 1;
    }

    Document { order, sections, trailing_newline }
}

/// The text [`parse`] would read back from, byte for byte, when nothing in
/// `doc` was changed since it was parsed — `AGENTS.md` §4's round-trip proof.
#[must_use]
pub fn serialize(doc: &Document) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for path in &doc.order {
        let Some(section) = doc.sections.get(path) else { continue };
        if let Some(header) = &section.header {
            parts.push(header);
        }
        for entry in &section.entries {
            let raw = match entry {
                Entry::Kv { raw, .. } | Entry::Other { raw } => raw,
            };
            parts.push(raw);
        }
    }
    let mut out = parts.join("\n");
    if doc.trailing_newline {
        out.push('\n');
    }
    out
}

/// Whether `key`'s name suggests a credential — `R39`'s guard, `ADR-038` §4,
/// `.rulings/` R-210 D16.
///
/// Matched on the key's name, never its value: `ADR-038` §4 makes
/// `[applications.*]` text `rivet-config` never interprets, so the name is
/// what this guard has to go on before a value has even reached a machine.
///
/// **Delegates to `rivet_core::package::application::is_credential_like_name`
/// — the one rule every check of a setting's name uses, everywhere in this
/// project.** This document's `[applications.*]` region is free text with no
/// setting-name alphabet of its own to check a key against, so this crate
/// keeps no needle list of its own and calls the shared rule instead.
///
/// **The one predicate every credential scan over a document's
/// `[applications.*]` region calls**, so two readers of that region —
/// [`credential_like_settings`]'s own line-based one and
/// `crate::commit`'s normalized, `rivetos_config`-based one — can disagree
/// about which KEYS they can see without ever disagreeing about which NAMES
/// count once seen.
#[must_use]
pub fn is_credential_like_key(key: &str) -> bool {
    rivet_core::package::application::is_credential_like_name(key)
}

/// Every `[applications.*]` key whose NAME suggests a credential, as
/// `(section path, key)` pairs — `R39`'s guard, `ADR-038` §4, and
/// `crate::commit`'s module documentation says why this is checked before a
/// commit rather than left to the schema: this region is exactly the one the
/// schema does not validate.
///
/// **This reader's own blind spot:** it recognises only an unindented bare
/// key (this module's own header), so a quoted key, an indented key, or a
/// quoted table segment never becomes an [`Entry::Kv`] at all and this scan
/// cannot see it — `crate::commit`'s normalized scan is what covers those
/// spellings, against the same [`is_credential_like_key`] this one calls.
#[must_use]
pub fn credential_like_settings(doc: &Document) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for path in &doc.order {
        if path != "applications" && !path.starts_with("applications.") {
            continue;
        }
        let Some(section) = doc.sections.get(path) else { continue };
        for entry in &section.entries {
            let Entry::Kv { key, .. } = entry else { continue };
            if is_credential_like_key(key) {
                out.push((path.clone(), key.clone()));
            }
        }
    }
    out
}

fn ensure(sections: &mut HashMap<String, Section>, order: &mut Vec<String>, path: &str) {
    if !sections.contains_key(path) {
        let header = if path.is_empty() { None } else { Some(format!("[{path}]")) };
        sections.insert(path.to_owned(), Section { header, entries: Vec::new() });
        order.push(path.to_owned());
    }
}

/// The current section. [`ensure`] has always already created it by the
/// time this is called — every path this module sets `current` to came
/// through [`ensure`] first — but this stays infallible rather than
/// panicking on a defect this module cannot actually produce: `or_default`
/// makes "never happens" true by construction instead of by argument.
fn section_mut<'a>(sections: &'a mut HashMap<String, Section>, path: &str) -> &'a mut Section {
    sections.entry(path.to_owned()).or_default()
}

/// Split `text` into lines the way `web/config.js`'s `text.split(/\r\n|\n/)`
/// does, and say whether `text` itself ended in a line terminator.
///
/// `\r\n` and `\n` are both accepted; a lone `\r` is left as ordinary text,
/// on the JS regex's own footing.
fn split_lines(text: &str) -> (Vec<String>, bool) {
    let mut lines = Vec::new();
    let mut rest = text;
    loop {
        if let Some(at) = rest.find('\n') {
            let (line, remainder) = rest.split_at(at);
            let line = line.strip_suffix('\r').unwrap_or(line);
            lines.push(line.to_owned());
            rest = &remainder[1..];
        } else {
            lines.push(rest.to_owned());
            break;
        }
    }
    let trailing = text.ends_with('\n') && lines.last().is_some_and(String::is_empty);
    if trailing {
        lines.pop();
    }
    (lines, trailing)
}

/// `^\[([^\[\]]+)\]\s*$`, applied to an already-trimmed line — a table
/// header this reader will track as a section, or `None` for anything else
/// (including an array-of-tables `[[x]]`, which this document's grammar does
/// not use and which is therefore kept as `Other` rather than misread).
fn header_path(trimmed: &str) -> Option<String> {
    if trimmed.len() < 2 || !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return None;
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.is_empty() || inner.contains('[') || inner.contains(']') {
        return None;
    }
    Some(inner.trim().to_owned())
}

/// `^([A-Za-z0-9_-]+)\s*=\s*(.*)$`, applied to the RAW (un-trimmed) line —
/// `web/config.js`'s `kv` regex matches the same way, anchored at column 0,
/// which is every key this document's writers actually produce. Returns the
/// key and the value text with its leading whitespace stripped (group 2 of
/// the JS regex, which already drops it via `\s*` before capturing).
fn kv_key_value(line: &str) -> Option<(String, String)> {
    let end = line
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(line.len());
    if end == 0 {
        return None;
    }
    let key = &line[..end];
    let rest = &line[end..];
    let after_space = rest.trim_start();
    let after_eq = after_space.strip_prefix('=')?;
    Some((key.to_owned(), after_eq.trim_start().to_owned()))
}

/// Whether a value that opened a `"""` string closes on the SAME line —
/// `web/config.js`'s `closesTripleQuote`.
fn closes_triple_quote(value_text: &str) -> bool {
    value_text.len() > 3 && value_text.ends_with("\"\"\"") && value_text != "\"\"\""
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **`rivet-01`'s document, read 2026-09-13** — the shape R39's brief
    /// gives, with the real 64-hex `write-token` replaced by a placeholder.
    /// `AGENTS.md` §4: a fixture captured from real hardware is worth more
    /// than a hand-composed one, and the difference is stated here rather
    /// than left to be assumed. **No trailing newline, and two tables with no
    /// blank line before them** — both deliberate, both present in the real
    /// file, and both are exactly what a naive round trip gets wrong.
    const FIXTURE: &str = "# Rivet's configuration document. Written by Rivet and canonically\n\
        # serialised (ADR-007 \u{a7}3.1); change it with the rivet tools, not an editor.\n\
        version = 1\n\
        hostname = \"rivet-01\"\n\
        timezone = \"America/Chicago\"\n\
        \n\
        [services.rivet-mdnsd]\n\
        enabled = true\n\
        \n\
        [services.rivet-proxyd]\n\
        enabled = true\n\
        \n\
        [network.eth0]\n\
        method = \"static\"\n\
        address = \"192.168.50.10/24\"\n\
        gateway = \"192.168.50.1\"\n\
        \n\
        [network.resolver]\n\
        nameservers = [\"192.168.50.1\", \"1.1.1.1\"]\n\
        \n\
        [control-plane]\n\
        listen = \"0.0.0.0:7777\"\n\
        \n\
        [applications.rivet-proxyd]\n\
        upstream-name = \"rivet-01.local\"\n\
        \n\
        [services.aura-controller]\n\
        enabled = true\n\
        \n\
        [services.aura-renderer]\n\
        enabled = true\n\
        \n\
        [services.rivet-launcher]\n\
        enabled = true\n\
        [kiosk]\n\
        auto-run = \"aura-renderer\"\n\
        [services.rivet-fleetd]\n\
        enabled = true\n\
        \n\
        [services.tempmon-rivet]\n\
        enabled = true\n\
        \n\
        [services.notes-rivet]\n\
        enabled = true\n\
        [services.rivet-bluetoothd]\n\
        enabled = true\n\
        [services.reliquary-rivet]\n\
        enabled = true\n\
        [applications.tempmon-rivet]\n\
        sensor-labels = \"GVH5075_1111=Sensor A; GVH5075_2222=Sensor B\"\n\
        write-token = \"PLACEHOLDER-NOT-A-REAL-TOKEN\"";

    /// **The round trip, byte for byte, with no edit.** This is R39's central
    /// proof: `web/config.js`'s reader/writer had never run against real
    /// bytes before this, and a naive port of it fails this exact test by
    /// adding a trailing newline the fixture does not have — see the module
    /// documentation. Broken deliberately and watched to fail
    /// (`AGENTS.md` §4) before being fixed with `trailing_newline`.
    #[test]
    fn the_fixture_round_trips_byte_for_byte_with_no_edit() {
        let doc = parse(FIXTURE);
        assert_eq!(serialize(&doc), FIXTURE);
    }

    /// The same proof holds for a document that DOES end in a newline —
    /// `trailing_newline` must not always be `false`, only ever accurate.
    #[test]
    fn a_document_that_does_end_in_a_newline_keeps_it() {
        let text = "version = 1\nhostname = \"x\"\n";
        let doc = parse(text);
        assert!(doc.trailing_newline);
        assert_eq!(serialize(&doc), text);
    }

    /// **An edit to one scalar changes only that line.** Every other byte of
    /// the fixture — comments, blank lines, every other table — survives
    /// untouched, which is `ADR-043` §6's own promise for a section this
    /// screen's schema does not recognise, proven here for the writer that
    /// underlies it.
    #[test]
    fn editing_one_scalar_changes_only_that_line() {
        let mut doc = parse(FIXTURE);
        let section = doc.sections.get_mut("").expect("top level");
        for entry in &mut section.entries {
            let Entry::Kv { key, raw } = entry else { continue };
            if key == "hostname" {
                *raw = "hostname = \"rivet-02\"".to_owned();
            }
        }
        let after = serialize(&doc);
        let before_lines: Vec<&str> = FIXTURE.lines().collect();
        let after_lines: Vec<&str> = after.lines().collect();
        assert_eq!(before_lines.len(), after_lines.len(), "a line was added or removed");
        let mut differences = 0;
        for (b, a) in before_lines.iter().zip(after_lines.iter()) {
            if b != a {
                differences += 1;
                assert_eq!(*a, "hostname = \"rivet-02\"", "an unexpected line changed: {b:?} -> {a:?}");
            }
        }
        assert_eq!(differences, 1, "more than one line changed");
    }

    /// **`[kiosk] auto-run` survives** — the table with no blank line before
    /// it, immediately after `[services.rivet-launcher]`.
    #[test]
    fn kiosk_auto_run_survives() {
        let doc = parse(FIXTURE);
        let kiosk = doc.sections.get("kiosk").expect("kiosk section");
        assert_eq!(kiosk.header.as_deref(), Some("[kiosk]"));
        let has_auto_run = kiosk.entries.iter().any(|e| matches!(
            e, Entry::Kv { key, raw } if key == "auto-run" && raw.contains("aura-renderer")
        ));
        assert!(has_auto_run, "{kiosk:?}");
    }

    /// **Every `[services.*]` and `[applications.*]` table in the fixture is
    /// read as its own section, in order**, whether or not a blank line
    /// separates it from the one before.
    #[test]
    fn every_services_and_applications_table_survives() {
        let doc = parse(FIXTURE);
        for expected in [
            "services.rivet-mdnsd",
            "services.rivet-proxyd",
            "services.aura-controller",
            "services.aura-renderer",
            "services.rivet-launcher",
            "services.rivet-fleetd",
            "services.tempmon-rivet",
            "services.notes-rivet",
            "services.rivet-bluetoothd",
            "services.reliquary-rivet",
            "applications.rivet-proxyd",
            "applications.tempmon-rivet",
        ] {
            assert!(doc.order.contains(&expected.to_owned()), "{expected} is missing from {:?}", doc.order);
            assert!(doc.sections.contains_key(expected), "{expected}");
        }
    }

    /// **The credential guard finds `write-token`, and names its table.**
    /// This is the fixture's whole reason for carrying a placeholder rather
    /// than an innocuous value — `[applications.tempmon-rivet] write-token`
    /// is `rivet-01`'s real shape, read 2026-09-13.
    #[test]
    fn the_placeholder_token_is_found_by_name() {
        let doc = parse(FIXTURE);
        let found = credential_like_settings(&doc);
        assert!(
            found.contains(&("applications.tempmon-rivet".to_owned(), "write-token".to_owned())),
            "{found:?}"
        );
    }

    /// **An ordinary application setting is NOT flagged** — `upstream-name`
    /// and `sensor-labels` are real settings on real machines and must not be
    /// refused as though they were secrets, or every document would need a
    /// second look for a false alarm.
    #[test]
    fn an_ordinary_setting_is_not_mistaken_for_a_credential() {
        let doc = parse(FIXTURE);
        let found = credential_like_settings(&doc);
        assert!(!found.iter().any(|(_, key)| key == "upstream-name"), "{found:?}");
        assert!(!found.iter().any(|(_, key)| key == "sensor-labels"), "{found:?}");
    }

    /// **A credential-shaped key OUTSIDE `[applications.*]` is not this
    /// guard's job.** `[services.<name>] enabled` and top-level `hostname`
    /// are validated by `rivet_config::Document::parse` itself, which has no
    /// key a credential could be written under at all (`ADR-007` §7) — this
    /// guard exists only for the one region that parser does not interpret.
    #[test]
    fn only_the_applications_region_is_scanned() {
        let text = "version = 1\n[services.x]\ntoken = \"not-scanned-here\"\n";
        let doc = parse(text);
        assert!(credential_like_settings(&doc).is_empty());
    }

    /// **Every needle, matched case-insensitively**, so `Token`, `API-Key`
    /// and `Password` are caught as readily as the lowercase spellings.
    #[test]
    fn every_needle_is_matched_case_insensitively() {
        let text = "[applications.x]\n\
            Token = \"a\"\nAPI-Key = \"b\"\nPassword = \"c\"\nSecret-Value = \"d\"\n\
            harmless-setting = \"e\"\n";
        let doc = parse(text);
        let pairs = credential_like_settings(&doc);
        let found: Vec<&str> = pairs.iter().map(|(_, k)| k.as_str()).collect();
        for expect in ["Token", "API-Key", "Password", "Secret-Value"] {
            assert!(found.contains(&expect), "{found:?}");
        }
        assert!(!found.contains(&"harmless-setting"), "{found:?}");
    }

    /// **`key` is a whole-word match, not a substring one — `.rulings/`
    /// R-210 D16.** `keyboard-layout` is an ordinary application-setting
    /// shape and must not be refused merely because `keyboard` starts with
    /// the three letters `k`, `e`, `y`.
    #[test]
    fn a_key_whose_name_only_contains_key_as_a_substring_is_not_flagged() {
        let text = "[applications.x]\nkeyboard-layout = \"us\"\nmonkey = \"a\"\n";
        let doc = parse(text);
        let found = credential_like_settings(&doc);
        assert!(found.is_empty(), "{found:?}");
    }
}
