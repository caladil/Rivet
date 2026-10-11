//! The argument surface: what a word on the command line is allowed to be.
//!
//! `ADR-006` §6.4 fixes the vocabulary and says why:
//!
//! > Administrative commands are drawn from a **closed set of verbs** — `get`,
//! > `set`, `list`, `add`, `remove`, `apply`, `plan`, `export`, `explain` — and
//! > a noun vocabulary that matches the configuration schema exactly. … The
//! > value is not tidiness. It is that a reader who knows one command can
//! > predict the others, and an agent can construct a valid command it has never
//! > seen from the schema alone.
//!
//! # The verb list moved to `rivet-core` on 2026-08-23
//!
//! This module used to hold it, with a comment saying where it belonged:
//!
//! > **The day an operation exists**, the verb and noun belong to the
//! > operation's type in the core and this list becomes a projection of it —
//! > which is `ADR-006` §7's generated catalogue, and is not built.
//!
//! That day is this change. [`rivet_core::Verb`] is the list, and this module
//! reads it: a word is looked up with `Verb::parse`, which produces the refusal
//! too, so the sentence naming the vocabulary is written once rather than once
//! per adapter.
//!
//! The worry the old comment implied — that a verb enum in `rivet-core` is the
//! `core → anything CLI-shaped` edge `ADR-014` §3 forbids — does not apply. The
//! forbidden thing is an *argument parser*: `clap`, or a type that knows what a
//! flag is. A verb is the administrative vocabulary itself, which every adapter
//! reads and none owns, and the network API of `ADR-006` §5.1 will read the same
//! one.
//!
//! # What is left here, and it is exactly `ADR-006` §3's list
//!
//! > **Every interface is an adapter containing no logic.** The CLI parses
//! > arguments and renders results. That is all it does.
//!
//! So: `--output`, `--help`, `--version`, `schema`, splitting the words after
//! the verb from the ones before it, and nothing else. What a verb *does* is
//! `rivet_core::operation::perform`'s.
//!
//! # `schema` WAS a bare word, and is `rivet get schema` since 2026-08-25
//!
//! This module argued the other way for two days. The argument is kept, struck,
//! because it was wrong in an instructive place:
//!
//! > ~~`ADR-006` §7 writes the command as `rivet schema`, and `schema` is not
//! > in §6.4's closed verb set. … §6.4 governs *administrative commands* —
//! > things done to a machine — while §7 is the surface **describing itself**,
//! > which `--version` and `--help` also do and which are also not verbs.~~
//!
//! **It broke on `explain`.** That operation performs no I/O, reads no
//! filesystem, and answers identically on every machine running a given build —
//! a property of the build, exactly like a schema — and it is a verb, in the
//! catalogue, first-class. So *"describes the build rather than the machine"*
//! cannot be what separates a verb from a bare word, because `explain` has that
//! property and is a verb.
//!
//! **Two things settled it**, one measured and one grammatical:
//!
//! 1. **It was undiscoverable.** `rivet --version` publishes the operation
//!    catalogue, and a board answered `apply, explain, get`. A machine reading
//!    that catalogue to learn what it could do would never find `schema`.
//!    `ADR-012` makes an agent a first-class operator, and a surface with
//!    invisible exceptions cannot be driven from its own description.
//! 2. **`rivet schema` has no verb in it.** The ruling is the maintainer's
//!    (.rulings/ R-017): `measure` names an action the tool is performing, so it
//!    is a verb, and `schema` named none.
//!
//! `schema` is now a **subject of `get`**, so §6.4's closed set is untouched —
//! which is the point of choosing an existing verb over a tenth one. The old
//! spelling refuses **by name**; the arm below says why that is a refusal and
//! not an alias.
//!
//! **On a fleet it is also genuinely a per-machine fact**, which the old reading
//! missed: A/B slots and staggered updates mean two boards can run different
//! builds and accept different documents, so *ask this machine what it accepts*
//! is the useful question and *what does the tool describe* is not.
//!
//! # `measure` is the second bare word, added 2026-08-23
//!
//! `measure` reaching [`Request::Handler`] follows the precedent above rather than widening it, and
//! `crate::measure`'s module documentation carries the argument: `ADR-029` §7.2
//! rules BLAKE3 the update payload's digest *"pending one measurement"* which can
//! only be taken through this binary, because this binary is the login shell and
//! refuses to execute anything else. `Verb::parse("measure")`
//! still refuses, exactly as it refuses `schema`.
//!
//! **The same open question applies to it and is sharper**, because unlike
//! `schema` this word appears nowhere in `ADR-006`: whether `measure` belongs in
//! §6.4's closed set, stays a bare word, or is deleted once the number is known,
//! is the maintainer's — §6.4 says the verb set "is extended only by amending
//! this ADR", and an agent adding a tenth verb because its command needed one is
//! precisely what that sentence exists to stop.

