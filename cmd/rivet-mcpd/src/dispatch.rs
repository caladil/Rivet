//! **Which endpoint a connection wants — `ADR-035` §3.2.**
//!
//! Phase 1's row in `ADR-035` is *"`rivet-mcpd` on TLS, beside what was then
//! still the way in. **Listener designed for §4A(c)**"*, and §3.2 says what
//! that means in one sentence:
//!
//! > So the control plane needs **two endpoints on one listener**: MCP for
//! > operations — the catalogue projection `ADR-006` §3.1 already requires —
//! > and a bulk endpoint — an HTTP `PUT`/`GET` with a content digest — for
//! > payloads.
//!
//! Until 2026-08-28 this daemon terminated TLS and then handed **every**
//! connection to `rivet mcp` without looking at it. That is not *"designed
//! for"* a second endpoint; it is one endpoint with no room for another, and
//! adding the bulk transfer would have meant changing how a connection is
//! dispatched rather than filling in an arm. This module is the dispatch, built
//! in the phase that asked for it so phase 2 is not a retrofit.
//!
//! # Why the first bytes decide, and why that is not a guess
//!
//! The two protocols are trivially distinguishable at their first byte, and
//! neither can be mistaken for the other:
//!
//! * **MCP over stdio is JSON-RPC, one message per line**, so a message begins
//!   with `{`. `cmd/rivet`'s `mcp` module accepts nothing else.
//! * **HTTP begins with a request line** — a method, a space, a target, a
//!   space, and `HTTP/1.x`.
//!
//! So this is a *recognition* rather than a heuristic: each shape is checked
//! against what its own specification requires, and anything matching neither
//! is refused by name rather than assumed to be the more likely one. Guessing
//! would send a malformed HTTP request to a JSON parser and report the failure
//! as bad JSON, which is the wrong error about the wrong thing.
//!
//! # What each answer is
//!
//! `Mcp` reaches `rivet mcp`'s stdin; `Http` reaches [`crate::bulk`], which is
//! `ADR-035` §3.2's payload transfer. Until phase 2 built that, this module
//! answered every HTTP request with a `501` naming the phase — a stub that
//! refused rather than faking success (`AGENTS.md` §3), and it is gone because
//! the thing it stood in for exists.

/// What the first bytes of a connection say it is.
#[derive(Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// JSON-RPC, one message per line — `rivet mcp`'s stdin.
    Mcp,
    /// An HTTP request line. `ADR-035` §3's bulk endpoint, [`crate::bulk`],
    /// and §4's smart HTTP for git, [`crate::git`] — both served; the target
    /// decides which one a request reaches ([`describe_http`]).
    Http,
    /// Neither, quoted back so the refusal names what arrived.
    Unrecognised(String),
}

/// The HTTP methods a request line may open with.
///
/// **A closed list rather than "an uppercase word"**, so that a JSON document
/// with an unusual first character cannot be read as a request line. `GET` and
/// `POST` are §4's smart HTTP; `PUT` and `HEAD` are §3.2's bulk transfer.
const METHODS: &[&str] = &["GET", "POST", "PUT", "HEAD", "OPTIONS", "DELETE", "PATCH"];

/// How much of the first read is quoted back in a refusal.
///
/// Enough to recognise a protocol by eye, short enough that a caller cannot
/// fill the journal by connecting and sending rubbish.
const QUOTED: usize = 40;

/// Classify the first bytes of a connection.
///
/// **Leading whitespace is skipped for the MCP test and not for HTTP**, because
/// the two specifications differ: JSON tolerates leading whitespace before a
/// value, and an HTTP request line does not — RFC 9112 §2.2 has servers ignore
/// at most a leading CRLF, and nothing else may precede the method.
#[must_use]
pub fn classify(first: &[u8]) -> Endpoint {
    let after_crlf = first.strip_prefix(b"\r\n").unwrap_or(first);

    if METHODS
        .iter()
        .any(|m| after_crlf.starts_with(m.as_bytes()) && after_crlf.get(m.len()) == Some(&b' '))
    {
        return Endpoint::Http;
    }

    if first
        .iter()
        .find(|b| !b.is_ascii_whitespace())
        .is_some_and(|b| *b == b'{')
    {
        return Endpoint::Mcp;
    }

    Endpoint::Unrecognised(quote(first))
}

/// The bytes that arrived, rendered so a log line stays one line.
fn quote(bytes: &[u8]) -> String {
    let shown: String = bytes
        .iter()
        .take(QUOTED)
        .map(|b| {
            if b.is_ascii_graphic() || *b == b' ' {
                char::from(*b)
            } else {
                '.'
            }
        })
        .collect();
    if bytes.len() > QUOTED {
        format!("{shown}…")
    } else {
        shown
    }
}

