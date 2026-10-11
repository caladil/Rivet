//! `rivet get catalogue` — **what this machine can be asked, asked over the
//! wire.**
//!
//! Ruled by the maintainer on 2026-08-30, `.rulings/` R-118, and recorded in
//! `ADR-006` §8.13. The finding that produced it is one sentence long, and it is
//! the machine's own:
//!
//! > `rivet --version` on the machine publishes the catalogue, which is
//! > generated from the types and is the authority for what exists.
//!
//! **There is no shell on the machine to run that in.** `ADR-035` makes the
//! control plane the way in, and its vocabulary is `ADR-006` §6.4's closed verb set, so
//! `-- --version` is refused before anything is sent. The second cold-agent run
//! discovered the subjects by trying words, and wrote down that it had:
//! *"the refusal did name the three live verbs, which saved me — but I
//! discovered the subjects by trying words."*
//!
//! # What was already visible one layer down, and what was not
//!
//! **Checked before this was built rather than assumed**, because the same
//! report qualified its own finding — *"the catalogue might be perfectly visible
//! one layer down"*. It half was. `cmd/rivet`'s MCP adapter generates one tool
//! per catalogue entry, so `tools/list` publishes each **verb**, its summary,
//! its input names, whether each is required, and each input's description —
//! and the `subject` input's description enumerates every subject, kept true by
//! `the_operations_own_description_lists_every_subject_it_accepts`.
//!
//! So a client that speaks MCP directly can already learn the verbs and the
//! subjects. **Three things it cannot learn there**, and they are what this
//! subject exists for:
//!
//! 1. **What an answer will contain.** [`super::Results`] is per-subject and
//!    `tools/list` has no place to put it, so the field names and their meanings
//!    — the largest part of `rivet --version` — are published nowhere a caller
//!    can reach.
//! 2. **How an operation refuses.** [`super::ErrorCode`] is part of the
//!    interface contract (`ADR-006` §4.1) and appears in no tool definition.
//! 3. **Which verbs exist and are not implemented.** `tools/list` shows the
//!    three entries in [`super::CATALOGUE`]; it cannot show that the vocabulary
//!    is nine words and that six of them answer
//!    [`super::ErrorCode::Unimplemented`]. A caller reading only the tool list
//!    concludes the vocabulary is three, which is a claim of absence the
//!    catalogue does not make.
//!
//! And the client the operator actually holds never asks: `cargo rivet run`
//! sends `initialize` and then `tools/call`, and there is no path through it
//! that prints a tool list. So on the transport that exists, the catalogue was
//! unreachable — which is the finding, narrowed to what survives checking.
//!
//! # This subject reads nothing, and therefore takes no port
//!
//! `PLAN.md` §102.3 states the recipe for a `get` subject as *"a method on
//! `Observed`, an implementation at every site (the fakes refuse), an adapter
//! under `cmd/rivet/`, the subject and its tests, and four edits in `get.rs`"*.
//! **The first three do not apply here, deliberately, and the reason is
//! `ADR-011`.**
//!
//! [`super::schema`] goes through a port even though its answer is constant per
//! build, and its own documentation says why: `ADR-014` §3.2 makes
//! `rivet-config` depend on `rivet-core` permanently, so `Document` and its
//! schema are **not types this crate may name**. The adapter has both crates and
//! hands the value over.
//!
//! No such constraint exists here. [`super::CATALOGUE`] is a `const` in *this*
//! crate, and [`super::catalogue`] already renders it. Routing it through
//! [`super::machine::Observed`] would create a port with exactly one truthful
//! implementation and an unbounded number of untruthful ones — an adapter could
//! publish a catalogue that differs from the one [`super::perform`] dispatches
//! on, which is `ADR-006` §7's *"trusted and wrong"* in the one place it would
//! do the most damage. **A machine's account of what it can do must come from
//! the thing that decides what it can do.**
//!
//! The fleet argument that carried `schema` onto a port does not transfer
//! either: two boards running different builds already answer differently here,
//! because each board's answer is generated from its own build's
//! [`super::CATALOGUE`]. It is a per-machine fact *because* it is a per-build
//! constant, not in spite of it.
//!
//! # Three depths, because the whole thing is 112 kilobytes
//!
//! Measured, on 2026-08-30: `rivet --version` on this build is 112 362 bytes.
//! That is a fine thing to print into a terminal and a poor thing to hand an
//! agent that asked *what can I do here*, because for the caller this surface is
//! designed for it is spent as context (`ADR-012`, and [`super::log`]'s
//! `LIMIT_DEFAULT` is the same argument about the same cost).
//!
//! | Command | Answers | Order of size |
//! | :-- | :-- | :-- |
//! | `get catalogue` | the nine verbs, which are implemented, and every subject of each | kilobytes |
//! | `get catalogue <verb>` | that verb's whole entry — inputs, per-subject outputs, errors | tens of kilobytes |
//! | `get catalogue <verb> <subject>` | just that subject's result fields | hundreds of bytes |
//!
//! **The second and third positions are `get`'s existing polymorphic
//! positionals**, which `ADR-006` §8.5.1 already priced and which `file` and
//! `diff` already reinterpret. Nothing is declared for this subject that was not
//! already there, so the verb's arity is unchanged.

