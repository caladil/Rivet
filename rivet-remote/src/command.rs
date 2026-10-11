//! **One catalogue operation, named in the machine's own words, as an MCP tool
//! call.**
//!
//! `crate::deploy::Remote::ask` documents the vocabulary this takes — *"the
//! machine's own CLI, spoken in the machine's own vocabulary"* — and
//! [`crate::tls::Tls`]'s implementation of it refuses, in words worth quoting
//! because they are the specification of this module:
//!
//! > MCP is the CATALOGUE, not a command line: a tool call names an operation
//! > and its declared inputs, and `ask` takes a word list precisely because the
//! > transports it was written for handed the machine a shell.
//!
//! **This is the translation that refusal declines to guess at.** It turns
//! `get storage` into `rivet_get {"subject":"storage"}` and
//! `apply edit /p --expect a --replace b confirm` into a different shape, using
//! [`rivet_core::operation::CATALOGUE`] to decide which — never a table of
//! subjects written here, which is the second list of nouns this project
//! refuses everywhere else.
//!
//! # Why this exists at all, recorded because it was measured
//!
//! `cargo rivet` could `package`, `update`, `deploy`, `inspect`, `key` and
//! `identity`, and **only `deploy --identity` spoke the control plane at all**.
//! To ask a board a question from a workstation an operator had to open a login
//! session or hand-write JSON-RPC. On 2026-08-29 a cold agent given this
//! repository and a board reached for the login and stayed there — *it was the
//! path of least resistance because it was the only path that worked for a
//! general query*. `ADR-035`'s phase 5 leaves only the control plane; until
//! that plane can carry an arbitrary operation it cannot be the interface no
//! matter what the documentation says.
//!
//! # The binding rule, and why it is positional
//!
//! `rivet_core::operation::Input` says it in its own words: *"Positional, and
//! the position is the declaration order … a future MCP or REST adapter has
//! named inputs it binds to these names."* So an input's **name** is a label for
//! a position rather than a description of a value, and the catalogue says so
//! out loud — `apply`'s `slot` is a slot letter for `trial-boot`, the absolute
//! path of a payload for `install`, and the path of a file for `edit`
//! (`ADR-006` §8.5.1's *"polymorphic positional input, paid again"*).
//!
//! So the words fill the declared inputs left to right, and the one input a
//! verb may declare `several` absorbs whatever is left over after reserving a
//! word for each remaining single-word input. `apply`'s `change` is the only
//! `several` input in the catalogue, and it is the tail of `apply`'s word vector
//! — which is exactly where `--expect`, `--append` and `ADR-012` §7.3's
//! `--accept` are typed.
//!
//! **The property that makes this faithful rather than plausible is a round
//! trip**, and it is a test rather than a claim:
//! [`ToolCall::words`](crate::command::ToolCall::words) flattens a composed call back into the argument vector
//! the machine will reconstruct, and
//! `every_command_survives_the_round_trip_through_the_catalogue` asserts it is
//! the vector the operator typed, word for word, for every command in a table.
//!
//! **That vector is the whole of what the machine performs**, so more than one
//! assignment of words to inputs can be correct: letting `change` swallow the
//! trailing `confirm` produces the same vector and therefore the same
//! operation. This build makes the assignment the catalogue reads most
//! naturally — every single-word input first, the several-word one taking only
//! the slack — and the tests pin it exactly, which is stricter than correctness
//! requires and is meant to be. The wire form should be readable beside the
//! schema the board publishes in `tools/list`.
//!
//! **[`ToolCall::words`](crate::command::ToolCall::words) is a restatement of a rule this crate does not own**,
//! said plainly rather than left to be discovered. `cmd/rivet`'s `mcp` module
//! holds the authority — it binds named arguments back to a positional vector
//! in declaration order, flattening a `several` input in place — and it is a
//! binary crate, so there is nothing to import. What that costs: if the two
//! ever disagree, this end composes a call the machine reads differently. What
//! it does not cost is silence — the machine's answer names the subject and the
//! selector it acted on, so a mis-binding is visible in the answer rather than
//! hidden by it.
//!
//! # What this refuses, and why refusing is the whole point
//!
//! * **A word that is not one of `ADR-006` §6.4's verbs** — refused in
//!   `rivet-core`'s own words, which list the vocabulary.
//! * **A verb this build's catalogue has no operation behind** — `set`, `list`,
//!   `add`, `remove`, `plan`, `export` today. There is no tool for it, and
//!   naming it is better than sending a `tools/call` that cannot exist.
//! * **More words than the operation has positions to hold them** — a fifth
//!   word for `get`, which declares four. There is nowhere to put it, so
//!   nothing is sent.
//! * **An operation declaring two `several` inputs**, which nothing in the
//!   catalogue does and which this binder cannot place unambiguously. Guarded
//!   by a test over the whole catalogue so the day it changes fails the build
//!   rather than an operator.
//!
//! What it deliberately does **not** refuse is a word the operation itself will
//! refuse. `get stroage` is sent, and the machine answers `E_UNKNOWN_SUBJECT`
//! naming every subject it reads — a better refusal than this end could write,
//! and the machine's own (`AGENTS.md` §5).
//!
//! # The catalogue read here is this build's, not the board's
//!
//! `cargo-rivet` and the `rivet` on the card are built from one workspace, so
//! normally they are the same catalogue. When they are not — an older board —
//! the binding is composed from the newer list. **The failure is loud rather
//! than silent:** an operation the board does not have answers a JSON-RPC error
//! for an unknown tool, and an input it does not declare is refused by name by
//! `cmd/rivet`'s own *"is not an input `rivet_get` takes, so it would be
//! discarded rather than acted on"*. Both arrive here as refusals and are
//! reported verbatim.

