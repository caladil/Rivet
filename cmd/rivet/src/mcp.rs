//! `rivet mcp` — **the Model Context Protocol adapter.**
//!
//! `ADR-032` §4, whose opening sentence records that the request which started
//! that whole enquiry was *"build an MCP adapter"*. This is it.
//!
//! # It is a projection, and that is the test it has to pass
//!
//! `ADR-012` §6 states the check in advance, and it is worth quoting because
//! no other document specifies what this module must do:
//!
//! > **if the MCP adapter is more than a thin mapping from the operation
//! > catalogue onto tool definitions, the boundary was drawn in the wrong
//! > place.**
//!
//! So there is no operation here, no verb here and no policy here. Every tool
//! is generated from `rivet_core::operation::CATALOGUE`, every argument is bound
//! by the core, and every answer is the `Value` the core returned.
//! `every_tool_is_an_operation_and_every_operation_is_a_tool` is what stops that
//! being an aspiration.
//!
//! # The transport was ruled, and it costs nothing
//!
//! `ADR-006` §3.3's table gives MCP **stdio**, with authentication left to
//! whatever hands this program its streams. No listening socket of its own, no
//! TLS, no identity system, no new attack surface: it reads stdin and writes
//! stdout, and `cmd/rivet-mcpd` is what puts a caller on the other end.
//!
//! `ADR-032` §4.2 found the one thing in the way — `root`'s login shell is
//! `rivet` and `cmd/rivet` executes nothing, so asking a machine to run
//! `rivet-mcp` as a program is refused — and ruled the answer: a **declared
//! table of internal handlers**, which is `crate::handler`. This module is the
//! second entry in it.
//!
//! # Confirmation is unchanged, and deliberately so
//!
//! `ADR-012` §7.1 makes confirmation per *change*, and an operation that is not
//! given its confirming word **returns its plan and does nothing**. That is
//! already true of every path into `rivet_core::operation::perform`, so an agent
//! calling `rivet_apply` without `confirmation` gets the plan — which is exactly
//! `ADR-012` §3's *"plan and apply is the safety mechanism"*, reached for free
//! rather than reimplemented here.
//!
//! **This module therefore contains no confirmation logic at all**, and a
//! reader looking for it should read that absence as the design working.
//!
//! # What is deliberately not here
//!
//! * **`crate::handler`'s own words**, `measure` among them. A handler is
//!   *"this binary's, reachable only through the login shell"* — an adapter's
//!   own invocation contract rather than an operation — so projecting them here
//!   would put a `cmd/rivet` concept into an interface that is supposed to be
//!   `rivet-core`'s. `ADR-006` §3.1's conformance property is about operations.
//! * **Capability scoping.** `ADR-012` §5 rules it *a filter over the
//!   catalogue* and defers enforcement to Stage 3. The shape permits it — a
//!   scope is a predicate over [`tools`] — and nothing here pretends it exists.
//! * **Notifications.** `ADR-032` §4.3 lists *"a decision about whether the
//!   machine may push"* as open. This server answers and never speaks first.

use rivet_core::Value;
use rivet_core::operation::{self, CATALOGUE, Input, Operation, Verb};

use crate::json::{self, Json};
use crate::value::json as render_json;

/// The MCP revision this build speaks.
///
/// **Declared rather than negotiated down.** A client that asks for another
/// revision is told what this one is and may proceed or not; guessing at
/// compatibility with a revision this build has never seen is the kind of
/// generosity `crate::json` refuses one layer down.
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// What this server calls itself to a client.
pub const SERVER_NAME: &str = "rivet";

/// The JSON-RPC revision. There is only one.
const JSONRPC: &str = "2.0";

/// The prefix every tool name carries.
///
/// **`rivet_`, so an agent holding several servers can tell whose `get` this
/// is.** The rest of the name is the verb, mechanically, so a tool name cannot
/// drift from the operation it performs.
const TOOL_PREFIX: &str = "rivet_";

/// JSON-RPC's own codes, which are the only ones this server invents.
///
/// An operation's refusal is **not** one of these: it is a successful tool call
/// whose result is marked an error, which is MCP's distinction between *the
/// protocol failed* and *the tool said no*. Collapsing the two would tell an
/// agent that Rivet is broken when Rivet has simply refused.
mod code {
    pub const PARSE: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
}

/// The name a verb takes as a tool.
#[must_use]
pub fn tool_name(verb: Verb) -> String {
    format!("{TOOL_PREFIX}{}", verb.as_str())
}

/// The verb a tool name means, if this build has one.
#[must_use]
pub fn verb_named(tool: &str) -> Option<Verb> {
    let word = tool.strip_prefix(TOOL_PREFIX)?;
    CATALOGUE
        .iter()
        .map(|op| op.verb)
        .find(|verb| verb.as_str() == word)
}

/// **Every tool this server offers**, generated from the catalogue.
///
/// One tool per operation, in the catalogue's order. There is no table here and
/// no place for one to drift from.
#[must_use]
pub fn tools() -> Value {
    Value::List(CATALOGUE.iter().map(tool).collect())
}

/// One operation, as a tool definition.
fn tool(op: &Operation) -> Value {
    Value::object(vec![
        ("name", Value::str(tool_name(op.verb))),
        ("description", Value::str(op.summary)),
        ("inputSchema", schema(op.inputs)),
    ])
}

