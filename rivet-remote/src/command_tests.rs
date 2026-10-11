//! What `rivet <words…>` becomes, and what it refuses to become.
//!
//! # What is proved here, and what is not — `AGENTS.md` §4 and §6
//!
//! **No board was available when this was written**, and nothing here has run
//! against one.
//!
//! What HAS been done, on 2026-08-29, is the check one layer short of that: the
//! `arguments` objects this module composes were piped into a **real
//! `rivet mcp`** — a Windows build of this workspace's own `cmd/rivet`, whose
//! MCP adapter is the program a board runs — and the answers below are what it
//! said. So the fixtures marked CAPTURED are that program's own words rather
//! than this author's, and `every_argument_object_here_was_read_by_a_real_
//! rivet_mcp` records what each of them establishes. That is the *format*
//! crossing an implementation boundary, which `AGENTS.md` §4 asks for.
//!
//! **The wire was closed too, on the same day and on one host.** A Windows
//! build of `rivet-mcpd` was given a minted identity and a peers file and bound
//! to `127.0.0.1:7423` — an arbitrary local port that day, not this project's,
//! which is `rivet_config::control_plane::DEFAULT_PORT` and is 7777;
//! `cargo rivet run --host 127.0.0.1:7423 --identity …
//! --pin … -- explain /etc/passwd` answered, `exit 0`, and the daemon logged
//! *"TLS 1.3, mutually authenticated"* and `rivet mcp` exiting `0` — no hang,
//! which is the failure `crate::tls::Tls::call`'s close-notify exists for. A
//! wrong `--pin` was refused by the trust model. So the whole path runs:
//! command line, translation, TLS, listener, adapter, operation, answer.
//!
//! **What is still open is a BOARD.** Every operation that needs a running
//! Linux refused on Windows for a platform reason — the correct answer, and not
//! a Rivet failing, but it means no `get storage` has yet come back with a
//! filesystem in it. That closes only against real hardware.
//!
//! **[`Mcp`] below is not a mock that agrees by construction.** It reads the
//! arguments object back into an argument vector with a reader written here, in
//! this file, following `cmd/rivet`'s rule rather than calling this module's
//! [`super::ToolCall::words`] — so a defect in either composition or flattening
//! shows up as a vector that is not the one the operator typed.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

use rivet_core::operation::CATALOGUE;
use std::cell::RefCell;

// ---------------------------------------------------------------------------
// The five shapes the catalogue produces
// ---------------------------------------------------------------------------

fn call_of(words: &[&str]) -> ToolCall {
    translate(words).unwrap_or_else(|e| panic!("`rivet {}` should translate: {e}", words.join(" ")))
}

