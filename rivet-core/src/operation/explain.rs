//! `rivet explain <path>` — **Rivet's first administrative operation.**
//!
//! `ADR-007` §3.6.1 item 3, which `ADR-007` §3.6.2 moved from *proposed* to
//! *required* on 2026-08-22:
//!
//! > **A reverse lookup**, so the question can be asked before the mistake
//! > rather than after it: `rivet explain /etc/resolv.conf` returns the field
//! > it is generated from, that it is read-only, and the command to change it.
//! > Cheap, because the generator already knows what it produced from what.
//!
//! `crate::etc::render_passwd` recorded the same requirement from the other end
//! — the three colon-separated databases *cannot* carry a provenance header,
//! because musl's parsers are not known to skip a comment, so this lookup is the
//! only place their provenance can be read at all.
//!
//! # Why this one is first
//!
//! `ADR-006` §8.1 says the first operation "must land in `rivet-core`, not
//! here", and `ADR-014` §3's forbidden edges decide *which* one can. An
//! operation over the configuration document cannot be this one:
//! `rivet-config` depends on `rivet-core` and `ADR-014` §3.2 makes that
//! direction permanent, so `Document` is not a type this crate can name. An
//! operation over observed state cannot be this one either: the kernel is the
//! authority for that (`AGENTS.md` §5) and reaching it needs an adapter
//! `ADR-014` §2 lists and nobody has written.
//!
//! What is left is the class of fact **`rivet-core` is itself authoritative
//! for**, and `crate::etc::ETC_FILES` is the largest of them: six files, their
//! modes, what each is derived from, and what an operator changes to change one.
//! `ADR-011` §2's second clause is the whole point — *the system says which one
//! is authoritative, at the moment the answer is needed* — and the moment is an
//! operator staring at a file they are about to try to edit.
//!
//! # It reads no filesystem, and that is not a shortcut
//!
//! This operation answers from the **definition**, never by looking at the
//! machine. That is `ADR-014` §3.1's standing check holding — `cargo test -p
//! rivet-core` needs no filesystem — but it is also the right answer: the
//! question is *what generates this path*, which is a property of Rivet rather
//! than of any particular boot. A path can be explained on a machine that has
//! not booted, from a workstation, before the mistake.
//!
//! The consequence, said plainly rather than left to be discovered: **`explain`
//! does not tell you whether the file is there.** A machine whose `/etc`
//! projection failed still explains `/etc/passwd` exactly as one that succeeded
//! does. Whether a file was written is the supervisor's journal
//! (`ADR-022`), and conflating the two would make this operation a second
//! authority for a fact the running system owns.

use super::{ErrorCode, Field, Input, Operation, OperationError, Results, Verb};
use crate::etc::{ETC_FILES, ETC_FSTYPE, ETC_MOUNT_POINT, EtcDefinition, MUSL_ETC_PATHS};
use crate::generated::{GENERATED_PATHS, GeneratedDefinition, Kind};
use crate::value::Value;

/// The catalogue entry, declared beside the implementation so the two cannot
/// drift — `super`'s module documentation, property 1.
pub(super) const OPERATION: Operation = Operation {
    verb: Verb::Explain,
    summary: "Say what generates a path, whether it may be written, and what to change instead.",
    inputs: &[Input {
        name: "path",
        required: true,
        several: false,
        description: "An absolute path on a Rivet machine, e.g. /etc/passwd.",
    }],
    // One shape, whatever the path — `explain` has no subject, and
    // `Results::Same` is how a verb says so.
    results: Results::Same(&[
        Field {
            name: "path",
            description: "The path that was asked about.",
        },
        Field {
            name: "kind",
            description: "What relationship the operator has to the bytes. `view` for the \
                          /etc projection (ADR-007 §3.6.2); otherwise one of `projection`, \
                          `directory`, `machine state`, `record`, `mechanism` — \
                          rivet_core::generated::Kind, a CLOSED vocabulary.",
        },
        Field {
            name: "generated-by",
            description: "What renders this path onto a running machine.",
        },
        Field {
            name: "derived-from",
            description: "What the contents are computed from — the same words the file's own \
                          provenance header uses where its format can carry one.",
        },
        Field {
            name: "writable",
            description: "Whether a write means anything. Always false for a view — Rivet \
                          does not accept an input it intends to discard (ADR-007 §3.6.5). \
                          FALSE IS NOT A CLAIM ABOUT THE KERNEL: /state is writable and root \
                          is root, so a write to a false one is accepted and then either \
                          discarded at the next render or acted on as a request. `why` says \
                          which.",
        },
        Field {
            name: "filesystem",
            description: "What the path lives on, which is why a write fails — or, on /state, \
                          why it does not fail and still does not last.",
        },
        Field {
            name: "mode",
            description: "The permissions, octal, as every Unix tool prints them. For a path \
                          whose mode rivet-core does not own, a SENTENCE saying so rather \
                          than a number — an invented one would be a second authority.",
        },
        Field {
            name: "why",
            description: "Why Rivet generates this path at all, and — where it is not \
                          writable — what actually happens to a write.",
        },
        Field {
            name: "change-with",
            description: "What to change in order to change this file. Never the file itself.",
        },
    ]),
    errors: &[ErrorCode::UnknownSubject],
};