/// An operation's inputs, as a JSON Schema object.
///
/// **The descriptions are the operation's own**, carried across verbatim. They
/// are the thing an agent reads to decide what to send, and rewording them here
/// would create a second account of what an input means — `ADR-011`, in the
/// place it would be least visible.
fn schema(inputs: &[Input]) -> Value {
    let properties = inputs
        .iter()
        .map(|input| {
            let kind = if input.several {
                // Several words arrive as several strings, which is what
                // `perform` binds them to.
                Value::object(vec![
                    ("type", Value::str("array")),
                    ("items", Value::object(vec![("type", Value::str("string"))])),
                ])
            } else {
                Value::object(vec![("type", Value::str("string"))])
            };
            let Value::Object(mut fields) = kind else {
                unreachable!("built as an object one line above")
            };
            fields.push(("description", Value::str(input.description)));
            (input.name, Value::Object(fields))
        })
        .collect::<Vec<_>>();

    let required: Vec<Value> = inputs
        .iter()
        .filter(|input| input.required)
        .map(|input| Value::str(input.name))
        .collect();

    Value::object(vec![
        ("type", Value::str("object")),
        ("properties", Value::Object(properties)),
        ("required", Value::List(required)),
    ])
}

/// Turn a call's arguments into the words an operation is given.
///
/// **In the order the operation declares them**, because that is the order
/// `perform` binds them in — the words an agent sends are the words an operator
/// would type, which is `ADR-006` §3.1's conformance property reaching all the
/// way down to the argument vector.
///
/// # Errors
///
/// A sentence, which becomes a tool result marked an error rather than a
/// protocol failure: the client's request was well-formed, and Rivet is saying
/// no to its contents.
fn words(op: &Operation, arguments: Option<&Json>) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let Some(arguments) = arguments else {
        // No arguments at all is legitimate for an operation whose inputs are
        // all optional, and is caught below for one whose are not.
        return missing(op, &out).map_or(Ok(out), Err);
    };
    if !matches!(arguments, Json::Object(_)) {
        return Err(format!(
            "`arguments` must be an object of this tool's inputs and is {}. \
             `tools/list` publishes the schema.",
            arguments.kind()
        ));
    }
    if let Some(name) = arguments.duplicate_keys().first() {
        return Err(format!(
            "`arguments` carries `{name}` more than once, so which one was meant is not \
             decidable. Nothing has been run."
        ));
    }

    // **A skipped input may not be followed by a given one.** These become a
    // POSITIONAL argument vector, so an absent optional does not leave a hole —
    // it shifts every later value one place left. Before 2026-08-28 this build
    // did that silently: `{"subject":"log","limit":"3"}` produced
    // `["log", "3"]`, and the operation read `3` as the BOOT SELECTOR and
    // answered `"boot":"3"` with a straight face.
    //
    // Refused rather than repaired, and the choice is deliberate. Filling the
    // hole means this adapter inventing a value for an input it was not given,
    // which is a default — and defaults belong to the operation (`ADR-011`),
    // which is where `get log`'s `latest` and `200` already live. An adapter
    // that guessed would be a second place they were written down.
    //
    // `several` inputs are exempt: they carry their own flag words, so
    // `apply edit`'s `--expect`/`--replace` name themselves and cannot shift.
    let mut skipped: Option<&str> = None;
    for input in op.inputs {
        let Some(given) = arguments.get(input.name) else {
            if !input.several {
                skipped = Some(input.name);
            }
            continue;
        };
        if let Some(hole) = skipped
            && !input.several
        {
            return Err(format!(
                "`{given_name}` was given and `{hole}` was not, and this build cannot \
                 place one without the other: `{tool}`'s inputs become a positional \
                 argument vector, so a missing `{hole}` would silently shift \
                 `{given_name}` into its place and the machine would answer a different \
                 question. Give `{hole}` as well — `tools/list` publishes what it means \
                 and what value stands for the default.",
                given_name = input.name,
                hole = hole,
                tool = tool_name(op.verb),
            ));
        }
        if input.several {
            let Some(items) = given.as_array() else {
                return Err(format!(
                    "`{}` is several words and must be an array of strings; it is {}.",
                    input.name,
                    given.kind()
                ));
            };
            for item in items {
                out.push(one_word(input.name, item)?);
            }
        } else {
            out.push(one_word(input.name, given)?);
        }
    }

    // Anything the schema does not declare is refused rather than ignored, on
    // `rivet-config`'s terms exactly (`ADR-007` §3.6): a key this build would
    // discard is one whose sender believed it did something.
    if let Json::Object(fields) = arguments {
        for (key, _) in fields {
            if !op.inputs.iter().any(|input| input.name == key) {
                return Err(format!(
                    "`{key}` is not an input `{}` takes, so it would be discarded rather than \
                     acted on. This build takes {}.",
                    tool_name(op.verb),
                    declared(op.inputs),
                ));
            }
        }
    }

    missing(op, &out).map_or(Ok(out), Err)
}

/// A required input that was not given, as a sentence.
fn missing(op: &Operation, given: &[String]) -> Option<String> {
    if given.is_empty()
        && let Some(first) = op.inputs.iter().find(|input| input.required)
    {
        return Some(format!(
            "`{}` requires `{}` and none was given. {}",
            tool_name(op.verb),
            first.name,
            first.description,
        ));
    }
    None
}

fn declared(inputs: &[Input]) -> String {
    if inputs.is_empty() {
        return "no inputs at all".to_string();
    }
    inputs
        .iter()
        .map(|input| format!("`{}`", input.name))
        .collect::<Vec<_>>()
        .join(", ")
}

fn one_word(name: &str, value: &Json) -> Result<String, String> {
    value.as_str().map(str::to_owned).ok_or_else(|| {
        format!(
            "`{name}` must be a string and is {}. Every input this catalogue declares is a \
             word an operator would type.",
            value.kind()
        )
    })
}

