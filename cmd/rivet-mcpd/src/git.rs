//! **Git's own smart-HTTP server, fronted — `ADR-035` §4, phase 3.**
//!
//! §4's recommendation is one sentence: *"do not implement a login server in
//! Rust"*, because it keeps the protocol the pivot exists to leave. The same
//! reasoning, one step further, says not to implement git's HTTP server either
//! — and `PLAN.md` §84 found that nobody has to:
//!
//! > `git-http-backend` **is** git's smart-HTTP server: a CGI that handles
//! > `/info/refs?service=…`, `git-upload-pack` and `git-receive-pack`,
//! > including pkt-line framing, the `application/x-git-*` content types, and
//! > the advertisement's service header.
//!
//! It was built by Buildroot on 2026-08-26, sat unused, and `PLAN.md` §85 ran
//! it on a board. This module is the front end for it, and it is deliberately
//! thin: **everything about git is git's**, and everything here is CGI.
//!
//! # This is a CGI front end, not an HTTP one, and the difference bites
//!
//! `crate::bulk` writes HTTP status lines directly — it *is* the server.
//! `git-http-backend` is a **CGI program**: it writes `Status: 404 Not Found`
//! as a *header* and expects its caller to turn that into a status line. The two
//! handlers sit on one listener with opposite conventions, and writing the
//! second by analogy with the first would produce a response whose first line is
//! a header.
//!
//! `PLAN.md` §85.3 recorded that before either was written, which is why it is
//! a paragraph here rather than a bug.
//!
//! # `PATH` is the one thing that must be right
//!
//! `git-http-backend` does not reach `upload-pack` through `--exec-path`. It
//! runs **`git`** and lets that dispatch the builtin. With no `PATH` it emits a
//! **valid but empty advertisement** and puts `error: cannot run upload-pack` on
//! stderr — so a `git clone` reports having cloned an empty repository rather
//! than failing (`PLAN.md` §85.2).
//!
//! That failure is silent to every client, so [`environment`] sets `PATH`
//! explicitly and a test asserts it. It is not left to whatever the supervisor
//! happened to export.

use std::io::{Read as _, Write as _};
use std::process::{Command, Stdio};

use crate::bulk::Response;

/// Everything a `git` client asks for lives under one of these.
///
/// **A closed list rather than "not `/payload/`"**, so a path this build does
/// not serve is refused by name instead of being handed to a CGI that would
/// answer `404` for a different reason. `ADR-035` §4 names exactly these three.
const ROUTES: &[&str] = &["/info/refs", "/git-upload-pack", "/git-receive-pack"];

/// Whether a request target is one `git-http-backend` should be given.
///
/// **Never one naming a repository set aside for deletion**
/// ([`rivet_core::service::is_set_aside`]). A destroyed repository is renamed
/// aside under the repository root and deleted afterwards by a helper
/// (`ADR-017` §3); git serves any repository under its root by path, so
/// without this it would answer for one already destroyed until the helper
/// finished.
#[must_use]
pub fn is_git(target: &str) -> bool {
    let path = target.split('?').next().unwrap_or(target);
    ROUTES.iter().any(|route| path.ends_with(route))
        && !path.split('/').any(rivet_core::service::is_set_aside)
}