use rivet_core::Verb;

use crate::handler;
use crate::refusal::{Code, Refusal};

/// Which rendering of the value the caller wants. `ADR-006` §4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    /// TTY-detected, which is §4's stated default: *"a terminal gets the human
    /// rendering, a pipe gets the machine form."*
    Auto,
    Json,
    Text,
}

impl Output {
    /// Resolve [`Output::Auto`] against a fact the caller observed.
    ///
    /// Taking the answer rather than calling `IsTerminal` here is what makes
    /// both branches testable on a build machine, where the test harness's
    /// stdout is whatever `cargo` gave it.
    pub const fn resolve(self, stdout_is_terminal: bool) -> Self {
        match self {
            Self::Auto if stdout_is_terminal => Self::Text,
            Self::Auto => Self::Json,
            other => other,
        }
    }
}

/// The **retired** spelling of the self-description command, kept so the
/// refusal that replaced it can be written once and tested by name.
/// `ADR-006` §7 wrote it this way; it is `rivet get schema` since 2026-08-25.
pub const SCHEMA: &str = "schema";

/// What the operator asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Help,
    Version,
    /// **A word the login shell answers to itself** — `ADR-032` §4.2's internal
    /// handler table, of which `measure` is the only entry this build has.
    ///
    /// ~~`Measure { rest }`~~ until 2026-08-26: one variant per bare word was
    /// the shape §4.2 ruled against, because *"the fourth special case is where
    /// somebody adds a fifth without noticing it has become a pattern"*. The
    /// name comes from [`crate::handler::HANDLERS`], so a variant can no longer
    /// exist for a word the table does not publish.
    ///
    /// The words after it are bound by the handler rather than here — the same
    /// division [`Self::Verb`] uses, so this parser still knows nothing about
    /// what any command means.
    Handler {
        name: &'static str,
        rest: Vec<String>,
    },
    /// A verb from `ADR-006` §6.4's set, and whatever followed it. The words
    /// after it are bound to the operation's declared inputs by
    /// `rivet_core::operation::perform`, not here.
    Verb {
        verb: Verb,
        rest: Vec<String>,
    },
}

/// A parsed command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub output: Output,
    pub request: Request,
}