/// **How a request's id comes back**, which is the one part of the envelope
/// `rivet_core::Value` cannot carry.
///
/// JSON-RPC requires `"id": null` on an error produced before an id could be
/// read, and `Value` has no null — deliberately, because nothing Rivet *says*
/// is null. Rather than add one to a core type for the sake of an envelope
/// this adapter owns, the envelope is composed here and the payload inside it
/// is still a `Value` rendered by `crate::value::json`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Id {
    Number(i64),
    Text(String),
    /// Present and null, which is what an error before parsing must echo.
    Null,
    /// A notification. There is no reply at all.
    Absent,
}

impl Id {
    /// The id a request carries, or why there is no reply to it.
    fn of(request: &Json) -> Self {
        match request.get("id") {
            None => Self::Absent,
            Some(Json::Integer(n)) => Self::Number(*n),
            Some(Json::Str(s)) => Self::Text(s.clone()),
            // Anything else — a float, an object — is not an id JSON-RPC
            // defines. It is echoed as null rather than refused outright,
            // because a reply the client cannot match is still better than
            // silence it cannot explain.
            Some(_) => Self::Null,
        }
    }

    fn render(&self) -> String {
        match self {
            Self::Number(n) => n.to_string(),
            Self::Text(s) => render_json(&Value::str(s.clone())),
            Self::Null | Self::Absent => "null".to_string(),
        }
    }
}

/// One reply, composed.
fn reply(id: &Id, body: &str) -> Answer {
    if matches!(id, Id::Absent) {
        // **A reply to a notification is a protocol error in the other
        // direction**, so a notification that fails fails silently. That is
        // JSON-RPC's rule and not a shortcut.
        return Answer::Silent;
    }
    Answer::Reply(format!(
        "{{\"jsonrpc\":\"{JSONRPC}\",\"id\":{},{body}}}",
        id.render()
    ))
}

fn result(id: &Id, value: &Value) -> Answer {
    reply(id, &format!("\"result\":{}", render_json(value)))
}

/// A **protocol** failure — the message was not one this server can act on.
///
/// Distinct from a tool that refused, which is a successful call carrying
/// `isError` — see [`code`]. An agent needs to tell *Rivet is broken* from
/// *Rivet said no*, and collapsing the two would report the first when the
/// second happened.
fn error(id: &Id, code: i64, message: &str) -> Answer {
    let body = format!(
        "\"error\":{{\"code\":{code},\"message\":{}}}",
        render_json(&Value::str(message.to_string()))
    );
    // **No special case for `Id::Absent`, and there was one until a test caught
    // it.** A message with no id is a NOTIFICATION, and JSON-RPC gives a
    // notification no reply *even when it fails* — so an error envelope with a
    // null id would be this server speaking unbidden. The genuinely
    // id-less case is a message that could not be parsed at all, and that path
    // passes `Id::Null` rather than `Id::Absent` precisely so the two stay
    // different: null means *there was a request and I could not read its id*.
    reply(id, &body)
}

/// A tool that ran and answered, whether it agreed or refused.
///
/// **`isError` is the refusal channel**, and the text is what the CLI would
/// have printed — the same `Value`, the same renderer. `ADR-006` §3.1: an
/// answer that exists in only one interface is a defect in that interface.
fn tool_result(id: &Id, value: &Value, refused: bool) -> Answer {
    let text = render_json(value);
    let body = format!(
        "\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":{}}}],\"isError\":{refused}}}",
        render_json(&Value::str(text))
    );
    reply(id, &body)
}

/// Answer one line of JSON-RPC.
///
/// **Everything above the operation is here and everything below it is the
/// core's.** No I/O, so every branch is reachable from a test with no stdin,
/// no board and no connection.
pub fn handle(line: &str, board: &mut dyn rivet_core::operation::Board) -> Answer {
    let request = match json::parse(line) {
        Ok(request) => request,
        Err(e) => {
            return error(
                &Id::Null,
                code::PARSE,
                &format!("that line is not JSON this build reads: {e}"),
            );
        }
    };
    let id = Id::of(&request);

    if !matches!(request, Json::Object(_)) {
        return error(
            &id,
            code::INVALID_REQUEST,
            &format!("a JSON-RPC message is an object and this is {}.", request.kind()),
        );
    }
    match request.get("jsonrpc").and_then(Json::as_str) {
        Some(JSONRPC) => {}
        Some(other) => {
            return error(
                &id,
                code::INVALID_REQUEST,
                &format!("this build speaks JSON-RPC {JSONRPC} and the message declares {other:?}."),
            );
        }
        None => {
            return error(
                &id,
                code::INVALID_REQUEST,
                &format!("a JSON-RPC message declares `\"jsonrpc\":\"{JSONRPC}\"` and this one declares nothing."),
            );
        }
    }
    let Some(method) = request.get("method").and_then(Json::as_str) else {
        return error(
            &id,
            code::INVALID_REQUEST,
            "a JSON-RPC message names a `method`, as a string, and this one does not.",
        );
    };

    match method {
        "initialize" => result(&id, &initialize()),
        "ping" => result(&id, &Value::Object(Vec::new())),
        "tools/list" => result(&id, &Value::object(vec![("tools", tools())])),
        "tools/call" => call(&id, request.get("params"), board),
        // **Every notification is answered with silence**, including ones this
        // build does not know: a notification asks for nothing, so not knowing
        // it is not a failure worth a message the client must then ignore.
        _ if method.starts_with("notifications/") => Answer::Silent,
        other => error(
            &id,
            code::METHOD_NOT_FOUND,
            &format!(
                "`{other}` is not a method this build answers. It answers `initialize`, \
                 `ping`, `tools/list` and `tools/call`, and ignores every `notifications/*`."
            ),
        ),
    }
}

