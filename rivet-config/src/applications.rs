//! `[applications.<name>]` — **what an application is set to on this machine.**
//!
//! `ADR-038` §4. The one region of the document whose schema `rivet-config`
//! does not own: the keys come from a signed artifact's manifest
//! (`rivet_core::package::application::Manifest::settings`), not from here.
//!
//! # Why the document and not a file beside the application
//!
//! Two things `ADR-038` §4.1 says are available no other way:
//!
//! * **Provisioned before first boot.** `ADR-034` §5.1's common case is a
//!   machine that has never been powered on. A setting in the document is
//!   written by `rivet-provision` at card-write time; a setting in a data
//!   directory cannot be, because there is no filesystem to put it in yet.
//! * **In the register, therefore reproducible.** `ADR-034` §6's *fleet
//!   configuration* row has had no contents since it was written. This is its
//!   contents: set a value for a fleet once and have it survive a card rewrite.
//!
//! # A bad setting must NEVER refuse the document — and that is the whole of it
//!
//! Everywhere else in this crate an unrecognised key is **fatal**: a document
//! silently carrying something nothing reads is
//! worse than one that will not load. **Inside this region that rule is
//! inverted**, and `ADR-038` §4.4 argues why it is not merely a weakening:
//!
//! * `PLAN.md` §104.19 records that a document a machine will not parse can
//!   **strand it**. Refusing to boot over a misspelled brightness key would be
//!   a catastrophic answer to a trivial mistake.
//! * Everywhere else an unknown key means *this document is from a future
//!   `rivet-config` cannot understand*. Here the platform explicitly does not
//!   own the schema, so it means something far smaller: **one application was
//!   told something it does not understand.**
//!
//! So this module **reports and keeps going**. What it could not represent at
//! all is carried in [`Applications::ignored`] for the supervisor to journal
//! and `get status` to show.
//!
//! # This crate validates STRUCTURE; the machine validates MEANING
//!
//! `rivet-config` runs on a workstation composing a document for a machine it
//! may never see, so it cannot know which applications are installed or what
//! they declare. It therefore keeps **every** well-formed `key = "value"` it is
//! given, including for an application that does not exist yet — which is
//! exactly what provisioning-before-install requires.
//!
//! Checking a key against a manifest is the supervisor's, at reload, where the
//! manifests are (`ADR-038` §4.3: *"the check moves, it does not vanish"*).

use std::collections::BTreeMap;

use crate::error::ConfigError;

/// One application's settings, as the document states them.
///
/// **Ordered by key**, because `ADR-007` §3.1 requires two machines in one
/// state to produce byte-identical documents and a TOML table has no order a
/// parser must preserve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationSettings {
    name: String,
    values: BTreeMap<String, String>,
}

impl ApplicationSettings {
    /// Build from parts. The map's ordering is the canonical one.
    #[must_use]
    pub const fn new(name: String, values: BTreeMap<String, String>) -> Self {
        Self { name, values }
    }

    /// Which application this is for.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Every setting, ordered by key.
    #[must_use]
    pub const fn values(&self) -> &BTreeMap<String, String> {
        &self.values
    }

    /// One setting's value.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// Whether it carries nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// **Something under `[applications]` this crate could not represent**, kept so
/// it can be reported rather than disappearing.
///
/// `ADR-038` §4.4: reported and ignored, never fatal. An operator finds out
/// from `get status` and from the reload's answer; the machine keeps running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ignored {
    /// The application whose table it was under.
    pub application: String,
    /// The key, as written.
    pub key: String,
    /// Why it could not be carried, as a sentence for a person.
    pub why: &'static str,
}

impl Ignored {
    /// The reason a value that is not a string is dropped.
    ///
    /// **Text-only, and `ADR-038` §10.2 accepted it unwillingly.** A number in
    /// the document would be a number this crate has to guess the type of on
    /// the way back out, and a setting that round-trips as `1` when it was
    /// written `1.0` is a document that changed itself.
    pub const NOT_TEXT: &'static str =
        "a setting's value must be a quoted string - ADR-038 §10.2 keeps values textual for a \
         first version, so a bare number or boolean is not carried. Quote it: `max-readers = \
         \"3\"`, not `max-readers = 3`";

    /// The reason a nested table is dropped.
    pub const NOT_FLAT: &'static str =
        "a setting is one key and one string; a nested table under an application is a shape \
         nothing delivers";

    /// **The value is not a string AND the key is one `[services.<name>]`
    /// declares** — added 2026-09-05, after a port lost a boot to it.
    ///
    /// The generic [`NOT_TEXT`](Self::NOT_TEXT) is true of `enabled = true` and
    /// is the worst possible thing to say about it, because the repair it
    /// suggests works: quoting the value produces a document that loads and an
    /// application that is still not running. So the reader gets one step
    /// further and then has to find out from the supervisor's journal — which
    /// until the same day said only that the key was undeclared.
    ///
    /// **This crate cannot make the same judgement about a quoted `enabled`.**
    /// It has no manifest, so it cannot know whether an application declares a
    /// setting of that name, and dropping one would narrow a region whose schema
    /// `ADR-038` §4.3 explicitly puts outside this crate. That case is
    /// `rivet_core::settings::Rejected::BELONGS_TO_THE_SERVICE`, at the layer
    /// where the manifests are.
    pub const BELONGS_TO_THE_SERVICE: &'static str =
        "a setting's value must be a quoted string, and this key is one [services.<name>] \
         declares. Quoting it will not do what you want: to switch an installed application on, \
         write `enabled = true` under [services.<name>]. [applications.<name>] carries only the \
         settings the application's own manifest names";