/// What generates a path, and what to change instead.
///
/// A typed result rather than a `Value`, so that a future adapter reading this
/// operation programmatically gets fields rather than a document it has to
/// index into. [`Explanation::value`] is the projection every adapter renders,
/// and it is the only place the field *names* are written down.
///
/// # Two variants, because there are two authorities
///
/// Added 2026-08-25, when this operation stopped knowing only `/etc`
/// ([`crate::generated`] carries the account and the defect that motivated it).
/// Each variant borrows the table that *owns* the fact rather than a copy of it,
/// which is `ADR-011` §2 held structurally: a definition added to either
/// becomes explainable with no change here.
#[derive(Debug, Clone, Copy)]
pub enum Explanation {
    /// A file of the runtime `/etc` projection — [`crate::etc::ETC_FILES`].
    Etc(&'static EtcDefinition),
    /// Everything else Rivet writes outside `/etc` —
    /// [`crate::generated::GENERATED_PATHS`].
    Generated(&'static GeneratedDefinition),
}

/// Two explanations are the same when they are about the same path.
///
/// Hand-written rather than derived because [`EtcDefinition`] deliberately is
/// not `PartialEq` — it holds a renderer, and comparing function pointers is
/// meaningless. The path is the identity of every one of these, in both
/// tables, so it is the right comparison for the enum as a whole.
impl PartialEq for Explanation {
    fn eq(&self, other: &Self) -> bool {
        self.path() == other.path()
    }
}

impl Eq for Explanation {}

impl Explanation {
    /// The path this explains.
    pub const fn path(&self) -> &'static str {
        match self {
            Self::Etc(def) => def.path,
            Self::Generated(def) => def.path,
        }
    }

    /// What the contents are derived from.
    pub const fn derived_from(&self) -> &'static str {
        match self {
            Self::Etc(def) => def.source,
            Self::Generated(def) => def.derived_from,
        }
    }