/// Parse the arguments after the program name.
pub fn parse(args: &[String]) -> Result<Parsed, Refusal> {
    let mut output = Output::Auto;
    let mut request: Option<Request> = None;

    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();

        // Once a verb has been read, everything after it belongs to the verb.
        // A flag after the verb is the verb's business, not this parser's — and
        // guessing which is which is how `rivet get --output` becomes
        // ambiguous. `measure` collects its words the same way and for the same
        // reason; the or-pattern is what keeps that one rule rather than two.
        if let Some(Request::Verb { rest, .. } | Request::Handler { rest, .. }) = &mut request {
            rest.push(arg.to_string());
            i += 1;
            continue;
        }

        match arg {
            "-h" | "--help" => request = Some(Request::Help),
            "-V" | "--version" => request = Some(Request::Version),
            // **Retired 2026-08-25, and refused by name rather than silently.**
            // `schema` was a bare word here; it is `rivet get schema` now.
            // Falling through to "unknown verb" would be technically correct
            // and useless — this is the one word whose old spelling is
            // *certain* to be typed, by a person or by a script, and a refusal
            // that names the new form costs one line and saves the guess.
            //
            // It is deliberately NOT an alias. `ADR-006` §6.4 keeps one
            // spelling per thing, and an alias that works for ever is how a
            // surface ends up with two names for one operation and a catalogue
            // that describes one of them.
            SCHEMA => {
                return Err(Refusal::new(
                    Code::ARGUMENTS,
                    "`rivet schema` is now `rivet get schema`. It was a bare word and a \
                     bare noun is not a command; it is a subject of `get`, which is where the \
                     operation catalogue can see it — `rivet --version` publishes that \
                     catalogue and did not name `schema` before. The RESULT SHAPE CHANGED with \
                     it: the JSON Schema document is now under a `schema` key rather than \
                     being the whole answer, so a pipeline wants `| get schema` on the end.",
                ));
            }
            // **The table decides, not this match arm.** `ADR-032` §4.2: the word
            // is a handler if `crate::handler::HANDLERS` says so, and a fifth
            // special case cannot be added here without appearing there.
            word if handler::parse(word).is_some() => {
                let Some(found) = handler::parse(word) else {
                    unreachable!("the guard above just matched")
                };
                // `found.name` rather than `word`: the NAME in the request is
                // the table's spelling, so a dispatch downstream compares two
                // `&'static str`s from one definition rather than one of them
                // against a slice of argv.
                request = Some(Request::Handler {
                    name: found.name,
                    rest: Vec::new(),
                });
            }
            "--output" => {
                i += 1;
                let Some(value) = args.get(i) else {
                    return Err(Refusal::new(
                        Code::ARGUMENTS,
                        "--output needs a value: json or text",
                    ));
                };
                output = output_value(value)?;
            }
            _ if arg.starts_with("--output=") => {
                output = output_value(arg.trim_start_matches("--output="))?;
            }
            _ if arg.starts_with('-') => {
                return Err(Refusal::about(
                    Code::ARGUMENTS,
                    arg,
                    "unrecognised option. `rivet --help` lists what this build accepts.",
                ));
            }
            // The vocabulary — and the sentence that lists it when a word is
            // not in it — is `rivet-core`'s. `ADR-011`: one authority, and it
            // says so at the moment the answer is needed.
            word => {
                request = Some(Request::Verb {
                    verb: Verb::parse(word)?,
                    rest: Vec::new(),
                });
            }
        }
        i += 1;
    }

    let Some(request) = request else {
        return Err(Refusal::new(
            Code::ARGUMENTS,
            "no command. `rivet --help` lists what this build accepts.",
        ));
    };
    Ok(Parsed { output, request })
}

