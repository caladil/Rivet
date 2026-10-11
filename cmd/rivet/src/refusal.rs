//! Every way this binary can decline, as a typed, coded value.
//!
//! `ADR-006` §4.1, and the order of authority is the order the fields are
//! written in:
//!
//! > Every failure carries a **stable machine-readable code**, a **structured
//! > payload**, and a **human string** — in that order of authority. An agent
//! > branches on the code; a human reads the string.
//!
//! **Error codes are part of the interface contract and are not renumbered**
//! (`ADR-006` §4.1). They are strings rather than integers precisely so that
//! there is no numbering to renumber, and
//! `the_codes_are_the_contract_and_this_is_the_list` is the test that makes
//! changing one a deliberate act.
//!
//! # Two owners, and no code with two definitions — amended 2026-08-23
//!
//! This module used to declare all five codes. Four of them were an
//! *operation's* failures rather than this binary's, and they moved to
//! [`rivet_core::ErrorCode`] with the first operation: an operation is the
//! core's (`ADR-006` §3), so what an operation may answer is the core's too, and
//! a second adapter — the network API `ADR-006` §5.1 names as a destination —
//! would otherwise have had to copy them.
//!
//! **What stayed is what only this binary can produce**: the login-shell
//! contract. `E_NOT_A_SHELL` and `E_NO_INTERACTIVE_SHELL`
//! describe a login session, and putting them in `rivet-core` would be the
//! `core → anything CLI-shaped` edge `ADR-014` §3 forbids.
//!
//! **No code moved, was renamed, or changed its exit status.** What changed is
//! where each is *defined*, and the property `ADR-011` actually asks for —
//! exactly one authority per fact — is asserted rather than asserted-about:
//! `no_error_code_is_defined_in_two_places` fails if the two sets ever overlap.
//! [`Code::all`] is the union, generated from both definitions, and
//! `rivet --version` publishes it.
//!
//! ## The exit codes, and why two of them are borrowed
//!
//! | | Means |
//! | :-- | :-- |
//! | 0 | the command produced a value |
//! | 1 | the operation refused — `E_UNIMPLEMENTED`, `E_UNKNOWN_SUBJECT` |
//! | 2 | the arguments were not usable — `E_ARGUMENTS`, `E_UNKNOWN_VERB` |
//! | **126** | an interactive session was asked for, and this image has none |
//! | **127** | `-c` named a program; `rivet` cannot execute programs |
//!
//! 126 and 127 are **the shell's conventions**, not invented ones — 127 is
//! "command not found" and 126 is "found and could not be executed" — and that
//! matters here for one specific reason: whatever execs the login shell returns its exit
//! status to the remote client, so these are the numbers a file-copy or login user
//! actually sees. Reporting 1 for `rivet -c /usr/lib/sftp-server` would tell
//! an `scp` client that the transfer failed; 127 tells it the remote could not
//! find the program, which is exactly what happened ("File
//! transfer costs a binary Rivet did not plan for").

use rivet_core::operation::plan::Plan;
use rivet_core::{ErrorCode, OperationError};

use crate::value::Value;

/// The stable machine-readable half. `ADR-006` §4.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    /// A code an **operation** answers with, defined in `rivet-core`.
    Operation(ErrorCode),
    /// Asked to behave like a shell — run a program, interpret an operator,
    /// split a quoted word. `rivet` is not one and does not approximate one.
    NotAShell,
    /// Exec'd as an interactive login shell. `ADR-005` §5.1 rules `nushell`
    /// the interactive login shell and the image does not contain it.
    NoInteractiveShell,
}

impl Code {
    /// This binary's own codes — the login-shell contract's, and nothing else.
    ///
    /// Kept separate from [`Self::all`] so that the disjointness test has two
    /// sets to compare rather than one list to trust.
    pub const ADAPTER: &'static [Code] = &[Code::NotAShell, Code::NoInteractiveShell];