use super::{ErrorCode, Field, Operation, OperationError, Verb};
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "catalogue";

/// **How a Rivet command is formed**, said once so that an answer to *what can
/// I do here* is not a list a reader still has to guess the grammar of.
///
/// It is a constant rather than an inline literal because
/// [`the_shape_sentence_names_this_subjects_own_drill_down`] asserts against it:
/// a sentence describing a drill-down that stopped working would be the exact
/// failure this whole subject exists to fix.
const SHAPE: &str = "A command is a VERB, then a SUBJECT, then that subject's words, bound to the \
                     operation's declared inputs BY POSITION and never by name: \
                     `rivet <verb> <subject> [word ...]`. The verb set is CLOSED (ADR-006 §6.4) \
                     and `verbs` below is all of it — a verb whose `implemented` is false is a \
                     word this build has no operation behind, which is E_UNIMPLEMENTED and not \
                     a gap in this answer. Ask `get catalogue <verb>` for one verb's whole \
                     entry: every input, what each one means, what the result carries for each \
                     subject, and every way it refuses. Ask `get catalogue <verb> <subject>` \
                     for just that subject's result fields. AN OPERATION WHOSE `confirms` IS \
                     TRUE CHANGES THIS MACHINE and does nothing without the literal last word \
                     `confirm` — without it, it answers with its plan (ADR-012 §7.1).";

/// What this subject's result carries.
///
/// **Three shapes, one declared order.** Each of the three depths emits a
/// subsequence of this list, which is [`super::get::in_declared_order`]'s
/// requirement and is asserted in the code path that builds each of them.
pub(super) const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `catalogue` in this build.",
    },
    Field {
        name: "shape",
        description: "How to turn anything below into a command a machine will accept. Present \
                      at every depth, because a caller who drilled straight to one verb still \
                      needs the grammar.",
    },
    Field {
        name: "verbs",
        description: "ADR-006 §6.4's CLOSED vocabulary, in its order, ALL NINE of them — each \
                      with `implemented`, and for an implemented one its `summary`, whether it \
                      `confirms`, the `subjects` it accepts and the `errors` it can answer \
                      with. A verb that is not implemented carries `implemented: false` and \
                      nothing else, because there is nothing else true to say about it. \
                      Present only when no verb was named.",
    },
    Field {
        name: "verb",
        description: "The verb that was asked about, as given. Present only when one was named.",
    },
    Field {
        name: "verb-subject",
        description: "The subject of that verb that was asked about, as given. Present only \
                      when one was named.",
    },
    Field {
        name: "operation",
        description: "One verb's WHOLE catalogue entry — the same value `rivet --version` \
                      publishes for it, generated from the types rather than written down: \
                      `verb`, `summary`, `inputs` with each one's description, `outputs` (the \
                      generated union), `subjects` with each one's own result fields, and \
                      `errors`. Present only when a verb was named and no subject was.",
    },
    Field {
        name: "outputs",
        description: "The fields ONE subject's result carries, and what each means. This is \
                      the cheapest useful answer on this surface and the one to ask for before \
                      parsing anything. Present only when a verb and a subject were both named.",
    },
];