    /// Which reason a non-string value earns.
    ///
    /// **One place, so the two cannot drift apart** — `ADR-011` again. The
    /// parser calls it; the test below asserts every reason it can produce.
    #[must_use]
    pub fn why_not_text(key: &str) -> &'static str {
        if rivet_core::service::OPERATOR_KEYS.contains(&key) {
            Self::BELONGS_TO_THE_SERVICE
        } else {
            Self::NOT_TEXT
        }
    }
}

/// `[applications]` — every application's settings, and what was ignored.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Applications {
    /// **Ordered by application name**, for `ApplicationSettings`' reason.
    apps: BTreeMap<String, ApplicationSettings>,
    /// What could not be carried. Empty on every well-formed document.
    ignored: Vec<Ignored>,
}

impl Applications {
    /// Build from parts.
    #[must_use]
    pub fn new(apps: Vec<ApplicationSettings>, ignored: Vec<Ignored>) -> Self {
        Self {
            apps: apps
                .into_iter()
                .map(|settings| (settings.name.clone(), settings))
                .collect(),
            ignored,
        }
    }

    /// One application's settings, if the document names it.
    ///
    /// **`None` is not an error.** An application with no table is one nobody
    /// has configured, which is every application on a fresh machine.
    #[must_use]
    pub fn get(&self, application: &str) -> Option<&ApplicationSettings> {
        self.apps.get(application)
    }

    /// Every application named, in canonical order.
    #[must_use]
    pub fn all(&self) -> impl Iterator<Item = &ApplicationSettings> {
        self.apps.values()
    }

    /// What could not be carried — `ADR-038` §4.4's report.
    #[must_use]
    pub fn ignored(&self) -> &[Ignored] {
        &self.ignored
    }

    /// Whether the document configured nothing at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.apps.is_empty()
    }
}