    /// What an operator changes in order to change this file.
    pub const fn changed_by(&self) -> &'static str {
        match self {
            Self::Etc(def) => def.changed_by,
            Self::Generated(def) => def.change_with,
        }
    }

    /// Whether an operator may write it and expect the write to mean something.
    ///
    /// **`false` is not a claim about the kernel.** `/state` is writable and
    /// root is root; this says the write will not survive, or will not do what
    /// it looks like it does. The `why` field is where the difference is spelled
    /// out, every time — [`crate::generated::GeneratedDefinition::writable`].
    pub const fn writable(&self) -> bool {
        match self {
            Self::Etc(_) => false,
            Self::Generated(def) => def.writable,
        }
    }

    /// What happens to a write an operator makes by hand, in a clause that
    /// follows *"it comes from X, and …"*.
    ///
    /// **This is the sentence `apply edit`'s refusal is built out of**, and it
    /// is here rather than there because it is a property of the path rather
    /// than of the operation refusing it — a second operation refusing the same
    /// path must not be able to give a different account of why.
    ///
    /// Until 2026-08-25 `apply edit` carried one hard-coded clause — *"the next
    /// boot would regenerate it"* — which was true of every path `explain` then
    /// knew, all of them `/etc`. It became false the moment this operation could
    /// answer for `/state/deploy-request`, which nothing regenerates and which a
    /// hand-written line does not describe but **actuates**.
    pub const fn consequence(&self) -> &'static str {
        match self {
            Self::Etc(_) => {
                "the next boot would regenerate it over anything written here even if the \
                 filesystem allowed the write, which it does not"
            }
            Self::Generated(def) => match def.kind {
                Kind::Projection => {
                    "it is re-rendered from that source every boot, so a write here is \
                     accepted, appears to work, and is gone after the next reboot — which on \
                     a headless board is the most expensive way to be wrong"
                }
                Kind::Directory => {
                    "it is a directory Rivet creates, so what belongs in it is each entry's \
                     own question rather than this one's"
                }
                Kind::MachineState => {
                    "nothing derives it, so a write here does not change a setting — it \
                     changes what this machine IS, and every client holding the old value \
                     will refuse it"
                }
                Kind::Record => {
                    "it has exactly one writer, and a line appended by hand races that \
                     writer's rotation — the damage lands on somebody else's record rather \
                     than on yours (ADR-022 §11)"
                }
                Kind::Mechanism => {
                    "a file written here is not a description of anything, it is a REQUEST, \
                     and something on this machine will act on whatever it finds"
                }
            },
        }
    }

    /// The word for what kind of thing this is.
    pub const fn kind_word(&self) -> &'static str {
        match self {
            // `ADR-007` §3.6.2's word, and the one every /etc answer has always
            // carried. It is `Kind::Projection`'s word too, and the two are the
            // same fact — but `ETC_FILES` predates `Kind` and does not name one,
            // so this arm states it rather than looking it up.
            Self::Etc(_) => "view",
            Self::Generated(def) => def.kind.word(),
        }
    }

    /// What renders it onto a running machine.
    const fn generated_by(&self) -> &'static str {
        match self {
            Self::Etc(_) => {
                "Rivet's runtime /etc projection, before any service starts (ADR-007 §3.5.1)"
            }
            Self::Generated(def) => def.generated_by,
        }
    }

    /// What it lives on, and why that matters to a write.
    fn filesystem(&self) -> String {
        match self {
            Self::Etc(_) => format!(
                "{ETC_FSTYPE} at {ETC_MOUNT_POINT}, remounted read-only once it is rendered \
                 (ADR-007 §3.6.1 item 1)"
            ),
            Self::Generated(def) => def.filesystem.to_string(),
        }
    }

    /// The mode, octal, as every Unix tool prints it.
    ///
    /// Absent where `rivet-core` is not the authority for the number, and
    /// **says so in words rather than being omitted**: a missing key reads as
    /// "this has no mode", which is false of every directory here. `AGENTS.md`
    /// §5 — an answer names something, even when the something is "not mine".
    fn mode(&self) -> String {
        match self {
            Self::Etc(def) => format!("{:04o}", def.mode),
            Self::Generated(def) => match def.mode {
                Some(mode) => format!("{mode:04o}"),
                None => "not fixed by rivet-core — whichever process creates it decides, \
                         and this operation will not invent a number for it"
                    .to_string(),
            },
        }
    }

    /// Why Rivet generates it at all.
    const fn why(&self) -> &'static str {
        match self {
            Self::Etc(def) => def.reason,
            Self::Generated(def) => def.why,
        }
    }

    /// The structured result, whose keys are `OPERATION`'s declared outputs.
    ///
    /// `every_declared_output_is_a_key_this_operation_actually_produces` is what
    /// keeps the catalogue honest about this one: a catalogue that named a field
    /// the result does not carry would be exactly the "trusted and wrong"
    /// document `ADR-006` §7 refuses. It runs over **one path of each variant**
    /// since 2026-08-25, because a shape asserted for one class and produced by
    /// two is a catalogue that is honest by luck.
    pub fn value(&self) -> Value {
        Value::object(vec![
            ("path", Value::str(self.path())),
            ("kind", Value::str(self.kind_word())),
            ("generated-by", Value::str(self.generated_by())),
            ("derived-from", Value::str(self.derived_from())),
            ("writable", Value::Bool(self.writable())),
            ("filesystem", Value::str(self.filesystem())),
            ("mode", Value::str(self.mode())),
            ("why", Value::str(self.why())),
            ("change-with", Value::str(self.changed_by())),
        ])
    }
}