    /// `argv` was not usable. ~~The one operation code this crate still
    /// *constructs*~~, because `--output yaml` is an argument error before any
    /// operation has been chosen.
    ///
    /// **No longer the only one, since 2026-08-23**: [`crate::measure`] is a bare
    /// word rather than a catalogued operation (`ADR-006` §6.4's verb set is
    /// closed and extending it is the maintainer's), so its refusals are
    /// constructed here in the adapter, from the same three codes an operation
    /// would have used. Struck rather than edited, per `AGENTS.md` §2.
    ///
    /// **No new code was invented for it, and that is the load-bearing part.**
    /// `ADR-006` §4.1 makes the codes the interface contract; a benchmark that
    /// answered `E_MEASUREMENT_FAILED` would have widened that contract for one
    /// command's convenience.
    pub const ARGUMENTS: Self = Self::Operation(ErrorCode::Arguments);

    /// A well-formed request naming something this command has nothing to say
    /// about — `measure throughput`, where the only subject is `hash`.
    pub const UNKNOWN_SUBJECT: Self = Self::Operation(ErrorCode::UnknownSubject);

    /// The machine could not be read, and said why. A device that will not open,
    /// a read that failed part way, a clock that is not there.
    pub const MACHINE_REFUSED: Self = Self::Operation(ErrorCode::MachineRefused);

    /// A well-formed Rivet command that this build does not implement.
    ///
    /// **`#[cfg(test)]`, and that is the interesting part.** Nothing in this
    /// binary constructs one any more: the refusal comes from
    /// `rivet_core::operation::perform`, which looks in the catalogue. A
    /// spelling kept here for a test to say what it means would otherwise read
    /// as an refusal this crate can still produce.
    #[cfg(test)]
    pub const UNIMPLEMENTED: Self = Self::Operation(ErrorCode::Unimplemented);
    /// A word that is not one of `ADR-006` §6.4's verbs at all. `#[cfg(test)]`
    /// for [`Self::UNIMPLEMENTED`]'s reason — `Verb::parse` produces it now.
    #[cfg(test)]
    pub const UNKNOWN_VERB: Self = Self::Operation(ErrorCode::UnknownVerb);

    /// The wire form. **Part of the interface contract** — never renumbered,
    /// never renamed (`ADR-006` §4.1).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Operation(code) => code.as_str(),
            Self::NotAShell => "E_NOT_A_SHELL",
            Self::NoInteractiveShell => "E_NO_INTERACTIVE_SHELL",
        }
    }

    /// What the process exits with. See the module documentation's table.
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::Operation(code) => code.exit_code(),
            Self::NoInteractiveShell => 126,
            Self::NotAShell => 127,
        }
    }

    /// **Every code this binary can produce**, generated from the two
    /// definitions rather than written out.
    ///
    /// `rivet --version` publishes this, because `ADR-006` §4.1 makes the
    /// codes part of the interface contract and a contract an agent has to
    /// discover by provoking each failure is not one it can read.
    pub fn all() -> Vec<Code> {
        ErrorCode::ALL
            .iter()
            .copied()
            .map(Code::Operation)
            .chain(Self::ADAPTER.iter().copied())
            .collect()
    }
}

/// One refusal: the code, the structured payload, and the human string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: Code,
    /// The offending value, whatever it was. Named `subject` rather than
    /// `detail` because `AGENTS.md` §5 wants what the *operator* said carried
    /// back verbatim rather than paraphrased.
    pub subject: Option<String>,
    /// **What the operation would have done**, for an `E_CONFIRMATION_REQUIRED`
    /// — `ADR-006` §6.5's typed plan, added 2026-08-26.
    ///
    /// This adapter never builds one; it carries the core's out unchanged, which
    /// is [`From<OperationError>`]'s rule applied to §4.1's second field rather
    /// than only its third. A refusal this binary constructs for itself has
    /// none, and there is no code it could belong to: the login-shell contract's
    /// two refusals are not changes to a machine.
    pub plan: Option<Plan>,
    /// For a human. Last in authority and first in usefulness.
    pub message: String,
}

