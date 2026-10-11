//! What the TLS transport reads, and what it refuses without a board.
//!
//! The refusals matter as much as the reader: this transport must never reach
//! for another channel, and every method it cannot serve has to say which phase
//! lands it rather than failing vaguely.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;

// ---------------------------------------------------------------------------
// the transfer, and the defect where a success was reported as a failure
// ---------------------------------------------------------------------------

/// **A socket that takes `accepts` bytes and then aborts**, with an answer
/// already waiting to be read.
///
/// This is `rivet-01` on 2026-09-12 reproduced in one struct: the machine
/// short-circuited a re-send of an artifact it already held, answered `201`
/// without draining the body, and hung up — which resets a connection whose peer
/// is still writing. The client's `write_all` failed at 196608 bytes, three
/// 64 KiB buffers in, and it called a transfer that had succeeded an error.
///
/// **The answer is readable after the abort here, and on a real socket it is
/// NOT.** Measured, Windows host over loopback, 2026-09-12: a server that
/// answered and closed without draining left the client's read returning zero
/// bytes and the same abort, twice out of two; the same server draining first
/// gave a clean send and a readable `201`. So this fixture is deliberately the
/// *favourable* case — it asserts what this end does when an answer survives a
/// failed write, which is the part of the exchange a client decides.
/// Whether one survives is settled on the other side, by `cmd/rivet-mcpd`'s
/// `bulk::drain`.
struct AbortsAfter {
    accepts: usize,
    written: usize,
    /// What the peer has already put on the wire, read back after the abort.
    answer: std::io::Cursor<Vec<u8>>,
}

impl AbortsAfter {
    fn new(accepts: usize, answer: &str) -> Self {
        Self {
            accepts,
            written: 0,
            answer: std::io::Cursor::new(answer.as_bytes().to_vec()),
        }
    }
}

impl std::io::Write for AbortsAfter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.written >= self.accepts {
            // The word Windows uses, verbatim from the board run.
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                "An established connection was aborted",
            ));
        }
        let took = buf.len().min(self.accepts - self.written);
        self.written += took;
        Ok(took)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl std::io::Read for AbortsAfter {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.answer.read(buf)
    }
}

/// The `201` the bulk endpoint composes for an artifact it already holds,
/// abbreviated to the lines this end reads.
fn a_201(extra: &str) -> String {
    let body = format!(
        "/state/artifacts/objects/ab12/thing.rvt\n\
         \x20 bytes   402198 (not written — this machine already holds this digest)\n\
         {extra}\
         \x20 next    nothing has been installed.\n"
    );
    format!(
        "HTTP/1.1 201 Created\r\nContent-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn a_transfer_into(stream: &mut AbortsAfter, body: &[u8]) -> Result<String, String> {
    let mut source = std::io::Cursor::new(body.to_vec());
    transfer(
        stream,
        "PUT /payload/thing.rvt HTTP/1.1\r\n\r\n",
        &mut source,
        body.len() as u64,
        std::path::Path::new("thing.rvt"),
        "board.local:7777",
        "thing.rvt",
    )
}

/// **The regression, and it is the whole of defect 1 from this end.** A write
/// that failed after the machine answered is a SUCCESS, because the machine is
/// the authority on whether it received the payload and it has said so.
///
/// Before this, `write_all`'s error was returned immediately and the answer
/// sitting in the receive buffer was never read. The deploy then stopped, and
/// the install the transfer had earned was never asked for.
#[test]
fn a_write_that_fails_after_the_machine_answered_is_a_success() {
    let body = vec![7u8; 402_198];
    // The head, then three 64 KiB buffers, then the abort — the board's numbers.
    let mut stream = AbortsAfter::new(36 + 196_608, &a_201(""));

    let said = a_transfer_into(&mut stream, &body)
        .expect("the machine answered 201 and that is the authority on this");
    assert!(said.contains("already holds this digest"), "{said}");
    // And it says the send failed, so an operator is not told a clean transfer
    // happened when one did not.
    assert!(said.contains("note"), "{said}");
    assert!(said.contains("An established connection was aborted"), "{said}");
    assert!(said.contains("196608 of 402198 bytes"), "{said}");
}

/// A transfer that succeeds end to end is unchanged: the answer comes back and
/// carries no note about anything unusual.
#[test]
fn a_transfer_that_completes_answers_with_the_machines_body_and_no_note() {
    let body = vec![3u8; 1024];
    let mut stream = AbortsAfter::new(usize::MAX, &a_201(" pruned  old-thing.rvt\n"));

    let said = a_transfer_into(&mut stream, &body).expect("a clean transfer");
    assert!(said.contains("pruned  old-thing.rvt"), "{said}");
    assert!(!said.contains("note"), "a clean transfer was annotated: {said}");
}

/// **A refusal is still a refusal.** The rule is *read the answer*, not
/// *believe the transfer worked* — a machine that answered `400` is reported as
/// refusing, whether or not this end's write also failed.
#[test]
fn a_machine_that_refused_is_reported_as_refusing_even_when_the_write_failed() {
    let body = vec![1u8; 100_000];
    let refusal = "HTTP/1.1 400 Bad Request\r\nContent-Length: 24\r\n\r\nno Rivet-Blake3 header\n\n";
    let mut stream = AbortsAfter::new(1024, refusal);

    let err = a_transfer_into(&mut stream, &body).expect_err("400 is not a success");
    assert!(err.contains("refused the payload"), "{err}");
    assert!(err.contains("Rivet-Blake3"), "{err}");
}

/// **A refusal at the door is reported as the ARTIFACT being refused, never as
/// the transfer failing** — the mirror image of the 2026-09-12 defect, where a
/// success was reported as a failure. Here a transfer that worked ends in a
/// refusal that is about the artifact, and an operator told "the transfer
/// failed" would retry something that will be refused identically.
///
/// The answer is the shape `cmd/rivet-mcpd`'s `refused_at_the_door` composes.
#[test]
fn a_refusal_at_the_door_says_the_artifact_was_refused_and_the_transfer_was_intact() {
    let body = vec![1u8; 4096];
    let said = "REFUSED AT THE DOOR: thing.rvt is not an artifact this machine would install, so \
                it is NOT IN THIS MACHINE'S ARTIFACT STORE.\n\
                \x20 why      this package is signed by key 863f6c8a…, which is not in this \
                machine's trust set\n";
    let answer = format!(
        "HTTP/1.1 422 Unprocessable Content\r\nContent-Length: {}\r\n\r\n{said}",
        said.len()
    );
    let mut stream = AbortsAfter::new(usize::MAX, &answer);

    let err = a_transfer_into(&mut stream, &body).expect_err("a refused artifact is not a success");
    assert!(err.contains("REFUSED THE ARTIFACT thing.rvt"), "{err}");
    assert!(err.contains("not a transfer failure"), "{err}");
    assert!(err.contains("not in this machine's trust set"), "the WHY was lost: {err}");
    assert!(!err.contains("the transfer failed"), "{err}");
}

/// **And a write that fails with NO answer is still an error**, naming what is
/// not known. This is the case a client cannot repair — a reset that reaches
/// this process before the answer does destroys the answer — and it must not be
/// dressed up as a success by a rule that was written for the other case.
#[test]
fn a_write_that_fails_with_nothing_to_read_is_an_error_that_says_what_is_unknown() {
    let body = vec![1u8; 100_000];
    let mut stream = AbortsAfter::new(1024, "");

    let err = a_transfer_into(&mut stream, &body).expect_err("nothing came back");
    assert!(err.contains("the transfer failed after"), "{err}");
    assert!(err.contains("is NOT known from this end"), "{err}");
    assert!(err.contains("get file"), "{err}");
}

/// **The same function over a REAL socket**, against a peer that behaves the way
/// `cmd/rivet-mcpd` behaves since its `bulk::drain` landed: it reads the whole
/// declared body, answers, and closes cleanly.
///
/// The fixtures above are stand-ins and prove the decisions; this proves the
/// function survives a `TcpStream` — 8 MiB, which is far past any socket buffer,
/// so the write really does block and really does complete.
///
/// **The opposite case — a peer that answers and hangs up mid-body — is
/// deliberately NOT a test here**, because its outcome depends on how much the
/// operating system's buffers happened to absorb, and a test whose result is a
/// race is worse than none. It was measured instead, by this exact code on this
/// exact fixture with the drain removed, on Windows, 2026-09-12:
///
/// ```text
/// Err("the transfer failed after 65536 of 8388608 bytes: An existing connection
///      was forcibly closed by the remote host. (os error 10054)
///        and   nothing could be read back afterwards, …")
/// ```
///
/// Nothing came back. **That is the finding that decides where the fix lives**:
/// the drain is the remedy and reading after a failed write is a defence beside
/// it, not a substitute for it.
#[test]
fn a_transfer_over_a_real_socket_sends_the_whole_body_and_reads_the_answer() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let port = listener.local_addr().expect("an address").port();
    let size = 8 * 1024 * 1024;

    let server = std::thread::spawn(move || -> usize {
        let (mut conn, _) = listener.accept().expect("a connection");
        let mut head = Vec::new();
        let mut chunk = vec![0u8; 64 * 1024];
        let at = loop {
            if let Some(at) = head.windows(4).position(|w| w == b"\r\n\r\n") {
                break at;
            }
            let n = conn.read(&mut chunk).expect("the request head");
            head.extend_from_slice(&chunk[..n]);
        };
        // Drain, as the endpoint does, then answer and close cleanly.
        let mut got = head.len() - at - 4;
        while got < size {
            match conn.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => got += n,
            }
        }
        conn.write_all(a_201(" pruned  old-thing.rvt\n").as_bytes()).expect("the answer");
        conn.flush().expect("the answer reaches the wire");
        got
    });

    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("the loopback peer");
    let body = vec![7u8; size];
    let mut source = std::io::Cursor::new(body);
    let said = transfer(
        &mut stream,
        &format!("PUT /payload/thing.rvt HTTP/1.1\r\nContent-Length: {size}\r\n\r\n"),
        &mut source,
        size as u64,
        std::path::Path::new("thing.rvt"),
        "127.0.0.1",
        "thing.rvt",
    )
    .expect("a peer that drains and answers");

    assert_eq!(server.join().expect("the server thread"), size, "the body was truncated");
    assert!(said.contains("pruned  old-thing.rvt"), "{said}");
    assert!(!said.contains("note"), "a clean exchange was annotated: {said}");
}

/// A real answer from `rivet mcp`, copied off the board on 2026-08-28.
const REAL_ANSWER: &str = concat!(
    r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"#,
    r#""{\"subject\":\"status\",\"phase\":\"converging\"}"}],"isError":false}}"#
);