/// Perform the operation. Arity is `super::perform`'s; this receives exactly the
/// inputs [`OPERATION`] declares.
///
/// # The order the two tables are consulted in does not matter, and is
/// asserted rather than assumed
///
/// `the_tables_do_not_overlap` in [`crate::generated`] is what makes that true:
/// a path in two tables would be answered by whichever is searched first, which
/// is the silent-wrong-answer failure this operation exists to remove.
pub(super) fn perform(arguments: &[&str]) -> Result<Explanation, OperationError> {
    let subject = arguments.first().copied().unwrap_or_default();

    if let Some(definition) = ETC_FILES.iter().find(|def| def.path == subject) {
        return Ok(Explanation::Etc(definition));
    }

    if let Some(definition) = GENERATED_PATHS.iter().find(|def| def.path == subject) {
        return Ok(Explanation::Generated(definition));
    }

    // A path musl reads that Rivet deliberately does **not** render. This is
    // a better answer than "unknown" and it is derived rather than written out:
    // the set is exactly `MUSL_ETC_PATHS` minus what `ETC_FILES` declares, so it
    // cannot go stale when a file starts or stops being rendered.
    if MUSL_ETC_PATHS.contains(&subject) {
        return Err(OperationError::about(
            ErrorCode::UnknownSubject,
            subject,
            format!(
                "musl reads {subject} and Rivet deliberately does not render it, because \
                 musl's own fallback is the correct answer for an unconfigured machine \
                 (rivet-core::etc). It is not a view of anything, so there is nothing to \
                 change. {}",
                explainable()
            ),
        ));
    }

    Err(OperationError::about(
        ErrorCode::UnknownSubject,
        subject,
        format!("Rivet does not generate {subject}. {}", explainable()),
    ))
}