/// The example in the header of `crate::tls` and the simplest call there is:
/// one subject, nothing else.
#[test]
fn a_get_with_nothing_but_a_subject_is_the_subject_and_nothing_else() {
    let call = call_of(&["get", "storage"]);
    assert_eq!(call.tool, "rivet_get");
    assert_eq!(call.arguments, r#"{"subject":"storage"}"#);
}

/// **The second word binds to the operation's SECOND declared input**, by
/// position and by its name — `boot` for `get`, which is what `get log
/// previous` means.
#[test]
fn a_get_with_one_argument_binds_it_to_the_second_declared_input() {
    let call = call_of(&["get", "log", "previous"]);
    assert_eq!(call.arguments, r#"{"subject":"log","boot":"previous"}"#);

    // The same position carries something else entirely for `get file`, which
    // is the catalogue's own polymorphism (ADR-006 §8.5.1) and the reason this
    // binds by position rather than by knowing what a subject means.
    let call = call_of(&["get", "file", "/state/config/machine.toml"]);
    assert_eq!(
        call.arguments,
        r#"{"subject":"file","boot":"/state/config/machine.toml"}"#
    );
}

/// **`confirm` is a WORD in the vector, not a flag**, so it lands wherever the
/// command line puts it — the second position for a subject that takes no path.
/// `cmd/rivet`'s `reload` reads `arguments[1]` as the confirmation, so this is
/// the vector it wants.
#[test]
fn an_apply_that_needs_confirming_carries_the_word_where_the_operation_reads_it() {
    let call = call_of(&["apply", "reload", "confirm"]);
    assert_eq!(call.tool, "rivet_apply");
    assert_eq!(call.arguments, r#"{"subject":"reload","slot":"confirm"}"#);
    // And the vector the machine reconstructs is the one `apply reload confirm`
    // means, which is the property that matters rather than which key held it.
    assert_eq!(call.words(), ["reload", "confirm"]);
}

/// The `several` input is the tail of the command line, which is where
/// `--expect`, `--append` and ADR-012 §7.3's `--accept` are typed.
#[test]
fn an_edit_puts_its_flag_words_in_the_one_several_input_and_confirms_after_them() {
    let call = call_of(&[
        "apply", "edit", "/state/config/machine.toml", "--expect", "a", "--replace", "b",
        "confirm",
    ]);
    assert_eq!(
        call.arguments,
        r#"{"subject":"edit","slot":"/state/config/machine.toml","change":["--expect","a","--replace","b"],"confirmation":"confirm"}"#
    );
}

/// `ADR-006` §8.10's subjects take their words in the same tail, so the one
/// several-word input carries them without this end knowing what they mean.
#[test]
fn a_file_subject_of_apply_binds_without_this_end_knowing_what_its_words_mean() {
    let call = call_of(&["apply", "rename", "/a", "/b", "confirm"]);
    assert_eq!(
        call.arguments,
        r#"{"subject":"rename","slot":"/a","change":["/b"],"confirmation":"confirm"}"#
    );
    assert_eq!(call.words(), ["rename", "/a", "/b", "confirm"]);
}

/// One input, one word, and no subject at all — `Results::Same`.
#[test]
fn explain_takes_one_path_and_names_the_input_the_catalogue_declares() {
    let call = call_of(&["explain", "/etc/passwd"]);
    assert_eq!(call.tool, "rivet_explain");
    assert_eq!(call.arguments, r#"{"path":"/etc/passwd"}"#);
}

/// **`cargo rivet fleet`'s restart steps (Fix 1) send exactly this shape** —
/// `Remote::perform_operation`'s whole reason to exist rather than
/// `Remote::ask`, which an MCP-backed board refuses a word list through.
#[test]
fn a_service_restart_binds_the_service_name_to_slot_and_the_tail_to_confirmation() {
    let call = call_of(&["apply", "service-restart", "tempmon-rivet", "confirm"]);
    assert_eq!(call.tool, "rivet_apply");
    assert_eq!(
        call.arguments,
        r#"{"subject":"service-restart","slot":"tempmon-rivet","confirmation":"confirm"}"#
    );
    assert_eq!(call.words(), ["service-restart", "tempmon-rivet", "confirm"]);
}

/// The proxy restart step names `rivet-proxyd` as the service — no different
/// a call than any other service's restart, which is the whole point: there
/// is no separate "restart the proxy" verb in the catalogue.
#[test]
fn a_proxy_restart_is_a_service_restart_of_rivet_proxyd() {
    let call = call_of(&["apply", "service-restart", "rivet-proxyd", "confirm"]);
    assert_eq!(
        call.arguments,
        r#"{"subject":"service-restart","slot":"rivet-proxyd","confirmation":"confirm"}"#
    );
}

/// **What is composed is JSON**, so a word carrying a quote or a newline is
/// escaped rather than sent raw — `crate::tls::json_escape`'s job, and the one
/// escaper this crate has.
#[test]
fn a_word_that_would_break_the_json_is_escaped_by_the_one_escaper() {
    let call = call_of(&["apply", "edit", "/p", "--append", "say \"hi\"\n", "confirm"]);
    assert!(
        call.arguments
            .contains(r#"["--append","say \"hi\"\n"]"#),
        "{}",
        call.arguments
    );
}

// ---------------------------------------------------------------------------
// The round trip — the property that makes this faithful rather than plausible
// ---------------------------------------------------------------------------

/// **Every command here flattens back to the words that were typed.**
///
/// This is the whole correctness argument for a positional binding: whatever
/// key a word ends up under, the machine rebuilds the argument vector in
/// declaration order and hands it to `rivet_core::operation::perform`, so the
/// binding is right exactly when that vector is the operator's own words.
///
/// # What this does and does not catch, measured rather than assumed
///
/// The binder was deliberately broken twice on 2026-08-29 to find out.
///
/// * Letting the `several` input swallow the reserved tail — so `confirm` lands
///   in `change` instead of `confirmation` — **does not fail this test, and
///   should not**: the flattened vector is identical, so the machine performs
///   exactly the same operation. Two assignments, one behaviour. The
///   literal-shape tests above are what pin *which* of them this build sends,
///   and they are therefore stricter than correctness requires — on purpose, so
///   that the wire form stays legible and comparable against the schema the
///   board publishes in `tools/list`.
/// * Reserving one word too many — so a word is placed on the wrong input and
///   another has nowhere to go — fails this test and the wire test below.
#[test]
fn every_command_survives_the_round_trip_through_the_catalogue() {
    for words in COMMANDS {
        let call = call_of(words);
        assert_eq!(
            call.words(),
            words[1..],
            "`rivet {}` does not survive the round trip",
            words.join(" ")
        );
    }
}

/// Commands an operator would actually type, across every shape the catalogue
/// has: no arguments, one, a path in a polymorphic position, a several-word
/// tail, a confirmation with and without one, and `ADR-012` §7.3's `--accept`.
const COMMANDS: &[&[&str]] = &[
    &["get", "storage"],
    &["get", "status"],
    &["get", "log"],
    &["get", "log", "previous"],
    &["get", "log", "all", "200"],
    &["get", "file", "/state/config/machine.toml"],
    &["explain", "/etc/resolv.conf"],
    &["apply", "reload"],
    &["apply", "reload", "confirm"],
    &["apply", "power-off", "confirm"],
    &["apply", "trial-boot", "b", "confirm"],
    &["apply", "install", "/state/x.rvt", "confirm"],
    &["apply", "rename", "/a", "/b", "confirm"],
    &["apply", "service-restart", "tempmon-rivet", "confirm"],
    &["apply", "set-mode", "/a", "0644", "confirm"],
    &[
        "apply", "remove-file", "/a", "--accept", "it will not start", "confirm",
    ],
    &[
        "apply", "edit", "/p", "--expect", "a", "--replace", "b", "confirm",
    ],
    &["apply", "edit", "/p", "--append", "x", "confirm"],
];

/// **The assumption this binder rests on, asserted rather than believed.**
///
/// A `several` input has no width of its own — it takes what is left over —
/// so two of them in one operation make a word vector that binds more than one
/// way. Nothing in the catalogue does that today. If something ever does, this
/// fails at the next `cargo test` rather than an operator discovering it, and
/// [`super::bind`] refuses that operation by name in the meantime.
#[test]
fn no_operation_in_the_catalogue_declares_two_several_word_inputs() {
    for op in CATALOGUE {
        let several: Vec<&str> = op
            .inputs
            .iter()
            .filter(|i| i.several)
            .map(|i| i.name)
            .collect();
        assert!(
            several.len() <= 1,
            "`{}` declares {several:?}, and this build binds one",
            op.verb
        );
    }
}

/// Every verb with an operation gets a tool name, and it is the `rivet_` one
/// `cmd/rivet`'s `mcp` module publishes. Three today; counted off the catalogue
/// rather than written down, because that number has moved before.
#[test]
fn every_catalogued_verb_names_a_tool_and_the_names_are_the_boards() {
    let names: Vec<String> = CATALOGUE
        .iter()
        .map(|op| {
            translate(&[op.verb.as_str(), "x"])
                .map(|c| c.tool)
                .unwrap_or_else(|e| panic!("{}: {e}", op.verb))
        })
        .collect();
    // The three the live board published on 2026-08-29, in the catalogue's
    // order. A fourth verb gaining an operation fails this and should: the
    // maintainer will want to know the surface grew.
    assert_eq!(names, ["rivet_apply", "rivet_explain", "rivet_get"]);
}

// ---------------------------------------------------------------------------
// The refusals this end owns
// ---------------------------------------------------------------------------

/// A word that is not one of `ADR-006` §6.4's verbs is refused in
/// `rivet-core`'s own words, which list the vocabulary — and this end adds why
/// a `rivet` handler is not reachable either.
#[test]
fn a_word_that_is_not_a_verb_is_refused_in_the_cores_own_words() {
    let refused = translate(&["measure", "/state/x.rvt"]).expect_err("not a verb");
    assert!(refused.contains("E_UNKNOWN_VERB"), "{refused}");
    assert!(refused.contains("not a Rivet verb"), "{refused}");
    for verb in rivet_core::operation::Verb::ALL {
        assert!(refused.contains(verb.as_str()), "{verb}: {refused}");
    }
    assert!(refused.contains("Nothing has been sent"), "{refused}");
}

/// **A verb with no operation is refused BY NAME, with the tool it would have
/// been.** `set` is in the closed vocabulary and has nothing behind it, so
/// there is no `rivet_set` to call and saying so is better than sending one.
#[test]
fn a_verb_with_no_operation_is_refused_by_name_and_names_the_tool_that_is_absent() {
    let refused = translate(&["set", "hostname", "pi"]).expect_err("no operation behind `set`");
    assert!(refused.contains("`set` is a Rivet verb"), "{refused}");
    assert!(refused.contains("rivet_set"), "{refused}");
    assert!(refused.contains("nothing has been sent"), "{refused}");
    // And it says where the authority is, so the next attempt can be right.
    assert!(refused.contains("rivet --version"), "{refused}");
    for absent in ["set", "list", "add", "remove", "plan", "export"] {
        assert!(
            translate(&[absent, "x"]).is_err(),
            "`{absent}` has no operation in this build and must be refused"
        );
    }
}

/// **A word with nowhere to go is refused rather than dropped**, and the
/// refusal names it. `get` declares four single-word inputs; a fifth word
/// cannot be placed, and a call that quietly discarded it would answer a
/// different question from the one that was asked.
#[test]
fn a_word_with_no_declared_position_is_refused_by_name() {
    let refused = translate(&["get", "log", "latest", "200", "cursor-x", "extra"])
        .expect_err("`get` has four inputs");
    assert!(refused.contains("`extra`"), "{refused}");
    assert!(refused.contains("`subject`"), "{refused}");
    assert!(refused.contains("`since`"), "{refused}");
    assert!(refused.contains("Nothing has been sent"), "{refused}");
    // It also warns about the flag most likely to have produced this, because
    // `rivet get log --follow` is a real command and the follow is a TRANSPORT
    // over the operation rather than an input of it (ADR-022 §9.5).
    assert!(refused.contains("--follow"), "{refused}");
}

#[test]
fn an_empty_command_is_refused_rather_than_sent_as_an_empty_call() {
    let refused = translate(&[]).expect_err("there is nothing to run");
    assert!(refused.contains("no command was given"), "{refused}");
}

// ---------------------------------------------------------------------------
// The drive-letter diagnostic, and content that is never mistaken for a path
// ---------------------------------------------------------------------------

/// **`C:/example` is content the operator typed, never a path a shell could
/// have rewritten**, so it must reach the machine exactly as typed rather than
/// being refused as a mangled path — for `--append`, and for `--expect` and
/// `--replace` together.
#[test]
fn drive_letter_shaped_content_after_append_expect_or_replace_is_never_refused() {
    let mcp = Mcp::answering(r#"{"subject":"edit"}"#);
    for words in [
        &[
            "apply", "edit", "/state/app/demo/notes.txt", "--append", "C:/example", "confirm",
        ][..],
        &[
            "apply",
            "edit",
            "/state/app/demo/notes.txt",
            "--expect",
            "C:/example",
            "--replace",
            "C:/example",
            "confirm",
        ][..],
    ] {
        perform(&mcp, words).unwrap_or_else(|e| panic!("`rivet {}`: {e}", words.join(" ")));
        assert_eq!(
            mcp.last_words(),
            words[1..],
            "`rivet {}` did not reach the machine as typed",
            words.join(" ")
        );
    }
}

/// **`--accept` carries the same shape.** `apply`'s warning phrase is
/// repeated back from what the machine printed, never a path, so a
/// drive-letter-shaped phrase there must reach the machine unrefused too.
#[test]
fn drive_letter_shaped_content_after_accept_is_never_refused() {
    let mcp = Mcp::answering(r#"{"subject":"remove-file"}"#);
    let words = &["apply", "remove-file", "/a", "--accept", "C:/example", "confirm"][..];
    perform(&mcp, words).unwrap_or_else(|e| panic!("`rivet {}`: {e}", words.join(" ")));
    assert_eq!(
        mcp.last_words(),
        words[1..],
        "`rivet {}` did not reach the machine as typed",
        words.join(" ")
    );
}

/// **The check still does its job.** A word that is NOT the one right after
/// `--append`, `--expect` or `--replace` — here, `edit`'s own path argument —
/// is still scanned, so a path a shell genuinely rewrote is still caught and
/// named in the refusal.
#[test]
fn a_genuinely_converted_path_still_gets_the_drive_letter_diagnostic() {
    let refused = translate(&[
        "apply",
        "edit",
        "C:/Program Files/Git/state/x",
        "--append",
        "text",
        "confirm",
    ])
    .expect_err("a shell-mangled path");
    assert!(refused.contains("DRIVE LETTER"), "{refused}");
    assert!(refused.contains("C:/Program Files/Git/state/x"), "{refused}");
}

// ---------------------------------------------------------------------------
// A fake `rivet mcp`, and what comes back from it
// ---------------------------------------------------------------------------

/// A stand-in for the far end of the control plane.
///
/// It answers on the wire — a JSON-RPC `tools/call` result line, through
/// `crate::tls::tool_result` — and it reconstructs the argument vector from the
/// arguments object the way `cmd/rivet`'s `mcp` module does, with the reader
/// below. See this file's header for why that matters.
struct Mcp {
    /// What it was called with, and what it rebuilt from it.
    seen: RefCell<Vec<(String, Vec<String>)>>,
    /// The text to answer with, and whether it is a refusal.
    answer: (String, bool),
}

impl Mcp {
    fn answering(text: &str) -> Self {
        Self {
            seen: RefCell::new(Vec::new()),
            answer: (text.to_string(), false),
        }
    }

    fn refusing(text: &str) -> Self {
        Self {
            seen: RefCell::new(Vec::new()),
            answer: (text.to_string(), true),
        }
    }

    /// The argument vector of the last call.
    fn last_words(&self) -> Vec<String> {
        self.seen
            .borrow()
            .last()
            .expect("nothing was called")
            .1
            .clone()
    }
}

impl crate::tls::CallsTools for Mcp {
    fn call_tool(&self, tool: &str, arguments: &str) -> Result<String, String> {
        let verb = rivet_core::operation::Verb::ALL
            .iter()
            .find(|v| format!("rivet_{v}") == tool)
            .copied()
            .unwrap_or_else(|| panic!("{tool} is not a tool the catalogue publishes"));
        let op = rivet_core::operation::operation(verb)
            .unwrap_or_else(|| panic!("{tool} has no operation behind it"));
        self.seen
            .borrow_mut()
            .push((tool.to_string(), argument_vector(op, arguments)));
        crate::tls::tool_result(&wire(&self.answer.0, self.answer.1))
    }
}

/// **The argument vector `cmd/rivet` rebuilds**, written here rather than
/// borrowed from [`super::ToolCall::words`].
///
/// The rule is that module's: the inputs in declaration order, each contributing
/// its word, and a `several` input contributing all of its words in place.
/// Written independently so that this test can disagree with the composer — a
/// helper that called the composer's own flattener would agree with it by
/// construction and prove nothing.
fn argument_vector(op: &rivet_core::operation::Operation, arguments: &str) -> Vec<String> {
    let fields = read_object(arguments);
    let mut out = Vec::new();
    for input in op.inputs {
        if let Some((_, values)) = fields.iter().find(|(key, _)| key == input.name) {
            out.extend(values.iter().cloned());
        }
    }
    // Anything the operation does not declare would be refused by the machine
    // rather than ignored, so this fake refuses it too — loudly.
    for (key, _) in &fields {
        assert!(
            op.inputs.iter().any(|i| i.name == key),
            "`{key}` is not an input `{}` takes",
            op.verb
        );
    }
    out
}

/// A flat JSON object of strings and arrays of strings, as this module composes
/// one. Not a JSON parser: it panics on anything else, which is what a test
/// helper should do when the thing under test produced a shape it did not mean.
fn read_object(text: &str) -> Vec<(String, Vec<String>)> {
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0usize;
    assert_eq!(chars[at], '{', "an arguments object: {text}");
    at += 1;
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    if chars[at] == '}' {
        return out;
    }
    loop {
        let (key, next) = read_string(&chars, at);
        at = next;
        assert_eq!(chars[at], ':', "a key is followed by a colon: {text}");
        at += 1;
        let values = if chars[at] == '[' {
            at += 1;
            let mut items = Vec::new();
            loop {
                let (item, next) = read_string(&chars, at);
                items.push(item);
                at = next;
                match chars[at] {
                    ',' => at += 1,
                    ']' => {
                        at += 1;
                        break;
                    }
                    other => panic!("an array is malformed at {other:?}: {text}"),
                }
            }
            items
        } else {
            let (value, next) = read_string(&chars, at);
            at = next;
            vec![value]
        };
        assert!(
            !out.iter().any(|(seen, _)| *seen == key),
            "`{key}` appears twice: {text}"
        );
        out.push((key, values));
        match chars[at] {
            ',' => at += 1,
            '}' => return out,
            other => panic!("the object is malformed at {other:?}: {text}"),
        }
    }
}

/// One JSON string from the opening quote at `at`: the decoded value, and the
/// position just past the closing quote.
fn read_string(chars: &[char], at: usize) -> (String, usize) {
    let mut index = at;
    assert_eq!(chars[index], '"', "a string starts with a quote");
    index += 1;
    let mut out = String::new();
    while index < chars.len() {
        match chars[index] {
            '"' => return (out, index + 1),
            '\\' => {
                index += 1;
                match chars[index] {
                    'n' => out.push('\n'),
                    'r' => out.push('\r'),
                    't' => out.push('\t'),
                    'b' => out.push('\u{08}'),
                    'f' => out.push('\u{0c}'),
                    'u' => {
                        let hex: String = chars[index + 1..index + 5].iter().collect();
                        let code = u32::from_str_radix(&hex, 16).expect("four hex digits");
                        index += 4;
                        if (0xD800..=0xDBFF).contains(&code) {
                            let low: String = chars[index + 3..index + 7].iter().collect();
                            let low = u32::from_str_radix(&low, 16).expect("four hex digits");
                            out.push(
                                char::from_u32(0x1_0000 + ((code - 0xD800) << 10) + (low - 0xDC00))
                                    .expect("a character"),
                            );
                            index += 6;
                        } else {
                            out.push(char::from_u32(code).expect("a character"));
                        }
                    }
                    other => out.push(other),
                }
                index += 1;
            }
            other => {
                out.push(other);
                index += 1;
            }
        }
    }
    panic!("unterminated string")
}

/// One JSON-RPC result line, as `rivet mcp` writes one. The same composition
/// `crate::tls`'s own tests use, kept beside the reader that undoes it.
fn wire(text: &str, is_error: bool) -> String {
    let mut escaped = String::new();
    for c in text.chars() {
        match u32::from(c) {
            0x22 => escaped.push_str("\\\""),
            0x5c => escaped.push_str("\\\\"),
            0x0a => escaped.push_str("\\n"),
            0x0d => escaped.push_str("\\r"),
            0x09 => escaped.push_str("\\t"),
            code if code < 0x20 => escaped.push_str(&format!("\\u{code:04x}")),
            _ => escaped.push(c),
        }
    }
    format!(
        r#"{{"jsonrpc":"2.0","id":2,"result":{{"content":[{{"type":"text","text":"{escaped}"}}],"isError":{is_error}}}}}"#
    )
}

/// **The whole path, against a fake that rebuilds the vector independently.**
///
/// Words in, one tool call over the wire format, and the argument vector the
/// machine would perform — which must be the words that were typed.
#[test]
fn a_command_reaches_the_far_end_as_the_argument_vector_it_was_typed_as() {
    for words in COMMANDS {
        let mcp = Mcp::answering(r#"{"subject":"storage"}"#);
        perform(&mcp, words).unwrap_or_else(|e| panic!("`rivet {}`: {e}", words.join(" ")));
        assert_eq!(
            mcp.last_words(),
            words[1..],
            "`rivet {}` arrived as something else",
            words.join(" ")
        );
        assert_eq!(mcp.seen.borrow().len(), 1, "one command, one tool call");
    }
}

// ---------------------------------------------------------------------------
// CAPTURED, 2026-08-29, from a real `rivet mcp`
// ---------------------------------------------------------------------------
//
// **Provenance.** These four are what `cmd/rivet`'s MCP adapter answered when
// the `arguments` object this module composes for the command named above each
// one was piped into it — three JSON-RPC lines on stdin, exactly as
// `crate::tls::Tls::call` writes them. The binary was a Windows debug build of
// this workspace, so every operation that needs a running Linux refused for
// that reason; the refusals are the machine's own and they are the point.
//
// **What each one establishes**, since a refusal proves less than it looks:
//
// * `E_MACHINE_REFUSED` naming `storage` means the subject was BOUND and the
//   operation ran — an unbound or undeclared input is refused by the adapter
//   before any operation sees it, with a different sentence.
// * The `get log` answer echoes `boot` and `limit` back, so all three of that
//   call's inputs landed on the positions the operator's words meant.
// * `apply reload` with `confirm` bound to `slot` reached the operation, which
//   then refused for want of a supervisor.
//
//   **It does NOT prove that `confirm` was read as the confirmation**, and the
//   first version of this note claimed it did. Running `apply reload` WITHOUT
//   `confirm` over the same channel produced the same `E_UNSAFE`, because that
//   subject asks `can_request_reload` on the plan path too — so the two are
//   indistinguishable on a machine with no supervisor. What is proved is that
//   `slot` is accepted for this subject and reaches it; that the word in that
//   position is the confirmation is `rivet-core`'s own property
//   (`reload::perform` reads `arguments[1]`, and its tests cover it), reached
//   through the vector `every_command_survives_the_round_trip_through_the_
//   catalogue` asserts.
// * The unknown subject is refused by the machine, which names its whole
//   vocabulary — the refusal this end deliberately does not write itself.

/// `-- get storage`, from `{"subject":"storage"}`.
const CAPTURED_STORAGE: &str = r#"{"error":"E_MACHINE_REFUSED","subject":"storage","message":"this rivet was built for windows, and measuring a filesystem needs Linux's /proc/self/mounts and statvfs(3). NOTHING WAS MEASURED — this is not a machine with no filesystems, it is a build that cannot see the ones it has. Run this against the board."}"#;

/// `-- get log previous 50`, from
/// `{"subject":"log","boot":"previous","limit":"50"}`.
const CAPTURED_LOG: &str = r#"{"subject":"log","boot":"previous","limit":50,"boots":[],"records":[],"returned":0,"held":0,"unreadable":0,"files":[],"source":"/state/log"}"#;

/// `-- apply reload confirm`, from `{"subject":"reload","slot":"confirm"}`.
const CAPTURED_RELOAD: &str = r#"{"error":"E_UNSAFE","subject":"reload","message":"this rivet was built for windows, which has no Rivet supervisor to ask. A reload is delivered to this machine's own `rivet-sysd` (ADR-032 §3.2), and this build cannot read a /proc to find one. NOTHING WAS SENT. Run this against the board."}"#;

/// `-- get stroage`, from `{"subject":"stroage"}`.
const CAPTURED_UNKNOWN: &str = r#"{"error":"E_UNKNOWN_SUBJECT","subject":"stroage","message":"`get` reads `log` and `storage` and `install` and `schema` and `status` and `desired-state` and `memory` and `process` and `thermal` and `boot` and `slot` and `network` and `device` and `file` and `diff` and `cpu` in this build. The desired-state `get <schema path>` of ADR-006 §6.4 is not built — ADR-006 §8.3 is the open question, not an omission. `rivet --version` publishes the operation catalogue, which is generated from the types and is the authority for what exists."}"#;

/// The whole result line for `get log previous 50`, as `rivet mcp` wrote it.
///
/// Kept alongside the decoded text so that `crate::tls`'s reader is exercised
/// against bytes the OTHER side actually produced rather than against
/// `super::tests::wire`'s reconstruction of them — `AGENTS.md` §4's rule about
/// a format that crosses an implementation boundary.
const CAPTURED_LOG_LINE: &str = r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"{\"subject\":\"log\",\"boot\":\"previous\",\"limit\":50,\"boots\":[],\"records\":[],\"returned\":0,\"held\":0,\"unreadable\":0,\"files\":[],\"source\":\"/state/log\"}"}],"isError":false}}"#;

/// **Every command whose arguments were checked against a real `rivet mcp`**,
/// and the answer it gave. See the note above for what each establishes.
///
/// This test asserts the part that can be asserted without that program: that
/// the composed object is still the one that was piped into it. If a change
/// here alters what is sent, the capture stops describing this build and this
/// fails, which is the cheap guard `AGENTS.md` §4 asks for around a check that
/// is expensive to repeat.
#[test]
fn every_argument_object_here_was_read_by_a_real_rivet_mcp() {
    for (words, arguments) in [
        (&["get", "storage"][..], r#"{"subject":"storage"}"#),
        (
            &["get", "log", "previous", "50"][..],
            r#"{"subject":"log","boot":"previous","limit":"50"}"#,
        ),
        (
            &["apply", "reload", "confirm"][..],
            r#"{"subject":"reload","slot":"confirm"}"#,
        ),
        (&["get", "stroage"][..], r#"{"subject":"stroage"}"#),
        (&["explain", "/etc/passwd"][..], r#"{"path":"/etc/passwd"}"#),
    ] {
        assert_eq!(
            call_of(words).arguments,
            arguments,
            "`rivet {}` no longer composes what was piped into `rivet mcp`",
            words.join(" ")
        );
    }
}

/// A line a real `rivet mcp` wrote, read by the reader that will read them.
#[test]
fn a_result_line_from_a_real_rivet_mcp_decodes_to_the_answer_it_carried() {
    assert_eq!(
        crate::tls::tool_result(CAPTURED_LOG_LINE),
        Ok(CAPTURED_LOG.to_string())
    );
}

/// **The answer is the machine's answer.** Byte for byte, not reformatted, not
/// summarised, not wrapped — `AGENTS.md` §5.
#[test]
fn what_comes_back_is_what_the_machine_said_and_nothing_added_to_it() {
    let mcp = Mcp::answering(CAPTURED_LOG);
    assert_eq!(
        perform(&mcp, &["get", "log", "previous", "50"]),
        Ok(CAPTURED_LOG.to_string())
    );
}

/// A refusal is passed through the same way, and the two captured ones are
/// different shapes — one from the platform, one from a safety interlock.
#[test]
fn a_refusal_is_passed_through_as_the_machine_wrote_it_too() {
    for (words, captured) in [
        (&["get", "storage"][..], CAPTURED_STORAGE),
        (&["apply", "reload", "confirm"][..], CAPTURED_RELOAD),
    ] {
        let mcp = Mcp::refusing(captured);
        assert_eq!(
            perform(&mcp, words).expect_err("the machine refused"),
            captured,
            "the machine's words, unaltered"
        );
    }
}

/// **A subject the machine does not read is the MACHINE's refusal**, arriving
/// intact. This end does not check subjects — there is no second list of nouns
/// here — so `stroage` is sent, and what comes back is a refusal that names
/// every subject the board reads, which is a better answer than a local
/// spell-check could give.
#[test]
fn an_unknown_subject_is_the_machines_refusal_and_arrives_word_for_word() {
    let mcp = Mcp::refusing(CAPTURED_UNKNOWN);
    let refused = perform(&mcp, &["get", "stroage"]).expect_err("the machine refused");
    assert_eq!(refused, CAPTURED_UNKNOWN, "the machine's words, unaltered");
    // And it was actually sent, rather than refused here: this end has no
    // opinion about what subjects exist.
    assert_eq!(mcp.last_words(), ["stroage"]);
}

/// A refusal this end owns never reaches the wire.
#[test]
fn a_command_that_cannot_be_translated_is_never_sent() {
    let mcp = Mcp::answering("{}");
    assert!(perform(&mcp, &["set", "hostname", "pi"]).is_err());
    assert!(perform(&mcp, &["get", "a", "b", "c", "d", "e"]).is_err());
    assert!(mcp.seen.borrow().is_empty(), "nothing may have been sent");
}