/// Which HTTP handler a request will reach, for the journal.
///
/// **The journal is the record**, and until 2026-08-29 every HTTP request was
/// logged as `ADR-035` §3.2's bulk endpoint — including the git ones, which
/// `crate::bulk` never saw (`PLAN.md` §88.11). A log that names a handler which
/// did not run is worse than one that names none.
///
/// This reads the request TARGET out of the opening bytes and asks
/// [`crate::git::is_git`], so the answer comes from the same predicate the
/// routing uses rather than from a second guess at the same question
/// (`ADR-011`).
#[must_use]
pub fn describe_http(opening: &[u8]) -> &'static str {
    let head = String::from_utf8_lossy(opening);
    let target = head
        .split("
")
        .next()
        .unwrap_or_default()
        .split(' ')
        .nth(1)
        .unwrap_or_default();
    if crate::backups::is_backup(target) {
        "an application's backup or restore, relayed over loopback (ADR-051 §8, §12)"
    } else if crate::git::is_git(target) {
        "ADR-035 §4's git, served by git-http-backend"
    } else if crate::repository::is_repository_objects(target) {
        "a fleet server's repository store (R-184 D1)"
    } else if crate::fleet_as_code::is_fleet_as_code(target) {
        "fleet-as-code's git operations (ADR-044)"
    } else {
        "ADR-035 §3.2's bulk endpoint"
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// The shape `cmd/rivet`'s `mcp` actually receives.
    #[test]
    fn a_json_rpc_message_is_mcp() {
        let line = br#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#;
        assert_eq!(classify(line), Endpoint::Mcp);
        assert_eq!(classify(b"  \r\n {\"jsonrpc\":\"2.0\"}"), Endpoint::Mcp);
    }

    /// Every method §3.2 and §4 need, and the request line shape they share.
    #[test]
    fn a_request_line_is_http() {
        for line in [
            &b"GET /repo/info/refs?service=git-upload-pack HTTP/1.1\r\n"[..],
            &b"POST /repo/git-upload-pack HTTP/1.1\r\n"[..],
            &b"PUT /payload/blake3-abc HTTP/1.1\r\n"[..],
            &b"HEAD / HTTP/1.1\r\n"[..],
        ] {
            assert_eq!(classify(line), Endpoint::Http, "{:?}", String::from_utf8_lossy(line));
        }
    }

    /// **A method-shaped word is not a method.** The list is closed so that a
    /// payload beginning with an uppercase token cannot be read as a request
    /// line — which would send it to an HTTP handler and report the failure as
    /// a bad request rather than as what it is.
    #[test]
    fn an_uppercase_word_that_is_not_a_method_is_not_http() {
        match classify(b"GETTY /x HTTP/1.1\r\n") {
            Endpoint::Unrecognised(what) => assert!(what.starts_with("GETTY"), "{what}"),
            other => panic!("read as {other:?}"),
        }
        // ...and a method with no space after it is not a request line either.
        assert!(matches!(classify(b"GET/x"), Endpoint::Unrecognised(_)));
    }

    /// Anything else is refused by name, with what arrived quoted back.
    #[test]
    fn anything_else_is_refused_and_the_refusal_quotes_what_came() {
        match classify(b"hello?\n") {
            Endpoint::Unrecognised(what) => assert_eq!(what, "hello?."),
            other => panic!("read as {other:?}"),
        }
        match classify(&[0x16, 0x03, 0x01, 0x02, 0x00]) {
            // A TLS ClientHello arriving on a plaintext listener — the mistake
            // an operator makes once, and it should not look like bad JSON.
            Endpoint::Unrecognised(what) => assert_eq!(what, "....."),
            other => panic!("read as {other:?}"),
        }
    }

    /// A long first read is truncated, so a caller cannot fill the journal by
    /// connecting and sending rubbish.
    #[test]
    fn a_long_first_read_is_truncated_in_the_refusal() {
        let noise = vec![b'x'; 500];
        match classify(&noise) {
            Endpoint::Unrecognised(what) => {
                assert!(what.ends_with('…'), "{what}");
                assert!(what.chars().count() <= QUOTED + 1, "{what}");
            }
            other => panic!("read as {other:?}"),
        }
    }

    /// **The log names the handler that will actually run.** It named the bulk
    /// endpoint for every request, git's included, and the journal is the
    /// record (`PLAN.md` §88.11).
    #[test]
    fn the_log_line_names_which_handler_a_request_reaches() {
        let git = b"GET /example-repo.git/info/refs?service=git-upload-pack HTTP/1.1\r\n\r\n";
        assert!(describe_http(git).contains("git"), "{}", describe_http(git));
        let payload = b"PUT /payload/rivet-abc-update.rvt HTTP/1.1\r\n\r\n";
        assert!(describe_http(payload).contains("3.2"), "{}", describe_http(payload));
        let object = b"GET /repository-objects/ HTTP/1.1\r\n\r\n";
        assert!(describe_http(object).contains("repository store"), "{}", describe_http(object));
        let code = b"POST /fleet-as-code/commit HTTP/1.1\r\n\r\n";
        assert!(describe_http(code).contains("fleet-as-code"), "{}", describe_http(code));
        // A request line this build cannot read still gets an answer rather
        // than a panic — a log line is never worth crashing a connection for.
        assert!(!describe_http(b"nonsense").is_empty());
    }

}
