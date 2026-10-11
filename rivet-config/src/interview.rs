//! **A provisioning interview, generated from [`crate::schema::DOCUMENT`].**
//!
//! Plan §5.2 item 3 (`.rulings/` R-162, Wave 3b): item 1 (`ADR-007` §7) gave
//! the schema declaration order, `if`/`then`/`else` conditions and a `prompt`
//! distinct from each property's `description`; item 2 (the same section)
//! gave `mode` and `fleet-role` real document keys. This module is what turns
//! those three things into an ordered list of questions, so
//! `cmd/rivet-provision`'s hand-written list and `cargo rivet provision`'s
//! orchestrator ask from the same one rather than two spellings of it
//! (`ADR-011`).
//!
//! # What is generated, and what is deliberately curated
//!
//! **Every top-level scalar property in [`crate::schema::DOCUMENT`] becomes a
//! question, in declaration order, with no hand-written list left to drift
//! from it.** `version` is the one exception — it carries a fixed
//! [`crate::schema::Kind::Integer`] `constant` and is never asked, exactly as
//! its own `prompt` says (*"not asked; always the current one"*). Adding a
//! new top-level scalar key to `DOCUMENT` is therefore the whole of adding a
//! question: [`generate`] walks the array and there is nothing else to touch.
//! `interview_tests.rs`'s `a_fixture_key_becomes_a_question_in_position`
//! proves this against a fixture array rather than the production one, so the
//! claim is checked and not merely asserted.
//!
//! **Two top-level keys are `Table`/`NamedTables` and are curated by name
//! through `EXPANSIONS`, not walked generically**, and that is a recorded
//! decision rather than an oversight (`AGENTS.md` §1.1, second row — it
//! changes nothing on disk, nothing a machine reads, and is a revert away).
//! The reason is `cmd/rivet-provision`'s own interview, pinned by
//! `the_document_order_matches_the_interview_order`
//! (`docs/PLAN-Feature-Set-2026-09-12.md` §5.2's "day one reproduces today's
//! order and question numbers exactly"): `network` asks only
//! `method`/`address`/`gateway`, never `ssid` — wireless provisioning has no
//! interview question today, schema-driven or not — and `[kiosk]` asks only
//! `auto-run`, never `locked`. A table with no entry in `EXPANSIONS` asks
//! nothing at all; it is configured after the fact (`apply edit`, and later
//! the portal — plan §5.3). Widening either expansion, or adding a third, is
//! the maintainer's to rule the same way `ssid` itself would be — it changes
//! what an interview asks, which is `AGENTS.md` §1.1's third row, not this
//! module's.
//!
//! # Conditions
//!
//! `address` and `gateway` are gated on `method == "static"`
//! (`ADR-007` §7) — `crate::schema::gate` (via `crate::schema::named_gate`,
//! this module's own door to it) republishes the same
//! `if`/`then`/`else` data item 1 put in the JSON Schema, so this module does
//! not carry a second copy of the rule. A [`Question::gate`] is not filtered
//! out of [`generate`]'s own output — the list is static and gates are
//! evaluated against an operator's *answers*, which do not exist until the
//! interview runs — but [`Question::applies`] answers whether a gated
//! question should be asked or composed into the document, given the
//! earlier answer it depends on.

use crate::schema::{self, Kind, Property};

/// What a question accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Free text, checked by whatever validator the caller attaches to this
    /// key — this module carries none, on `rivet-config`'s own no-I/O,
    /// no-caller-vocabulary footing (`ADR-014` §3).
    Text,
    /// One of a closed set, offered in this order.
    Choice(&'static [&'static str]),
}

/// A question this key is asked only when a sibling key equals a value —
/// republishes `crate::schema::gate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gate {
    /// The sibling question's own key.
    pub on: &'static str,
    /// The value of `on` under which this question is asked.
    pub when: &'static str,
}