/// The list of paths this operation can answer about, generated from the same
/// definitions it answers from.
///
/// A hand-written list here would be the second authority `ADR-011` §3 names,
/// and it would be read at exactly the moment somebody is already confused.
///
/// **It is grouped**, because an undifferentiated list of paths is harder to
/// read than the two sentences it replaces, and because the grouping is itself
/// the answer to "why does this machine have a file there".
fn explainable() -> String {
    let etc: Vec<&str> = ETC_FILES.iter().map(|def| def.path).collect();
    let rest: Vec<&str> = GENERATED_PATHS.iter().map(|def| def.path).collect();
    format!(
        "This build can explain the /etc projection ({}), and what Rivet writes elsewhere \
         ({}).",
        etc.join(", "),
        rest.join(", ")
    )
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::etc::{EtcInputs, LOGIN_SHELL_PATHS, PASSWD, SHELLS, etc_projection};

    fn explain(path: &str) -> Result<Explanation, OperationError> {
        perform(&[path])
    }

    fn rendered(path: &str) -> Vec<(&'static str, Value)> {
        let value = explain(path)
            .unwrap_or_else(|e| panic!("{path} is rendered by the projection: {e}"))
            .value();
        match value {
            Value::Object(fields) => fields,
            other => panic!("an explanation must be an object, got {other:?}"),
        }
    }

    fn value_of(path: &str, key: &str) -> Value {
        match rendered(path).into_iter().find(|(k, _)| *k == key) {
            Some((_, value)) => value,
            None => panic!("{path} has no {key} field"),
        }
    }

    fn field(path: &str, key: &str) -> String {
        match value_of(path, key) {
            Value::Str(s) => s,
            other => panic!("{path}'s {key} is {other:?}, not a string"),
        }
    }

    // -- every generated file can be explained --------------------------------

    /// **The property that makes this operation worth having.** Every file the
    /// projection renders can be asked about — including the three colon
    /// databases, whose format cannot carry a provenance header and for which
    /// this is therefore the *only* way to read one.
    #[test]
    fn every_file_the_projection_renders_can_be_explained() {
        for file in etc_projection(&EtcInputs {
            login_shells: LOGIN_SHELL_PATHS,
            // A machine with nothing installed. `explain` answers from
            // `ETC_FILES` without rendering anything, so what this machine
            // happens to have installed cannot change any of the three answers
            // — which is the property `EtcDefinition` carries no `EtcInputs`
            // for, and this literal is where a reader can see it.
            ..EtcInputs::default()
        }) {
            let explanation = explain(file.path).unwrap_or_else(|e| {
                panic!("{} is rendered and cannot be explained: {e}", file.path)
            });
            assert_eq!(explanation.path(), file.path);
            assert_eq!(explanation.derived_from(), file.source);
            assert_eq!(explanation.changed_by(), file.changed_by);
        }
    }

    /// `ADR-007` §3.6.1 item 3 asks for three things by name. Each is a field,
    /// and each has to say something.
    #[test]
    fn an_explanation_answers_all_three_questions_adr_007_3_6_1_asks() {
        for def in ETC_FILES {
            assert!(!field(def.path, "derived-from").is_empty(), "{}", def.path);
            assert_eq!(
                value_of(def.path, "writable"),
                Value::Bool(false),
                "{} is reported as writable; ADR-007 §3.6.2 makes a view read-only",
                def.path
            );
            assert!(!field(def.path, "change-with").is_empty(), "{}", def.path);
        }
    }

    /// The catalogue may not name a field the result does not carry — that is
    /// `ADR-006` §7's "trusted and wrong" in the smallest available form.
    #[test]
    fn every_declared_output_is_a_key_this_operation_actually_produces() {
        let keys: Vec<&str> = rendered(PASSWD).iter().map(|(k, _)| *k).collect();
        for output in OPERATION.outputs() {
            assert!(
                keys.contains(&output.name),
                "{} is declared and absent",
                output.name
            );
        }
        assert_eq!(
            keys.len(),
            OPERATION.outputs().len(),
            "an undeclared key: {keys:?}"
        );
    }

    /// And the declared order is the produced order, so an agent reading the
    /// catalogue and an agent reading a result see the same shape.
    #[test]
    fn the_result_carries_its_fields_in_the_order_the_catalogue_declares_them() {
        let keys: Vec<&str> = rendered(SHELLS).iter().map(|(k, _)| *k).collect();
        let published = OPERATION.outputs();
        let declared: Vec<&str> = published.iter().map(|o| o.name).collect();
        assert_eq!(keys, declared);
    }

    /// A mode is octal by universal convention, and `420` would be read as
    /// decimal by the one reader that matters — a person at 2am.
    #[test]
    fn a_mode_is_reported_in_the_octal_every_unix_tool_prints() {
        assert_eq!(field(PASSWD, "mode"), "0644");
        assert_eq!(field("/etc/shadow", "mode"), "0600");
    }

    /// `ADR-007` §3.6.2 makes "view" vocabulary (`AGENTS.md` §10.2), so the
    /// operation uses that word and no other.
    #[test]
    fn every_generated_path_is_reported_as_a_view_and_never_as_writable() {
        for def in ETC_FILES {
            assert_eq!(field(def.path, "kind"), "view", "{}", def.path);
        }
    }

    // -- what it refuses -------------------------------------------------------

    /// The paths musl reads and Rivet deliberately leaves out. An operator
    /// asking about one has a real question — *why is there no
    /// `/etc/resolv.conf`* — and "unknown" would be a true and useless answer,
    /// which is the same defect `ADR-007` §3.6.1 finds in a bare `EROFS`.
    ///
    /// **The set is DERIVED here, as it is in `perform`.** It used to be a
    /// literal list of three, and by 2026-09-02 that list had rotted twice in
    /// the same direction: `/etc/resolv.conf` started being rendered on
    /// 2026-09-01 and was replaced by a **duplicate** of `/etc/localtime`
    /// rather than by the path that had actually taken its place, so the test
    /// silently checked two paths instead of three; then `/etc/localtime`
    /// itself started being rendered. Deriving it means the day a file moves
    /// sides, this test moves with it — `AGENTS.md` §7, in both directions.
    #[test]
    fn a_path_musl_reads_and_rivet_leaves_out_is_told_that_it_is_deliberate() {
        let left_out: Vec<&str> = MUSL_ETC_PATHS
            .iter()
            .copied()
            .filter(|p| !ETC_FILES.iter().any(|def| def.path == *p))
            .collect();
        assert!(
            !left_out.is_empty(),
            "every path musl reads is now rendered; this test has nothing to say"
        );
        for path in left_out {
            let refusal = explain(path).expect_err("that path is not rendered");
            assert_eq!(refusal.code(), ErrorCode::UnknownSubject, "{path}");
            assert_eq!(refusal.subject(), Some(path));
            assert!(
                refusal.message().contains("deliberately"),
                "{path} reads as an omission: {}",
                refusal.message()
            );
        }
    }

    /// Anything else, and the refusal has to be actionable: it lists what *can*
    /// be explained, generated from the same definitions.
    #[test]
    fn an_unknown_path_is_refused_with_the_list_of_paths_that_are_not() {
        // `/etc/motd` was this test's first example until it briefly became a
        // generated view (`ADR-035` §9B.1, 2026-08-26 to 2026-09-13). It would
        // work again now that `[greeting]` is gone (`.rulings/` R-168.1), but
        // `/etc/fstab` stays: a path an operator plausibly reaches for and
        // Rivet does not generate.
        for path in ["/etc/fstab", "/state/config/machine.toml", "", "passwd"] {
            let refusal = explain(path).expect_err("Rivet generates no such path");
            assert_eq!(refusal.code(), ErrorCode::UnknownSubject, "{path:?}");
            for def in ETC_FILES {
                assert!(
                    refusal.message().contains(def.path),
                    "{path:?}: the refusal does not offer {}: {}",
                    def.path,
                    refusal.message()
                );
            }
        }
    }

    /// The subject is carried back verbatim (`AGENTS.md` §5), including bytes
    /// that would break a naively escaped document downstream.
    #[test]
    fn a_refusal_carries_back_exactly_what_the_operator_typed() {
        let awkward = "/etc/\"quoted\"\n";
        let refusal = explain(awkward).expect_err("no such path");
        assert_eq!(refusal.subject(), Some(awkward));
    }

    /// **This operation performs no I/O and must never start.** `ADR-014` §3.1's
    /// standing check in the smallest form available: the answer for a path is
    /// the same whether or not this machine has the file, because the question
    /// is about Rivet rather than about a boot.
    #[test]
    fn the_answer_does_not_depend_on_this_machine_having_the_file() {
        // `/etc/passwd` exists on the Linux host this may run on and does not
        // exist on the Windows one. The explanation is byte-identical on both.
        assert_eq!(rendered(PASSWD), rendered(PASSWD));
        assert_eq!(field(PASSWD, "path"), PASSWD);
    }

    // -- both tables, added 2026-08-25 ----------------------------------------

    /// Every path in both tables is answerable. This is the coverage `PLAN.md`
    /// §0.0 asked for, and it is derived from the tables rather than from a list
    /// somebody maintains beside them.
    #[test]
    fn every_declared_path_in_every_table_can_be_explained() {
        let all = ETC_FILES
            .iter()
            .map(|d| d.path)
            .chain(GENERATED_PATHS.iter().map(|d| d.path));
        for path in all {
            let explanation = explain(path).unwrap_or_else(|e| {
                panic!("{path} is declared and cannot be explained: {}", e.message())
            });
            assert_eq!(explanation.path(), path);
        }
    }

    /// **The catalogue is honest for both variants, not just for `/etc`.**
    ///
    /// `every_declared_output_is_a_key_this_operation_actually_produces` runs
    /// over `/etc/passwd` alone, which was the whole operation until 2026-08-25.
    /// A shape asserted for one class and produced by two is honest by luck.
    #[test]
    fn both_variants_produce_the_declared_shape() {
        for path in [
            PASSWD,
            crate::state::LOG_DIR,
            crate::state::ACTIVE_LOG,
        ] {
            let keys: Vec<&str> = rendered(path).iter().map(|(k, _)| *k).collect();
            let declared: Vec<&str> = OPERATION.outputs().iter().map(|f| f.name).collect();
            assert_eq!(keys, declared, "{path} does not produce the declared shape");
        }
    }

    /// A path whose mode `rivet-core` does not own gets a **sentence**, never a
    /// number and never an absent key. `AGENTS.md` §5: an answer names
    /// something, even when the something is "not mine".
    #[test]
    fn a_mode_this_crate_does_not_own_is_a_sentence_rather_than_an_invention() {
        let mode = field(crate::state::STATE_MOUNT_POINT, "mode");
        assert!(
            !mode.chars().all(|c| c.is_ascii_digit()),
            "a number was invented for a mode nothing here owns: {mode}"
        );
        assert!(mode.contains("rivet-core"), "{mode}");
    }

    /// The refusal for an unknown path offers both groups, so an operator who
    /// guessed wrong can see the whole surface rather than half of it.
    #[test]
    fn the_refusal_lists_both_groups() {
        let message = explain("/state/nope")
            .expect_err("nothing generates that")
            .message()
            .to_string();
        for path in [PASSWD, crate::state::ACTIVE_LOG] {
            assert!(message.contains(path), "the refusal omits {path}: {message}");
        }
    }

    /// Every explanation's `consequence` is a clause that reads after *"and"*,
    /// because that is where `apply edit`'s refusal puts it. A capitalised
    /// opening word or a trailing full stop would produce a broken sentence in
    /// the one message an operator reads at the moment they are already wrong.
    #[test]
    fn every_consequence_is_a_clause_and_not_a_sentence() {
        let all = ETC_FILES
            .iter()
            .map(|d| d.path)

            .chain(GENERATED_PATHS.iter().map(|d| d.path));
        for path in all {
            let clause = explain(path).expect("declared").consequence();
            let first = clause.chars().next().expect("not empty");
            assert!(
                first.is_lowercase(),
                "{path}: the clause starts with {first:?}: {clause}"
            );
            assert!(!clause.ends_with('.'), "{path}: {clause}");
        }
    }
}