#[test]
fn the_text_of_a_real_answer_is_read_and_unescaped() {
    let text = tool_result(REAL_ANSWER).expect("a successful call");
    assert_eq!(text, r#"{"subject":"status","phase":"converging"}"#);
}

/// Escapes JSON actually uses, since the answers are JSON inside JSON.
#[test]
fn the_reader_undoes_the_escapes_that_arrive() {
    let line = r#"{"result":{"content":[{"text":"a\nb\tc\\d\"e§f"}],"isError":false}}"#;
    assert_eq!(extract_text(line), Some("a\nb\tc\\d\"e§f".to_string()));
}

/// **`isError` is honoured**, so a tool that refused is an `Err` here and not a
/// success whose text happens to describe a failure.
#[test]
fn a_tool_that_reported_an_error_is_an_error() {
    let line = r#"{"id":2,"result":{"content":[{"text":"E_ARGUMENTS: no"}],"isError":true}}"#;
    let refused = tool_result(line).expect_err("isError must not be ignored");
    assert_eq!(refused, "E_ARGUMENTS: no");
}

/// A JSON-RPC level error is distinguished from a tool-level one.
#[test]
fn a_protocol_error_is_reported_as_the_machine_refusing_the_call() {
    let line = r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32602,"message":"no such tool"}}"#;
    let refused = tool_result(line).expect_err("an error object must be refused");
    assert!(refused.contains("refused the call"), "{refused}");
    assert!(refused.contains("no such tool"), "{refused}");
}

/// **It refuses a shape it does not read, rather than degrading.** A reader for
/// one message that behaved like a parser would answer confidently and wrongly.
#[test]
fn an_answer_of_another_shape_is_refused_with_the_line_quoted() {
    let line = r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[]}}"#;
    let refused = tool_result(line).expect_err("an unread shape must be refused");
    assert!(refused.contains("not the shape this build reads"), "{refused}");
    assert!(refused.contains("content[0].text"), "{refused}");
    assert!(refused.contains(line), "the refusal must quote what came: {refused}");
}

/// The same closed rule the endpoint applies, checked before 683 MB is sent.
#[test]
fn a_path_the_endpoint_would_refuse_is_refused_before_anything_is_sent() {
    assert_eq!(payload_name("/state/rivet-0c17f74-aarch64-rpi4-update.rvt"),
               Ok("rivet-0c17f74-aarch64-rpi4-update.rvt"));
    for bad in [
        "/state/../etc/passwd",
        "/state/sub/dir.rvt",
        "/state/.hidden",
        "/tmp/elsewhere.rvt",
        "/state/",
    ] {
        assert!(payload_name(bad).is_err(), "{bad} should be refused");
    }
}

fn a_transport() -> Tls {
    Tls::new(
        "127.0.0.1:1".to_string(),
        rivetos_tls::Identity { certificate: vec![1, 2, 3], seed: [7u8; 32] },
        vec![[9u8; 32]],
    )
}

/// **`blake3` never invents an answer.** The check is folded into the transfer,
/// so a path this session did not push has no digest and says so.
#[test]
fn a_digest_is_refused_for_a_path_this_session_did_not_push() {
    let mut remote = a_transport();
    let refused = remote
        .blake3("/state/never-sent.rvt")
        .expect_err("no digest can be known for it");
    assert!(refused.contains("has not sent"), "{refused}");
    assert!(refused.contains("no `measure` verb"), "{refused}");
}

/// `ask` explains the shape difference rather than reporting a bare failure —
/// MCP is a catalogue and `ask` takes a word list because the transports it was
/// written for handed over a shell.
#[test]
fn ask_explains_why_a_word_list_is_not_a_tool_call() {
    let mut remote = a_transport();
    let said = remote.ask(&["get", "status"]).expect_err("refused");
    assert!(said.contains("rivet get status"), "{said}");
    assert!(said.contains("CATALOGUE"), "{said}");
    assert!(said.contains("ADR-006 §3.1"), "{said}");
}

/// Hex is what the header carries, and the endpoint parses exactly this.
#[test]
fn the_digest_header_is_lower_case_hex() {
    assert_eq!(hex(&[0xde, 0xad, 0x00, 0xff]), "dead00ff");
    assert_eq!(hex(&[0u8; 32]).len(), 64);
}

// ---------------------------------------------------------------------------
// The desired-state document: two captured answers, then a fake `rivet mcp`
// ---------------------------------------------------------------------------
//
// **Provenance, because `AGENTS.md` §4 wants it visible in the file.** The two
// constants below were NOT composed here and did NOT come off the board. They
// are what `rivet mcp` — this workspace's own binary, `cargo build -p
// rivetos-cli`, version 0.1.8 — wrote to its stdout on 2026-08-29 when the
// three-line script `Tls::call` sends was piped into it on the development
// host. That is the same code the board runs; what differs is the machine under
// it. Nothing in this file has been verified against a board.
//
// **One hand edit, recorded so the word "captured" stays true.** These were
// captured while the subject was still spelled `document`; it was renamed to
// `desired-state` the same day (`ADR-006` §8.11) by a change that landed
// alongside this one. The subject word — and only the subject word — was
// rewritten here afterwards. Every byte that matters to what these tests prove
// (the escaping, the lengths, the `present` flag, the non-ASCII round trip) is
// the binary's own output, unedited. Re-capture on the next build and this note
// goes away.

/// `rivet get document` where there is no document.
///
/// The `present: false` case, which `PLAN.md` §91.4 records as never having run
/// on hardware — the board has one. Here it is ordinary: the host has no
/// `/state/config/machine.toml`.
const ABSENT: &str = concat!(
    r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"#,
    r#""{\"subject\":\"desired-state\",\"path\":\"/state/config/machine.toml\","#,
    r#"\"present\":false,\"contents\":\"\",\"bytes\":0}"}],"isError":false}}"#,
);

/// `rivet get document` reading [`CAPTURED_DOCUMENT`] off the disk.
///
/// **Every character this module's escaping has to survive is in it**: quotes
/// and backslashes that the file itself carries escaped, a literal tab, several
/// newlines, and non-ASCII from three widths — `é` (two bytes), `§` (two), and
/// `🚀`, which is above the BMP and arrives as itself rather than as a surrogate
/// pair.
const PRESENT: &str = concat!(
    r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"#,
    r#""{\"subject\":\"desired-state\",\"path\":\"/state/config/machine.toml\","#,
    r#"\"present\":true,\"contents\":\"version = 1\\n\\n[services.photo-index]"#,
    r#"\\nenabled = false\\nnote = \\\"a \\\\\\\"quoted\\\\\\\" word, a backslash "#,
    r#"\\\\\\\\ and a tab\\there\\\"\\ncomment = \\\"café § 🚀\\\"\\n\",\"bytes\":136}"#,
    r#""}],"isError":false}}"#,
);

/// The file on disk that [`PRESENT`] is the machine's answer about.
const CAPTURED_DOCUMENT: &str = "version = 1\n\n[services.photo-index]\nenabled = false\n\
     note = \"a \\\"quoted\\\" word, a backslash \\\\ and a tab\there\"\n\
     comment = \"café § 🚀\"\n";

/// **The two layers come apart in the right order and give back the file.**
#[test]
fn a_captured_answer_decodes_to_the_document_that_was_on_disk() {
    let answer = tool_result(PRESENT).expect("a successful call");
    let got = document_from(&answer).expect("a readable answer").expect("a document");
    assert_eq!(got, CAPTURED_DOCUMENT);
    // The machine counted 136 bytes; if this end had lost a backslash or turned
    // a `\n` into an `n` the length would differ, which is what `document_from`
    // checks and what this asserts independently.
    assert_eq!(got.len(), 136);
}

/// **No document is `Ok(None)`, and never an empty string.** `--enable` refuses
/// on `None` and would happily append to `Some("")`.
#[test]
fn a_machine_with_no_document_answers_none_rather_than_empty_text() {
    let answer = tool_result(ABSENT).expect("a successful call");
    assert_eq!(document_from(&answer), Ok(None));
}

/// A length that disagrees with the text is a refusal, not a shrug. This is the
/// tripwire on both unescaping layers at once.
#[test]
fn a_document_that_decodes_to_the_wrong_length_is_refused() {
    let answer = tool_result(PRESENT).expect("a successful call").replace("136", "999");
    let refused = document_from(&answer).expect_err("the length disagrees");
    assert!(refused.contains("999"), "{refused}");
    assert!(refused.contains("nearly right"), "{refused}");
}

/// An answer about something else is refused rather than read for a `contents`
/// field that would mean something different.
#[test]
fn an_answer_about_another_subject_is_refused() {
    let answer = r#"{"subject":"storage","present":true,"contents":"x","bytes":1}"#;
    let refused = document_from(answer).expect_err("a different question");
    assert!(refused.contains("different question"), "{refused}");
}

/// Fields this build needs are refused when absent — never assumed.
#[test]
fn an_answer_missing_present_or_contents_is_refused_rather_than_assumed() {
    let no_present = r#"{"subject":"desired-state","contents":"x","bytes":1}"#;
    let refused = document_from(no_present).expect_err("no `present`");
    assert!(refused.contains("will not assume"), "{refused}");

    let no_contents = r#"{"subject":"desired-state","present":true,"bytes":1}"#;
    let refused = document_from(no_contents).expect_err("no `contents`");
    assert!(refused.contains("no `contents` string"), "{refused}");
}

// ---- the escaping, in both directions -------------------------------------

/// **Outbound.** The table this writes is multi-line, and a raw newline inside a
/// JSON string is a call the machine refuses whole.
#[test]
fn a_two_line_table_survives_as_a_json_string() {
    let escaped = json_escape("\n[services.x]\nenabled = true\n");
    assert!(!escaped.contains('\n'), "a raw newline is not legal in a JSON string");
    assert_eq!(escaped, "\\n[services.x]\\nenabled = true\\n");
}

/// RFC 8259 §7, including the parts `cmd/rivet`'s writer names as the ones
/// usually got wrong: every code point below 0x20, and `DEL` left alone.
#[test]
fn the_characters_json_reads_specially_are_escaped() {
    assert_eq!(json_escape("a\"b"), "a\\\"b");
    assert_eq!(json_escape("a\\b"), "a\\\\b");
    assert_eq!(json_escape("a\tb"), "a\\tb");
    assert_eq!(json_escape("a\rb"), "a\\rb");
    assert_eq!(json_escape("a\u{08}b\u{0c}c"), "a\\bb\\fc");
    assert_eq!(json_escape("a\u{01}b\u{1f}c"), "a\\u0001b\\u001fc");
    assert_eq!(json_escape("a\u{7f}b"), "a\u{7f}b", "DEL is legal unescaped");
}