fn output_value(value: &str) -> Result<Output, Refusal> {
    match value {
        "json" => Ok(Output::Json),
        "text" => Ok(Output::Text),
        other => Err(Refusal::about(
            Code::ARGUMENTS,
            other,
            "--output takes json or text",
        )),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::handler::MEASURE;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|w| (*w).to_string()).collect()
    }

    /// **The contract, written out**, because `ADR-006` §6.4 makes the verb set
    /// part of the interface and extensible only by amending the ADR.
    ///
    /// Asserted here as well as in `rivet-core` on purpose: this is the
    /// surface an operator types at, and the core's test is about the
    /// vocabulary. If the projection ever stops being a projection, the two
    /// disagree and this fails.
    #[test]
    fn every_verb_this_cli_accepts_is_one_rivet_core_declares() {
        let accepted: Vec<&str> = Verb::ALL.iter().map(|v| v.as_str()).collect();
        assert_eq!(
            accepted,
            [
                "get", "set", "list", "add", "remove", "apply", "plan", "export", "explain"
            ]
        );
        for verb in Verb::ALL {
            let parsed = parse(&argv(&[verb.as_str()]));
            assert!(
                matches!(parsed, Ok(Parsed { request: Request::Verb { verb: v, .. }, .. }) if v == *verb),
                "{verb} did not parse: {parsed:?}"
            );
            assert!(!verb.as_str().starts_with('-'), "{verb} looks like a flag");
        }
    }

    /// The distinction `ADR-006` §4.1 exists for: *not yet* and *never* must be
    /// different answers.
    #[test]
    fn a_word_outside_the_vocabulary_is_a_different_refusal_from_an_unimplemented_verb() {
        let Err(r) = parse(&argv(&["frobnicate"])) else {
            panic!("an invented verb parsed");
        };
        assert_eq!(r.code, Code::UNKNOWN_VERB);
        assert_eq!(r.subject.as_deref(), Some("frobnicate"));
        // And the message lists the vocabulary, so the next attempt can be right.
        for verb in Verb::ALL {
            assert!(
                r.message.contains(verb.as_str()),
                "{verb} missing from {}",
                r.message
            );
        }
    }

    /// **Near-misses matter more than nonsense.** `status`, `show` and `info`
    /// are what a person types from habit, and `ADR-006` §6.4 names exactly that
    /// habit as the thing the closed vocabulary is for: *"a surface where
    /// `net status` and `service list` and `image show` all mean 'tell me
    /// about' is a surface that must be memorised instead of understood."*
    #[test]
    fn the_verbs_other_tools_use_are_refused_rather_than_quietly_aliased() {
        for word in ["status", "show", "info", "ls", "GET", "Get"] {
            let got = parse(&argv(&[word]));
            assert!(
                matches!(
                    got,
                    Err(Refusal {
                        code: Code::UNKNOWN_VERB,
                        ..
                    })
                ),
                "{word} was accepted: {got:?}"
            );
        }
    }

    #[test]
    fn everything_after_the_verb_belongs_to_the_verb() {
        let parsed = parse(&argv(&["get", "network.eth0.address", "--depth", "2"]));
        assert_eq!(
            parsed,
            Ok(Parsed {
                output: Output::Auto,
                request: Request::Verb {
                    verb: Verb::Get,
                    rest: argv(&["network.eth0.address", "--depth", "2"]),
                },
            })
        );
    }

    #[test]
    fn output_is_accepted_in_both_spellings_and_refused_otherwise() {
        assert!(matches!(
            parse(&argv(&["--output", "json", "get"])),
            Ok(Parsed {
                output: Output::Json,
                ..
            })
        ));
        assert!(matches!(
            parse(&argv(&["--output=text", "get"])),
            Ok(Parsed {
                output: Output::Text,
                ..
            })
        ));
        assert!(matches!(
            parse(&argv(&["--output", "yaml", "get"])),
            Err(Refusal {
                code: Code::ARGUMENTS,
                ..
            })
        ));
        assert!(matches!(
            parse(&argv(&["--output"])),
            Err(Refusal {
                code: Code::ARGUMENTS,
                ..
            })
        ));
    }

    /// `ADR-006` §4's default, in both directions. A pipe getting the human
    /// rendering is the defect that makes `--output json` look like a bolt-on.
    #[test]
    fn auto_is_text_at_a_terminal_and_json_down_a_pipe() {
        assert_eq!(Output::Auto.resolve(true), Output::Text);
        assert_eq!(Output::Auto.resolve(false), Output::Json);
    }

    /// An explicit choice is not overridden by what stdout happens to be.
    #[test]
    fn an_explicit_output_survives_tty_detection() {
        assert_eq!(Output::Json.resolve(true), Output::Json);
        assert_eq!(Output::Text.resolve(false), Output::Text);
    }

    #[test]
    fn an_unknown_option_is_refused_rather_than_taken_as_a_verb() {
        let Err(r) = parse(&argv(&["--wat"])) else {
            panic!("--wat was accepted");
        };
        assert_eq!(r.code, Code::ARGUMENTS);
        assert_eq!(r.subject.as_deref(), Some("--wat"));
    }

    #[test]
    fn no_arguments_at_all_is_an_argument_error_rather_than_a_default_command() {
        assert!(matches!(
            parse(&[]),
            Err(Refusal {
                code: Code::ARGUMENTS,
                ..
            })
        ));
    }

    #[test]
    fn help_and_version_parse_in_both_spellings() {
        for spelling in ["-h", "--help"] {
            assert!(matches!(
                parse(&argv(&[spelling])),
                Ok(Parsed {
                    request: Request::Help,
                    ..
                })
            ));
        }
        for spelling in ["-V", "--version"] {
            assert!(matches!(
                parse(&argv(&[spelling])),
                Ok(Parsed {
                    request: Request::Version,
                    ..
                })
            ));
        }
    }

    /// **The retired spelling refuses BY NAME**, and does not fall through to
    /// "unknown verb". `rivet schema` is the one old form certain to be typed
    /// again — by a person or by a script — so the refusal names the new one.
    ///
    /// It is a refusal and not an alias: `ADR-006` §6.4 keeps one spelling per
    /// thing, and an alias that works for ever is how a surface ends up with two
    /// names for one operation and a catalogue describing one of them.
    #[test]
    fn the_retired_bare_word_refuses_and_names_what_replaced_it() {
        let refusal = parse(&argv(&[SCHEMA])).expect_err("`schema` is no longer a command");
        let message = format!("{refusal:?}");
        assert!(
            message.contains("get schema"),
            "the refusal must name the new spelling: {message}"
        );
        assert!(
            message.to_ascii_lowercase().contains("shape"),
            "and must say the result shape changed with it — a caller that only \
             renames the command still gets a different answer: {message}"
        );
    }

    /// **And it never was a verb, before or after.** Moving it into `get` made
    /// it a SUBJECT; `ADR-006` §6.4.s closed set is untouched, which is the
    /// point of choosing an existing verb over a tenth one.
    #[test]
    fn schema_is_not_a_verb_and_the_core_still_refuses_it_as_one() {
        assert!(Verb::parse(SCHEMA).is_err());
        assert!(!Verb::ALL.iter().any(|v| v.as_str() == SCHEMA));
    }

    /// `ADR-029` §7.2's measurement, parsed the way its module documents it:
    /// a bare word that collects everything after it.
    #[test]
    fn measure_collects_its_words_the_way_a_verb_does() {
        assert_eq!(
            parse(&argv(&[MEASURE, "hash", "/dev/mmcblk0p3"])),
            Ok(Parsed {
                output: Output::Auto,
                request: Request::Handler {
                    name: MEASURE,
                    rest: argv(&["hash", "/dev/mmcblk0p3"]),
                },
            })
        );
        // And `--output` before it still binds to the parser rather than being
        // swallowed as one of the command's own words.
        assert_eq!(
            parse(&argv(&["--output", "json", MEASURE, "hash", "/dev/x"])),
            Ok(Parsed {
                output: Output::Json,
                request: Request::Handler {
                    name: MEASURE,
                    rest: argv(&["hash", "/dev/x"]),
                },
            })
        );
    }

    /// **The second bare word is still not a verb**, and the closed set is
    /// unchanged by this build recognising it. Same assertion as `schema`'s
    /// above, and it is duplicated rather than generalised because each word is
    /// its own decision to have made.
    #[test]
    fn measure_is_not_a_verb_and_the_closed_set_is_the_same_nine_words() {
        assert!(Verb::parse(MEASURE).is_err());
        assert!(!Verb::ALL.iter().any(|v| v.as_str() == MEASURE));
        assert_eq!(Verb::ALL.len(), 9, "ADR-006 §6.4's set has nine verbs");
    }
}
