//! **The command line, generated from the operation table.**
//!
//! `ADR-034` §11.1 rules the shape and the maintainer's reason is that `rivet`
//! and `rivet-fleet` share one vocabulary while the machinery under each
//! differs (.rulings/ R-055).
//!
//! ```text
//! rivet-fleet get device --device kitchen-pi
//! rivet-fleet add device --name kitchen-pi --role proxy
//! rivet-fleet list devices
//! ```
//!
//! # Nothing here names an operation
//!
//! Every subcommand below is built by walking [`crate::operation::OPERATIONS`].
//! There is no `match` on a verb string and no list of flags — a row added to
//! that table appears here, and an operation this file could offer that the
//! table does not have cannot be written, because there is nowhere to write it.
//! That is `ADR-034` §10.1's *"one binary cannot offer the human an operation
//! the command line does not have"* made structural rather than aspirational,
//! which one binary alone does not achieve.
//!
//! # Ignition owns the standard surface
//!
//! `--help`, `--version` and `update` are `ignition::cli`'s and are not spelled
//! here. `ADR-034` §10.3 makes this an Ignition application and that is what an
//! Ignition application is: the consumer owns *what* it does and Ignition owns
//! *how it presents itself*. This module extends Ignition's `clap::Command`
//! rather than building its own.

use clap::{Arg, ArgAction, ArgMatches, Command};

use rivet_core::operation::Verb;

use crate::operation::{CONFIRMATION, OPERATIONS, Operation};

/// What the operator asked for, once `clap` has agreed it is well formed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The operation, resolved out of the table. Never a string pair — a
    /// caller holding this cannot name an operation that does not exist.
    pub operation: &'static Operation,
    /// The values given, by flag name, in the order the operation declares
    /// them. A flag not given is absent rather than empty.
    pub inputs: Vec<(&'static str, Vec<String>)>,
    /// Whether the operator typed the last word.
    pub confirmed: bool,
}

impl Request {
    /// The first value given for `flag`, or `None`.
    #[must_use]
    pub fn value(&self, flag: &str) -> Option<&str> {
        self.inputs
            .iter()
            .find(|(name, _)| *name == flag)
            .and_then(|(_, values)| values.first())
            .map(String::as_str)
    }
}

/// The `clap` command tree for this tool's own operations.
///
/// The consumer half of `ignition::cli`: Ignition builds the standard surface
/// and this adds the verbs to it.
#[must_use]
pub fn extend(mut command: Command) -> Command {
    for verb in Verb::ALL {
        let subjects: Vec<&'static Operation> =
            OPERATIONS.iter().filter(|op| op.verb == *verb).collect();
        if subjects.is_empty() {
            // **A verb with no operations is absent, not empty.** `ADR-006`
            // §6.4's vocabulary is nine words for the whole project; a fleet
            // tool that showed all nine with six of them doing nothing would be
            // advertising a surface it does not have.
            continue;
        }
        let mut verb_command = Command::new(verb.as_str())
            .about(summary_for(*verb))
            .subcommand_required(true)
            .arg_required_else_help(true);
        for op in subjects {
            verb_command = verb_command.subcommand(subject_command(op));
        }
        command = command.subcommand(verb_command);
    }
    command
        .arg(
            Arg::new(IDENTITY)
                .long(IDENTITY)
                .global(true)
                .action(ArgAction::Set)
                .help(
                    "This OPERATOR's own control-plane identity — a directory \
                     `cargo rivet identity new` wrote. Without it this tool can reach no \
                     machine over ADR-035's control plane.",
                ),
        )
        .arg(
            Arg::new(PIN)
                .long(PIN)
                .global(true)
                .action(ArgAction::Set)
                .help(
                    "The machine's public key, 64 hex characters. A device record that \
                     carries one is used without this; `set pin` records one and `add \
                     identity` records the key it mints. This OVERRIDES the record, for a \
                     machine whose key has been replaced and whose record is not caught up \
                     yet. A device with neither is a refusal, not a connection without \
                     pinning.",
                ),
        )
}

/// **`--identity`, and `rivet-fleet`'s half of `ADR-034` §11.5.1.**
///
/// The spelling is `cargo rivet run`'s and `cargo rivet deploy`'s, and the
/// reading is literally theirs — `rivet_remote::endpoint`. Two tools that each
/// parsed `--pin` their own way would agree today and diverge the first time
/// one grew a check.
pub const IDENTITY: &str = "identity";

/// **`--pin`**, which this tool needs least of the three and still takes.
///
/// `ADR-034` §3.5 has the register hold the PUBLIC half of a device's key, so
/// `rivet-fleet` already knows the pin for every device it enrolled and this
/// flag is an override rather than the source. `crate::operator` is where that
/// preference is expressed.
pub const PIN: &str = "pin";