/// **Non-ASCII leaves as `\uXXXX`, and the reason is a measured defect** — see
/// [`json_escape`]. `cmd/rivet`'s `read_line_capped` promotes each byte of a
/// message to a code point, so `é` sent as itself arrives as `Ã©`.
#[test]
fn non_ascii_leaves_as_escapes_because_the_other_end_reads_bytes_as_characters() {
    assert_eq!(json_escape("café"), "caf\\u00e9");
    assert_eq!(json_escape("§"), "\\u00a7");
    // Above the BMP, so a surrogate pair — which `cmd/rivet`'s parser
    // reassembles; measured, not assumed.
    assert_eq!(json_escape("🚀"), "\\ud83d\\ude80");
    // And the whole of it is ASCII, which is the property that matters.
    assert!(json_escape("café § 🚀 — ünïcode").is_ascii());
}

/// What leaves as an escape has to come back as the character. The two halves
/// of this module's escaping are inverses, checked here against each other and
/// against the captured answer above independently.
#[test]
fn what_this_escapes_is_what_the_reader_gives_back() {
    for text in [
        "café § 🚀 — ünïcode",
        "a\tb\nc\\d\"e",
        "\u{01}\u{1f}\u{08}\u{0c}",
        "[services.x]\nenabled = true\n",
    ] {
        let line = format!(r#"{{"contents":"{}"}}"#, json_escape(text));
        assert_eq!(json_string_field(&line, "contents"), Ok(text.to_owned()), "{text:?}");
    }
}

/// **Inbound.** Every escape JSON has, including the two this project's own
/// writer never emits.
#[test]
fn the_reader_undoes_every_escape_json_has() {
    let line = r#"{"contents":"a\nb\tc\\d\"e\/f\bg\fh\u00e9i\u0001j\ud83d\ude80k"}"#;
    assert_eq!(
        json_string_field(line, "contents"),
        Ok("a\nb\tc\\d\"e/f\u{08}g\u{0c}h\u{e9}i\u{01}j\u{1f680}k".to_owned())
    );
}

/// **A wrong decode succeeds and writes a wrong document**, so everything this
/// reader does not understand is an error naming what it found.
#[test]
fn the_reader_refuses_what_it_does_not_understand() {
    for (line, expected) in [
        (r#"{"contents":"a\qb"}"#, "not one JSON defines"),
        (r#"{"contents":"a\u00zzb"}"#, "four hexadecimal digits"),
        (r#"{"contents":"a\ud83dz"}"#, "the other half is missing"),
        (r#"{"contents":"a\ud83d\u0041"}"#, "not a low surrogate"),
        (r#"{"contents":"a\ude80b"}"#, "no high surrogate before it"),
        (r#"{"contents":"unterminated"#, "never closed"),
        ("{\"contents\":\"a\u{01}b\"}", "a raw control character"),
    ] {
        let refused = json_string_field(line, "contents")
            .expect_err(&format!("{line} must be refused"));
        assert!(refused.contains(expected), "{line}\ngot: {refused}");
    }
}

/// The key search is safe because a bare `"` cannot occur inside a value — see
/// [`json_string_field`]'s note. A document that *contains* the later keys must
/// not shift the reader onto its own text.
#[test]
fn a_document_that_contains_the_later_keys_is_still_read_correctly() {
    let document = "trap = \"\\\"bytes\\\":42\"\n";
    let answer = format!(
        r#"{{"subject":"desired-state","path":"/state/config/machine.toml","present":true,"contents":"{}","bytes":{}}}"#,
        json_escape(document),
        document.len(),
    );
    assert_eq!(document_from(&answer), Ok(Some(document.to_owned())));
}

// ---- the migration to `rivet_core::json`: order, nesting, whitespace ------
//
// `json_string_field` and its siblings used to find a key by searching the
// raw text for its bytes. That is order- and whitespace-sensitive in ways
// real JSON is not, and blind to the difference between a document's own
// field and a same-named one nested inside a list it carries. The tests
// below drive each reader with inputs a hand-rolled substring search gets
// wrong, through `rivet_core::json` instead.

/// **The bug this migration exists to fix.** `get file` writes the path's own
/// `type` before `entries`, but nothing requires that order, and every entry
/// in `entries` carries a `type` of its own. Written here with `entries`
/// FIRST, so the textually first `"type":"` in the raw answer is a directory
/// entry's rather than the path's — a substring search answers for the wrong
/// object; the structured reader reads the top-level field only, regardless
/// of where in the text it sits.
#[test]
fn a_nested_field_of_the_same_name_does_not_shadow_the_top_level_one() {
    let answer = r#"{"entries":[{"name":"a","type":"file"}],"type":"directory","present":true}"#;
    assert_eq!(json_string_field(answer, "type"), Ok("directory".to_owned()));
}

/// [`json_string_field_all`] finds `name` wherever it recurs — the shape
/// `get file` answers a directory with, one `name` per entry, nested one
/// level under `entries` rather than at the document's own top level.
#[test]
fn json_string_field_all_finds_a_repeated_key_nested_in_an_array() {
    let answer = r#"{"type":"directory","entries":[{"name":"a","type":"file"},{"name":"b","type":"directory"}]}"#;
    assert_eq!(
        json_string_field_all(answer, "name"),
        Ok(vec!["a".to_owned(), "b".to_owned()])
    );
}

/// JSON permits whitespace around every structural character; a needle that
/// requires exact adjacency does not, so any encoder that formats its output
/// differently from this one's own — a pretty-printer, a future rewrite —
/// would have silently broken every field read here. The structured reader
/// follows the grammar instead of one encoder's habits.
#[test]
fn whitespace_around_colons_and_commas_does_not_defeat_a_field_read() {
    let answer =
        r#"{ "subject" : "desired-state" , "present" : true , "contents" : "hi" , "bytes" : 2 }"#;
    assert_eq!(json_string_field(answer, "subject"), Ok("desired-state".to_owned()));
    assert_eq!(json_bool_field(answer, "present"), Ok(true));
    assert_eq!(json_string_field(answer, "contents"), Ok("hi".to_owned()));
}

/// A document that never closes is refused as a whole rather than mined for
/// whichever fields happen to appear before the cut. A truncated answer is
/// exactly the case a field must not be read from with confidence — the very
/// field being asked about might be what got cut — and a needle search that
/// never checks the document is complete cannot tell the difference.
#[test]
fn a_truncated_document_is_refused_rather_than_mined_for_the_fields_before_the_cut() {
    let answer = r#"{"subject":"desired-state","present":true"#;
    assert!(json_string_field(answer, "subject").is_err(), "a cut document named a field");
    assert!(json_bool_field(answer, "present").is_err(), "a cut document named a field");
}

/// A character above the Basic Multilingual Plane, as JSON must spell one: a
/// surrogate pair. Already correct before this migration — kept as coverage
/// of the case named explicitly in this repair's acceptance criteria, now
/// proven through the shared reader rather than this module's own decoder.
#[test]
fn a_non_bmp_character_escaped_as_a_surrogate_pair_reads_correctly() {
    let answer = r#"{"contents":"🚀"}"#;
    assert_eq!(json_string_field(answer, "contents"), Ok("🚀".to_owned()));
}

/// An escaped quote at either end of a value. Already correct before this
/// migration — see `a_document_that_contains_the_later_keys_is_still_read_correctly`
/// above for the case that motivated the original needle's escape-awareness —
/// kept here as direct coverage of [`json_string_field`] alone rather than
/// through [`document_from`].
#[test]
fn an_escaped_quote_at_either_end_of_a_value_reads_correctly() {
    let answer = r#"{"contents":"\"quoted\" at both ends\""}"#;
    assert_eq!(
        json_string_field(answer, "contents"),
        Ok("\"quoted\" at both ends\"".to_owned())
    );
}

/// `isError` with the whitespace any conforming JSON encoder may use — a
/// literal `"isError":true` match would miss it, silently turning a tool's
/// reported refusal into a reported success.
#[test]
fn tool_result_honours_is_error_regardless_of_whitespace() {
    let line = r#"{"id":2,"result":{"content":[{"text":"boom"}],"isError": true}}"#;
    let refused = tool_result(line).expect_err("isError must still be honoured with whitespace");
    assert_eq!(refused, "boom");
}

/// Text that is not JSON at all is refused the same way an oddly-shaped but
/// valid document is — this reader does not distinguish "not JSON" from "not
/// the JSON it wants" any more than the substring version did.
#[test]
fn tool_result_refuses_text_that_is_not_json_at_all() {
    let refused = tool_result("not json at all").expect_err("garbage must be refused");
    assert!(refused.contains("not the shape this build reads"), "{refused}");
}

// ---- a `rivet mcp` that is not there ---------------------------------------

/// **A fake board, at the `tools/call` boundary.**
///
/// It answers the two tools this transport calls, keeps a document, and — the
/// part that matters — **composes its answers the way the machine does**:
/// the operation's JSON, escaped into `content[0].text`, escaped again into the
/// wire line, which then goes through [`tool_result`] exactly as
/// [`Tls::call`] would hand it over. Both layers are therefore exercised.
///
/// Its escaper and its reader are written here rather than borrowed from the
/// module under test — `AGENTS.md` §4: *a test that round-trips a reader against
/// its own writer proves the two agree with each other and nothing about the
/// format.*
struct FakeMcp {
    document: std::cell::RefCell<Option<String>>,
    /// Every `(tool, arguments)` this was asked, in order.
    calls: std::cell::RefCell<Vec<(String, String)>>,
    /// What to put in the document the moment after a read answers — the
    /// concurrent editor that makes [`put_document_via`]'s compare-and-swap
    /// something other than theatre.
    race: std::cell::RefCell<Option<String>>,
    /// Read every word of an incoming call the way `cmd/rivet`'s
    /// `read_line_capped` does — one code point per BYTE. That is the defect
    /// [`json_escape`] defends against, reproduced here so the defence and the
    /// read-back have something real to fail against.
    mangle: std::cell::RefCell<bool>,
}

/// Every byte of `text` promoted to a code point, which is what
/// `char::from(byte)` does to UTF-8.
fn as_latin1(text: &str) -> String {
    text.bytes().map(char::from).collect()
}

impl FakeMcp {
    fn holding(document: Option<&str>) -> Self {
        Self {
            document: std::cell::RefCell::new(document.map(str::to_owned)),
            calls: std::cell::RefCell::new(Vec::new()),
            race: std::cell::RefCell::new(None),
            mangle: std::cell::RefCell::new(false),
        }
    }

    fn document(&self) -> Option<String> {
        self.document.borrow().clone()
    }

    fn tools_called(&self) -> Vec<String> {
        self.calls.borrow().iter().map(|(tool, _)| tool.clone()).collect()
    }

    fn last_arguments(&self) -> String {
        self.calls.borrow().last().map(|(_, a)| a.clone()).unwrap_or_default()
    }

    /// `rivet get document`, as `crate::operation::document` composes it.
    fn get_document(&self) -> String {
        let held = self.document();
        let answer = match &held {
            None => concat!(
                r#"{"subject":"desired-state","path":"/state/config/machine.toml","#,
                r#""present":false,"contents":"","bytes":0}"#,
            )
            .to_owned(),
            Some(text) => format!(
                r#"{{"subject":"desired-state","path":"/state/config/machine.toml","present":true,"contents":"{}","bytes":{}}}"#,
                as_json_string(text),
                text.len(),
            ),
        };
        if let Some(meanwhile) = self.race.borrow_mut().take() {
            *self.document.borrow_mut() = Some(meanwhile);
        }
        wire(&answer, false)
    }

    /// `rivet apply edit`, with the one refusal that is its whole safety story.
    fn apply_edit(&self, arguments: &str) -> String {
        let mut words = change_words(arguments);
        if *self.mangle.borrow() {
            words = words.iter().map(|w| as_latin1(w)).collect();
        }
        let before = self.document();
        let (after, kind) = match words.as_slice() {
            [flag, text] if flag == "--append" => {
                // The real operation CREATES the file when it is not there, and
                // this mirrors that deliberately: the refusal being tested
                // elsewhere is this transport's, not the machine's.
                (format!("{}{text}", before.clone().unwrap_or_default()), "append")
            }
            [expect_flag, expect, replace_flag, replace]
                if expect_flag == "--expect" && replace_flag == "--replace" =>
            {
                let Some(text) = before.clone() else {
                    return wire("E_MACHINE_REFUSED: there is no such file", true);
                };
                match text.matches(expect.as_str()).count() {
                    1 => (text.replacen(expect.as_str(), replace, 1), "replace"),
                    n => {
                        return wire(
                            &format!("E_ARGUMENTS: --expect matches {n} times, not once"),
                            true,
                        );
                    }
                }
            }
            other => return wire(&format!("E_ARGUMENTS: {other:?}"), true),
        };
        *self.document.borrow_mut() = Some(after.clone());
        wire(
            &format!(
                r#"{{"subject":"edit","path":"/state/config/machine.toml","change":"{kind}","created":{},"before-bytes":{},"after-bytes":{}}}"#,
                before.is_none(),
                before.map_or(0, |t| t.len()),
                after.len(),
            ),
            false,
        )
    }
}

impl CallsTools for FakeMcp {
    fn call_tool(&self, tool: &str, arguments: &str) -> Result<String, String> {
        self.calls.borrow_mut().push((tool.to_owned(), arguments.to_owned()));
        let line = match tool {
            "rivet_get" => {
                assert_eq!(arguments, r#"{"subject":"desired-state"}"#);
                self.get_document()
            }
            "rivet_apply" if arguments.contains(r#""subject":"edit""#) => {
                assert!(
                    arguments.contains(r#""slot":"/state/config/machine.toml""#),
                    "{arguments}"
                );
                assert!(arguments.contains(r#""confirmation":"confirm""#), "{arguments}");
                self.apply_edit(arguments)
            }
            // `Remote::perform_operation`'s own catalogue call (Fix 1) — the
            // shape `command::translate` composes for
            // `apply service-restart <name> confirm`. This fake does not
            // model a real supervisor, so it just answers success; what the
            // test below checks is that this arm was reached AT ALL, which
            // it never was through `ask`.
            "rivet_apply" if arguments.contains(r#""subject":"service-restart""#) => {
                wire(r#"{"subject":"service-restart"}"#, false)
            }
            other => panic!("this transport called {other} with {arguments}, which is not in the catalogue"),
        };
        tool_result(&line)
    }
}

/// One JSON-RPC result line, as `rivet mcp` writes one.
fn wire(text: &str, is_error: bool) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":2,"result":{{"content":[{{"type":"text","text":"{}"}}],"isError":{is_error}}}}}"#,
        as_json_string(text),
    )
}

/// The test's own JSON string escaper — written here, on purpose.
fn as_json_string(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match u32::from(c) {
            0x22 => out.push_str("\\\""),
            0x5c => out.push_str("\\\\"),
            0x0a => out.push_str("\\n"),
            0x0d => out.push_str("\\r"),
            0x09 => out.push_str("\\t"),
            code if code < 0x20 => out.push_str(&format!("\\u{code:04x}")),
            _ => out.push(c),
        }
    }
    out
}

/// The test's own JSON string reader: from the opening quote at `at`, the
/// decoded value and the position just past the closing one.
///
/// **In characters and not bytes**, which is not a detail — the first version of
/// this walked a `char` index into a byte slice and fell over on the first
/// document containing `é`, which is exactly the content these tests exist for.
fn read_json_string(chars: &[char], at: usize) -> (String, usize) {
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
                    'u' => {
                        let hex: String = chars[index + 1..index + 5].iter().collect();
                        let code = u32::from_str_radix(&hex, 16).expect("four hex digits");
                        index += 4;
                        // A high surrogate is half a character; the low one
                        // follows as a second escape.
                        if (0xD800..=0xDBFF).contains(&code) {
                            assert_eq!(chars[index + 1], '\\', "half a surrogate pair");
                            assert_eq!(chars[index + 2], 'u', "half a surrogate pair");
                            let low: String = chars[index + 3..index + 7].iter().collect();
                            let low = u32::from_str_radix(&low, 16).expect("four hex digits");
                            let combined =
                                0x1_0000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                            out.push(char::from_u32(combined).expect("a character"));
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

/// The words of the `change` array, as `cmd/rivet`'s `mcp` module would flatten
/// them into the operation's argument vector.
fn change_words(arguments: &str) -> Vec<String> {
    let chars: Vec<char> = arguments.chars().collect();
    let opening: Vec<char> = "\"change\":[".chars().collect();
    let mut at = chars
        .windows(opening.len())
        .position(|window| window == opening.as_slice())
        .expect("a change array")
        + opening.len();
    let mut words = Vec::new();
    loop {
        let (word, next) = read_json_string(&chars, at);
        words.push(word);
        match chars.get(next) {
            Some(',') => at = next + 1,
            Some(']') => return words,
            other => panic!("the change array is malformed at {other:?}"),
        }
    }
}

/// **Nothing this does leaves the catalogue.** Every document method is one of
/// the three tools `rivet mcp` publishes, and there is no path here that reaches
/// for another channel.
#[test]
fn every_document_method_is_a_catalogue_call_and_nothing_else() {
    let board = FakeMcp::holding(Some("version = 1\n"));
    read_document_via(&board).expect("a document");
    edit_document_via(&board, &ConfigChange::Append { text: "\n[services.x]\n".to_owned() })
        .expect("an edit");
    put_document_via(&board, "version = 1\nnew = true\n").expect("a document written");
    assert_eq!(
        board.tools_called(),
        // read; edit; then put — which reads, edits, and reads back what it
        // wrote rather than trusting the channel (`push`'s reason).
        ["rivet_get", "rivet_apply", "rivet_get", "rivet_apply", "rivet_get"],
    );
}

/// The exact arguments an append composes — the shape the machine's `words()`
/// binds positionally between `slot` and `confirmation`.
#[test]
fn an_append_is_a_two_word_change_array() {
    let board = FakeMcp::holding(Some("version = 1\n"));
    edit_document_via(
        &board,
        &ConfigChange::Append { text: "\n[services.metrics]\nenabled = true\n".to_owned() },
    )
    .expect("an edit");
    assert_eq!(
        board.last_arguments(),
        concat!(
            r#"{"subject":"edit","slot":"/state/config/machine.toml","#,
            r#""change":["--append","\n[services.metrics]\nenabled = true\n"],"#,
            r#""confirmation":"confirm"}"#,
        )
    );
    assert_eq!(
        board.document(),
        Some("version = 1\n\n[services.metrics]\nenabled = true\n".to_owned())
    );
}

/// And a replace is four.
#[test]
fn a_replace_is_a_four_word_change_array() {
    let board = FakeMcp::holding(Some("version = 1\n[services.x]\nenabled = false\n"));
    edit_document_via(
        &board,
        &ConfigChange::Replace {
            expect: "[services.x]\nenabled = false".to_owned(),
            replace: "[services.x]\nenabled = true".to_owned(),
        },
    )
    .expect("an edit");
    assert_eq!(
        board.last_arguments(),
        concat!(
            r#"{"subject":"edit","slot":"/state/config/machine.toml","#,
            r#""change":["--expect","[services.x]\nenabled = false","--replace","#,
            r#""[services.x]\nenabled = true"],"confirmation":"confirm"}"#,
        )
    );
    assert_eq!(
        board.document(),
        Some("version = 1\n[services.x]\nenabled = true\n".to_owned())
    );
}

/// A document with every character this module's two escaping layers can get
/// wrong: quotes and backslashes the file itself carries escaped, a literal tab,
/// two control characters, and non-ASCII from three widths — including one above
/// the BMP, which JSON can only spell as a surrogate pair.
const NASTY: &str = "version = 1\n\n[services.awkward]\nenabled = true\n\
                     quoted = \"a \\\"word\\\" and a backslash \\\\\"\n\
                     tabbed = \"a\tb\"\n\
                     wide = \"café § 🚀 — ünïcode\"\n\
                     bell = \"\u{01}\u{1f}\"\n";

/// **`rivet get document` after a real `rivet mcp` performed the edit this
/// module composed for [`NASTY`].**
///
/// Provenance, and it is the strongest thing in this file: the arguments
/// [`put_document_via`] composed were written out by the test that used to live
/// here, piped into a Linux build of this workspace's own `rivet mcp` running as
/// root over a real `/state/config/machine.toml` (WSL, 2026-08-29), and this is
/// what a following `get document` answered. `cat -A` on the file showed 152
/// bytes and the UTF-8 intact. **The machine wrote it; this constant is what the
/// machine then said about it.** Still not the board — the same code, a
/// different computer.
const READ_BACK: &str = r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"{\"subject\":\"desired-state\",\"path\":\"/state/config/machine.toml\",\"present\":true,\"contents\":\"version = 1\\n\\n[services.awkward]\\nenabled = true\\nquoted = \\\"a \\\\\\\"word\\\\\\\" and a backslash \\\\\\\\\\\"\\ntabbed = \\\"a\\tb\\\"\\nwide = \\\"café § 🚀 — ünïcode\\\"\\nbell = \\\"\\u0001\\u001f\\\"\\n\",\"bytes\":152}"}],"isError":false}}"#;

/// **The acceptance test, against the other end's own answer.** What this module
/// composed, a real `rivet mcp` wrote, and what it said afterwards decodes back
/// to exactly what was sent.
#[test]
fn a_document_full_of_nasty_characters_comes_back_from_a_real_rivet_mcp_unchanged() {
    let answer = tool_result(READ_BACK).expect("a successful call");
    let got = document_from(&answer).expect("a readable answer").expect("a document");
    assert_eq!(got, NASTY);
    assert_eq!(got.len(), 152, "the machine counted 152 bytes");
}

/// The same round trip in this process, which is what proves the two directions
/// are inverses on every path rather than on the one that was run by hand.
#[test]
fn a_document_full_of_nasty_characters_survives_the_round_trip() {
    let board = FakeMcp::holding(Some("version = 1\n"));
    put_document_via(&board, NASTY).expect("the document is written");
    assert_eq!(board.document().as_deref(), Some(NASTY), "the fake board's own copy");
    let read = read_document_via(&board).expect("a readable document");
    assert_eq!(read.as_deref(), Some(NASTY), "what came back");
}

/// **Compare-and-swap, and it is the feature.** A document that changed between
/// the read and the write is refused, not clobbered.
#[test]
fn a_document_that_changed_underneath_is_refused_rather_than_clobbered() {
    let board = FakeMcp::holding(Some("version = 1\n"));
    *board.race.borrow_mut() = Some("version = 2\nsomebody-else = true\n".to_owned());
    let refused = put_document_via(&board, "version = 1\nmine = true\n")
        .expect_err("the document moved under it");
    assert!(refused.contains("--expect matches 0 times"), "{refused}");
    assert_eq!(
        board.document().as_deref(),
        Some("version = 2\nsomebody-else = true\n"),
        "the other edit was overwritten"
    );
}

/// **The limit of the swap, and what catches it instead.**
///
/// The comparison is `apply edit`'s — *this text occurs exactly once* — and a
/// concurrent editor that only **appended** leaves the text that was read
/// occurring exactly once. So the swap itself does not stop that one: the
/// replacement lands and the appended tail survives after it.
///
/// **The read-back does.** What is on the machine is then neither document, and
/// this refuses saying so rather than reporting a write that did not do what was
/// asked. The other edit is not lost, which is more than a transport that
/// overwrote the file could say.
#[test]
fn a_concurrent_append_survives_the_swap_and_is_caught_by_the_read_back() {
    let board = FakeMcp::holding(Some("version = 1\n"));
    *board.race.borrow_mut() = Some("version = 1\nsomebody-else = true\n".to_owned());
    let refused = put_document_via(&board, "version = 1\nmine = true\n")
        .expect_err("what is on the machine is neither document");
    assert!(refused.contains("NOT the document that was sent"), "{refused}");
    assert!(refused.contains("fault in the CHANNEL"), "{refused}");
    assert_eq!(
        board.document().as_deref(),
        Some("version = 1\nmine = true\nsomebody-else = true\n"),
        "the other edit survived after this one, rather than being lost"
    );
}

/// **The read-back is what a mangling channel runs into.** A board whose line
/// reader corrupts what it is sent — `cmd/rivet`'s does, for non-ASCII; see
/// [`json_escape`] — must not produce a silent success here.
#[test]
fn a_channel_that_mangles_what_it_wrote_is_reported_and_not_reported_as_written() {
    let board = FakeMcp::holding(Some("version = 1\n"));
    *board.mangle.borrow_mut() = true;
    let refused = put_document_via(&board, "version = 1\nname = \"café\"\n")
        .expect_err("the machine's copy is not what was sent");
    assert!(refused.contains("NOT the document that was sent"), "{refused}");
    assert!(refused.contains("U+00E9"), "the difference is named: {refused}");
    assert!(refused.contains("before anything reloads"), "{refused}");
}

/// **No document is a refusal**, even though the machine would have created one
/// — see [`put_document_via`]. The fake appends and creates, as the real
/// operation does, so this proves the refusal is this transport's own.
#[test]
fn there_being_no_document_is_refused_rather_than_written() {
    let board = FakeMcp::holding(None);
    let refused = put_document_via(&board, "version = 1\n").expect_err("nothing to replace");
    assert!(refused.contains("will not write the first one"), "{refused}");
    assert!(refused.contains("ADR-004 §6"), "{refused}");
    assert_eq!(board.document(), None, "it wrote a first document anyway");
    assert_eq!(board.tools_called(), ["rivet_get"], "it tried to write");
}

/// An empty document is present, not absent, and is written by appending —
/// `--expect ""` would be ambiguous and the machine refuses ambiguity.
#[test]
fn an_empty_document_is_replaced_by_appending() {
    let board = FakeMcp::holding(Some(""));
    put_document_via(&board, "version = 1\n").expect("a document written");
    assert_eq!(board.document().as_deref(), Some("version = 1\n"));
    assert!(
        board.calls.borrow().iter().any(|(_, a)| a.contains("--append")),
        "{:?}",
        board.calls.borrow()
    );
}

/// A document that is already what was asked for costs one call and no write.
#[test]
fn a_document_that_is_already_right_is_not_rewritten() {
    let board = FakeMcp::holding(Some("version = 1\n"));
    put_document_via(&board, "version = 1\n").expect("nothing to do");
    assert_eq!(board.tools_called(), ["rivet_get"]);
}

// ---------------------------------------------------------------------------
// What a truncated answer says — PLAN.md §92's first defect
// ---------------------------------------------------------------------------

/// The exact text rustls produces for a peer that went away without saying
/// goodbye, kept verbatim so this test is about the string an operator saw.
///
/// **Provenance: measured**, not composed. `rustls`'s `conn.rs` defines it as
/// `UNEXPECTED_EOF_MESSAGE` and a board produced it on 2026-08-29 in answer to
/// `cargo rivet run … -- get log latest all`.
const RUSTLS_EOF: &str = "peer closed connection without sending TLS close_notify: \
                          https://docs.rs/rustls/latest/rustls/manual/_03_howto/index.html\
                          #unexpected-eof";

fn eof() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::UnexpectedEof, RUSTLS_EOF)
}

/// **A dropped answer is explained by Rivet, and the library is quoted rather
/// than being the whole message.**
///
/// This is what the cold agent of 2026-08-29 was given instead of an answer: a
/// rustls sentence and a link to rustls's manual, from a system it had just
/// called *"outstanding at explaining its own refusals"*. Every assertion here
/// is one of the things only this end could have supplied.
#[test]
fn a_connection_that_died_mid_answer_is_explained_in_this_projects_own_words() {
    let said = read_failure("pi:7777", "the `rivet_get` call", &eof(), 38_142);

    // Which machine, which call, and how much arrived.
    assert!(said.contains("pi:7777"), "{said}");
    assert!(said.contains("rivet_get"), "{said}");
    assert!(said.contains("38142"), "{said}");
    // That nothing was read from a partial answer, which is the operator's
    // first question and the reason the refusal is correct.
    assert!(said.contains("NOTHING is read"), "{said}");
    // Where the other half of the account lives — the machine wrote it down.
    // **The command has to be one the machine can bind.** Until 2026-08-30 this
    // said `get log rivet-mcpd`, and `get log` has no service filter: that word
    // selects a BOOT of that name. A refusal that names an unusable command is
    // a refusal that costs a round trip.
    assert!(said.contains("`svc` is `rivet-mcpd`"), "{said}");
    assert!(said.contains("NO SERVICE FILTER"), "{said}");
    assert!(
        !said.contains("get log rivet-mcpd"),
        "the service name must not appear as a bare word after `get log`: {said}"
    );
    // The one cause known to produce exactly this, offered as something to rule
    // out rather than as a diagnosis this end cannot make.
    assert!(said.contains("64 KiB"), "{said}");
    assert!(said.contains("get log latest 100"), "{said}");
    // And the library's own words are still there, at the end.
    assert!(said.contains(RUSTLS_EOF), "{said}");
    // It does not OPEN with them, which is the whole complaint.
    assert!(
        !said.starts_with("peer closed"),
        "the library's sentence is still the answer: {said}"
    );
}

/// A failure that is **not** a truncated stream is not dressed up as one.
///
/// A refusal that explains the wrong thing confidently is the failure this
/// repository keeps finding, so the close_notify account is reached only by the
/// error kind that means it.
#[test]
fn an_ordinary_io_failure_is_reported_as_itself_and_not_as_a_truncation() {
    let reset = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "connection reset");
    let said = read_failure("pi:7777", "the `rivet_get` call", &reset, 0);
    assert!(said.contains("connection reset"), "{said}");
    assert!(!said.contains("close_notify"), "{said}");
    assert!(!said.contains("64 KiB"), "{said}");
}

// ---- and the whole of `--enable`, over this transport ----------------------

/// The `Remote` this transport is, with the socket replaced by [`FakeMcp`].
///
/// The three document methods go through the **same** functions [`Tls`]'s do —
/// that is what the [`CallsTools`] seam is for. `push`, `blake3` and `install`
/// are the fake's own, and `blake3` hashes the bytes it was given rather than
/// answering, because `crate::deploy::run` compares it against the local digest
/// and a fixture would prove nothing.
struct FakeTls {
    mcp: FakeMcp,
    order: Vec<&'static str>,
}

impl Remote for FakeTls {
    fn push(&mut self, _local: &Path, _remote_path: &str) -> Result<String, String> {
        self.order.push("push");
        Ok(String::new())
    }
    fn blake3(&mut self, _remote_path: &str) -> Result<[u8; 32], String> {
        Err("this fake is not asked for a digest".to_owned())
    }
    fn install(&mut self, _remote_path: &str) -> Result<String, String> {
        self.order.push("install");
        Ok(String::new())
    }
    fn read_document(&mut self) -> Result<Option<String>, String> {
        read_document_via(&self.mcp)
    }
    fn edit_document(&mut self, change: &ConfigChange) -> Result<String, String> {
        self.order.push("edit");
        edit_document_via(&self.mcp, change)
    }
    fn put_document(&mut self, text: &str) -> Result<(), String> {
        put_document_via(&self.mcp, text)
    }
    fn ask(&mut self, _words: &[&str]) -> Result<String, String> {
        Err("this fake does not run commands".to_owned())
    }
    /// **The whole of Fix 1's proof against a `CallsTools` transport rather
    /// than a real socket.** `crate::tls::Tls`'s own override is
    /// `crate::command::perform(self, words)`; this is the same call against
    /// `self.mcp` instead of `self`, which is what makes it exercisable with
    /// no board.
    fn perform_operation(&mut self, words: &[&str]) -> Result<String, String> {
        self.order.push("perform_operation");
        crate::command::perform(&self.mcp, words)
    }
}

/// **What this whole change was for.** `--enable` reads the document, works out
/// what to change, and says it — over the control plane and nothing else.
///
/// `crate::deploy`'s own logic is untouched; this exercises it against a board
/// that speaks MCP.
#[test]
fn enable_reads_the_document_and_says_enabled_over_the_control_plane() {
    let document = "version = 1\n\n[services.rivet-mdnsd]\nenabled = true\n";
    let mut remote = FakeTls {
        mcp: FakeMcp::holding(Some(document)),
        order: Vec::new(),
    };

    // The document has no table for this service, so `--enable` appends one.
    let standing = crate::deploy::standing(document, "rivet-todo");
    let change = crate::deploy::change_for(&standing, "rivet-todo").expect("something to do");
    remote.edit_document(&change).expect("the edit is accepted");

    assert_eq!(
        remote.mcp.document().as_deref(),
        Some("version = 1\n\n[services.rivet-mdnsd]\nenabled = true\n\n[services.rivet-todo]\nenabled = true\n"),
    );
    // And it is now readable back, which is the asymmetry `get document` closed:
    // the transport can form the belief `--expect` needs.
    let read = remote.read_document().expect("a document").expect("present");
    assert_eq!(
        crate::deploy::standing(&read, "rivet-todo"),
        crate::deploy::Standing::Enabled
    );
    assert_eq!(remote.order, ["edit"]);
}

/// The one-word change, over the same transport: `enabled = false` becomes
/// `true` and nothing else in the document moves.
#[test]
fn enable_changes_one_word_when_the_service_is_already_named() {
    let document = "version = 1\n\n[services.rivet-todo]\nenabled = false\n\n\
                    [network.eth0]\nmethod = \"dhcp\"\n";
    let mut remote = FakeTls {
        mcp: FakeMcp::holding(Some(document)),
        order: Vec::new(),
    };
    let standing = crate::deploy::standing(document, "rivet-todo");
    assert_eq!(standing, crate::deploy::Standing::Disabled);
    let change = crate::deploy::change_for(&standing, "rivet-todo").expect("something to do");
    remote.edit_document(&change).expect("the edit is accepted");
    assert_eq!(
        remote.mcp.document().as_deref(),
        Some(
            "version = 1\n\n[services.rivet-todo]\nenabled = true\n\n\
             [network.eth0]\nmethod = \"dhcp\"\n"
        ),
    );
}

// ---------------------------------------------------------------------------
// perform_operation — Fix 1. A restart step reaches the MCP catalogue over a
// `CallsTools` transport, which `Remote::ask` on this same transport refuses
// by design. `Tls::perform_operation` is `crate::command::perform(self,
// words)`; `FakeTls::perform_operation` above is the identical call against
// its `CallsTools` fake, so this is a real exercise of `command::translate`
// composing the tool call and `CallsTools::call_tool` receiving it — the only
// piece `Tls`'s own override adds beyond this is a real socket.
// ---------------------------------------------------------------------------

/// `ask` still refuses a word list on this transport — unchanged by Fix 1,
/// which added a second way in rather than widening `ask`.
#[test]
fn ask_still_refuses_a_restart_word_list_on_the_fake_transport() {
    let mut remote = FakeTls { mcp: FakeMcp::holding(None), order: Vec::new() };
    let err = remote
        .ask(&["apply", "service-restart", "tempmon-rivet", "confirm"])
        .expect_err("ask refuses a word list on this transport, restart or not");
    assert!(err.contains("does not run commands"), "{err}");
}

/// **What Fix 1 actually buys**: the same word list, sent through
/// `perform_operation` instead, reaches the catalogue as a real `rivet_apply`
/// tool call rather than being refused.
#[test]
fn perform_operation_sends_a_restart_as_a_real_tool_call() {
    let mut remote = FakeTls { mcp: FakeMcp::holding(None), order: Vec::new() };
    let said = remote
        .perform_operation(&["apply", "service-restart", "tempmon-rivet", "confirm"])
        .expect("the catalogue call succeeds against the fake");
    assert!(said.contains("service-restart"), "{said}");

    assert_eq!(remote.mcp.tools_called(), ["rivet_apply"]);
    assert_eq!(
        remote.mcp.last_arguments(),
        r#"{"subject":"service-restart","slot":"tempmon-rivet","confirmation":"confirm"}"#
    );
    assert_eq!(remote.order, ["perform_operation"]);
}

/// The proxy restart step is the same call with `rivet-proxyd` named —
/// `cargo rivet fleet`'s `StepKind::RestartReader` arm sends exactly this.
#[test]
fn perform_operation_sends_a_proxy_restart_as_a_service_restart_of_rivet_proxyd() {
    let mut remote = FakeTls { mcp: FakeMcp::holding(None), order: Vec::new() };
    remote
        .perform_operation(&["apply", "service-restart", "rivet-proxyd", "confirm"])
        .expect("the catalogue call succeeds against the fake");
    assert_eq!(
        remote.mcp.last_arguments(),
        r#"{"subject":"service-restart","slot":"rivet-proxyd","confirmation":"confirm"}"#
    );
}

// ---------------------------------------------------------------------------
// One connection, several calls — ADR-034 §11.5.2
// ---------------------------------------------------------------------------

/// **The whole of the batch, on the wire.** One handshake, one initialized
/// notification, and one `tools/call` per subject with ids counting from 2.
///
/// This is the half of the protocol this end owns, and it is asserted here
/// because it is the only half that can be asserted without a board.
#[test]
fn a_batch_is_one_handshake_and_one_call_per_subject() {
    let calls = vec![
        ("rivet_get".to_owned(), r#"{"subject":"memory"}"#.to_owned()),
        ("rivet_get".to_owned(), r#"{"subject":"cpu"}"#.to_owned()),
    ];
    let script = batch_script(&calls);
    let lines: Vec<&str> = script.lines().collect();
    assert_eq!(lines.len(), 4, "{script}");
    assert!(lines[0].contains(r#""method":"initialize""#), "{script}");
    assert!(lines[1].contains("notifications/initialized"), "{script}");
    assert!(lines[2].contains(r#""id":2"#) && lines[2].contains(r#""subject":"memory""#));
    assert!(lines[3].contains(r#""id":3"#) && lines[3].contains(r#""subject":"cpu""#));
    // Every message is terminated: `rivet mcp` reads a line at a time, and a
    // final message with no newline sits unread until the stream closes.
    assert!(script.ends_with('\n'), "{script:?}");
}

/// **A single call composes exactly what it used to.** The one-call path is now
/// the batch path with one element, and this is what says the change did not
/// alter the message a board has already answered.
#[test]
fn one_call_is_the_batch_of_one_and_still_carries_id_two() {
    let script = batch_script(&[(
        "rivet_apply".to_owned(),
        r#"{"subject":"reload"}"#.to_owned(),
    )]);
    assert_eq!(script.lines().count(), 3, "{script}");
    assert!(
        script.contains(
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"rivet_apply","arguments":{"subject":"reload"}}}"#
        ),
        "{script}"
    );
}

/// Answers are matched by id and **not by position**: the far end writes the
/// initialize reply first, and a notification produces no reply at all.
#[test]
fn answers_are_matched_by_id_rather_than_by_the_order_they_arrive() {
    let all = concat!(
        r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18"}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":3,"result":{"content":[{"text":"second"}],"isError":false}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"text":"first"}],"isError":false}}"#,
        "\n",
    );
    let answers = answers_from(all, 2, "pi:7777");
    assert_eq!(answers[0].as_deref(), Ok("first"));
    assert_eq!(answers[1].as_deref(), Ok("second"));
}

/// **`"id":1` must not be read as `"id":10`.** Nine subjects is a real batch —
/// the monitor has nine — so the last call's id shares a prefix with the
/// handshake's. A reader matching on the prefix would hand the initialize reply
/// back as a subject's answer: confidently wrong rather than absent, which is
/// the shape of defect this project keeps finding.
#[test]
fn a_two_digit_id_is_not_confused_with_the_handshake() {
    let mut all =
        String::from(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18"}}"#);
    all.push('\n');
    for id in 2..=10 {
        all.push_str(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"content":[{{"text":"answer {id}"}}],"isError":false}}}}"#
        ));
        all.push('\n');
    }
    let answers = answers_from(&all, 9, "pi:7777");
    assert_eq!(answers.len(), 9);
    for (at, answer) in answers.iter().enumerate() {
        assert_eq!(answer.as_deref(), Ok(format!("answer {}", at + 2).as_str()));
    }
}

/// **One call with no answer is one failure, not nine.** The others were paid
/// for and arrived; discarding them would waste the connection this batching
/// exists to save.
#[test]
fn a_missing_answer_fails_only_the_call_that_is_missing() {
    let all = concat!(
        r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"text":"here"}],"isError":false}}"#,
        "\n",
    );
    let answers = answers_from(all, 2, "pi:7777");
    assert_eq!(answers[0].as_deref(), Ok("here"));
    let missing = answers[1].as_ref().expect_err("no line carried id 3");
    assert!(missing.contains("pi:7777"), "{missing}");
    assert!(missing.contains("call 3 of 2"), "{missing}");
}