/// What this server says it is.
fn initialize() -> Value {
    Value::object(vec![
        ("protocolVersion", Value::str(PROTOCOL_VERSION)),
        // Tools and nothing else. **No `resources` and no `prompts`**, because
        // this build has neither and declaring a capability it does not
        // implement is the "trusted and wrong" answer `ADR-006` §7 refuses.
        (
            "capabilities",
            Value::object(vec![("tools", Value::Object(Vec::new()))]),
        ),
        (
            "serverInfo",
            Value::object(vec![
                ("name", Value::str(SERVER_NAME)),
                ("version", Value::str(env!("CARGO_PKG_VERSION"))),
            ]),
        ),
    ])
}

/// `tools/call` — the one method that does anything.
fn call(
    id: &Id,
    params: Option<&Json>,
    board: &mut dyn rivet_core::operation::Board,
) -> Answer {
    let Some(params) = params else {
        return error(id, code::INVALID_PARAMS, "`tools/call` takes `params`, and none were given.");
    };
    let Some(name) = params.get("name").and_then(Json::as_str) else {
        return error(
            id,
            code::INVALID_PARAMS,
            "`tools/call` names the tool in `params.name`, as a string. `tools/list` publishes \
             the names.",
        );
    };
    let Some(verb) = verb_named(name) else {
        // **A protocol error rather than a refusal**, because the client asked
        // for something that is not on the menu it was given — that is a bug in
        // the client, not a decision by Rivet.
        return error(
            id,
            code::INVALID_PARAMS,
            &format!(
                "`{name}` is not a tool this build offers. It offers {}.",
                CATALOGUE
                    .iter()
                    .map(|op| tool_name(op.verb))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    };
    let Some(op) = operation::operation(verb) else {
        return error(
            id,
            code::INVALID_PARAMS,
            &format!("`{name}` is catalogued and this build has no operation behind it."),
        );
    };

    // From here on a failure is Rivet's answer rather than a protocol fault,
    // so it goes back as a tool result with `isError`.
    let words = match words(op, params.get("arguments")) {
        Ok(words) => words,
        Err(why) => return tool_result(id, &Value::str(why), true),
    };
    let borrowed: Vec<&str> = words.iter().map(String::as_str).collect();
    match operation::perform(verb, &borrowed, board) {
        Ok(value) => tool_result(id, &value, false),
        // **The core's own refusal, rendered by the core's own renderer.** An
        // agent reading this and an operator reading the CLI are reading one
        // object, which is what makes `ADR-006` §3.1 true rather than claimed.
        Err(refusal) => tool_result(id, &refusal.value(), true),
    }
}

/// **This module's entry in `ADR-032` §4.2's internal-handler table.**
///
/// Declared here rather than in `crate::handler`, on `crate::measure::HANDLER`'s
/// rule: a description kept beside the table instead of beside the code is the
/// thing `ADR-006` §7 says will be wrong within a month.
pub const HANDLER: crate::handler::Handler = crate::handler::Handler {
    name: crate::handler::MCP,
    summary: "Speak the Model Context Protocol on stdin and stdout, so an agent can drive this \
              machine through the operation catalogue (ADR-032 4).",
    takes: "nothing. It reads JSON-RPC messages from stdin, one per line, and writes replies to \
            stdout — so it is what rivet-mcpd spawns for each connection, and what a local \
            agent on the machine runs directly. From a workstation the transport is TLS and the \
            client is `cargo rivet run` (ADR-035); this is the far end of it, not the way in.",
    why_not_a_verb: "It is not an administrative operation and performs none itself: it is an \
                     ADAPTER, and every adapter in ADR-006 3 sits beside the CLI rather than \
                     inside it. A verb here would make the catalogue contain its own projection.",
};

/// Read JSON-RPC from `input`, write replies to `output`, until input ends.
///
/// **One message per line, and a line that is not one is answered rather than
/// fatal.** A client that sends a malformed message gets a parse error and the
/// session continues; ending the session would make one bad line cost every
/// message after it.
///
/// # Errors
///
/// Only an I/O failure on the streams themselves — which is the session
/// ending, and is the ordinary way this returns.
pub fn serve(
    input: &mut dyn std::io::BufRead,
    output: &mut dyn std::io::Write,
    board: &mut dyn rivet_core::operation::Board,
) -> std::io::Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        // **A bounded read**, because the other end is a network. `crate::json`
        // caps depth; this caps length, and the two together are what stop a
        // hostile client costing this process its memory.
        let read = read_line_capped(input, &mut line)?;
        if read == 0 {
            return Ok(());
        }
        let trimmed = line.trim();
        // Blank lines between messages are ordinary framing and mean nothing.
        if trimmed.is_empty() {
            continue;
        }
        if let Answer::Reply(reply) = handle(trimmed, board) {
            writeln!(output, "{reply}")?;
            // **Flushed every message.** stdout down a pipe is block
            // buffered, and a reply that sits in a buffer is a client that
            // waits for ever.
            output.flush()?;
        }
    }
}