/// A global flag, wherever `clap` put it.
///
/// **`global(true)` means the value is visible on the root matches AND on every
/// subcommand's**, so this reads the root's and falls back through the two
/// levels of subcommand this tool has. Written once here rather than at each
/// call site, because a caller that read only the root would silently see
/// nothing for `rivet-fleet apply document --identity …`.
#[must_use]
pub fn global(matches: &ArgMatches, flag: &str) -> Option<String> {
    if let Some(value) = matches.get_one::<String>(flag) {
        return Some(value.clone());
    }
    let (_, verb) = matches.subcommand()?;
    if let Some(value) = verb.get_one::<String>(flag) {
        return Some(value.clone());
    }
    let (_, subject) = verb.subcommand()?;
    subject.get_one::<String>(flag).cloned()
}

/// One `<verb> <subject>` pair, with its flags.
fn subject_command(op: &'static Operation) -> Command {
    let mut command = Command::new(op.subject).about(op.summary);
    for input in op.inputs {
        command = command.arg(
            Arg::new(input.name)
                .long(input.name)
                .required(input.required)
                .action(if input.several {
                    ArgAction::Append
                } else {
                    ArgAction::Set
                })
                .help(input.description),
        );
    }
    if op.confirms {
        command = command.arg(
            Arg::new(CONFIRMATION)
                .long(CONFIRMATION)
                .action(ArgAction::SetTrue)
                // The wording is `rivet`'s, because the mechanism is:
                // `ADR-012` §7.1. Without it the operation prints its plan.
                .help("Actually do it. Without this, the plan is printed and nothing changes."),
        );
    }
    command
}

/// What a verb means, in one line, for `--help`'s list.
///
/// These are `ADR-006` §6.4's own glosses. They are here rather than on
/// `rivet_core::operation::Verb` because a verb's meaning is the same in both
/// tools but the *subjects* differ, and a shared gloss that had to cover both
/// would say less than either needs.
const fn summary_for(verb: Verb) -> &'static str {
    match verb {
        Verb::Get => "Read one thing out of the register.",
        Verb::Set => "Change one value on a record.",
        Verb::List => "Everything of a kind.",
        Verb::Add => "Put something into the register.",
        Verb::Remove => "Take something out of the register.",
        Verb::Apply => "Do something to a machine.",
        Verb::Plan => "What an operation would do, without doing it.",
        Verb::Export => "Produce a document.",
        Verb::Explain => "Where a value came from.",
    }
}