/// Parse just the `[applications.*]` region of `src`, through the same TOML
/// grammar [`crate::Document::parse`] uses for it — so a quoted key, an
/// indented key, or a quoted table segment resolve exactly as they would
/// there — without validating anything else in the document.
///
/// **Why this exists beside [`crate::Document::parse`].** That entry point
/// validates the WHOLE document and refuses on the first problem anywhere in
/// it. A caller that only needs to know what an application is named here —
/// `cmd/rivet-fleetd`'s credential scan, which must see every application
/// setting's real name before it can trust its own answer — should not have
/// to depend on every other section already being valid to get one.
///
/// # Errors
///
/// `src` is not well-formed TOML, or `applications` itself is not a table of
/// tables — the same two failures [`crate::Document::parse`] reports for
/// this region.
pub fn parse_only(src: &str) -> Result<Applications, ConfigError> {
    crate::parse::applications_only(src)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    fn settings(name: &str, pairs: &[(&str, &str)]) -> ApplicationSettings {
        ApplicationSettings::new(
            name.to_string(),
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
        )
    }

    /// Applications and their keys are both ordered, which is what
    /// `ADR-007` §3.1's byte-identical requirement needs from a TOML table.
    #[test]
    fn applications_and_their_keys_are_both_in_canonical_order() {
        let apps = Applications::new(
            vec![
                settings("zebra", &[("b", "2"), ("a", "1")]),
                settings("aura-renderer", &[("brightness", "60")]),
            ],
            Vec::new(),
        );
        let names: Vec<&str> = apps.all().map(ApplicationSettings::name).collect();
        assert_eq!(names, ["aura-renderer", "zebra"]);
        let keys: Vec<&str> = apps
            .get("zebra")
            .expect("zebra")
            .values()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["a", "b"]);
    }

    /// An application nobody configured is `None`, and that is not a failure —
    /// it is every application on a fresh machine.
    #[test]
    fn an_application_with_no_table_is_absent_rather_than_empty() {
        let apps = Applications::default();
        assert!(apps.is_empty());
        assert_eq!(apps.get("aura-renderer"), None);
        assert!(apps.ignored().is_empty());
    }

    /// A value is reachable by name, and a near miss is not.
    #[test]
    fn a_setting_is_reachable_by_its_own_name_only() {
        let apps = Applications::new(
            vec![settings("aura-renderer", &[("brightness", "60")])],
            Vec::new(),
        );
        let one = apps.get("aura-renderer").expect("present");
        assert_eq!(one.get("brightness"), Some("60"));
        assert_eq!(one.get("brightnes"), None);
        assert!(!one.is_empty());
    }

    /// **Both reasons read as sentences**, because an operator meets them in a
    /// status answer rather than in a stack trace.
    #[test]
    fn every_ignored_reason_is_a_sentence_naming_what_to_do() {
        for why in [
            Ignored::NOT_TEXT,
            Ignored::NOT_FLAT,
            Ignored::BELONGS_TO_THE_SERVICE,
        ] {
            assert!(why.len() > 60, "too terse to act on: {why}");
            assert!(why.contains("setting"), "{why}");
        }
        assert!(Ignored::NOT_TEXT.contains("ADR-038"), "the reason should cite its ruling");
    }

    /// **A key of `[services.<name>]` gets the reason that names that table.**
    ///
    /// Driven off `OPERATOR_KEYS` rather than the literal `"enabled"`, so a key
    /// added to that table is covered the day it is added.
    #[test]
    fn a_service_key_earns_the_reason_that_names_the_service_table() {
        for key in rivet_core::service::OPERATOR_KEYS {
            assert_eq!(
                Ignored::why_not_text(key),
                Ignored::BELONGS_TO_THE_SERVICE,
                "`{key}` earned the generic quoted-string reason"
            );
        }
        for key in ["brightness", "max-readers", "enable", "enabled-x"] {
            assert_eq!(Ignored::why_not_text(key), Ignored::NOT_TEXT, "{key}");
        }
    }

    /// **The repair each reason suggests must actually work**, which is the
    /// defect in one sentence: quoting `enabled = true` produces a document
    /// that loads and an application that is still off.
    #[test]
    fn the_service_reason_says_that_quoting_will_not_help() {
        let why = Ignored::BELONGS_TO_THE_SERVICE;
        assert!(why.contains("[services.<name>]"), "{why}");
        assert!(why.contains("[applications.<name>]"), "{why}");
        assert!(why.contains("enabled = true"), "{why}");
        assert!(
            !Ignored::NOT_TEXT.contains("[services."),
            "the two reasons must stay distinguishable"
        );
        // And the ordinary one now shows the repair rather than only the rule,
        // which is what Trends' integer settings needed.
        assert!(Ignored::NOT_TEXT.contains("Quote it"), "{}", Ignored::NOT_TEXT);
    }

    // ---- parse_only -------------------------------------------------------

    /// **A bare, unindented key is the baseline** [`parse_only`] must keep
    /// reading exactly as [`crate::Document::parse`] already does.
    #[test]
    fn parse_only_reads_a_bare_key() {
        let apps = parse_only("[applications.tempmon-rivet]\nwrite-token = \"x\"\n").expect("parses");
        assert_eq!(apps.get("tempmon-rivet").and_then(|a| a.get("write-token")), Some("x"));
    }

    /// **A quoted key** — `"write-token" = "x"` — is TOML exactly as valid as
    /// a bare one, and normalizes to the same name.
    #[test]
    fn parse_only_reads_a_quoted_key() {
        let apps = parse_only("[applications.tempmon-rivet]\n\"write-token\" = \"x\"\n").expect("parses");
        assert_eq!(apps.get("tempmon-rivet").and_then(|a| a.get("write-token")), Some("x"));
    }

    /// **A literal (single-quoted) key** — TOML's other quoted-key form.
    #[test]
    fn parse_only_reads_a_literal_quoted_key() {
        let apps = parse_only("[applications.tempmon-rivet]\n'write-token' = \"x\"\n").expect("parses");
        assert_eq!(apps.get("tempmon-rivet").and_then(|a| a.get("write-token")), Some("x"));
    }

    /// **An indented key** — TOML has no meaningful indentation, so leading
    /// whitespace before a key is exactly as valid as none.
    #[test]
    fn parse_only_reads_an_indented_key() {
        let apps = parse_only("[applications.tempmon-rivet]\n    write-token = \"x\"\n").expect("parses");
        assert_eq!(apps.get("tempmon-rivet").and_then(|a| a.get("write-token")), Some("x"));
    }

    /// **A table header with a quoted segment** — including the `applications`
    /// segment itself — names the same table a bare header would.
    #[test]
    fn parse_only_reads_a_quoted_table_segment() {
        let apps = parse_only("[\"applications\".\"tempmon-rivet\"]\nwrite-token = \"x\"\n").expect("parses");
        assert_eq!(apps.get("tempmon-rivet").and_then(|a| a.get("write-token")), Some("x"));
    }

    /// **Everything else in the document can be nonsense** — `parse_only`
    /// answers from `[applications]` alone, which is the whole reason it
    /// exists beside `Document::parse`.
    #[test]
    fn parse_only_does_not_require_the_rest_of_the_document_to_be_valid() {
        let apps = parse_only("this-is-not-a-declared-top-level-key = 1\n[applications.a]\nk = \"v\"\n")
            .expect("parses despite an unknown top-level key");
        assert_eq!(apps.get("a").and_then(|a| a.get("k")), Some("v"));
    }

    /// Malformed TOML is refused, named, rather than read as an empty table.
    #[test]
    fn parse_only_refuses_malformed_toml() {
        assert!(parse_only("[applications\n").is_err());
    }
}