/// **An error answer stays that call's error.** `isError` is per call, so a
/// batch in which one subject is unsupported still delivers the rest.
#[test]
fn one_subject_refusing_does_not_refuse_the_others() {
    let all = concat!(
        r#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"text":"E_UNKNOWN_SUBJECT: thermal"}],"isError":true}}"#,
        "\n",
        r#"{"jsonrpc":"2.0","id":3,"result":{"content":[{"text":"{}"}],"isError":false}}"#,
        "\n",
    );
    let answers = answers_from(all, 2, "pi:7777");
    assert_eq!(
        answers[0].as_ref().expect_err("isError"),
        "E_UNKNOWN_SUBJECT: thermal"
    );
    assert_eq!(answers[1].as_deref(), Ok("{}"));
}

/// **The subject is composed by the catalogue and not by this module.**
/// [`Tls::get_each`] goes through `crate::command::translate`, so there is one
/// spelling of `rivet_get {"subject":…}` in this workspace.
///
/// And **one word reaches it**: `get_each` takes subjects rather than words, so
/// the arguments object carries the subject and nothing else. That is what keeps
/// [`Remote::ask`]'s refusal intact — a caller holding this cannot widen it into
/// a command line, because there is nowhere to put a second word.
#[test]
fn a_get_subject_is_composed_by_the_catalogue_and_binds_one_input() {
    let call = crate::command::translate(&["get", "memory"]).expect("a get is translatable");
    assert_eq!(call.tool, "rivet_get");
    assert_eq!(call.arguments, r#"{"subject":"memory"}"#);
    assert_eq!(call.words(), ["memory"]);
}

/// **An empty batch opens no connection**, which is what makes a poll cycle with
/// nothing due free rather than merely cheap.
#[test]
fn nothing_due_is_no_connection_at_all() {
    let identity = rivetos_tls::Identity {
        seed: [0_u8; 32],
        certificate: Vec::new(),
    };
    // The host is deliberately unroutable. If this opened a socket the test
    // would fail by hanging or refusing, and it does neither.
    let transport = Tls::new("240.0.0.1:1".to_owned(), identity, Vec::new());
    assert!(transport.get_each(&[]).is_empty());
}

// ---------------------------------------------------------------------------
// `Tls::handshake` — ADR-034 §11.5.1's reach probe
// ---------------------------------------------------------------------------

/// Mint an identity the way `cargo rivet identity new` does, and hand back the
/// pin that names it.
///
/// **A copy of `rivet-tls`'s own `mint`, and that is not a second spelling of a
/// rule.** The minting itself lives in `cmd/cargo-rivet`, which is a binary
/// crate with nothing to import; `rivet-tls` reproduces it in its own tests for
/// the same reason, and its comment on the PKCS#8 v2 offset carries the whole
/// of why the seed is sliced out rather than taken from the end.
fn an_identity() -> (rivetos_tls::Identity, rivetos_tls::Pin) {
    let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("an Ed25519 pair");
    let mut params = rcgen::CertificateParams::new(vec!["rivet".to_string()]).expect("params");
    params.is_ca = rcgen::IsCa::NoCa;
    let leaf = params.self_signed(&key).expect("a self-signed leaf");
    let der = key.serialize_der();
    let at = der
        .windows(4)
        .position(|w| w == [0x04, 0x22, 0x04, 0x20])
        .expect("PKCS#8 for Ed25519 wraps the seed in 04 22 04 20")
        + 4;
    let mut seed = [0_u8; 32];
    seed.copy_from_slice(&der[at..at + 32]);
    let certificate = leaf.der().to_vec();
    let pin = rivetos_tls::public_key_in_certificate(&certificate)
        .expect("a freshly minted leaf carries its own key");
    (rivetos_tls::Identity { certificate, seed }, pin)
}

/// A listener that presents `identity` and accepts peers holding `accepts`.
///
/// It serves **one** connection and then stops. That is the whole of what a
/// probe does, and a server that outlived the test would leave a thread behind
/// in every run.
fn a_listener(
    identity: rivetos_tls::Identity,
    accepts: Vec<rivetos_tls::Pin>,
) -> (String, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a local port");
    let host = listener.local_addr().expect("an address").to_string();
    let handle = std::thread::spawn(move || {
        let Ok(config) = rivetos_tls::server_config(&identity, accepts, rivetos_tls::provider())
        else {
            return;
        };
        let config = Arc::new(config);
        if let Ok((socket, _)) = listener.accept() {
            let mut socket = socket;
            if let Ok(mut connection) = rustls::ServerConnection::new(config) {
                // Driven until it stops handshaking or refuses; either way the
                // client has had its answer by the time this returns.
                let _ = connection.complete_io(&mut socket);
            }
        }
    });
    (host, handle)
}

/// **The probe completes a handshake against a board it pins.**
///
/// This is the one claim a reachable dot rests on, and it is proved against a
/// real rustls server rather than at a seam — the pin check happens inside the
/// certificate verifier, so anything above it would prove nothing.
#[test]
fn a_peer_holding_the_pinned_key_answers_as_trusted() {
    let (board, board_pin) = an_identity();
    let (operator, operator_pin) = an_identity();
    let (host, server) = a_listener(board, vec![operator_pin]);

    let probe = Tls::new(host, operator, vec![board_pin]);
    assert_eq!(
        probe.handshake(std::time::Duration::from_secs(5)),
        Handshake::Trusted
    );
    let _ = server.join();
}

/// **A machine that answers and is NOT the one the register records is not
/// `down`.** `ADR-034` §6.1's neighbouring case: an address answering TLS while
/// presenting an unexpected key is the failure an operator most needs told
/// apart, because it is either a replaced identity or something else wearing
/// that address.
#[test]
fn a_peer_presenting_an_unpinned_key_is_untrusted_rather_than_unreachable() {
    let (board, _) = an_identity();
    let (stranger, stranger_pin) = an_identity();
    let (operator, operator_pin) = an_identity();
    let (host, server) = a_listener(board, vec![operator_pin]);
    drop(stranger);

    // Pinned to the STRANGER's key, so the board's own is the wrong one.
    let probe = Tls::new(host, operator, vec![stranger_pin]);
    let answer = probe.handshake(std::time::Duration::from_secs(5));
    match &answer {
        Handshake::Untrusted(why) => assert!(
            why.contains("would not trust it"),
            "the refusal should say what happened: {why}"
        ),
        other => panic!("an unpinned peer must not read as {other:?}"),
    }
    // And the detail is carried rather than swallowed.
    assert!(answer.detail().is_some());
    let _ = server.join();
}

/// **Nothing listening is `Unreachable`, and it is a different answer from
/// `Untrusted`.** A port that refuses a connection says nothing about keys.
///
/// # Why this retries, and what it is racing
///
/// "A port nothing is listening on" is obtained by binding an ephemeral port
/// and dropping the listener — and between the drop and the probe, the kernel
/// is free to hand that exact port to **another test in this same binary**.
/// Several of them bind `127.0.0.1:0` and run a server on it. When that
/// happens this test connects to a real TLS server and gets `Untrusted`, which
/// is a correct answer to a different question.
///
/// It failed that way once in three runs of the whole workspace on 2026-09-04
/// and passed on its own every time, which is the signature. A flaky test in
/// the suite `tools/test-all.sh` gates on is worse than no test: it teaches
/// whoever sees it that a red run is nothing to worry about.
///
/// So the race is **acknowledged rather than assumed away**. A handful of
/// attempts, each with a freshly-taken port; the first `Unreachable` settles
/// it, and only if every attempt lands on somebody else's live listener does
/// this fail — which is no longer a race but a fact worth reporting.
#[test]
fn a_port_with_nothing_behind_it_is_unreachable() {
    const ATTEMPTS: usize = 8;
    let mut collided = Vec::new();
    for _ in 0..ATTEMPTS {
        let (operator, _) = an_identity();
        let (_, pin) = an_identity();
        // Bound and immediately dropped, so this is a port nothing is
        // listening on — until somebody else takes it.
        let host = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a local port");
            listener.local_addr().expect("an address").to_string()
        };
        let probe = Tls::new(host, operator, vec![pin]);
        match probe.handshake(std::time::Duration::from_secs(2)) {
            Handshake::Unreachable(why) => {
                assert!(!why.is_empty(), "it must say what happened");
                return;
            }
            // A neighbouring test was handed the port back. Take another one.
            other => collided.push(format!("{other:?}")),
        }
    }
    panic!(
        "every one of {ATTEMPTS} freed ephemeral ports had something listening on it, which is \
         no longer a race: {collided:?}"
    );
}