/// Read a [`Request`] out of `clap`'s answer, or `None` when the operator asked
/// for nothing this module owns.
///
/// `None` is not a failure: bare `rivet-fleet` opens the terminal interface
/// (`ADR-034` §10.1), and Ignition's own subcommands are handled before this is
/// reached.
#[must_use]
pub fn request(matches: &ArgMatches) -> Option<Request> {
    let (verb_word, verb_matches) = matches.subcommand()?;
    let verb = Verb::ALL.iter().copied().find(|v| v.as_str() == verb_word)?;
    let (subject, subject_matches) = verb_matches.subcommand()?;
    let operation = crate::operation::find(verb, subject)?;

    let mut inputs = Vec::new();
    for input in operation.inputs {
        let values: Vec<String> = subject_matches
            .get_many::<String>(input.name)
            .map(|given| given.cloned().collect())
            .unwrap_or_default();
        if !values.is_empty() {
            inputs.push((input.name, values));
        }
    }
    let confirmed = operation.confirms && subject_matches.get_flag(CONFIRMATION);
    Some(Request {
        operation,
        inputs,
        confirmed,
    })
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn parse(words: &[&str]) -> Result<Option<Request>, clap::Error> {
        let command = extend(Command::new("rivet-fleet"));
        let mut argv = vec!["rivet-fleet"];
        argv.extend_from_slice(words);
        Ok(request(&command.try_get_matches_from(argv)?))
    }

    /// **The property the whole module exists for.** Every row of the table is
    /// reachable from the command line, and reachable by the words the table
    /// spells — so a surface cannot go missing without the table changing.
    #[test]
    fn every_operation_in_the_table_is_reachable_from_the_command_line() {
        for op in OPERATIONS {
            let mut words = vec![op.verb.as_str(), op.subject];
            for input in op.inputs {
                if input.required {
                    words.push(Box::leak(format!("--{}", input.name).into_boxed_str()));
                    words.push("something");
                }
            }
            let request = parse(&words)
                .unwrap_or_else(|e| panic!("{} {} did not parse: {e}", op.verb.as_str(), op.subject))
                .unwrap_or_else(|| {
                    panic!("{} {} parsed to no request", op.verb.as_str(), op.subject)
                });
            assert_eq!(request.operation, op);
        }
    }

    /// And nothing else is. An operation the table does not carry is a `clap`
    /// error rather than a request, which is what stops the two surfaces
    /// drifting in the direction the table cannot see.
    #[test]
    fn a_subject_the_table_does_not_carry_is_refused() {
        assert!(parse(&["get", "frobnicator"]).is_err());
        assert!(parse(&["set", "device", "--device", "x"]).is_err());
        assert!(parse(&["frobnicate", "device"]).is_err());
    }

    /// **One deployment surface** — `PLAN-Feature-Set-2026-09-12` §3, ruled
    /// 2026-09-12. Software reaches a machine through `cargo rivet deploy` and
    /// nothing in this tool; two surfaces is how the repository bypass survived.
    /// So `apply deploy` is refused by `clap`, is absent from the table, and
    /// `apply --help` does not offer it.
    #[test]
    fn apply_deploy_is_not_a_command_this_tool_offers() {
        assert!(parse(&["apply", "deploy", "--device", "x", "--rvt", "y.rvt"]).is_err());
        assert_eq!(crate::operation::find(Verb::Apply, "deploy"), None);

        let command = extend(Command::new("rivet-fleet"));
        let apply = command
            .get_subcommands()
            .find(|c| c.get_name() == "apply")
            .expect("apply still has operations");
        let subjects: Vec<&str> = apply.get_subcommands().map(Command::get_name).collect();
        assert!(!subjects.contains(&"deploy"), "{subjects:?}");
        let help = apply.clone().render_help().to_string();
        assert!(!help.contains("deploy"), "{help}");
    }

    /// A required flag is required, and the refusal is `clap`'s rather than a
    /// silent default.
    #[test]
    fn an_operation_missing_a_required_input_is_refused_rather_than_defaulted() {
        let error = parse(&["get", "device"]).expect_err("--device is required");
        let text = error.to_string();
        assert!(text.contains("--device"), "{text}");
    }

    /// Values arrive under the names the table declares.
    #[test]
    fn the_values_given_arrive_under_the_names_the_table_declares() {
        let request = parse(&["add", "device", "--name", "kitchen-pi", "--role", "proxy"])
            .expect("well formed")
            .expect("a request");
        assert_eq!(request.value("name"), Some("kitchen-pi"));
        assert_eq!(request.value("role"), Some("proxy"));
        assert_eq!(request.value("location"), None);
    }

    /// **`ADR-012` §7.1 in a host tool.** An operation that changes something
    /// is unconfirmed until the operator says so, and one that does not is
    /// never asked.
    #[test]
    fn a_confirming_operation_is_unconfirmed_until_the_word_is_given() {
        let plan = parse(&["remove", "device", "--device", "kitchen-pi"])
            .expect("well formed")
            .expect("a request");
        assert!(!plan.confirmed);

        let done = parse(&["remove", "device", "--device", "kitchen-pi", "--confirm"])
            .expect("well formed")
            .expect("a request");
        assert!(done.confirmed);
    }

    /// And an operation that changes nothing has no confirmation to give — a
    /// flag offered where it means nothing is a surface an operator has to
    /// learn the exceptions to.
    #[test]
    fn an_operation_that_changes_nothing_does_not_offer_a_confirmation() {
        assert!(parse(&["get", "device", "--device", "x", "--confirm"]).is_err());
    }

    /// Bare `rivet-fleet` is not an error: `ADR-034` §10.1 gives it to the
    /// terminal interface.
    #[test]
    fn no_arguments_at_all_is_not_a_command_line_request() {
        assert_eq!(parse(&[]).expect("well formed"), None);
    }

    /// A verb with no operations is absent rather than shown empty. Today that
    /// is `set`, `apply` and `plan`; the assertion is about the rule, not about
    /// which verbs are on which side of it.
    #[test]
    fn a_verb_with_no_operations_is_not_advertised() {
        let command = extend(Command::new("rivet-fleet"));
        let offered: Vec<&str> = command.get_subcommands().map(clap::Command::get_name).collect();
        for verb in Verb::ALL {
            let has_any = OPERATIONS.iter().any(|op| op.verb == *verb);
            assert_eq!(
                offered.contains(&verb.as_str()),
                has_any,
                "{} is offered without having any operations, or the reverse",
                verb.as_str()
            );
        }
    }

    /// Every flag `--help` shows carries the sentence the table wrote, so the
    /// two cannot describe one input differently.
    #[test]
    fn the_help_text_is_the_tables_own_words() {
        let command = extend(Command::new("rivet-fleet"));
        let help = command
            .get_subcommands()
            .find(|c| c.get_name() == "add")
            .and_then(|c| c.get_subcommands().find(|c| c.get_name() == "device"))
            .expect("add device")
            .clone()
            .render_help()
            .to_string();
        assert!(help.contains("the human label"), "{help}");
    }
}