/// The longest message this build will read.
///
/// A `tools/call` is a few hundred bytes. 1 MiB is far past anything real and
/// far short of a board's 926 MB — it exists so that a client which never sends
/// a newline cannot grow this process without bound.
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// `read_line`, but it stops at [`MAX_MESSAGE_BYTES`] instead of at memory.
///
/// # It decodes UTF-8, and until 2026-08-29 it did not
///
/// **This function silently corrupted every non-ASCII character that reached
/// it.** It read one byte at a time and did `into.push(char::from(byte))`, which
/// is not a decode: `char::from(u8)` is **Latin-1**, mapping each byte to the
/// code point of the same number. A UTF-8 `é` arrives as `0xC3 0xA9` and became
/// two characters, `Ã` and `©`.
///
/// **Measured rather than reasoned about**, by the agent that found it: a
/// 152-byte document sent through `apply edit` over MCP was written to the card
/// as **167 bytes** of mojibake, and `rivet get` refused a subject named `café`
/// as `cafÃ©`. `json.rs` was never at fault — the line was already wrong before
/// the parser saw it, which is why that module's own tests all passed.
///
/// It is the worst shape a defect can have here: **no error anywhere**, a
/// plausible-looking result, and the damage written to disk. `AGENTS.md` §5.
///
/// # Invalid UTF-8 is REFUSED, not replaced
///
/// The bytes are accumulated and decoded once. A sequence that is not UTF-8
/// makes this return an error rather than substituting `U+FFFD`, because
/// JSON-RPC is UTF-8 by definition (RFC 8259 §8.1) and a lossy decode is how
/// the original defect got to be silent. A caller that sends broken bytes is
/// told so.
fn read_line_capped(
    input: &mut dyn std::io::BufRead,
    into: &mut String,
) -> std::io::Result<usize> {
    let mut total = 0;
    let mut byte = [0_u8; 1];
    // **Bytes, then one decode at the end.** A per-byte decode is impossible:
    // a multi-byte character is not a character until its last byte arrives.
    let mut line: Vec<u8> = Vec::new();
    loop {
        let n = std::io::Read::read(input, &mut byte)?;
        if n == 0 {
            break;
        }
        total += n;
        line.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
        if total >= MAX_MESSAGE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "a message reached {MAX_MESSAGE_BYTES} bytes with no newline; this build \
                     reads one JSON-RPC message per line"
                ),
            ));
        }
    }
    match std::str::from_utf8(&line) {
        Ok(text) => {
            into.push_str(text);
            Ok(total)
        }
        Err(e) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "this message is not valid UTF-8 at byte {} of {}; JSON-RPC is UTF-8 (RFC 8259 \
                 §8.1) and this build will not guess an encoding. Nothing was read from it",
                e.valid_up_to(),
                line.len()
            ),
        )),
    }
}