use rivet_core::operation::{ACCEPT_FLAG, Operation, Verb, operation};

use crate::tls::{CallsTools, json_escape};

/// The prefix every tool name carries in `ADR-032` §4's projection.
///
/// **A literal, and it is the one spelling this crate has** — the position
/// `crate::tls::DOCUMENT_SUBJECT` is already in, and for the same reason.
/// `cmd/rivet`'s `mcp` module owns the projection and is a binary crate, so
/// there is nothing to import; the alternative is moving the name into
/// `rivet-core`, which is a change to what that crate is for and is the
/// maintainer's.
///
/// The risk is smaller than it looks: a machine whose adapter no longer spells
/// tools this way answers a JSON-RPC `method not found` for the call, and
/// `crate::tls::tool_result` reports it as *"the machine refused the call"*
/// quoting what it said. That is a refusal an operator can read rather than a
/// wrong answer.
const TOOL_PREFIX: &str = "rivet_";

/// The four flags whose one following word is literal text — never a path on
/// the machine, and so never checked for one by `shell_rewrote`. Three are
/// `apply edit`'s; `ACCEPT_FLAG` is `apply`'s own warning phrase, repeated
/// back exactly as it was printed, and it is exactly as literal as the other
/// three.
///
/// **Three literals and one import, not four literals.** `ACCEPT_FLAG` is
/// public — `rivet_core::operation` re-exports it — so it is read from there
/// rather than duplicated; `EXPECT_FLAG`, `REPLACE_FLAG` and `APPEND_FLAG` are
/// not (`rivet_core::operation::edit` is a private module), which is
/// `TOOL_PREFIX`'s reason for the three that stay local: exporting three
/// constants to save one other spelling is a bigger decision about that
/// crate's public surface than three short literals here, which
/// `every_command_survives_the_round_trip_through_the_catalogue` already
/// exercises against the real thing.
const CONTENT_FLAGS: [&str; 4] = ["--append", "--expect", "--replace", ACCEPT_FLAG];

/// One value bound to one declared input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bound {
    /// The input's name, as the catalogue declares it.
    pub input: &'static str,
    /// The word or words that filled it. Never empty — an input that got no
    /// words is omitted rather than sent empty, because an empty array and an
    /// absent key are two ways to say one thing and the machine reads them
    /// differently.
    pub words: Vec<String>,
    /// Whether the catalogue declares this input `several`, and therefore
    /// whether it goes over the wire as a JSON array.
    pub several: bool,
}

/// One `tools/call`, composed and ready to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    /// The verb this came from, for a caller that wants to say what it is
    /// about to run.
    pub verb: Verb,
    /// `rivet_get`, `rivet_apply`, `rivet_explain`.
    pub tool: String,
    /// The `arguments` object, as JSON text.
    pub arguments: String,
    /// What each declared input was bound to, in declaration order.
    pub bound: Vec<Bound>,
}

impl ToolCall {
    /// **The argument vector the machine will reconstruct from this call.**
    ///
    /// The words after the verb, in declaration order, with a `several` input
    /// flattened in place. See this module's header for why this is a
    /// restatement of a rule `cmd/rivet` owns rather than a shared
    /// implementation, and what that costs.
    #[must_use]
    pub fn words(&self) -> Vec<String> {
        self.bound
            .iter()
            .flat_map(|b| b.words.iter().cloned())
            .collect()
    }
}