/// The CGI environment for one request.
///
/// **`GIT_HTTP_EXPORT_ALL` is set, and that is a decision rather than a
/// convenience.** Without it `git-http-backend` serves only repositories
/// carrying a `git-daemon-export-ok` marker file. Rivet's access control is
/// `ADR-025` §2's pin — a caller reached this code only by presenting a key
/// this machine accepts — and `ADR-032` §4.2 already records that **there is no
/// per-repository access control**: every account that can reach the git
/// service can reach every repository. A marker file would be a second,
/// weaker answer to a question the pin already settles (`ADR-011`).
///
/// **`PATH` is set explicitly**, and this module's header says why.
#[must_use]
pub fn environment(
    method: &str,
    path_info: &str,
    query: &str,
    content_type: Option<&str>,
    content_encoding: Option<&str>,
    content_length: Option<u64>,
    principal: Option<&str>,
) -> Vec<(String, String)> {
    let mut env = vec![
        ("REQUEST_METHOD".to_string(), method.to_string()),
        ("PATH_INFO".to_string(), path_info.to_string()),
        ("QUERY_STRING".to_string(), query.to_string()),
        (
            "GIT_PROJECT_ROOT".to_string(),
            rivet_core::git::REPOSITORY_ROOT.to_string(),
        ),
        ("GIT_HTTP_EXPORT_ALL".to_string(), "1".to_string()),
        // The whole of what §85.2 measured.
        (
            "PATH".to_string(),
            parent_of(rivet_core::git::BINARY).to_string(),
        ),
        // git will look for a configuration file it must not find: a machine's
        // git behaviour is the image's and the document's, not a home
        // directory's. `PLAN.md` §43.6 proved a push works with this exact
        // value.
        ("HOME".to_string(), "/nonexistent".to_string()),
    ];
    // **`REMOTE_USER` is what makes a PUSH possible, and it is the pin.**
    //
    // `git-http-backend` serves `git-receive-pack` only when `http.receivepack`
    // is true in the repository, **or** when the caller is authenticated — which
    // it decides by whether the gateway set `REMOTE_USER`. Without it a push is
    // refused with `403` and the repository is read-only, which `PLAN.md` §88.9
    // measured on the board.
    //
    // Setting it here is not a loosening. A caller reached this code only by
    // completing a TLS handshake against `ADR-025` §2's mutual pin, so it *is*
    // authenticated, and the pinned key is the whole of what this machine knows
    // about it. Writing anything else — a fixed `git`, a name from the
    // certificate's cosmetic subject — would be a second answer to a question
    // the pin already settles (`ADR-011`), so the value is the same
    // `pinned-<hex>` principal `ADR-028` §4.5 records for an install.
    //
    // **Absent when there is no pin.** A caller over the plain loopback
    // transport presented nothing, so it is not authenticated and git will
    // refuse its push — which is the correct answer rather than an oversight.
    if let Some(who) = principal {
        env.push(("REMOTE_USER".to_string(), who.to_string()));
    }
    if let Some(kind) = content_type {
        env.push(("CONTENT_TYPE".to_string(), kind.to_string()));
    }
    // **A gzipped body must be SAID to be gzipped**, or `git-http-backend`
    // copies it to `upload-pack` as it arrived. `git` compresses a fetch's
    // request once it passes 1 KiB — a clone with history negotiating against
    // a server that has moved on — and upload-pack then reads the gzip magic as
    // a pkt-line length: `fatal: protocol error: bad line length character`,
    // and the client sees `the remote end hung up unexpectedly`. The CGI reads
    // the header as `HTTP_CONTENT_ENCODING`, RFC 3875 §4.1.18's spelling for a
    // request header, and inflates. Found 2026-09-14 by
    // `tools/qemu/adr-035-git-proof.sh`, on a guest.
    if let Some(encoding) = content_encoding {
        env.push(("HTTP_CONTENT_ENCODING".to_string(), encoding.to_string()));
    }
    if let Some(length) = content_length {
        env.push(("CONTENT_LENGTH".to_string(), length.to_string()));
    }
    env
}

/// The directory a path is in. `/system/bin/git` → `/system/bin`.
fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(directory, _)| directory)
}

/// Split a CGI response into its status line and the rest.
///
/// CGI's `Status:` header is the program's way of setting an HTTP status, and
/// **the header itself must not be forwarded** — it is an instruction to the
/// gateway, not part of the response. Everything else is.
///
/// # Errors
///
/// If the header block never ends. A CGI program that produced no blank line
/// produced no response, and forwarding what it did write would be forwarding
/// a fragment as though it were whole.
pub fn translate(head: &str) -> Result<(u16, &'static str, String), String> {
    let mut status = (200u16, "OK");
    let mut forwarded = String::new();

    for line in head.split("\r\n").flat_map(|l| l.split('\n')) {
        if line.is_empty() {
            continue;
        }
        if let Some(value) = line.strip_prefix("Status:").map(str::trim) {
            let mut parts = value.splitn(2, ' ');
            let code = parts.next().unwrap_or_default().parse::<u16>().map_err(|_| {
                format!("git-http-backend wrote a Status header this build cannot read: {line:?}")
            })?;
            status = (code, reason(code));
            continue;
        }
        forwarded.push_str(line);
        forwarded.push_str("\r\n");
    }
    Ok((status.0, status.1, forwarded))
}

/// The reason phrase for the statuses `git-http-backend` actually sets.
///
/// **A closed list, and anything else is `"Status"`.** A reason phrase is
/// decoration — RFC 9110 says a client must not act on it — so inventing one
/// for an unknown code is safer than refusing a response git meant to send.
fn reason(code: u16) -> &'static str {
    match code {
        200 => "OK",
        304 => "Not Modified",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        _ => "Status",
    }
}