/// One generated question.
#[derive(Debug, Clone, Copy)]
pub struct Question {
    /// The schema property's own name — `"hostname"`, `"method"`,
    /// `"auto-run"` — never a display label. `label` is what a caller shows;
    /// this is what a caller looks the answer up by.
    pub key: &'static str,
    /// A short display label. Historical spelling for the eight keys
    /// `cmd/rivet-provision`'s rehearsal scripts already quote by word
    /// (`SPEC-05` §4); a generated `Title Case` rendering of `key` for
    /// anything added since — see `label_for`.
    pub label: &'static str,
    /// The schema's own short form — `crate::schema::Property::prompt`.
    pub prompt: &'static str,
    /// The schema's own long form — `crate::schema::Property::description`.
    pub description: &'static str,
    /// The schema's own value rule, if it has one distinct from `answer`'s
    /// shape — `crate::schema::Property::rule`.
    pub rule: Option<&'static str>,
    /// What this question accepts.
    pub answer: Answer,
    /// Asked, and composed into the document, only when the earlier
    /// question named by `on` was answered `when`.
    pub gate: Option<Gate>,
}

impl Question {
    /// Whether this question's gate is satisfied, given the earlier
    /// answers gathered so far as `(key, value)` pairs. `true` for an
    /// ungated question. A borrowed slice rather than a closure, so a caller
    /// holding ordinary (non-`'static`) `String` answers — the real
    /// interview, not a fixture of string literals — has something to pass
    /// with no lifetime contortion.
    #[must_use]
    pub fn applies(&self, answers: &[(&str, &str)]) -> bool {
        match self.gate {
            None => true,
            Some(Gate { on, when }) => answers.iter().any(|(k, v)| *k == on && *v == when),
        }
    }
}

/// A top-level `Table`/`NamedTables` key that contributes questions, and
/// which of its own nested properties do — see the module documentation for
/// why this is curated rather than generic.
///
/// **Carries no property list of its own.** The nested properties always
/// come from `generate`'s own match on `Kind::Table`/`Kind::NamedTables`,
/// which reads them straight out of `crate::schema::DOCUMENT` at the one
/// place they are already guaranteed correct — see
/// `crate::schema::named_gate`'s own documentation for why a second copy
/// carried through this struct is the one thing to avoid.
struct Expansion {
    /// The top-level `DOCUMENT` key this expansion answers for — also what
    /// `crate::schema::named_gate` is keyed by.
    top_level: &'static str,
    /// Which nested properties become questions, in this order — always a
    /// subsequence of the nested table's own declaration order, so a
    /// question this array names still respects `ADR-007` §7's declaration-order
    /// guarantee.
    ask: &'static [&'static str],
}

/// The name every generated interview writes `network`'s single answered
/// interface under. **One interface, unchanged from today** —
/// `cmd/rivet-provision`'s `compose` has always written `[network.eth0]`
/// alone; asking about more than one is out of scope here exactly as it was
/// before this module existed.
pub const NETWORK_DEFAULT_INTERFACE: &str = "eth0";

const EXPANSIONS: &[Expansion] = &[
    Expansion { top_level: "network", ask: &["method", "address", "gateway"] },
    Expansion { top_level: "kiosk", ask: &["auto-run"] },
];

/// Historical display labels for keys whose spelling predates this module and
/// is quoted verbatim in the maintainer's own rehearsal notes
/// (`cmd/rivet-provision`'s old, hand-written `questions()`; `SPEC-05` §4:
/// *"Hostname is 1 and Fleet is 6 in every note, every rehearsal script and
/// every memory of doing this"*). A key with no entry here gets
/// [`label_for`]'s generated rendering instead — new keys were never in a
/// rehearsal script to begin with.
const LEGACY_LABELS: &[(&str, &str)] = &[
    ("hostname", "Hostname"),
    ("method", "Addressing"),
    ("address", "Address"),
    ("gateway", "Gateway"),
    ("mode", "Mode"),
    ("fleet-role", "Fleet"),
    ("timezone", "Timezone"),
    ("auto-run", "Auto-run"),
];