/// **Turn `rivet <words…>` into the tool call that performs it.**
///
/// # Errors
///
/// A sentence naming what could not be translated and why, for every case this
/// end can decide without the machine — see this module's header. Nothing is
/// sent for any of them.
/// **The caller's shell rewrote an argument, and only the caller's side can
/// know that.**
///
/// # Why this is here and not left to the machine
///
/// The machine already refuses a path that is not absolute, and its refusal is
/// good. **But it refuses the path it RECEIVED, which is not the path anybody
/// typed** — a Unix-shell-on-Windows translates an absolute path into a Windows
/// one before this program starts, and `/state/secrets/peers` arrives as
/// `C:/Program Files/Git/state/secrets/peers`. The operator then reads a
/// refusal about a path they have never seen, from a machine that is behaving
/// correctly.
///
/// **This is not a second copy of the machine's rule** — `ADR-011` would forbid
/// that, and this does not decide whether a path is acceptable. It states a fact
/// about the CLIENT's environment: an argument reached this process already
/// carrying a drive letter, and no path on a Rivet has one. The machine's
/// verdict is still the machine's; this one never reaches it.
///
/// It also fails **before a connection is opened**, which is the practical half:
/// the same mistake cost a round trip and a confusing answer on 2026-08-29.
///
/// # What this does not check
///
/// **The word right after `--append`, `--expect`, `--replace` or `--accept` is
/// content, not a path**, and is never scanned. `apply edit`'s several-word
/// tail is typed content — the text to append, or the text to find and its
/// replacement — and `apply`'s own `--accept` repeats back a warning phrase
/// the machine printed; a shell does not rewrite any of them, because none is
/// a path in the command line to begin with. `C:/example` typed there is
/// exactly what the operator meant, and refusing it was this end inventing a
/// second opinion about a word it was never its place to read.
///
/// This runs on the raw words, before `bind` knows which declared input a word
/// will fill — so it cannot ask the catalogue which inputs are paths, and falls
/// back to the one shape it does know: these four flags are always followed
/// by exactly one word of literal text.
fn shell_rewrote(words: &[&str]) -> Option<String> {
    let mangled = words
        .iter()
        .enumerate()
        .find_map(|(at, w)| {
            let is_content = at > 0 && CONTENT_FLAGS.contains(&words[at - 1]);
            if is_content {
                return None;
            }
            let b = w.as_bytes();
            (b.len() > 2 && b[0].is_ascii_alphabetic() && w[1..].starts_with(":/")).then_some(w)
        })?;
    Some(format!(
        "`{mangled}` carries a DRIVE LETTER, and no path on a Rivet has one. NOTHING WAS \
         SENT.\n\
         \x20 why   a Unix shell on Windows (MSYS, Git Bash) rewrites an absolute path into a \
         Windows one BEFORE this program is started, and a path on the machine looks exactly \
         like one it should translate. The argument you typed is not the argument that arrived.\n\
         \x20 fix   set `MSYS_NO_PATHCONV=1` for the command, and the path reaches the machine \
         as written.\n\
         \x20 note  the machine would refuse this too, and its refusal would name the \
         TRANSLATED path — which is why this one is raised here instead."
    ))
}