/// A name that does not resolve is `Unreachable` too — from the operator's seat
/// it is the same situation as a board switched off.
#[test]
fn a_name_that_does_not_resolve_is_unreachable() {
    let (operator, _) = an_identity();
    let (_, pin) = an_identity();
    let probe = Tls::new(
        "this-name-does-not-resolve.invalid:7777".to_owned(),
        operator,
        vec![pin],
    );
    match probe.handshake(std::time::Duration::from_secs(2)) {
        Handshake::Unreachable(why) => assert!(why.contains("resolve"), "{why}"),
        other => panic!("an unresolvable name must not read as {other:?}"),
    }
}

/// **This end being unusable is its own answer**, and it is reached before a
/// socket is opened: the address below would hang if anything connected to it.
#[test]
fn an_identity_this_end_cannot_use_is_neither_up_nor_down() {
    let identity = rivetos_tls::Identity { certificate: vec![1, 2, 3], seed: [7_u8; 32] };
    let probe = Tls::new("240.0.0.1:1".to_owned(), identity, vec![[9_u8; 32]]);
    match probe.handshake(std::time::Duration::from_millis(50)) {
        Handshake::Unusable(why) => assert!(why.contains("this end's own"), "{why}"),
        other => panic!("an unreadable identity must not read as {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// `Tls::exchange` — one HTTP request over the control plane, for `fleet push`
// ---------------------------------------------------------------------------

/// A pinned listener that reads ONE request — head, then exactly the
/// `Content-Length` it declares — hands back what arrived, and answers
/// `answer` with a `close_notify`, as `cmd/rivet-mcpd` does after a response.
fn an_http_listener(
    identity: rivetos_tls::Identity,
    accepts: Vec<rivetos_tls::Pin>,
    answer: &'static str,
) -> (String, std::thread::JoinHandle<Vec<u8>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a local port");
    let host = listener.local_addr().expect("an address").to_string();
    let handle = std::thread::spawn(move || {
        let config = Arc::new(
            rivetos_tls::server_config(&identity, accepts, rivetos_tls::provider()).expect("a config"),
        );
        let Ok((socket, _)) = listener.accept() else { return Vec::new() };
        let Ok(connection) = rustls::ServerConnection::new(config) else { return Vec::new() };
        let mut stream = rustls::StreamOwned::new(connection, socket);
        let mut got = Vec::new();
        let mut byte = [0_u8; 1];
        while !got.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(1) => got.push(byte[0]),
                _ => return got,
            }
        }
        let head = String::from_utf8_lossy(&got).to_string();
        let length = head
            .lines()
            .find_map(|l| l.strip_prefix("Content-Length: "))
            .and_then(|n| n.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = vec![0_u8; length];
        if stream.read_exact(&mut body).is_err() {
            return got;
        }
        got.extend_from_slice(&body);
        let _ = stream.write_all(answer.as_bytes());
        stream.conn.send_close_notify();
        let _ = stream.flush();
        got
    });
    (host, handle)
}

/// **The bytes go out unchanged and the whole answer comes back**, status line
/// included — the relay above this owns the HTTP, and this owns the pin.
#[test]
fn an_exchange_sends_the_request_whole_and_returns_the_answer_whole() {
    let (board, board_pin) = an_identity();
    let (operator, operator_pin) = an_identity();
    let answer = "HTTP/1.1 200 OK\r\nContent-Type: application/x-git-receive-pack-result\r\nConnection: close\r\n\r\n\x00PACK-ish";
    let (host, server) = an_http_listener(board, vec![operator_pin], answer);

    let head = "POST /fleet.git/git-receive-pack HTTP/1.1\r\nHost: rivet\r\nContent-Length: 5\r\nConnection: close\r\n\r\n";
    let transport = Tls::new(host, operator, vec![board_pin]);
    let got = transport.exchange(head, b"hello").expect("a pinned exchange answers");
    assert_eq!(got, answer.as_bytes());

    let received = server.join().expect("the listener finished");
    assert_eq!(received, [head.as_bytes(), b"hello"].concat(), "the request did not arrive unchanged");
}

/// **A machine that will not take this operator's key is an error, never an
/// empty answer** — the case `fleet push` must report as a refused identity
/// rather than as a repository with nothing in it.
#[test]
fn an_exchange_with_an_identity_the_machine_does_not_accept_is_an_error() {
    let (board, board_pin) = an_identity();
    let (operator, _) = an_identity();
    let (_, someone_else) = an_identity();
    let (host, server) = an_http_listener(board, vec![someone_else], "HTTP/1.1 200 OK\r\n\r\n");

    let transport = Tls::new(host, operator, vec![board_pin]);
    let head = "GET /fleet.git/info/refs?service=git-receive-pack HTTP/1.1\r\nHost: rivet\r\n\r\n";
    let refused = transport.exchange(head, b"").expect_err("the machine refuses this operator");
    assert!(!refused.is_empty());
    let _ = server.join();
}

// ---------------------------------------------------------------------------
// `Tls::http` — a backup or a restore over the control plane, ADR-051 §8
// ---------------------------------------------------------------------------

/// What [`a_recording_listener`] received — the request's bytes — and the
/// certificate chain the client presented, each certificate's DER.
type Recorded = (Vec<u8>, Vec<Vec<u8>>);

/// A pinned listener that reads one request — head, then the body it declares —
/// records the certificate chain the client presented, and answers `answer`.
fn a_recording_listener(
    identity: rivetos_tls::Identity,
    accepts: Vec<rivetos_tls::Pin>,
    answer: Vec<u8>,
) -> (String, std::thread::JoinHandle<Recorded>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a local port");
    let host = listener.local_addr().expect("an address").to_string();
    let handle = std::thread::spawn(move || {
        let config = Arc::new(
            rivetos_tls::server_config(&identity, accepts, rivetos_tls::provider()).expect("a config"),
        );
        let Ok((socket, _)) = listener.accept() else { return (Vec::new(), Vec::new()) };
        let Ok(connection) = rustls::ServerConnection::new(config) else {
            return (Vec::new(), Vec::new());
        };
        let mut stream = rustls::StreamOwned::new(connection, socket);
        let mut got = Vec::new();
        let mut byte = [0_u8; 1];
        while !got.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(1) => got.push(byte[0]),
                _ => return (got, Vec::new()),
            }
        }
        let presented: Vec<Vec<u8>> = stream
            .conn
            .peer_certificates()
            .map(|chain| chain.iter().map(|c| c.as_ref().to_vec()).collect())
            .unwrap_or_default();
        let head = String::from_utf8_lossy(&got).to_string();
        let length = head
            .lines()
            .find_map(|l| l.strip_prefix("Content-Length: "))
            .and_then(|n| n.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = vec![0_u8; length];
        if stream.read_exact(&mut body).is_ok() {
            got.extend_from_slice(&body);
        }
        let _ = stream.write_all(&answer);
        stream.conn.send_close_notify();
        let _ = stream.flush();
        (got, presented)
    });
    (host, handle)
}

/// **A backup over the control plane presents the operator's pinned key and
/// NOTHING ELSE** — `.rulings/` R-234's regression guard. The fleet's browser
/// certificate retires once backups stop needing it, so a backup that ever
/// presented a certificate chaining to a fleet authority again would be the
/// dependency coming back. The chain the machine saw is exactly one
/// certificate, and it is the operator's control-plane identity.
#[test]
fn a_backup_over_the_control_plane_presents_only_the_operators_pinned_identity() {
    let (board, board_pin) = an_identity();
    let (operator, operator_pin) = an_identity();
    let operator_certificate = operator.certificate.clone();
    let body: Vec<u8> = (0..300_000u32).map(|i| (i % 253) as u8).collect();
    let mut answer = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    answer.extend_from_slice(&body);
    let (host, server) = a_recording_listener(board, vec![operator_pin], answer.clone());

    let transport = Tls::new(host, operator, vec![board_pin]);
    let head = "GET /backup/probe-rivet HTTP/1.1\r\nHost: rivet\r\nConnection: close\r\n\r\n";
    let mut reader = transport
        .http(head, &mut std::io::empty(), 0, std::time::Duration::from_secs(10))
        .expect("a pinned request");
    let mut got = Vec::new();
    reader.read_to_end(&mut got).expect("the whole answer");
    assert_eq!(got, answer, "the answer is read as it arrived");

    let (received, presented) = server.join().expect("the listener finished");
    assert_eq!(received, head.as_bytes());
    assert_eq!(
        presented,
        vec![operator_certificate],
        "the machine must see exactly the operator's pinned identity, and no other certificate"
    );
}

/// A restore's body goes out whole, in pieces, and exactly as long as declared.
#[test]
fn a_restore_over_the_control_plane_sends_exactly_the_declared_body() {
    let (board, board_pin) = an_identity();
    let (operator, operator_pin) = an_identity();
    let reply = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_vec();
    let (host, server) = a_recording_listener(board, vec![operator_pin], reply.clone());
    let body: Vec<u8> = (0..200_000u32).map(|i| (i % 13) as u8).collect();
    let head = format!(
        "POST /restore/probe-rivet HTTP/1.1\r\nHost: rivet\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let transport = Tls::new(host, operator, vec![board_pin]);
    let mut reader = transport
        .http(&head, &mut body.as_slice(), body.len() as u64, std::time::Duration::from_secs(10))
        .expect("sent");
    let mut got = Vec::new();
    reader.read_to_end(&mut got).expect("the answer");
    assert_eq!(got, reply);
    let (received, _) = server.join().expect("the listener finished");
    assert_eq!(received, [head.as_bytes(), &body].concat());

    // A body shorter than it said is this end's refusal, never a short send.
    let (board, board_pin) = an_identity();
    let (operator, operator_pin) = an_identity();
    let (host, server) = a_recording_listener(board, vec![operator_pin], reply);
    let transport = Tls::new(host, operator, vec![board_pin]);
    let refused = transport
        .http(&head, &mut &b"short"[..], 10, std::time::Duration::from_secs(10))
        .err()
        .expect("a body that ends early is refused");
    assert!(refused.contains("short"), "{refused}");
    drop(transport);
    let _ = server.join();
}

// ---------------------------------------------------------------------------
// reading one repository object back — ADR-063 §6
// ---------------------------------------------------------------------------

fn object_digest(bytes: &[u8]) -> String {
    use rivet_core::package::port::Digest as _;
    hex(&rivet_crypto::Blake3::new().of(bytes))
}

fn object_answer(status: &str, digest: &str, name: &str, length: usize, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {length}\r\nRivet-Blake3: {digest}\r\n\
         Rivet-Name: {name}\r\nConnection: close\r\n\r\n"
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

fn object_scratch(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rivet-remote-object-{}-{label}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

const OBJECT_NAME: &str = "rivet-0.1.0-aarch64-rpi4-card.rvt";

/// **An object arrives whole and is kept** — more than one 64 KiB piece, so the
/// bytes that came with the head and the bytes read after it are both written.
#[test]
fn an_object_that_hashes_to_its_digest_is_kept_whole() {
    let dir = object_scratch("whole");
    let body: Vec<u8> = (0..200_000usize).map(|i| (i % 253) as u8).collect();
    let digest = object_digest(&body);
    let answer = object_answer("200 OK", &digest, OBJECT_NAME, body.len(), &body);
    let into = dir.join("incoming");
    let got = receive_object(&mut std::io::Cursor::new(answer), &digest, &into, "dev-01").expect("kept");
    assert_eq!(got, Received { name: OBJECT_NAME.to_owned(), length: body.len() as u64 });
    assert_eq!(std::fs::read(&into).expect("the file"), body);
    let _ = std::fs::remove_dir_all(&dir);
}

/// **Bytes that do not hash to the digest asked for are not kept**, and
/// neither is a body that stops short: the file is removed either way.
#[test]
fn a_tampered_or_short_object_is_removed() {
    let dir = object_scratch("tampered");
    let body = b"the object the store holds".to_vec();
    let digest = object_digest(&body);
    let mut tampered = body.clone();
    tampered[0] ^= 0xff;
    let into = dir.join("incoming");

    let answer = object_answer("200 OK", &digest, OBJECT_NAME, tampered.len(), &tampered);
    let refused = receive_object(&mut std::io::Cursor::new(answer), &digest, &into, "dev-01")
        .expect_err("a tampered body");
    assert!(refused.contains("not the object asked for"), "{refused}");
    assert!(!into.exists(), "a tampered object was left behind");

    let answer = object_answer("200 OK", &digest, OBJECT_NAME, body.len() + 10, &body);
    let refused = receive_object(&mut std::io::Cursor::new(answer), &digest, &into, "dev-01")
        .expect_err("a short body");
    assert!(refused.contains("stopped after"), "{refused}");
    assert!(!into.exists(), "a short object was left behind");
    let _ = std::fs::remove_dir_all(&dir);
}

/// **The head is checked before a byte is written**: another digest, a name
/// that is not an artifact's, and a refusal are each refused and create
/// nothing, and the machine's own words are carried.
#[test]
fn a_wrong_digest_a_bad_name_or_a_refusal_writes_nothing() {
    let dir = object_scratch("head");
    let body = b"an object".to_vec();
    let digest = object_digest(&body);
    let into = dir.join("incoming");

    let other = "0".repeat(64);
    let answer = object_answer("200 OK", &other, OBJECT_NAME, body.len(), &body);
    let refused = receive_object(&mut std::io::Cursor::new(answer), &digest, &into, "dev-01")
        .expect_err("another digest");
    assert!(refused.contains("not the object asked for"), "{refused}");

    let answer = object_answer("200 OK", &digest, "../secrets", body.len(), &body);
    let refused = receive_object(&mut std::io::Cursor::new(answer), &digest, &into, "dev-01")
        .expect_err("a bad name");
    assert!(refused.contains("not an ADR-027 artifact name"), "{refused}");

    let answer = b"HTTP/1.1 404 Not Found\r\nContent-Length: 30\r\n\r\nthis store holds no object x\n".to_vec();
    let refused = receive_object(&mut std::io::Cursor::new(answer), &digest, &into, "dev-01")
        .expect_err("not held");
    assert!(refused.contains("answered 404") && refused.contains("holds no object"), "{refused}");
    assert!(!into.exists(), "a refused answer created a file");
    let _ = std::fs::remove_dir_all(&dir);
}

/// **A repeated `Content-Length` is refused, agreeing or not, and nothing is
/// written**: two readers of the same bytes that honour different fields
/// disagree about where the object ends.
#[test]
fn an_object_answer_with_a_repeated_content_length_writes_nothing() {
    let dir = object_scratch("repeated-length");
    let body = b"an object".to_vec();
    let digest = object_digest(&body);
    let into = dir.join("incoming");
    for second in [body.len(), body.len() - 2] {
        let mut answer = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Length: {second}\r\nRivet-Blake3: {digest}\r\n{}: {OBJECT_NAME}\r\n\r\n",
            body.len(),
            rivet_core::repository_objects::NAME_HEADER,
        )
        .into_bytes();
        answer.extend_from_slice(&body);
        let refused = receive_object(&mut std::io::Cursor::new(answer), &digest, &into, "dev-01")
            .expect_err("a repeated Content-Length");
        assert!(refused.contains("Content-Length"), "{refused}");
        assert!(!into.exists(), "a refused answer created a file");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