/// `key`'s display label: the historical spelling if [`LEGACY_LABELS`] names
/// one, else `Title Case With Spaces` derived from the schema's own key —
/// the rendering a schema key gets the moment it is declared, with no second
/// list to update.
fn label_for(key: &'static str) -> &'static str {
    if let Some((_, label)) = LEGACY_LABELS.iter().find(|(k, _)| *k == key) {
        return label;
    }
    // Titlecased and leaked once, into the same `'static` lifetime every
    // other label already carries — this module has no allocator-free path
    // to a computed `&'static str` and a schema has, at most, a few dozen
    // keys ever declared in a process's lifetime, so the leak is bounded and
    // permanent by construction rather than by discipline.
    let title: String = key
        .split(['-', '_'])
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + chars.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ");
    Box::leak(title.into_boxed_str())
}

/// One property's [`Answer`] shape.
fn answer_for(kind: Kind) -> Answer {
    match kind {
        Kind::Text { one_of } if !one_of.is_empty() => Answer::Choice(one_of),
        Kind::Boolean => Answer::Choice(&["true", "false"]),
        _ => Answer::Text,
    }
}

fn question_for(p: &Property, gate: Option<Gate>) -> Question {
    Question {
        key: p.name,
        label: label_for(p.name),
        prompt: p.prompt,
        description: p.description,
        rule: p.rule,
        answer: answer_for(p.kind),
        gate,
    }
}

/// **Generate an interview from a document-shaped schema array**, in
/// declaration order.
///
/// Generic over the array rather than hard-coded to
/// [`crate::schema::DOCUMENT`] so a test can prove the mechanism — that
/// placing a schema key in the right position is enough by itself to add a
/// question, with nothing else to touch — against a small fixture instead of
/// the production schema, where the same claim would be far more expensive
/// to observe.
#[must_use]
pub fn generate(document: &'static [Property]) -> Vec<Question> {
    let mut out = Vec::new();
    for p in document {
        match p.kind {
            // A pinned constant (`version`) is never a question — there is
            // nothing for an operator to decide.
            Kind::Integer { constant: Some(_) } => {}
            Kind::Integer { constant: None } | Kind::Text { .. } | Kind::Boolean => {
                out.push(question_for(p, None));
            }
            // No top-level `DOCUMENT` key is a bare `TextArray` today —
            // nothing to generate.
            Kind::TextArray => {}
            // A `Table`/`NamedTables` key asks nothing UNLESS `EXPANSIONS`
            // names it — see the module documentation for why this is
            // curated rather than walked generically.
            Kind::Table { properties } => push_expansion(&mut out, p.name, properties),
            Kind::NamedTables { entry } | Kind::NamedTablesWithReserved { entry, .. } => {
                push_expansion(&mut out, p.name, entry);
            }
            // No top-level `DOCUMENT` key is this kind today either
            // (`applications`, `Kind::NamedTablesOfStrings`) — and even if one
            // were, its entries have no `&'static [Property]` for `EXPANSIONS`
            // to name a nested question from.
            Kind::NamedTablesOfStrings => {}
        }
    }
    out
}

/// The shared half of the two `Kind` arms that expand into nested questions.
fn push_expansion(out: &mut Vec<Question>, top_level: &'static str, nested: &'static [Property]) {
    let Some(expansion) = EXPANSIONS.iter().find(|e| e.top_level == top_level) else {
        return;
    };
    for name in expansion.ask {
        // `EXPANSIONS` is authored to name only properties its own nested
        // table declares — a mismatch is a bug in this module, not
        // something an operator or a document produced, so it is skipped
        // rather than panicked on: a missing question is a smaller failure
        // than a crashed interview.
        let Some(property) = nested.iter().find(|q| q.name == *name) else {
            continue;
        };
        let gate = schema::named_gate(top_level, name).map(|(on, when)| Gate { on, when });
        out.push(question_for(property, gate));
    }
}

#[cfg(test)]
#[path = "interview_tests.rs"]
mod tests;