/// What one request produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// A line to write back.
    Reply(String),
    /// A notification, which JSON-RPC answers with silence. **Silence rather
    /// than an empty object**: a reply to a notification is a protocol error in
    /// the other direction.
    Silent,
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// **The Latin-1 defect, pinned so it cannot come back.**
    ///
    /// Until 2026-08-29 `read_line_capped` did `push(char::from(byte))`, which
    /// decodes each byte as Latin-1. `é` is `0xC3 0xA9` in UTF-8 and came out
    /// as `Ã©` — two characters where one was sent, and **no error anywhere**.
    ///
    /// The assertion is on the round trip rather than on the implementation,
    /// so it still holds if the reader is rewritten. Every character here is
    /// one a real document or application name can contain: an accent, a
    /// section sign this project uses constantly, an em dash, and an
    /// astral-plane character that needs a surrogate pair in JSON.
    #[test]
    fn a_line_of_utf8_survives_the_reader_byte_for_byte() {
        let sent = "{\"subject\":\"café\", \"note\":\"§7 — ok 🚀\"}\n";
        let mut input = std::io::BufReader::new(sent.as_bytes());
        let mut got = String::new();
        let read = read_line_capped(&mut input, &mut got).expect("a well-formed UTF-8 line");
        assert_eq!(got, sent, "the line came back changed");
        assert_eq!(
            read,
            sent.len(),
            "the count is BYTES read, not characters — they differ here and that is the point"
        );
        assert!(
            !got.contains('Ã'),
            "Latin-1 decoding is back: {got}"
        );
    }

    /// Invalid UTF-8 is **refused**, never replaced with `U+FFFD`.
    ///
    /// A lossy decode is how the original defect stayed silent: it produced a
    /// plausible string and wrote it to a card. JSON-RPC is UTF-8 by definition
    /// (RFC 8259 §8.1), so bytes that are not UTF-8 are a caller error and get
    /// said out loud.
    #[test]
    fn a_line_that_is_not_utf8_is_refused_rather_than_replaced() {
        // 0xC3 starts a two-byte sequence; 0x28 cannot continue it.
        let bytes: &[u8] = b"{\"a\":\"\xC3\x28\"}\n";
        let mut input = std::io::BufReader::new(bytes);
        let mut got = String::new();
        let error = read_line_capped(&mut input, &mut got)
            .expect_err("invalid UTF-8 must not be accepted");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(
            got.is_empty(),
            "a refused line must leave nothing behind: {got:?}"
        );
        let said = error.to_string();
        assert!(said.contains("not valid UTF-8"), "{said}");
        assert!(
            said.contains("byte 6"),
            "the refusal must say WHERE, so a caller can find it: {said}"
        );
    }

    /// A field of a `Value::Object`, for a test.
    ///
    /// Written here rather than added to `rivet_core::Value`, on
    /// `rivet_core::operation::uninstall`'s precedent: reading a field back is
    /// a thing tests do, and a method on the type would be a convenience the
    /// production code never uses.
    fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
        match value {
            Value::Object(fields) => fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// **`ADR-012` §6's test, as a test.** Every operation is a tool and every
    /// tool is an operation — so an operation added to the core appears here
    /// with no edit, and one cannot be hidden from an agent by omission.
    #[test]
    fn every_tool_is_an_operation_and_every_operation_is_a_tool() {
        let Value::List(tools) = tools() else {
            panic!("tools() is a list")
        };
        assert_eq!(tools.len(), CATALOGUE.len());
        for op in CATALOGUE {
            let name = tool_name(op.verb);
            assert_eq!(verb_named(&name), Some(op.verb), "{name}");
            assert!(
                tools.iter().any(|t| matches!(
                    field(t, "name"),
                    Some(Value::Str(s)) if *s == name
                )),
                "{name} is not published"
            );
        }
    }

    /// A tool name is the verb, mechanically, so the two cannot drift.
    #[test]
    fn a_tool_name_is_its_verb_and_nothing_else() {
        assert_eq!(tool_name(Verb::Get), "rivet_get");
        assert_eq!(verb_named("rivet_get"), Some(Verb::Get));
        assert_eq!(verb_named("get"), None, "an unprefixed name is not a tool");
        assert_eq!(verb_named("rivet_nonsense"), None);
        // A verb the catalogue does not carry is not a tool, even though it is
        // a real verb — the catalogue is the authority, not `Verb::ALL`.
        assert_eq!(verb_named("rivet_plan"), None);
    }

    /// **The descriptions are the operation's own.** An agent reading a tool
    /// and an operator reading `--help` are reading one sentence.
    #[test]
    fn a_tools_description_is_the_operations_summary_verbatim() {
        let Value::List(tools) = tools() else {
            panic!("a list")
        };
        for (tool, op) in tools.iter().zip(CATALOGUE) {
            assert_eq!(field(tool, "description"), Some(&Value::str(op.summary)));
        }
    }

    /// Every declared input reaches the schema, and the required ones are named
    /// as required.
    #[test]
    fn the_schema_publishes_every_input_the_operation_declares() {
        for op in CATALOGUE {
            let schema = schema(op.inputs);
            let properties = field(&schema, "properties").expect("properties");
            for input in op.inputs {
                let published = field(properties, input.name).unwrap_or_else(|| {
                    panic!("{} does not publish {}", op.verb.as_str(), input.name)
                });
                assert_eq!(
                    field(published, "description"),
                    Some(&Value::str(input.description))
                );
            }
            let Some(Value::List(required)) = field(&schema, "required") else {
                panic!("required is a list")
            };
            let wanted = op.inputs.iter().filter(|i| i.required).count();
            assert_eq!(required.len(), wanted, "{}", op.verb.as_str());
        }
    }

    // ---- binding arguments to words ----------------------------------------

    fn get() -> &'static Operation {
        operation::operation(Verb::Get).expect("get is catalogued")
    }

    fn args(text: &str) -> Json {
        json::parse(text).expect("a valid fixture")
    }

    /// **The words an agent sends are the words an operator would type.**
    #[test]
    fn arguments_become_the_argument_vector_in_declared_order() {
        let given = args(r#"{"boot":"previous","subject":"log"}"#);
        // Sent out of order; bound in the order the operation declares.
        assert_eq!(words(get(), Some(&given)), Ok(vec!["log".into(), "previous".into()]));
    }

    /// **The bug this refusal exists for, as a test.** These become a POSITIONAL
    /// argument vector, so before 2026-08-28 a caller who gave `limit` and not
    /// `boot` got `["log", "3"]` — and `get log` read `3` as the boot selector
    /// and answered `"boot":"3"`, with no error anywhere.
    ///
    /// It is refused rather than repaired because repairing it means this
    /// adapter inventing the value of an input it was not given, and that value
    /// is a default that already lives in the operation (`ADR-011`).
    #[test]
    fn a_given_input_after_a_skipped_one_is_refused_rather_than_shifted() {
        let given = args(r#"{"subject":"log","limit":"3"}"#);
        let refused = words(get(), Some(&given)).expect_err("a shifted argument must be refused");
        assert!(refused.contains("`limit` was given and `boot` was not"), "{refused}");
        assert!(refused.contains("silently shift"), "{refused}");

        // ...and giving both is fine, in any order.
        let ok = args(r#"{"limit":"3","subject":"log","boot":"latest"}"#);
        assert_eq!(
            words(get(), Some(&ok)),
            Ok(vec!["log".into(), "latest".into(), "3".into()])
        );
    }

    /// A trailing input may be omitted — only a HOLE is refused, not brevity.
    #[test]
    fn omitting_the_last_inputs_is_still_fine() {
        let given = args(r#"{"subject":"log","boot":"latest"}"#);
        assert_eq!(words(get(), Some(&given)), Ok(vec!["log".into(), "latest".into()]));
    }

    #[test]
    fn a_single_required_input_is_enough() {
        let given = args(r#"{"subject":"status"}"#);
        assert_eq!(words(get(), Some(&given)), Ok(vec!["status".into()]));
    }

    /// **Refused rather than discarded**, on `ADR-007` §3.6's terms: a key this
    /// build would drop is one whose sender believed it did something.
    #[test]
    fn an_input_the_operation_does_not_declare_is_refused() {
        let given = args(r#"{"subject":"status","verbose":"true"}"#);
        let e = words(get(), Some(&given)).expect_err("refused");
        assert!(e.contains("verbose"), "{e}");
        assert!(e.contains("discarded rather than acted on"), "{e}");
    }

    #[test]
    fn a_missing_required_input_says_which_and_what_it_is() {
        let e = words(get(), Some(&args("{}"))).expect_err("refused");
        assert!(e.contains("subject"), "{e}");
        // The operation's own description of it, so there is one account.
        assert!(e.contains("What to read"), "{e}");
        assert!(words(get(), None).is_err());
    }

    #[test]
    fn arguments_that_are_not_an_object_are_refused_by_shape() {
        for text in ["[]", r#""status""#, "7", "null"] {
            let e = words(get(), Some(&args(text))).expect_err(text);
            assert!(e.contains("must be an object"), "{text}: {e}");
        }
    }

    #[test]
    fn an_input_that_is_not_a_string_is_refused_by_shape() {
        let e = words(get(), Some(&args(r#"{"subject":7}"#))).expect_err("refused");
        assert!(e.contains("must be a string"), "{e}");
    }

    /// A repeated key is not decidable, so nothing is run.
    #[test]
    fn a_repeated_argument_is_refused_rather_than_resolved() {
        let e = words(get(), Some(&args(r#"{"subject":"log","subject":"status"}"#)))
            .expect_err("refused");
        assert!(e.contains("more than once"), "{e}");
        assert!(e.contains("Nothing has been run"), "{e}");
    }

    // ---- the protocol ------------------------------------------------------

    /// A board that refuses every port, so a test here that started depending
    /// on a machine fails loudly rather than passing on a fiction
    /// (`AGENTS.md` §3).
    fn board() -> crate::boot::Detached {
        crate::boot::Detached::new("a test")
    }

    fn ask(line: &str) -> String {
        match handle(line, &mut board()) {
            Answer::Reply(reply) => reply,
            Answer::Silent => panic!("{line} was answered with silence"),
        }
    }

    fn asked(line: &str) -> Json {
        let reply = ask(line);
        json::parse(&reply).unwrap_or_else(|e| panic!("the reply is not JSON: {e}\n{reply}"))
    }

    /// **The handshake.** A client that speaks this protocol gets told what it
    /// reached, and the version is this build's own.
    #[test]
    fn initialize_says_what_this_server_is() {
        let reply = asked(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#);
        assert_eq!(reply.get("jsonrpc").and_then(Json::as_str), Some("2.0"));
        assert_eq!(reply.get("id").and_then(Json::as_integer), Some(1));
        let result = reply.get("result").expect("a result");
        assert_eq!(
            result.get("protocolVersion").and_then(Json::as_str),
            Some(PROTOCOL_VERSION)
        );
        let info = result.get("serverInfo").expect("serverInfo");
        assert_eq!(info.get("name").and_then(Json::as_str), Some(SERVER_NAME));
        assert_eq!(
            info.get("version").and_then(Json::as_str),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }

    /// **No capability is declared that this build does not implement.**
    /// `ADR-006` §7's "trusted and wrong" is the failure being avoided.
    #[test]
    fn only_the_capability_this_build_has_is_declared() {
        let reply = asked(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
        let caps = reply
            .get("result")
            .and_then(|r| r.get("capabilities"))
            .expect("capabilities");
        assert!(caps.get("tools").is_some());
        assert!(caps.get("resources").is_none(), "resources is not built");
        assert!(caps.get("prompts").is_none(), "prompts is not built");
    }

    #[test]
    fn tools_list_publishes_the_catalogue() {
        let reply = asked(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let listed = reply
            .get("result")
            .and_then(|r| r.get("tools"))
            .and_then(Json::as_array)
            .expect("tools");
        assert_eq!(listed.len(), CATALOGUE.len());
        for op in CATALOGUE {
            let name = tool_name(op.verb);
            assert!(
                listed
                    .iter()
                    .any(|t| t.get("name").and_then(Json::as_str) == Some(name.as_str())),
                "{name} is not published"
            );
        }
    }

    /// **A notification is answered with silence**, which is JSON-RPC's rule
    /// rather than a shortcut — a reply to one is an error in the other
    /// direction.
    #[test]
    fn a_notification_is_answered_with_silence() {
        for line in [
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{}}"#,
            // One this build has never heard of. A notification asks for
            // nothing, so not knowing it is not worth a message to ignore.
            r#"{"jsonrpc":"2.0","method":"notifications/from-the-future"}"#,
        ] {
            assert_eq!(handle(line, &mut board()), Answer::Silent, "{line}");
        }
    }

    /// A request with no id is a notification whatever its method, so even a
    /// failure is silent.
    #[test]
    fn a_request_with_no_id_gets_no_reply_even_when_it_fails() {
        assert_eq!(
            handle(r#"{"jsonrpc":"2.0","method":"nonsense"}"#, &mut board()),
            Answer::Silent
        );
    }

    #[test]
    fn a_string_id_comes_back_as_a_string() {
        let reply = asked(r#"{"jsonrpc":"2.0","id":"abc","method":"ping"}"#);
        assert_eq!(reply.get("id").and_then(Json::as_str), Some("abc"));
    }

    // ---- protocol failures, which are not refusals -------------------------

    #[test]
    fn a_line_that_is_not_json_is_answered_rather_than_fatal() {
        let reply = asked("{not json");
        let e = reply.get("error").expect("an error");
        assert_eq!(e.get("code").and_then(Json::as_integer), Some(code::PARSE));
        // Answered with a null id, because there was no id to read.
        assert_eq!(reply.get("id"), Some(&Json::Null));
    }

    #[test]
    fn a_message_that_is_not_a_jsonrpc_request_is_refused_by_shape() {
        for (line, wanted) in [
            (r#"{"id":1,"method":"ping"}"#, code::INVALID_REQUEST),
            (r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#, code::INVALID_REQUEST),
            (r#"{"jsonrpc":"2.0","id":1}"#, code::INVALID_REQUEST),
            (r#"{"jsonrpc":"2.0","id":1,"method":7}"#, code::INVALID_REQUEST),
        ] {
            let reply = asked(line);
            assert_eq!(
                reply.get("error").and_then(|e| e.get("code")).and_then(Json::as_integer),
                Some(wanted),
                "{line}"
            );
        }
    }

    #[test]
    fn a_method_this_build_does_not_answer_says_which_it_does() {
        let reply = asked(r#"{"jsonrpc":"2.0","id":1,"method":"resources/list"}"#);
        let e = reply.get("error").expect("an error");
        assert_eq!(
            e.get("code").and_then(Json::as_integer),
            Some(code::METHOD_NOT_FOUND)
        );
        let said = e.get("message").and_then(Json::as_str).expect("a message");
        assert!(said.contains("tools/list"), "{said}");
    }

    /// **A tool that is not on the menu is a client bug**, so it is a protocol
    /// error rather than a refusal — the client asked for something it was
    /// never offered.
    #[test]
    fn a_tool_that_was_never_offered_is_a_protocol_error() {
        let reply = asked(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"rivet_wat"}}"#,
        );
        assert_eq!(
            reply.get("error").and_then(|e| e.get("code")).and_then(Json::as_integer),
            Some(code::INVALID_PARAMS)
        );
    }

    // ---- refusals, which are not protocol failures -------------------------

    /// **The distinction this adapter turns on.** Rivet saying no is a
    /// SUCCESSFUL call carrying `isError`; only a broken message is an `error`.
    /// Collapsing the two would tell an agent Rivet is broken when it refused.
    #[test]
    fn a_refusal_is_a_successful_call_marked_an_error_and_not_a_protocol_failure() {
        let reply = asked(
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call",
                "params":{"name":"rivet_get","arguments":{"subject":"nonsense"}}}"#,
        );
        assert!(reply.get("error").is_none(), "a refusal became a protocol error");
        let result = reply.get("result").expect("a result");
        assert_eq!(result.get("isError"), Some(&Json::Bool(true)));
        // And the refusal is the core's own object, carried whole.
        let text = result
            .get("content")
            .and_then(Json::as_array)
            .and_then(|c| c.first())
            .and_then(|c| c.get("text"))
            .and_then(Json::as_str)
            .expect("text");
        assert!(text.contains("E_"), "{text}");
    }

    /// An argument that is not an input is refused as a tool result, because
    /// the message was well-formed and Rivet is the one saying no.
    #[test]
    fn a_bad_argument_is_a_tool_refusal_rather_than_a_protocol_error() {
        let reply = asked(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name":"rivet_get","arguments":{"nonsense":"x"}}}"#,
        );
        assert!(reply.get("error").is_none());
        assert_eq!(
            reply.get("result").and_then(|r| r.get("isError")),
            Some(&Json::Bool(true))
        );
    }

    // ---- a whole session ---------------------------------------------------

    /// **End to end, over the streams the transport actually gives it.** A
    /// handshake, a listing and a call, one line each, with the replies read
    /// back as JSON.
    #[test]
    fn a_whole_session_reads_and_writes_one_message_per_line() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            "\n",
        );
        let mut reader = std::io::BufReader::new(input.as_bytes());
        let mut out: Vec<u8> = Vec::new();
        serve(&mut reader, &mut out, &mut board()).expect("the session runs to EOF");

        let text = String::from_utf8(out).expect("utf-8");
        let lines: Vec<&str> = text.lines().collect();
        // Two requests, one notification, one blank line -> two replies.
        assert_eq!(lines.len(), 2, "{text}");
        for line in &lines {
            let reply = json::parse(line).expect("each line is one JSON document");
            assert_eq!(reply.get("jsonrpc").and_then(Json::as_str), Some("2.0"));
        }
    }

    /// **One bad line does not cost the session.** A client that sends
    /// something malformed gets a parse error and keeps going; ending the
    /// session would make one bad message cost every one after it.
    #[test]
    fn a_malformed_line_is_answered_and_the_session_continues() {
        let input = "not json at all\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n";
        let mut reader = std::io::BufReader::new(input.as_bytes());
        let mut out: Vec<u8> = Vec::new();
        serve(&mut reader, &mut out, &mut board()).expect("runs to EOF");

        let text = String::from_utf8(out).expect("utf-8");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert!(lines[0].contains("-32700"), "{}", lines[0]);
        assert!(lines[1].contains("\"result\""), "{}", lines[1]);
    }

    /// A client that never sends a newline cannot grow this process without
    /// bound — the other end is a network.
    #[test]
    fn a_message_with_no_newline_is_capped_rather_than_read_for_ever() {
        let flood = "x".repeat(MAX_MESSAGE_BYTES + 16);
        let mut reader = std::io::BufReader::new(flood.as_bytes());
        let mut out: Vec<u8> = Vec::new();
        let e = serve(&mut reader, &mut out, &mut board()).expect_err("capped");
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
    }

    /// Every reply is one line, because the framing is line-delimited and a
    /// reply carrying a raw newline would be two messages to the reader.
    #[test]
    fn no_reply_ever_carries_a_raw_newline() {
        for line in [
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"rivet_get","arguments":{"subject":"nonsense"}}}"#,
        ] {
            let reply = ask(line);
            assert!(!reply.contains('\n'), "a reply spans lines: {reply}");
        }
    }

    /// **There is no confirmation logic in this module**, and that is the
    /// design rather than an omission: `perform` already returns a plan for an
    /// unconfirmed change (`ADR-012` §7.1), so an agent gets `ADR-012` §3's
    /// safety mechanism for free. A reader looking for it here should find
    /// this test instead.
    #[test]
    fn this_adapter_contains_no_confirmation_logic_of_its_own() {
        let source = include_str!("mcp.rs");
        let body = source.split("#[cfg(test)]").next().expect("a body");
        assert!(!body.contains("\"confirm\""), "this adapter knows the word");
    }
}