pub fn translate(words: &[&str]) -> Result<ToolCall, String> {
    if let Some(refusal) = shell_rewrote(words) {
        return Err(refusal);
    }
    let Some((verb_word, rest)) = words.split_first() else {
        return Err(
            "no command was given. This runs ONE operation from the machine's catalogue \
             — `get storage`, `apply reload confirm` — and there is nothing here to run."
                .to_string(),
        );
    };

    let verb = Verb::parse(verb_word).map_err(|refusal| {
        format!(
            "{refusal}\n\
             \x20 why  the control plane projects the operation CATALOGUE onto tool calls \
             (ADR-012 §6), so a word that is not one of those verbs names no tool — even \
             when `rivet` on the machine answers to it, as it does for the handlers its \
             login shell owns. Nothing has been sent."
        )
    })?;

    let op = operation(verb).ok_or_else(|| {
        format!(
            "`{verb}` is a Rivet verb (ADR-006 §6.4) and this build's catalogue has no \
             operation behind it, so there is no `{TOOL_PREFIX}{verb}` tool to call and \
             nothing has been sent.\n\
             \x20 why  every tool is generated from the catalogue (ADR-012 §6), so a verb \
             with no operation has no tool rather than a tool that answers \
             E_UNIMPLEMENTED.\n\
             \x20 fix  `rivet --version` on the machine publishes the catalogue, which is \
             generated from the types and is the authority for what exists. The verbs with \
             an operation in THIS build are: {}.",
            rivet_core::operation::CATALOGUE
                .iter()
                .map(|o| o.verb.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        )
    })?;

    let bound = bind(op, rest)?;
    Ok(ToolCall {
        verb,
        tool: format!("{TOOL_PREFIX}{verb}"),
        arguments: compose(&bound),
        bound,
    })
}

/// **Run one command on a machine and hand back what it answered.**
///
/// The answer is the operation's own JSON, exactly as the machine composed it.
/// Nothing here reformats or summarises it — `AGENTS.md` §5, and the reason a
/// refusal comes back the same way.
///
/// # Errors
///
/// This end's refusal when the command cannot be translated, and otherwise
/// **the machine's own words**: the text of a tool result marked an error, or
/// whatever the transport failed with.
pub fn perform(caller: &dyn CallsTools, words: &[&str]) -> Result<String, String> {
    let call = translate(words)?;
    caller.call_tool(&call.tool, &call.arguments)
}

/// Fill the operation's declared inputs from the words, left to right.
///
/// See this module's header for the rule and why it is positional. The
/// arithmetic is the whole of it: a `several` input takes everything that is
/// left after reserving one word for each single-word input still to come, so
/// the tail of the vector lands where the tail of the command line is typed.
fn bind(op: &Operation, words: &[&str]) -> Result<Vec<Bound>, String> {
    let several = op.inputs.iter().filter(|i| i.several).count();
    if several > 1 {
        return Err(format!(
            "`{verb}` declares {several} several-word inputs ({names}) and this build can \
             place only one, so it will not guess where the boundary between them is. \
             Nothing has been sent.\n\
             \x20 why  a several-word input has no width of its own — it takes whatever is \
             left over — and two of them make a word vector that binds more than one way. \
             Running the wrong one of those is the failure this refusal exists to prevent.\n\
             \x20 fix  this is a defect in this tool rather than in the command; the \
             catalogue grew a shape it cannot bind.",
            verb = op.verb,
            names = op
                .inputs
                .iter()
                .filter(|i| i.several)
                .map(|i| format!("`{}`", i.name))
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }

    let mut bound: Vec<Bound> = Vec::new();
    let mut left = words;
    for (at, input) in op.inputs.iter().enumerate() {
        if left.is_empty() {
            break;
        }
        if input.several {
            let reserved = op.inputs[at + 1..].iter().filter(|i| !i.several).count();
            let take = left.len().saturating_sub(reserved);
            if take > 0 {
                bound.push(Bound {
                    input: input.name,
                    words: left[..take].iter().map(|w| (*w).to_string()).collect(),
                    several: true,
                });
                left = &left[take..];
            }
        } else {
            bound.push(Bound {
                input: input.name,
                words: vec![left[0].to_string()],
                several: false,
            });
            left = &left[1..];
        }
    }

    if !left.is_empty() {
        return Err(format!(
            "`rivet {verb}` declares {count} input{plural} — {names} — and none of them is \
             several words, so there is nowhere to put {extra}. Nothing has been sent.\n\
             \x20 why  a tool call names an operation and its DECLARED inputs (ADR-012 §6); \
             this is not a shell command line, and a word with no position would be \
             discarded rather than acted on.\n\
             \x20 fix  `rivet --version` on the machine publishes what each operation takes. \
             A flag `rivet` itself consumes — `--follow` is the one to watch for — is not an \
             input of any operation and cannot be sent this way.",
            verb = op.verb,
            count = op.inputs.len(),
            plural = if op.inputs.len() == 1 { "" } else { "s" },
            names = op
                .inputs
                .iter()
                .map(|i| format!("`{}`", i.name))
                .collect::<Vec<_>>()
                .join(", "),
            extra = left
                .iter()
                .map(|w| format!("`{w}`"))
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }

    Ok(bound)
}

/// The bound inputs as one JSON object.
///
/// **`crate::tls::json_escape` and no second escaper**, which is the whole of
/// why that function is visible to this module: the outbound half of two layers
/// of JSON is one problem with one answer, and its documentation carries the
/// measurement that says why non-ASCII leaves as `\uXXXX`.
fn compose(bound: &[Bound]) -> String {
    let fields: Vec<String> = bound
        .iter()
        .map(|b| {
            let key = json_escape(b.input);
            if b.several {
                let items: Vec<String> = b
                    .words
                    .iter()
                    .map(|w| format!("\"{}\"", json_escape(w)))
                    .collect();
                format!("\"{key}\":[{}]", items.join(","))
            } else {
                // A single-word input is bound to exactly one word by `bind`,
                // and joining is how that is said without an index that could
                // panic in a crate that denies `unwrap`.
                format!("\"{key}\":\"{}\"", json_escape(&b.words.join(" ")))
            }
        })
        .collect();
    format!("{{{}}}", fields.join(","))
}

#[cfg(test)]
#[path = "command_tests.rs"]
mod tests;
