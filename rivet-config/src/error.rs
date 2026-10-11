//! Why a document was refused.
//!
//! Every variant names the *field*, because `ADR-007` §3.2 asks for "a typed
//! error naming the field, rather than producing a machine that half-works",
//! and `ADR-006` §8 item 4 asks that the first error type be a typed, coded
//! enumeration rather than a string.
//!
//! **What these errors deliberately do not carry: the offending text.**
//! `AGENTS.md` §5 says a response that does not parse is an error carrying the
//! raw bytes, and for a wire protocol that is right. A configuration document
//! is the one place it is wrong: the provisioning file on the boot partition
//! carries a wireless passphrase (`ADR-004` §6), and an error that quotes the
//! line it failed on is a secret in a log. `ADR-020` §4 puts secrecy in the
//! type; this module keeps it out of the diagnostic by carrying a
//! [`Location`] instead of the content at it.

/// Where in a document something went wrong.
///
/// One-based, so it matches what an editor shows the operator. Columns count
/// characters rather than bytes — the operator is looking at characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Location {
    /// One-based line number.
    pub line: usize,
    /// One-based column, counted in characters.
    pub column: usize,
}

impl Location {
    /// Translate a byte offset into `src` into a line and column.
    ///
    /// An offset past the end of `src` lands on the last position, which is
    /// what a parser reporting "unexpected end of input" wants.
    pub(crate) fn of(src: &str, offset: usize) -> Self {
        let offset = offset.min(src.len());
        // Snap to a character boundary so the slice below cannot panic on a
        // multi-byte character; a parser span always lands on one, but this
        // module is not the place to rely on that.
        let mut start = offset;
        while start > 0 && !src.is_char_boundary(start) {
            start -= 1;
        }
        let before = &src[..start];
        let line = before.matches('\n').count() + 1;
        let column = match before.rfind('\n') {
            Some(nl) => before[nl + 1..].chars().count() + 1,
            None => before.chars().count() + 1,
        };
        Self { line, column }
    }
}

impl core::fmt::Display for Location {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "line {}, column {}", self.line, self.column)
    }
}

/// Every way a configuration document can be refused.
///
/// Closed on purpose, per `ADR-006` §6.4 — this is exactly the kind of
/// enumeration that grows one convenient value at a time until it means
/// nothing. Adding a variant is a change to the interface contract.
///
/// There is no variant for "the value was odd so a default was used", because
/// `ADR-007` §3.6.5 forbids it: *Rivet does not accept an input it intends to
/// discard.*
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigError {
    /// The bytes are not well-formed TOML, so no field can be named yet.
    ///
    /// `detail` is the parser's own words, copied rather than reworded —
    /// `AGENTS.md` §5, report what the system said. It describes the grammar
    /// (what was expected where), never the document's content.
    Malformed {
        /// Where the parser gave up, if it recorded a position.
        at: Option<Location>,
        /// The parser's description of what it wanted.
        detail: String,
    },

    /// A key the schema does not declare.
    ///
    /// Never ignored, never tolerated: `ADR-007` §3.6.5 — "a document
    /// containing a misspelled key that loads successfully is the same defect
    /// as a discarded `/etc` edit".
    UnknownKey {
        /// The dotted path of the offending key.
        path: String,
        /// Where it appears.
        at: Option<Location>,
        /// What the schema does declare at that level, so a caller — human or
        /// agent (`ADR-012`) — can correct it without consulting anything.
        expected: &'static [&'static str],
    },

    /// A key whose *name* the schema constrains, and which is not a usable
    /// name.
    ///
    /// Distinct from [`ConfigError::UnknownKey`], which means "the schema does
    /// not declare this key". This means "the schema lets you choose this key's
    /// name, and that is not a name it can be". The services table is the first
    /// place the operator names keys rather than the schema doing it: a service
    /// name is a cgroup path component and part of a socket's identity
    /// (`ADR-017` §3.1.3, §3.4), so it has to be checkable without knowing
    /// anything about the machine.
    ///
    /// `rule` is `&'static str` rather than a formatted message on purpose:
    /// this variant **cannot** carry any part of the document beyond the key
    /// name already in `path`.
    InvalidKey {
        /// The dotted path of the offending key.
        path: String,
        /// Where it appears.
        at: Option<Location>,
        /// The rule the name breaks, in Rivet's words.
        rule: &'static str,
    },

    /// A key that belongs to the service's **definition**, offered as
    /// configuration.
    ///
    /// Distinct from [`ConfigError::UnknownKey`] because the operator's mistake
    /// is a model rather than a typo, and "unknown key" would leave the model
    /// uncorrected. `ADR-017` §4.1 split the two authorities: what a service
    /// *is* ships inside the signed artifact that provides it, and what the
    /// operator wants *of* it is the document. `ADR-015` §4.1 is why that is not
    /// merely tidy — a definition an operator could rewrite is a signature that
    /// guarantees nothing.
    ///
    /// **Gained [`ConfigError::NotConfigurable::why`] on 2026-08-24**, when
    /// `listen` moved from this table into the manifest (the maintainer's
    /// split-by-direction ruling, 2026-08-24). Until then every key in this class had *always* been the
    /// definition's, so one sentence covered them all; `listen` is the first one
    /// an operator can have read out of an accepted ADR — `ADR-007` §7.2 and
    /// `ADR-015` §5.2 both told them to write it — and a refusal that did not
    /// say so would read as a defect in Rivet rather than as a ruling.
    NotConfigurable {
        /// The dotted path of the offending key.
        path: String,
        /// Where it appears.
        at: Option<Location>,
        /// Why this particular key is not the operator's, in one sentence.
        ///
        /// `&'static str` for the reason [`ConfigError::InvalidKey`]'s `rule`
        /// is: a refusal built from it cannot carry any part of the document.
        why: &'static str,
        /// What the operator *may* set on a service instead.
        expected: &'static [&'static str],
    },

    /// A key the schema requires is absent.
    MissingKey {
        /// The dotted path of the missing key.
        path: String,
        /// What the schema expects to find there.
        expected: &'static str,
    },

    /// The key is declared, and holds something of the wrong shape.
    WrongType {
        /// The dotted path of the key.
        path: String,
        /// Where the value appears.
        at: Option<Location>,
        /// The TOML type the schema requires.
        expected: &'static str,
        /// The TOML type actually found, as the parser names it.
        found: &'static str,
    },

    /// The key and its type are right; the value is outside what the schema
    /// admits.
    InvalidValue {
        /// The dotted path of the key.
        path: String,
        /// Where the value appears.
        at: Option<Location>,
        /// What is wrong with it, in Rivet's words.
        detail: String,
    },

    /// The document declares a schema version this build cannot read.
    ///
    /// `ADR-007` §3.2 requires that an older document be "migrated explicitly
    /// rather than silently misread". Until a second schema version exists
    /// there is nothing to migrate *from*, so the only honest answer is to
    /// refuse. When migration arrives, an older version stops producing this
    /// and a newer one keeps producing it.
    UnsupportedSchemaVersion {
        /// What the document declared.
        found: u32,
        /// What this build reads and writes.
        supported: u32,
        /// Where the declaration appears.
        at: Option<Location>,
    },
}