/// Perform `get catalogue`.
///
/// **It takes no machine at all**, which is why its signature differs from every
/// other subject's — see this module's documentation for the argument. It cannot
/// fail for any reason that is not the caller's, and every refusal names the
/// word it is about.
///
/// # Errors
///
/// [`ErrorCode::Arguments`] for a verb this vocabulary does not contain, a verb
/// this build has no operation behind, a subject that verb does not accept, or a
/// fourth word.
pub(super) fn perform(arguments: &[&str]) -> Result<Value, OperationError> {
    if let Some(extra) = arguments.get(3).copied() {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            extra,
            format!(
                "`get {SUBJECT}` takes at most a verb and one of that verb's subjects, and \
                 {extra:?} is a fourth word. NOTHING WAS READ. It is refused rather than \
                 ignored, because a word that does nothing is a word whose author believed it \
                 did something (ADR-006 §6.4)."
            ),
        ));
    }

    let Some(verb_word) = arguments.get(1).copied() else {
        return Ok(index());
    };
    let operation = named(verb_word)?;

    let Some(subject_word) = arguments.get(2).copied() else {
        let fields = vec![
            ("subject", Value::str(SUBJECT)),
            ("shape", Value::str(SHAPE)),
            ("verb", Value::str(verb_word)),
            ("operation", operation.value()),
        ];
        debug_assert!(
            super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
            "a catalogue result's keys must be a subsequence of OUTPUTS, in the declared order"
        );
        return Ok(Value::Object(fields));
    };

    let subjects = operation.subjects();
    let Some(found) = subjects.iter().find(|s| s.name == subject_word) else {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            subject_word,
            format!(
                "`{verb_word}` has no subject {subject_word:?}. NOTHING WAS READ. {}",
                accepted(&operation.verb, &subjects),
            ),
        ));
    };

    let fields = vec![
        ("subject", Value::str(SUBJECT)),
        ("shape", Value::str(SHAPE)),
        ("verb", Value::str(verb_word)),
        ("verb-subject", Value::str(subject_word)),
        (
            "outputs",
            Value::List(
                found
                    .outputs
                    .iter()
                    .map(|f| {
                        Value::object(vec![
                            ("name", Value::str(f.name)),
                            ("description", Value::str(f.description)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ];
    debug_assert!(
        super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
        "a catalogue result's keys must be a subsequence of OUTPUTS, in the declared order"
    );
    Ok(Value::Object(fields))
}

/// What a verb accepts, as a clause a refusal can end with.
///
/// **One sentence, two callers**, so that the two refusals a caller can trip
/// over on the way to a subject name say the same thing in the same words —
/// `ADR-011` in the small, and [`super::get`]'s `unknown` is the same shape one
/// level up.
fn accepted(verb: &Verb, subjects: &[super::Subject]) -> String {
    if subjects.is_empty() {
        return format!(
            "`{verb}`'s result shape does not depend on a subject, so it has none to name — ask \
             `get catalogue {verb}` for its whole entry."
        );
    }
    format!(
        "It accepts {}.",
        subjects
            .iter()
            .map(|s| format!("`{}`", s.name))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// The operation for a word, or the refusal that says which kind of nothing it
/// is.
///
/// **Two refusals rather than one**, because the two are different facts about
/// this machine and collapsing them would lose the more useful one: `frobnicate`
/// is not a Rivet word at all, and `export` is a Rivet word this build has
/// nothing behind. An agent can act on the second — it is in the vocabulary and
/// may arrive in a later build — and can only stop on the first.
fn named(word: &str) -> Result<&'static Operation, OperationError> {
    let verb = Verb::ALL
        .iter()
        .copied()
        .find(|v| v.as_str() == word)
        .ok_or_else(|| {
            OperationError::about(
                ErrorCode::Arguments,
                word,
                format!(
                    "{word:?} is not a Rivet verb. NOTHING WAS READ. The vocabulary is CLOSED \
                     (ADR-006 §6.4) and is: {}. Ask `get {SUBJECT}` with no verb for which of \
                     them this build implements.",
                    Verb::ALL
                        .iter()
                        .map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
        })?;
    super::operation(verb).ok_or_else(|| {
        OperationError::about(
            ErrorCode::Arguments,
            word,
            format!(
                "`{word}` IS a Rivet verb (ADR-006 §6.4) and this build has no operation behind \
                 it, so there is no entry to answer with and NOTHING WAS READ. Asking the \
                 machine to perform it answers E_UNIMPLEMENTED, which is not the same as the \
                 word being wrong. Ask `get {SUBJECT}` with no verb for the ones that are \
                 implemented."
            ),
        )
    })
}

/// **The index** — every verb of the closed vocabulary, and what this build does
/// with each.
///
/// Generated from [`Verb::ALL`] and [`CATALOGUE`], both of which are generated
/// from the types. There is no list of verbs here and no place for one to drift
/// from, which is the property `get.rs`'s own header states as *"the copy nobody
/// maintains is the one that is wrong"*.
fn index() -> Value {
    let verbs = Verb::ALL
        .iter()
        .map(|verb| {
            let mut entry = vec![("verb", Value::str(verb.as_str()))];
            let Some(op) = super::operation(*verb) else {
                // **Absence carries no summary and no subjects**, rather than
                // empty ones: `crate::value` has no null, so an empty list here
                // would assert that this verb accepts no subjects, which is a
                // different claim from *this build has no operation behind it*.
                entry.push(("implemented", Value::Bool(false)));
                return Value::Object(entry);
            };
            entry.push(("implemented", Value::Bool(true)));
            entry.push(("summary", Value::str(op.summary)));
            entry.push(("confirms", Value::Bool(op.confirms())));
            let subjects = op.subjects();
            if !subjects.is_empty() {
                entry.push(("subjects", Value::strings(subjects.iter().map(|s| s.name))));
            }
            entry.push((
                "errors",
                Value::strings(op.errors.iter().map(|e| e.as_str())),
            ));
            Value::Object(entry)
        })
        .collect();

    let fields = vec![
        ("subject", Value::str(SUBJECT)),
        ("shape", Value::str(SHAPE)),
        ("verbs", Value::List(verbs)),
    ];
    debug_assert!(
        super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
        "a catalogue result's keys must be a subsequence of OUTPUTS, in the declared order"
    );
    Value::Object(fields)
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::operation::CATALOGUE;

    fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
        match v {
            Value::Object(fields) => fields
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v)
                .unwrap_or_else(|| panic!("no {key} in {v:?}")),
            other => panic!("not an object: {other:?}"),
        }
    }

    fn maybe<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
        match v {
            Value::Object(fields) => fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v),
            other => panic!("not an object: {other:?}"),
        }
    }

    fn list(v: &Value) -> &[Value] {
        match v {
            Value::List(items) => items,
            other => panic!("not a list: {other:?}"),
        }
    }

    fn text(v: &Value) -> &str {
        match v {
            Value::Str(s) => s,
            other => panic!("not a string: {other:?}"),
        }
    }

    /// **The index names ALL NINE verbs, not the three that are implemented.**
    ///
    /// This is the property the whole subject exists for: a caller reading
    /// `tools/list` sees three tools and has no way to learn that the vocabulary
    /// is nine words, six of which are recognised and unimplemented. Answering
    /// with only the implemented three would reproduce that defect here.
    #[test]
    fn the_index_names_every_verb_of_the_closed_vocabulary() {
        let got = perform(&["catalogue"]).expect("the index takes no arguments");
        let verbs = list(field(&got, "verbs"));
        assert_eq!(verbs.len(), Verb::ALL.len());
        for (entry, verb) in verbs.iter().zip(Verb::ALL) {
            assert_eq!(field(entry, "verb"), &Value::str(verb.as_str()));
        }
    }

    /// Implemented and not implemented are told apart, and derived from
    /// [`CATALOGUE`] rather than from a list written here.
    #[test]
    fn the_index_says_which_verbs_this_build_implements_and_derives_it() {
        let got = perform(&["catalogue"]).expect("the index takes no arguments");
        for entry in list(field(&got, "verbs")) {
            let word = text(field(entry, "verb"));
            let implemented = CATALOGUE.iter().any(|op| op.verb.as_str() == word);
            assert_eq!(
                field(entry, "implemented"),
                &Value::Bool(implemented),
                "{word}"
            );
            assert_eq!(
                maybe(entry, "summary").is_some(),
                implemented,
                "a summary belongs to an implemented verb and to no other: {word}"
            );
        }
    }

    /// **Every subject of every verb reaches the index**, taken off each
    /// operation's own `subjects()` rather than off a list. The tripwire for
    /// a subject added to a verb and not visible from here.
    #[test]
    fn the_index_carries_every_subject_of_every_implemented_verb() {
        let got = perform(&["catalogue"]).expect("the index takes no arguments");
        let entries = list(field(&got, "verbs"));
        for op in CATALOGUE {
            let entry = entries
                .iter()
                .find(|e| text(field(e, "verb")) == op.verb.as_str())
                .unwrap_or_else(|| panic!("{} is in CATALOGUE and not in the index", op.verb));
            let listed: Vec<&str> = match maybe(entry, "subjects") {
                Some(v) => list(v).iter().map(text).collect(),
                None => Vec::new(),
            };
            let declared: Vec<&str> = op.subjects().iter().map(|s| s.name).collect();
            assert_eq!(listed, declared, "{}", op.verb);
        }
    }

    /// **`confirms` is derived from the declared input**, so a verb that grows
    /// a confirmation cannot be described here as one that does not.
    #[test]
    fn confirms_is_true_for_exactly_the_verbs_that_declare_a_confirmation() {
        let got = perform(&["catalogue"]).expect("the index takes no arguments");
        for entry in list(field(&got, "verbs")) {
            let word = text(field(entry, "verb"));
            let Some(op) = CATALOGUE.iter().find(|op| op.verb.as_str() == word) else {
                continue;
            };
            assert_eq!(field(entry, "confirms"), &Value::Bool(op.confirms()), "{word}");
        }
        // And it is not vacuous in either direction on this build.
        let apply = list(field(&got, "verbs"))
            .iter()
            .find(|e| text(field(e, "verb")) == "apply")
            .expect("apply is implemented");
        assert_eq!(field(apply, "confirms"), &Value::Bool(true));
        let get = list(field(&got, "verbs"))
            .iter()
            .find(|e| text(field(e, "verb")) == "get")
            .expect("get is implemented");
        assert_eq!(field(get, "confirms"), &Value::Bool(false));
    }

    /// Naming a verb answers with that verb's whole entry — the same value
    /// `rivet --version` publishes for it, and not a second rendering of it.
    #[test]
    fn a_verb_answers_with_that_verbs_own_catalogue_entry() {
        let got = perform(&["catalogue", "get"]).expect("`get` is implemented");
        assert_eq!(field(&got, "verb"), &Value::str("get"));
        let entry = field(&got, "operation");
        let expected = super::super::operation(Verb::Get)
            .expect("get is implemented")
            .value();
        assert_eq!(entry, &expected);
        assert!(maybe(&got, "verbs").is_none(), "the index is not repeated");
    }

    /// Naming a verb and a subject answers with that subject's fields alone —
    /// the cheapest useful answer on this surface.
    #[test]
    fn a_verb_and_a_subject_answer_with_that_subjects_result_fields() {
        let got = perform(&["catalogue", "get", "storage"]).expect("`get storage` exists");
        assert_eq!(field(&got, "verb"), &Value::str("get"));
        assert_eq!(field(&got, "verb-subject"), &Value::str("storage"));
        let names: Vec<&str> = list(field(&got, "outputs"))
            .iter()
            .map(|f| text(field(f, "name")))
            .collect();
        let declared: Vec<&str> = super::super::storage::OUTPUTS.iter().map(|f| f.name).collect();
        assert_eq!(names, declared);
        assert!(
            maybe(&got, "operation").is_none(),
            "the whole entry is not repeated beside one subject's fields"
        );
    }

    /// **This subject can describe itself**, which is the check that it is in
    /// the table rather than only in the dispatcher.
    #[test]
    fn the_catalogue_can_be_asked_about_the_catalogue() {
        let got = perform(&["catalogue", "get", SUBJECT]).expect("catalogue is a subject of get");
        let names: Vec<&str> = list(field(&got, "outputs"))
            .iter()
            .map(|f| text(field(f, "name")))
            .collect();
        let declared: Vec<&str> = OUTPUTS.iter().map(|f| f.name).collect();
        assert_eq!(names, declared);
    }

    /// **The grammar sentence names the drill-down it describes.** A shape
    /// sentence that described a command this subject does not answer would be
    /// the defect this subject was built to fix, committed inside the fix.
    #[test]
    fn the_shape_sentence_names_this_subjects_own_drill_down() {
        assert!(SHAPE.contains("get catalogue <verb>"), "{SHAPE}");
        assert!(SHAPE.contains("get catalogue <verb> <subject>"), "{SHAPE}");
        // And both forms answer.
        perform(&["catalogue", "apply"]).expect("`get catalogue apply` must answer");
        perform(&["catalogue", "apply", "restart"]).expect("`get catalogue apply restart`");
    }

    /// A word that is not in the vocabulary and a word that is but has no
    /// operation are **different refusals**, because they are different facts
    /// about this machine.
    #[test]
    fn an_unknown_word_and_an_unimplemented_verb_refuse_differently() {
        let not_a_verb = perform(&["catalogue", "frobnicate"]).expect_err("not a verb");
        assert!(not_a_verb.to_string().contains("is not a Rivet verb"), "{not_a_verb}");

        let unimplemented = perform(&["catalogue", "export"]).expect_err("no operation behind it");
        let said = unimplemented.to_string();
        assert!(said.contains("IS a Rivet verb"), "{said}");
        assert!(said.contains("E_UNIMPLEMENTED"), "{said}");
    }

    /// Every refusal this subject can produce names the word it is about, cites
    /// the ADR that decides it, and says that nothing was read.
    #[test]
    fn every_refusal_names_itself_cites_something_and_says_nothing_was_read() {
        let refusals = [
            perform(&["catalogue", "frobnicate"]).expect_err("not a verb"),
            perform(&["catalogue", "export"]).expect_err("unimplemented"),
            perform(&["catalogue", "get", "nonesuch"]).expect_err("not a subject of get"),
            perform(&["catalogue", "get", "storage", "extra"]).expect_err("a fourth word"),
        ];
        for refused in refusals {
            let said = refused.to_string();
            let word = refused
                .subject()
                .expect("every refusal here is ABOUT a word the caller said");
            assert!(!word.is_empty(), "{said}");
            assert!(said.contains(word), "a refusal names its word: {said}");
            assert!(said.contains("ADR-0") || said.contains("E_"), "{said} cites nothing");
            assert!(
                said.contains("NOTHING WAS READ"),
                "a refusal must say what was not done: {said}"
            );
        }
    }

    /// A subject of the *wrong* verb is refused and the refusal lists the right
    /// ones — `get`'s subjects are not `apply`'s.
    #[test]
    fn a_subject_of_another_verb_is_refused_with_the_list_of_this_verbs_own() {
        let refused = perform(&["catalogue", "get", "restart"]).expect_err("apply's, not get's");
        let said = refused.to_string();
        assert!(said.contains("`storage`"), "{said}");
        assert!(said.contains("restart"), "{said}");
    }

    /// `explain`'s result shape does not depend on a subject, so it has none —
    /// and the refusal says that rather than printing an empty list.
    #[test]
    fn a_verb_with_no_subjects_says_so_rather_than_listing_none() {
        let refused = perform(&["catalogue", "explain", "anything"]).expect_err("no subjects");
        let said = refused.to_string();
        assert!(said.contains("does not depend on a subject"), "{said}");
    }
}