impl Refusal {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            subject: None,
            plan: None,
            message: message.into(),
        }
    }

    pub fn about(code: Code, subject: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            subject: Some(subject.into()),
            plan: None,
            message: message.into(),
        }
    }

    /// The refusal as a [`Value`], so that the machine form and the human form
    /// are the same two renderers everything else uses (`ADR-006` §4). An error
    /// path with its own formatting is the second source §4 forbids.
    pub fn value(&self) -> Value {
        let mut fields = vec![("error", Value::str(self.code.as_str()))];
        if let Some(subject) = &self.subject {
            fields.push(("subject", Value::str(subject.clone())));
        }
        // Between the subject and the message, which is §4.1's order of
        // authority written into the object — an agent reads down and stops at
        // the first field it understands. Rendered by `rivet_core`'s own
        // `Plan::value`, so the CLI and a future network adapter cannot show two
        // shapes of one plan.
        if let Some(plan) = &self.plan {
            fields.push(("plan", plan.value()));
        }
        fields.push(("message", Value::str(self.message.clone())));
        Value::Object(fields)
    }

    pub const fn exit_code(&self) -> u8 {
        self.code.exit_code()
    }
}

/// An operation's refusal, carried out to the operator unchanged.
///
/// **Nothing is added, reworded or re-coded on the way through**, which is
/// `ADR-006` §3's "the CLI parses arguments and renders results" applied to the
/// failure path. A rendering that improved on the core's message would be a
/// second account of why the operation refused, and the network API would
/// eventually give a third.
impl From<OperationError> for Refusal {
    fn from(error: OperationError) -> Self {
        Self {
            code: Code::Operation(error.code()),
            subject: error.subject().map(str::to_owned),
            plan: error.plan().cloned(),
            message: error.message().to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value;

    /// **The contract, written out.** `ADR-006` §4.1 says error codes are part
    /// of the interface and are not renumbered; this is what makes changing one
    /// a decision rather than a rename refactor that compiles.
    ///
    /// The list is unchanged by the 2026-08-23 move of four codes into
    /// `rivet-core` — which is the point, because the codes are the *external*
    /// contract and where they are declared is not. `E_UNKNOWN_SUBJECT` is new,
    /// and it is new because an operation that can refuse a subject now exists.
    #[test]
    fn the_codes_are_the_contract_and_this_is_the_list() {
        let codes: Vec<_> = Code::all().iter().map(|c| c.as_str()).collect();
        assert_eq!(
            codes,
            [
                "E_UNIMPLEMENTED",
                "E_UNKNOWN_VERB",
                "E_ARGUMENTS",
                "E_UNKNOWN_SUBJECT",
                // Added 2026-08-23 in `rivet-core`, with the first operation
                // that changes a machine. They appear here because `Code::all()`
                // is generated from both definitions rather than written out —
                // which is the property this test exists to hold.
                "E_CONFIRMATION_REQUIRED",
                "E_UNSAFE",
                "E_MACHINE_REFUSED",
                "E_NOT_A_SHELL",
                "E_NO_INTERACTIVE_SHELL",
            ]
        );
    }

    /// **`ADR-011` §2, clause one, as a test rather than a paragraph.** The two
    /// vocabularies exist because an operation's failures and a login shell's
    /// are different facts; that is only defensible while no code has two
    /// definitions.
    #[test]
    fn no_error_code_is_defined_in_two_places() {
        let operation: Vec<&str> = ErrorCode::ALL.iter().map(|c| c.as_str()).collect();
        for adapter in Code::ADAPTER {
            assert!(
                !operation.contains(&adapter.as_str()),
                "{} is defined in rivet-core and in this crate",
                adapter.as_str()
            );
        }
        assert_eq!(Code::all().len(), operation.len() + Code::ADAPTER.len());
    }

    #[test]
    fn every_code_starts_with_e_underscore_and_is_distinct() {
        let mut seen: Vec<&str> = Vec::new();
        for code in Code::all() {
            let s = code.as_str();
            assert!(s.starts_with("E_"), "{s} does not look like a code");
            assert!(!seen.contains(&s), "{s} appears twice");
            seen.push(s);
        }
    }

    /// The two borrowed numbers, asserted as literals rather than against their
    /// own definitions. 127 is what an `scp` client needs to see when the
    /// program it asked for is not there — see the module documentation.
    #[test]
    fn the_shell_conventions_are_used_for_the_two_shell_shaped_refusals() {
        assert_eq!(Code::NotAShell.exit_code(), 127);
        assert_eq!(Code::NoInteractiveShell.exit_code(), 126);
    }

    #[test]
    fn no_refusal_exits_zero_because_a_refusal_is_not_a_result() {
        for code in Code::all() {
            assert_ne!(code.exit_code(), 0, "{} exits 0", code.as_str());
        }
    }

    /// An unimplemented operation and an unknown word must not be the same
    /// answer: one is "not yet", the other is "never". `ADR-006` §4.1's whole
    /// argument is that a failure distinguishable only by English prose cannot
    /// be handled programmatically.
    #[test]
    fn not_yet_and_never_are_different_codes() {
        assert_ne!(Code::UNIMPLEMENTED.as_str(), Code::UNKNOWN_VERB.as_str());
        assert_ne!(
            Code::UNIMPLEMENTED.exit_code(),
            Code::UNKNOWN_VERB.exit_code()
        );
    }

    #[test]
    fn a_refusal_carries_the_offending_value_verbatim() {
        let r = Refusal::about(Code::UNKNOWN_VERB, "frobnicate", "not a Rivet verb");
        let rendered = value::json(&r.value());
        assert!(
            rendered.contains(r#""error":"E_UNKNOWN_VERB""#),
            "{rendered}"
        );
        assert!(rendered.contains(r#""subject":"frobnicate""#), "{rendered}");
        assert!(rendered.contains("not a Rivet verb"), "{rendered}");
    }

    /// The subject may be anything the operator typed, including bytes that
    /// would break a naively-escaped document. The escaping is `value`'s and
    /// this asserts the refusal path actually goes through it.
    #[test]
    fn a_subject_full_of_quotes_does_not_break_the_machine_form() {
        let r = Refusal::about(Code::NotAShell, "echo \"hi\"\n", "not a shell");
        let rendered = value::json(&r.value());
        assert!(rendered.contains(r#"\"hi\""#), "{rendered}");
        assert!(!rendered.contains('\n'), "a raw newline reached the JSON");
    }

    #[test]
    fn a_refusal_with_no_subject_omits_the_field_rather_than_emitting_null() {
        let r = Refusal::new(Code::NoInteractiveShell, "no shell");
        let rendered = value::json(&r.value());
        assert!(!rendered.contains("subject"), "{rendered}");
        assert!(!rendered.contains("null"), "{rendered}");
    }

    /// **An operation's refusal arrives unchanged.** `ADR-006` §3 leaves the
    /// adapter no logic, and a message improved on the way out would be a
    /// second account of the same failure — the one a future API would then
    /// disagree with.
    #[test]
    fn an_operations_refusal_is_carried_out_without_being_reworded_or_re_coded() {
        let core = OperationError::about(ErrorCode::UnknownSubject, "/etc/motd", "no such view");
        let refusal = Refusal::from(core.clone());
        assert_eq!(refusal.code.as_str(), core.code().as_str());
        assert_eq!(refusal.subject.as_deref(), core.subject());
        assert_eq!(refusal.message, core.message());
        assert_eq!(refusal.exit_code(), core.exit_code());
        // And the two render the same structure, since both build it the same
        // way from the same three fields.
        assert_eq!(value::json(&refusal.value()), value::json(&core.value()));
    }
}