/// Run `git-http-backend` for one request and hand back what to send.
///
/// **The body is streamed in and the answer read out**, because a clone's pack
/// is as large as the repository. Nothing here parses git's output: the pkt-line
/// framing, the content types and the advertisement are git's, and this module
/// exists so that they stay git's.
///
/// # Errors
///
/// A [`Response`] to send instead, when the program cannot be run or answers
/// with something this build will not forward.
pub fn serve(
    request: &crate::bulk::Request,
    body: &[u8],
    more: &mut dyn FnMut(&mut [u8]) -> std::io::Result<usize>,
    remaining: u64,
    principal: Option<&str>,
) -> Result<(u16, &'static str, String, Vec<u8>), Response> {
    let method = request.method.as_str();
    let target = request.target.as_str();
    let (path_info, query) = target.split_once('?').unwrap_or((target, ""));
    let length = u64::try_from(body.len()).unwrap_or(0).saturating_add(remaining);

    let mut child = Command::new(rivet_core::git::HTTP_BACKEND)
        // **The environment is REPLACED, not extended, and this comes FIRST.** Whatever started this
        // daemon may have exported anything, and git reads a great many
        // variables — `GIT_DIR` above all, which would silently redirect every
        // request to one repository.
        .env_clear()
        .envs(environment(
            method,
            path_info,
            query,
            request.content_type.as_deref(),
            request.content_encoding.as_deref(),
            (method == "POST").then_some(length),
            principal,
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Inherited: git's diagnostics belong in the journal beside everything
        // else this daemon says (`ADR-022` §4), and §85.2's `error: cannot run
        // upload-pack` is exactly the line an operator needs to see.
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| {
            Response {
                status: 500,
                reason: "Internal Server Error",
                body: format!(
                    "cannot run {}: {e}\n\
                     \x20 note  ADR-035 §4 serves repositories with git's own smart-HTTP \
                     program, and cmd/rivet-image installs it. A card built before 2026-08-28 \
                     does not carry it (PLAN.md §84).\n",
                    rivet_core::git::HTTP_BACKEND
                ),
            }
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        let sent = stdin
            .write_all(body)
            .and_then(|()| {
                let mut chunk = vec![0u8; 64 * 1024];
                let mut left = remaining;
                while left > 0 {
                    let want = usize::try_from(left.min(64 * 1024)).unwrap_or(64 * 1024);
                    let got = more(&mut chunk[..want])?;
                    if got == 0 {
                        break;
                    }
                    stdin.write_all(&chunk[..got])?;
                    left -= got as u64;
                }
                Ok(())
            });
        // Dropped before waiting: git reads until end of input, and a pipe still
        // open is a program still waiting — the same deadlock `rivet-remote`'s
        // TLS transport hit on 2026-08-28 (`PLAN.md` §81.2).
        drop(stdin);
        sent.map_err(|e| Response {
            status: 400,
            reason: "Bad Request",
            body: format!("the request body could not be handed to git: {e}\n"),
        })?;
    }

    let mut answer = Vec::new();
    if let Some(mut stdout) = child.stdout.take() {
        stdout.read_to_end(&mut answer).map_err(|e| Response {
            status: 500,
            reason: "Internal Server Error",
            body: format!("git's answer could not be read: {e}\n"),
        })?;
    }
    let _ = child.wait();

    let at = find_blank_line(&answer).ok_or_else(|| Response {
        status: 500,
        reason: "Internal Server Error",
        body: "git-http-backend produced no header block, so it produced no response. Its own \
               diagnostics are in this machine's journal.\n"
            .to_string(),
    })?;
    let (head, rest) = answer.split_at(at.0);
    let head = String::from_utf8_lossy(head).into_owned();
    let (status, reason, headers) = translate(&head).map_err(|why| Response {
        status: 500,
        reason: "Internal Server Error",
        body: format!("{why}\n"),
    })?;

    Ok((
        status,
        reason,
        headers,
        rest.get(at.1..).unwrap_or_default().to_vec(),
    ))
}

/// Where the header block ends, and how long the separator is.
///
/// **Both spellings**, because a CGI program is entitled to either and
/// `git-http-backend` writes `\r\n`. Accepting only one would work until it did
/// not.
fn find_blank_line(bytes: &[u8]) -> Option<(usize, usize)> {
    if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
        return Some((at, 4));
    }
    bytes.windows(2).position(|w| w == b"\n\n").map(|at| (at, 2))
}

/// The HTTP response head to send in front of git's body.
///
/// # The blank line is the whole point
///
/// A response head ends with an EMPTY line, and forgetting it does not produce
/// a missing-newline error: the client reads the first bytes of the body as
/// another header. With a pack that means `curl` answering **"Header without
/// colon"** and a clone that fails for a reason nowhere near the cause — which
/// is exactly what `PLAN.md` §88.6 hit on the board, after the routing was
/// already right.
///
/// `headers` is git's own, forwarded whole, and [`translate`] has already
/// terminated each of its lines. `Content-Length` among them is git's: it
/// wrote one or it did not, and writing one here would be this build
/// describing a body it did not produce.
#[must_use]
pub fn response_head(status: u16, reason: &str, headers: &str) -> String {
    format!("HTTP/1.1 {status} {reason}\r\n{headers}Connection: close\r\n\r\n")
}

#[cfg(test)]
#[path = "git_tests.rs"]
mod tests;