/// Render `at line L, column C` when a position is known, and nothing when it
/// is not, so the message reads correctly either way.
fn at(location: &Option<Location>) -> String {
    match location {
        Some(l) => format!(" at {l}"),
        None => String::new(),
    }
}

impl core::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Malformed { at: a, detail } => {
                write!(f, "the document is not valid TOML{}: {detail}", at(a))
            }
            Self::UnknownKey { path, at: a, expected } => write!(
                f,
                "unknown configuration key `{path}`{}; this document declares: {}",
                at(a),
                expected.join(", ")
            ),
            Self::InvalidKey { path, at: a, rule } => {
                write!(f, "`{path}`{} is not a usable name; {rule}", at(a))
            }
            Self::NotConfigurable { path, at: a, why, expected } => write!(
                f,
                "configuration key `{path}`{} is part of the service's definition, not its \
                 configuration; a definition ships inside the signed artifact that provides the \
                 service and is not operator-editable. {why} A service accepts: {}",
                at(a),
                expected.join(", ")
            ),
            Self::MissingKey { path, expected } => write!(
                f,
                "configuration key `{path}` is required and is absent; it must be {expected}"
            ),
            Self::WrongType { path, at: a, expected, found } => write!(
                f,
                "configuration key `{path}`{} is {found}; it must be {expected}",
                at(a)
            ),
            Self::InvalidValue { path, at: a, detail } => {
                write!(f, "configuration key `{path}`{}: {detail}", at(a))
            }
            Self::UnsupportedSchemaVersion { found, supported, at: a } => write!(
                f,
                "the document declares schema version {found}{}; this build of Rivet reads \
                 version {supported}",
                at(a)
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_location_is_one_based_at_the_very_start() {
        assert_eq!(Location::of("version = 1", 0), Location { line: 1, column: 1 });
    }

    #[test]
    fn a_location_counts_lines_from_the_newlines_before_it() {
        let src = "a\nb\nversion = 1\n";
        let offset = src.find("version").expect("the fixture contains it");
        assert_eq!(Location::of(src, offset), Location { line: 3, column: 1 });
    }

    #[test]
    fn a_location_counts_columns_in_characters_not_bytes() {
        // "é" is two bytes. A byte column would say 3; an operator's editor
        // says 2, and the operator is who reads this.
        let src = "é = 1";
        let offset = src.find('=').expect("the fixture contains it");
        assert_eq!(Location::of(src, offset), Location { line: 1, column: 3 });
    }

    #[test]
    fn an_offset_past_the_end_lands_on_the_end_rather_than_panicking() {
        let src = "version = 1";
        let l = Location::of(src, 9_999);
        assert_eq!(l.line, 1);
        assert_eq!(l.column, src.chars().count() + 1);
    }

    #[test]
    fn an_offset_inside_a_multibyte_character_does_not_panic() {
        // A parser span should never land here. This module must not be the
        // thing that panics if one ever does — it runs in PID 1's process.
        let src = "é = 1";
        let _ = Location::of(src, 1);
    }

    #[test]
    fn every_message_names_the_field_it_is_about() {
        let cases = [
            ConfigError::UnknownKey {
                path: "net.wlan.ssid".to_owned(),
                at: Some(Location { line: 4, column: 1 }),
                expected: &["version"],
            },
            ConfigError::MissingKey {
                path: "version".to_owned(),
                expected: "an integer",
            },
            ConfigError::WrongType {
                path: "version".to_owned(),
                at: None,
                expected: "an integer",
                found: "a string",
            },
            ConfigError::InvalidValue {
                path: "version".to_owned(),
                at: None,
                detail: "must not be negative".to_owned(),
            },
            ConfigError::InvalidKey {
                path: "services.a/b".to_owned(),
                at: Some(Location { line: 3, column: 2 }),
                rule: "a service name is one path component",
            },
            ConfigError::NotConfigurable {
                path: "services.rivet-apid.exec".to_owned(),
                at: Some(Location { line: 5, column: 1 }),
                why: "no such field exists anywhere.",
                expected: &["enabled", "memory"],
            },
        ];
        for case in cases {
            let rendered = case.to_string();
            let path = match &case {
                ConfigError::UnknownKey { path, .. }
                | ConfigError::MissingKey { path, .. }
                | ConfigError::WrongType { path, .. }
                | ConfigError::InvalidValue { path, .. }
                | ConfigError::InvalidKey { path, .. }
                | ConfigError::NotConfigurable { path, .. } => path.clone(),
                other => panic!("this case has no path: {other:?}"),
            };
            assert!(
                rendered.contains(&path),
                "{rendered:?} does not name `{path}`"
            );
        }
    }

    #[test]
    fn a_message_without_a_position_does_not_say_at_nothing() {
        let e = ConfigError::MissingKey {
            path: "version".to_owned(),
            expected: "an integer",
        };
        let rendered = e.to_string();
        assert!(!rendered.contains(" at "), "{rendered:?}");
    }

    #[test]
    fn an_unknown_key_message_lists_what_is_accepted_instead() {
        let e = ConfigError::UnknownKey {
            path: "verison".to_owned(),
            at: Some(Location { line: 1, column: 1 }),
            expected: &["version"],
        };
        let rendered = e.to_string();
        assert!(rendered.contains("verison"), "{rendered:?}");
        assert!(rendered.contains("version"), "{rendered:?}");
        assert!(rendered.contains("line 1, column 1"), "{rendered:?}");
    }

    /// The refusal that teaches the `ADR-017` §4.1 split has to actually teach
    /// it. An operator who wrote `exec` has a wrong model, and a message that
    /// only said "unknown key" would leave it wrong.
    #[test]
    fn refusing_a_definition_key_says_why_rather_than_only_that() {
        let e = ConfigError::NotConfigurable {
            path: "services.rivet-apid.exec".to_owned(),
            at: Some(Location { line: 5, column: 1 }),
            why: "There is no such field anywhere, not even in the manifest.",
            expected: &["enabled", "memory"],
        };
        let rendered = e.to_string();
        assert!(rendered.contains("services.rivet-apid.exec"), "{rendered:?}");
        assert!(rendered.contains("definition"), "{rendered:?}");
        assert!(rendered.contains("signed artifact"), "{rendered:?}");
        // And it must say what may be set instead, or the operator is stuck.
        assert!(rendered.contains("enabled"), "{rendered:?}");
        assert!(rendered.contains("line 5, column 1"), "{rendered:?}");
    }

    /// `InvalidKey` carries a `&'static str`, so it is structurally incapable of
    /// reproducing the document. This test is the guard on that property.
    #[test]
    fn an_invalid_key_refusal_carries_a_rule_rather_than_the_document() {
        let e = ConfigError::InvalidKey {
            path: "services.bad name".to_owned(),
            at: None,
            rule: "a service name is 1 to 64 characters of ASCII letters and digits",
        };
        let rendered = format!("{e}\n{e:?}");
        assert!(rendered.contains("services.bad name"), "{rendered:?}");
        assert!(rendered.contains("ASCII letters"), "{rendered:?}");
    }

    #[test]
    fn the_error_is_usable_as_a_std_error() {
        fn assert_error<T: std::error::Error + Send + Sync + 'static>() {}
        assert_error::<ConfigError>();
    }
}
