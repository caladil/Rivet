//! `cargo rivet backup` — **fetch what a machine's applications offer, check
//! it, and store it.** `ADR-051`.
//!
//! `ADR-030` ruled application data out of Rivet's own backup — whole `/state`
//! *"remains the operator's"* — so TempMon's readings and Reliquary's blobs
//! were covered by nothing. `rivet_core::backup` is the
//! contract that lets an application offer them; this is the consumer that
//! reads it, and it exists because **a contract with no consumer is worse than
//! no contract, because it looks built**. `ADR-045` shipped a health vocabulary
//! and an SDK `report` call the supervisor read nowhere, and that is the
//! cautionary tale this verb was written against.
//!
//! # One transport: the control plane
//!
//! `ADR-035`'s control plane, port 7777, mutually authenticated with the
//! operator's pinned key (`.rulings/` R-234). The table is read with one
//! `get file /run/rivet/backups`, exactly as `crate::screenshot` reads a capture
//! off a board, and each payload is fetched with `GET /backup/<service>` on the
//! same listener: the machine reaches the application over loopback, as
//! `rivet-proxyd` does, and streams its answer back (`ADR-051` §8).
//!
//! **No certificate other than the operator's control-plane identity is
//! presented, ever.** `--authority`, `--name` and `--port` named the fleet's
//! browser certificate and the proxy it was presented to; they are refused by
//! name rather than ignored, so a script still passing them finds out.
//!
//! # Retention, cadence and scheduling are NOT here
//!
//! **This verb fetches once and stores.** How often, how many to keep and where
//! they go afterwards are an operator's decisions and belong to a program that
//! makes them; this is the headless, testable half that a scheduler drives.
//! There is deliberately no `--keep`, no `--daily` and no state file — a verb
//! that remembered anything would be the beginning of one.
//!
//! # `--service NAME` filters WHICH applications, not how often
//!
//! Repeatable. Named once or more, only those applications' bytes cross the
//! network — `filter_by_service` narrows the table before `fetch_one` is
//! ever called for anything, not after. A version of this that fetched every
//! application and discarded the ones the operator did not name would be the
//! defect this flag exists to close wearing a flag of its own.
//!
//! A name matching nothing on the machine is an **error**, not an empty fetch.
//! An application renamed on one side and not the other leaves a filter naming
//! nobody, and fetching quietly in that case would leave an operator believing
//! a backup had run when nothing was asked for at all. `ADR-051` §8.
//!
//! # What it refuses to store, and why the ladder is where it is
//!
//! Every check is `rivet_core::backup::validate`'s, which is in the core rather
//! than here for `ADR-011`'s reason: the supervisor derives the declaration and
//! the client checks the payload, and *what a SQLite backup must look like* is
//! one fact. **The ladder runs here, on the workstation**, on the bytes as they
//! arrived — the machine relays and checks nothing about the format. **Nothing
//! here validates a schema.** *"Is this a valid SQLite database"* cannot rot;
//! *"is this a valid Trends backup"* rots at every migration.
//!
//! # The feed half is DEFERRED, and this says so rather than pretending
//!
//! `rivet_core::backup` carries `BackupSurface::Feed` because the manifest has
//! to be able to express it — an application declaring a feed must be refused
//! or understood, not silently misread as an artifact. This verb **reports a
//! feed and does not fetch it**, and `usage` says what is missing.

use std::io::Read as _;
use std::path::{Component, Path, PathBuf};

use rivet_remote::endpoint::{Endpoint, IDENTITY_AND_PIN};
use rivet_remote::tls::CallsTools;

use rivet_core::backup::Declaration;
use rivet_core::json::Json;
use rivet_core::package::application::{BackupFormat, BackupSurface};

/// The largest body this will hold in memory.
///
/// A bound exists because the bytes come from a service on another machine, and
/// this end reads the whole thing before it validates any of it — the ladder's
/// fourth rung is a statement about the whole file. 512 MiB is far past
/// anything this fleet produces (a TempMon database is single-digit megabytes,
/// a Reliquary archive tens of megabytes) and is still a number.
///
/// **`cargo rivet restore` holds the same bound in the other direction**
/// (`ADR-051` §12): a file this verb would have refused to fetch is a file that
/// one refuses to send.
pub(crate) const MAX_BODY: u64 = 512 * 1024 * 1024;

/// The largest answer head read before refusing.
const MAX_HEAD: usize = 64 * 1024;

/// How long one fetch or restore may sit with nothing moving.
///
/// **Generous, because the thing being fetched is a database.** A backup of two
/// years of readings over a home network is seconds; five minutes with no byte
/// in either direction is the point at which something is wrong rather than
/// slow — a machine or an application that stopped answering, which would
/// otherwise hold a scheduler's run for ever.
pub(crate) const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

pub fn usage() -> String {
    format!(
        "usage: cargo rivet backup --host HOST:PORT --identity DIR --pin HEX --into DIR [confirm]\n\
         \n\
         Reads what a machine's applications OFFER to back up, fetches each one\n\
         over the control plane, checks it, and writes it into --into. ADR-051.\n\
         \n\
         WITHOUT `confirm` IT FETCHES NOTHING. It reads the machine's table and\n\
         prints what it would fetch and where each file would go, which is how\n\
         to see the plan before anything is written.\n\
         \n\
         backup options:\n\
         \x20 --host HOST:PORT   REQUIRED. Where rivet-mcpd is listening. Both the\n\
         \x20                    table and every payload come over it.\n\
         {IDENTITY_AND_PIN}\
         \x20 --into DIR         REQUIRED. One subdirectory per application is\n\
         \x20                    created under it, so two applications cannot\n\
         \x20                    name one file.\n\
         \x20 --service NAME     repeatable. Fetch only the named application(s)\n\
         \x20                    -- their bytes cross the network, nothing else's\n\
         \x20                    does. Absent, every application the machine\n\
         \x20                    offers is fetched. A NAME the machine does not\n\
         \x20                    offer is an ERROR naming both what was asked for\n\
         \x20                    and what the machine actually offers -- not a\n\
         \x20                    silent empty run.\n\
         \x20 confirm            fetch and write. Without it, the plan.\n\
         \n\
         HOW A PAYLOAD TRAVELS: the machine fetches the application's own\n\
         backup route over loopback and streams it back on the control plane\n\
         (R-234). Nothing but your control-plane identity is presented, so\n\
         --authority, --name and --port -- which named the fleet's browser\n\
         certificate and the proxy on 443 -- are REFUSED, not ignored.\n\
         \n\
         THERE IS NO SCHEDULE AND NO RETENTION HERE, deliberately. This fetches\n\
         once and stores; how often and how many to keep are an operator's\n\
         decisions and belong to whatever drives this. --service is WHICH\n\
         applications, never how often or how many.\n\
         \n\
         WHAT IS CHECKED, in order, and every rung is about the FORMAT:\n\
         \x20 1 non-empty\n\
         \x20 2 bytes received == Content-Length\n\
         \x20 3 the format's magic\n\
         \x20 4 the format's own statement about its own size\n\
         Rung 3 alone is a trap: a truncated SQLite file still begins `SQLite\n\
         format 3`, looks like a backup, and would rotate a real one out. Not\n\
         every format has every rung -- CSV has 1 and 2 and nothing else -- and\n\
         the report says which ran rather than printing one word for all four.\n\
         NOTHING HERE VALIDATES A SCHEMA. `is this a valid SQLite database`\n\
         cannot rot; `is this a valid Trends backup` rots at every migration.\n\
         \n\
         A FEED IS REPORTED AND NOT FETCHED. `backup-feed` is the resumable\n\
         shape -- a cursor, a low-water mark, items named by their own digest --\n\
         and this build implements the artifact half only. An application\n\
         declaring one is listed with that said out loud.\n"
    )
}

/// What `backup` was asked to do.
#[derive(Debug)]
pub struct BackupArgs {
    pub endpoint: Endpoint,
    pub into: PathBuf,
    /// `--service NAME`, repeatable. Empty means what it always meant: every
    /// application the machine's table names. See [`filter_by_service`].
    pub services: Vec<String>,
    pub confirm: bool,
}

/// **The refusal for a flag that reached the proxy**, which no transport here
/// does any longer.
///
/// Refused rather than accepted and ignored: a scheduler still passing
/// `--authority` believes it is choosing the certificate a backup presents, and
/// silently ignoring the flag would leave it believing that.
pub(crate) fn retired_flag(flag: &str, verb: &str) -> String {
    format!(
        "{flag} is not an option `{verb}` takes any more.\n\
         \x20 why  it named the fleet's browser certificate (--authority), the name on the \
         machine's certificate (--name) or the proxy's port (--port), and `{verb}` no longer \
         reaches the proxy: the payload travels over the control plane, presenting your \
         control-plane identity and nothing else (ADR-051 §8, R-234).\n\
         \x20 fix  remove {flag} and its value; --host, --identity and --pin are all the \
         transport needs."
    )
}

/// # Errors
///
/// A flag with no value, a flag this verb does not take, a missing `--into`, or
/// either half of `ADR-025` §2's trust without the other.
pub fn parse(args: &[String]) -> Result<BackupArgs, String> {
    let mut endpoint = Endpoint::default();
    let mut into: Option<PathBuf> = None;
    let mut services: Vec<String> = Vec::new();
    let mut confirm = false;

    let mut index = 0;
    while index < args.len() {
        if let Some(next) = endpoint.read(args, index)? {
            index = next;
            continue;
        }
        let value = |flag: &str| -> Result<String, String> {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match args[index].as_str() {
            "--into" => {
                into = Some(PathBuf::from(value("--into")?));
                index += 2;
            }
            flag @ ("--authority" | "--name" | "--port") => {
                return Err(retired_flag(flag, "backup"));
            }
            // **Repeatable, and collected in the order given.** Order does not
            // matter to `filter_by_service` -- it is a membership test -- but
            // keeping it is free and makes the error naming "what was asked
            // for" read the way the operator typed it.
            "--service" => {
                services.push(value("--service")?);
                index += 2;
            }
            "confirm" => {
                confirm = true;
                index += 1;
            }
            other => {
                return Err(format!(
                    "{other:?} is not an option `backup` takes.\n\n{}",
                    usage()
                ));
            }
        }
    }
    endpoint.paired()?;

    let into = into.ok_or_else(|| {
        format!(
            "--into is required: there is nowhere to put what this fetches, and a default \
             would be this tool choosing where somebody's backups live.\n\n{}",
            usage()
        )
    })?;

    Ok(BackupArgs {
        endpoint,
        into,
        services,
        confirm,
    })
}

/// Read the machine's table over the control plane.
///
/// # Errors
///
/// The machine's own refusal, an answer about another path, a path that is not
/// there, or content this end will not decode.
pub(crate) fn read_table(transport: &dyn CallsTools) -> Result<Vec<Declaration>, String> {
    let path = rivet_core::backup::PATH;
    let call = rivet_remote::command::translate(&["get", "file", path])?;
    let answer = transport.call_tool(&call.tool, &call.arguments)?;
    let json = rivet_core::json::parse(&answer).map_err(|why| {
        format!("the machine's answer is not JSON this build can read ({why}). It said:\n{answer}")
    })?;
    let string = |key: &str| json.get(key).and_then(Json::as_str);

    // **The answer is checked to be about the question**, for the reason
    // `crate::screenshot` checks it: one connection can carry several calls,
    // and an answer bound to the wrong one would produce a plan for the wrong
    // machine with nothing saying so.
    match string("path") {
        Some(seen) if seen == path => {}
        Some(seen) => {
            return Err(format!(
                "this asked about {path} and the machine answered about {seen}. Nothing is read \
                 from it."
            ));
        }
        None => {
            return Err(format!(
                "the machine's answer carries no `path`, so this build cannot tell what it is \
                 about. It said:\n{answer}"
            ));
        }
    }

    if json.get("present") != Some(&Json::Bool(true)) {
        return Err(format!(
            "there is nothing at {path} on that machine.\n\
             \x20 why  the supervisor writes it every convergence, even when it is empty — so \
             an absent file means this machine is running a Rivet from before ADR-051 rather \
             than one whose applications offer nothing.\n\
             \x20 fix  install a build that publishes it, or check `rivet explain {path}`."
        ));
    }
    let Some(content) = string("content") else {
        let why = string("content_omitted").unwrap_or(
            "the machine gave no reason, which is itself worth reporting: `get file` promises \
             `content_omitted` whenever `content` is absent",
        );
        return Err(format!(
            "the machine read {path} and would not give its contents:\n\x20 {why}"
        ));
    };
    // The table is ASCII by construction, so `text` is what a machine writing
    // it correctly produces. Base64 would mean the file is not UTF-8, which
    // means it is not this file.
    let text = match string("content_encoding") {
        Some("text") => content.to_string(),
        Some("base64") => {
            let bytes = rivet_core::base64::decode(content)
                .ok_or("the machine said `content` is base64 and it does not decode")?;
            String::from_utf8(bytes)
                .map_err(|_| format!("{path} is not text, so it is not the backup table"))?
        }
        other => {
            return Err(format!(
                "the machine says `content_encoding` is {other:?}, which this build does not \
                 read."
            ));
        }
    };
    Ok(rivet_core::backup::parse(&text))
}

/// Split the machine's whole table by the operator's `--service` filter.
///
/// **`execute` calls this once, on the table [`read_table`] returned, before
/// the plan is rendered and before the first fetch, and nowhere else narrows
/// the table.** Both the plan and the fetch loop read `included` from that
/// one call, so what the plan promises is exactly what the fetch does.
///
/// With `wanted` empty this is the identity: every declaration comes back in
/// `included`, in the order the table gave them, and `excluded` is empty.
///
/// # Errors
///
/// Any name in `wanted` this machine's table does not carry — naming both
/// what was asked for and what the machine actually offers. **This is an
/// error and never a silent empty fetch.**
fn filter_by_service<'a>(
    declarations: &'a [Declaration],
    wanted: &[String],
) -> Result<(Vec<&'a Declaration>, Vec<&'a Declaration>), String> {
    if wanted.is_empty() {
        return Ok((declarations.iter().collect(), Vec::new()));
    }

    let offered: Vec<&str> = declarations.iter().map(|d| d.service.as_str()).collect();
    let missing: Vec<&str> = wanted
        .iter()
        .map(String::as_str)
        .filter(|name| !offered.contains(name))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "--service asked for {} which this machine does not offer.\n\
             \x20 asked    {}\n\
             \x20 offers   {}\n\
             \x20 why      an application renamed on one side and not the other leaves a \
             filter matching nothing, and fetching quietly in that case would leave you \
             believing a backup ran when nothing was asked for at all (ADR-051 §8).",
            missing.join(", "),
            wanted.join(", "),
            if offered.is_empty() {
                "nothing -- no application on this machine offers a backup at all".to_string()
            } else {
                offered.join(", ")
            },
        ));
    }

    let included = declarations
        .iter()
        .filter(|d| wanted.iter().any(|name| name == &d.service))
        .collect();
    let excluded = declarations
        .iter()
        .filter(|d| !wanted.iter().any(|name| name == &d.service))
        .collect();
    Ok((included, excluded))
}

// ---------------------------------------------------------------------------
// The payload, over the control plane
// ---------------------------------------------------------------------------

/// **What came back for one backup or restore request** — the application's
/// answer as the machine relayed it, or the machine's own refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Relayed {
    /// The status code on the first line.
    pub status: u16,
    /// Every header, its name lower-cased, in the order sent.
    pub headers: Vec<(String, String)>,
    /// The body, decoded from its framing, up to the bound it was read with.
    pub body: Vec<u8>,
    /// Whether the body was longer than the bound and was cut — only for a
    /// read that asked to truncate rather than refuse.
    pub truncated: bool,
}

impl Relayed {
    /// One header's value.
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// **Whether the MACHINE composed this answer** rather than relaying the
    /// application's — `rivet_core::backup::ANSWERED_BY_HEADER`.
    pub(crate) fn answered_by_the_machine(&self) -> bool {
        self.header(rivet_core::backup::ANSWERED_BY_HEADER).is_some()
    }

    /// The `Content-Length` the application declared — rung 2's other
    /// operand. `None` for a chunked body, which states no length up front.
    pub(crate) fn declared_length(&self) -> Option<u64> {
        if self.header("transfer-encoding").is_some() {
            return None;
        }
        self.header("content-length").and_then(|v| v.trim().parse().ok())
    }

    /// The body as text, for a refusal.
    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).trim_end().to_string()
    }
}

/// What a body longer than the bound means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Overflow {
    /// A refusal: the whole of it was needed, as for a backup.
    Refuse,
    /// Keep the bound's worth and say so, as for an application's answer to a
    /// restore, which is only printed.
    Truncate,
}

/// **The one thing these two verbs ask of a machine besides its table.**
///
/// A trait so the verbs — plan, refusals, the ladder, the report — are tested
/// against a fake machine, and so the real one is one implementation over
/// `rivet_remote::tls::Tls`.
pub(crate) trait ReachesApplications {
    /// `GET /backup/<service>` — the application's backup, as relayed.
    ///
    /// # Errors
    ///
    /// No answer arrived, or a body over [`MAX_BODY`]. An answer of any status
    /// is `Ok`: judging it is the caller's.
    fn fetch(&self, service: &str) -> Result<Relayed, String>;

    /// `POST /restore/<service>` carrying `body` — the application's answer,
    /// as relayed, cut at `answer_limit` bytes.
    ///
    /// # Errors
    ///
    /// No answer arrived.
    fn restore(&self, service: &str, body: &[u8], answer_limit: u64) -> Result<Relayed, String>;
}

impl ReachesApplications for rivet_remote::tls::Tls {
    fn fetch(&self, service: &str) -> Result<Relayed, String> {
        let head = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            rivet_core::backup::backup_route(service),
            self.host
        );
        let mut reader = self.http(&head, &mut std::io::empty(), 0, TIMEOUT)?;
        read_answer(&mut reader, MAX_BODY, Overflow::Refuse)
            .map_err(|why| format!("{service}: {} answered {why}", self.host))
    }

    fn restore(&self, service: &str, body: &[u8], answer_limit: u64) -> Result<Relayed, String> {
        let head = format!(
            "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            rivet_core::backup::restore_route(service),
            self.host,
            body.len()
        );
        let mut reader = self.http(&head, &mut &body[..], body.len() as u64, TIMEOUT)?;
        read_answer(&mut reader, answer_limit, Overflow::Truncate)
            .map_err(|why| format!("{service}: {} answered {why}", self.host))
    }
}

/// **Read one HTTP answer**: its status, its headers, and its body by the
/// framing it declares — `Content-Length`, `chunked`, or until the connection
/// ends.
///
/// The body is decoded and nothing else is done to it: no content coding is
/// undone, because rung 2 compares bytes received against the declared length
/// and a decompressed body would compare against the compressed one.
///
/// # Errors
///
/// A head that does not end or is not HTTP, a body over `limit` under
/// [`Overflow::Refuse`], a chunked framing this cannot read, or a connection
/// that failed. A body that simply ends short of its `Content-Length` is NOT
/// an error here — the ladder's second rung is where that is judged and
/// reported with the numbers.
pub(crate) fn read_answer(
    reader: &mut dyn std::io::Read,
    limit: u64,
    overflow: Overflow,
) -> Result<Relayed, String> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut piece = vec![0u8; 64 * 1024];
    let at = loop {
        if let Some(at) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        if buffer.len() > MAX_HEAD {
            return Err(format!("a head that did not end within {MAX_HEAD} bytes"));
        }
        let got = reader
            .read(&mut piece)
            .map_err(|e| format!("nothing readable: {e}"))?;
        if got == 0 {
            return Err(format!(
                "nothing complete: the connection closed inside the answer head ({} byte(s))",
                buffer.len()
            ));
        }
        buffer.extend_from_slice(&piece[..got]);
    };
    let head = String::from_utf8_lossy(&buffer[..at]).into_owned();
    let rest = buffer[at + 4..].to_vec();
    let mut lines = head.split("\r\n");
    let first = lines.next().unwrap_or_default();
    let status: u16 = first
        .strip_prefix("HTTP/1.")
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| format!("a first line that is not an HTTP status: {first:?}"))?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    // **A repeated `Content-Length` is refused whether or not the values agree**,
    // as every request parser here refuses one: two readers of the same bytes
    // that honour different fields disagree about where the body ends, and the
    // ladder's length rung would be judging one of them.
    let lengths = headers.iter().filter(|(n, _)| n == "content-length").count();
    if lengths > 1 {
        return Err(format!(
            "an answer carrying {lengths} Content-Length headers, so where its body ends is not \
             one answer"
        ));
    }
    let mut answer = Relayed {
        status,
        headers,
        body: Vec::new(),
        truncated: false,
    };

    let mut source = std::io::Cursor::new(rest).chain(reader);
    let chunked = answer
        .header("transfer-encoding")
        .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"));
    if chunked {
        answer.body = read_chunked(&mut source, limit, overflow, &mut answer.truncated)?;
    } else {
        let declared = answer.declared_length();
        if let (Some(length), Overflow::Refuse) = (declared, overflow)
            && length > limit
        {
            return Err(format!(
                "a body of {length} bytes, over the {limit}-byte bound. The bound is a policy \
                 rather than a measurement — this end holds the whole file before it checks any \
                 of it, because the last rung is a statement about the whole file"
            ));
        }
        let want = declared.map_or(limit + 1, |n| n.min(limit + 1));
        let mut taken = (&mut source).take(want);
        if let Err(e) = taken.read_to_end(&mut answer.body) {
            // A cut connection after part of a body: kept, and judged by the
            // ladder, which names how much arrived.
            if answer.body.is_empty() && declared != Some(0) {
                return Err(format!("a body that could not be read: {e}"));
            }
        }
        if answer.body.len() as u64 > limit {
            match overflow {
                Overflow::Refuse => {
                    return Err(format!("a body over the {limit}-byte bound"));
                }
                Overflow::Truncate => {
                    answer.body.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
                    answer.truncated = true;
                }
            }
        }
    }
    Ok(answer)
}

/// A `chunked` body, decoded.
fn read_chunked(
    source: &mut dyn std::io::Read,
    limit: u64,
    overflow: Overflow,
    truncated: &mut bool,
) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    loop {
        let size_line = chunk_line(source)?;
        let size = u64::from_str_radix(size_line.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| format!("a chunk size that is not hexadecimal: {size_line:?}"))?;
        if size == 0 {
            // Trailers, then the blank line.
            while !chunk_line(source)?.is_empty() {}
            return Ok(body);
        }
        if body.len() as u64 + size > limit {
            match overflow {
                Overflow::Refuse => return Err(format!("a body over the {limit}-byte bound")),
                Overflow::Truncate => {
                    *truncated = true;
                    return Ok(body);
                }
            }
        }
        let start = body.len();
        body.resize(start + usize::try_from(size).unwrap_or(0), 0);
        source
            .read_exact(&mut body[start..])
            .map_err(|e| format!("a chunk that ended early: {e}"))?;
        if !chunk_line(source)?.is_empty() {
            return Err("a chunk not followed by its line ending".to_string());
        }
    }
}

/// One `\r\n`-ended line of a chunked body, without its ending.
fn chunk_line(source: &mut dyn std::io::Read) -> Result<String, String> {
    let mut out = Vec::new();
    let mut byte = [0u8; 1];
    while !out.ends_with(b"\r\n") {
        if out.len() > 1024 {
            return Err("a chunk-size line longer than 1 KiB".to_string());
        }
        match source.read(&mut byte) {
            Ok(1) => out.push(byte[0]),
            Ok(_) => return Err("a chunked body that ended before its last chunk".to_string()),
            Err(e) => return Err(format!("a chunked body that could not be read: {e}")),
        }
    }
    Ok(String::from_utf8_lossy(&out[..out.len() - 2]).into_owned())
}

/// What one fetch produced, for the report.
pub(crate) struct Stored {
    pub(crate) service: String,
    pub(crate) file: PathBuf,
    pub(crate) report: rivet_core::backup::Report,
}

/// **Fetch one artifact over the control plane**, then hand it to [`store`].
///
/// Kept apart from the filesystem half below so that half can be driven
/// directly, with bytes that never crossed a network at all.
///
/// # Errors
///
/// The machine's refusal, the application's non-2xx answer, a body over
/// [`MAX_BODY`], or anything [`store`] refuses.
pub(crate) fn fetch_one(
    reach: &dyn ReachesApplications,
    declaration: &Declaration,
    into: &Path,
    format: BackupFormat,
) -> Result<Stored, String> {
    let answer = reach.fetch(&declaration.service)?;
    if answer.answered_by_the_machine() {
        return Err(format!(
            "{}: the machine refused before the application was reached ({}):\n{}",
            declaration.service,
            answer.status,
            answer.text()
        ));
    }
    if !(200..300).contains(&answer.status) {
        return Err(format!(
            "{}: the application answered {} for GET {}:\n{}",
            declaration.service,
            answer.status,
            declaration.request_path(),
            answer.text()
        ));
    }
    let offered_name = answer.header("content-disposition").and_then(disposition_filename);
    store(
        declaration,
        into,
        format,
        &answer.body,
        answer.declared_length(),
        offered_name.as_deref(),
    )
}

/// Whether `name` is safe to join onto a directory as-is. It must be a service
/// name by `rivet_core`'s own rule, and also exactly one plain path component,
/// with no separator, drive, root, `.` or `..` anywhere in it. The component
/// half is checked with [`Path::components`] rather than a blocklist, and is
/// accepted only when the sole component reads back as the whole string, so
/// nothing about it was silently normalised away.
///
/// **Both halves, so the answer is the same on every platform.** The
/// component check alone depends on the host: on Linux a backslash and a colon
/// are ordinary filename bytes, so `C:\x` is one component there. The
/// service-name rule refuses both everywhere.
///
/// **Independent of `rivet_core::backup::parse`'s own check on this same
/// field.** A `Declaration`'s `service` came from another machine over the
/// network, and [`store`] does not trust that whatever built it was that
/// parser.
#[must_use]
fn is_single_plain_component(name: &str) -> bool {
    if rivet_core::service::ServiceName::new(name).is_none() {
        return false;
    }
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(only)), None) => only.to_str() == Some(name),
        _ => false,
    }
}

/// **The filesystem half of a fetch**: check the ladder, then write beneath
/// `<into>/<service>/`. Kept apart from the network half above so it can be
/// driven directly — with a hostile `declaration.service`, or with bytes
/// that never crossed a network at all.
///
/// # It is written to `<name>.partial` and renamed
///
/// A file that failed the ladder must never be left where a name suggests it is
/// a backup, and a process killed mid-write must not leave one either. The
/// rename makes a complete, checked file appear under its own name in one
/// step, so a name under that directory always means a file that passed.
///
/// # Errors
///
/// A `declaration.service` that is not exactly one plain path component, any
/// rung of `rivet_core::backup::validate`, an existing file at the
/// destination, or a file that cannot be written.
pub(crate) fn store(
    declaration: &Declaration,
    into: &Path,
    format: BackupFormat,
    bytes: &[u8],
    declared: Option<u64>,
    offered_name: Option<&str>,
) -> Result<Stored, String> {
    // **Checked before anything else.** The service name becomes a directory
    // component below, and this must not create or write anything before it
    // knows that component stays inside `into` — whether or not the bytes
    // themselves turn out to be a whole backup.
    if !is_single_plain_component(&declaration.service) {
        return Err(format!(
            "{:?} is not usable as a directory name, so nothing was created or written for it.\n\
             \x20 why  a service name becomes a directory component under --into, and this will \
             not create or write anything outside the directory you chose.",
            declaration.service
        ));
    }

    let report = rivet_core::backup::validate(format, bytes, declared).map_err(|fault| {
        format!(
            "{}: what was fetched is NOT stored: {fault}.\n\
             \x20 why  a file that fails a rung of ADR-051's ladder looks exactly like a \
             backup and is not one, and storing it would rotate a real one out.",
            declaration.service
        )
    })?;

    let directory = into.join(&declaration.service);
    std::fs::create_dir_all(&directory)
        .map_err(|e| format!("cannot create {}: {e}", directory.display()))?;
    let file = directory.join(file_name(&declaration.service, offered_name, format));
    // **Refused rather than overwritten.** Whether yesterday's copy should make
    // way for today's is a RETENTION decision, and retention is not this
    // verb's — see the module note.
    if file.exists() {
        return Err(format!(
            "{} already exists, so nothing was written.\n\
             \x20 why  replacing one backup with another is a retention decision and this verb \
             makes none. Move it, or fetch into a different --into.",
            file.display()
        ));
    }
    let partial = directory.join(format!(
        "{}.partial",
        file.file_name().and_then(|n| n.to_str()).unwrap_or("backup")
    ));
    std::fs::write(&partial, bytes)
        .map_err(|e| format!("cannot write {}: {e}", partial.display()))?;
    std::fs::rename(&partial, &file).map_err(|e| {
        format!(
            "cannot rename {} to {}: {e}",
            partial.display(),
            file.display()
        )
    })?;

    Ok(Stored {
        service: declaration.service.clone(),
        file,
        report,
    })
}

/// What the stored file is called.
///
/// **The service's own `Content-Disposition` filename when it gave one**, and
/// that is not deference for its own sake: TempMon names its backup for the
/// instant it was taken, in UTC, so a directory of them sorts lexicographically
/// into chronological order — which is precisely what somebody keeping the last
/// N of them needs and is a fact only the service holds.
///
/// `offered` has already been through [`disposition_filename`], which is
/// where it is **sanitised to one path component** — the name came from
/// another machine, and a header saying `../../id_ed25519` must not be able
/// to name a file outside the directory this verb created.
///
/// Where the service offered no name, `<service>-<seconds since the epoch>.<ext>`.
/// Not a civil date: this crate has no date library and adding one to spell a
/// filename would be a dependency for a cosmetic.
fn file_name(service: &str, offered: Option<&str>, format: BackupFormat) -> String {
    if let Some(offered) = offered {
        return offered.to_string();
    }
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{service}-{seconds}.{}", format.extension())
}

/// The `filename="…"` of a `Content-Disposition`, as one safe path component.
///
/// **Refuses rather than repairs.** A name holding a separator, a dot segment
/// or anything outside `[A-Za-z0-9._-]` is not trimmed into shape — it is
/// declined, and the caller falls back to a name this end composed. Repairing a
/// hostile name is how a repaired name ends up being used.
fn disposition_filename(header: &str) -> Option<String> {
    let at = header.to_ascii_lowercase().find("filename=")?;
    let rest = &header[at + "filename=".len()..];
    let raw = rest
        .strip_prefix('"')
        .and_then(|r| r.split('"').next())
        .unwrap_or_else(|| rest.split(';').next().unwrap_or(rest))
        .trim();
    if raw.is_empty()
        || raw.len() > 128
        || raw == "."
        || raw == ".."
        || raw.starts_with('.')
        || !raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
    {
        return None;
    }
    Some(raw.to_string())
}

/// The plan, printed when `confirm` was not given.
///
/// **`excluded` is reported too, whenever `--service` narrowed the table.**
/// `ADR-051` §8 requires the plan to say which applications were excluded by
/// the filter as well as which will be fetched. When `excluded` is empty — the
/// unfiltered case — no section is printed for it.
///
/// **The shape is read by a scheduler** (Rivet Monitor parses the count on the
/// first line, the two-space application names and the four-space `feed`
/// lines), so a change to it is a change to that program's input.
fn plan(included: &[&Declaration], excluded: &[&Declaration], args: &BackupArgs, host: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} application(s) on this machine offer something to back up.\n\n",
        included.len()
    ));
    for one in included {
        match &one.surface {
            BackupSurface::Artifact { format, .. } => out.push_str(&format!(
                "  {}\n    fetch  GET {} on {host}, which fetches the application's {} over \
                 loopback\n    as     {}\n    into   {}\n",
                one.service,
                rivet_core::backup::backup_route(&one.service),
                one.request_path(),
                format,
                args.into.join(&one.service).display(),
            )),
            BackupSurface::Feed { .. } => out.push_str(&format!(
                "  {}\n    feed   {}\n    NOT FETCHED — this build implements the artifact half \
                 only (ADR-051)\n",
                one.service,
                one.request_path(),
            )),
        }
    }
    if included.is_empty() {
        out.push_str(
            "  Nothing is missing: an application offers a backup by declaring backup-path and\n\
             \x20 backup-format, or backup-feed, in its signed manifest. ADR-030 leaves\n\
             \x20 application data to the operator and this is how an application answers that.\n",
        );
    }
    if !excluded.is_empty() {
        out.push_str(&format!(
            "\n{} application(s) excluded by --service -- their bytes will NOT cross the \
             network:\n",
            excluded.len()
        ));
        for one in excluded {
            out.push_str(&format!("  {}\n", one.service));
        }
    }
    out.push_str("\nNothing was fetched. Add `confirm` to fetch and write.\n");
    out
}

/// `backup`, with a real connection behind it.
///
/// # Errors
///
/// See [`backup_via`].
pub fn execute(args: &[String]) -> Result<String, String> {
    let parsed = parse(args)?;
    let transport = parsed.endpoint.open()?.ok_or_else(|| {
        format!(
            "--identity and --pin are required: a backup goes over ADR-035's control plane, \
             and there is no second way to ask.\n\n{}",
            usage()
        )
    })?;
    let host = transport.host.clone();
    backup_via(&parsed, &transport, &transport, &host)
}

/// **The whole backup, against any [`CallsTools`] and any
/// [`ReachesApplications`]** — `execute` passes one connection's worth of the
/// control plane for both.
///
/// # Errors
///
/// This end's refusals, the machine's own refusal, or any fetch that failed —
/// one application's failure does not stop the others, and the report names
/// every one.
pub(crate) fn backup_via(
    parsed: &BackupArgs,
    caller: &dyn CallsTools,
    reach: &dyn ReachesApplications,
    host: &str,
) -> Result<String, String> {
    let declarations = read_table(caller)?;
    // **The filter runs once, here, before the plan is rendered and before
    // the first fetch.** Both branches below read `included` from this one
    // call, so the plan and the fetch loop can never disagree about which
    // applications are in scope — see `filter_by_service`.
    let (included, excluded) = filter_by_service(&declarations, &parsed.services)?;

    if !parsed.confirm {
        return Ok(plan(&included, &excluded, parsed, host));
    }

    let mut stored = Vec::new();
    let mut deferred = Vec::new();
    let mut failed = Vec::new();
    // **`included`, not `declarations`.** An application `--service` excluded
    // is never handed to `fetch_one`, so its bytes never cross the network —
    // the filter is on the fetch, not a discard after the fact.
    for one in included {
        match one.surface.format() {
            Some(format) => match fetch_one(reach, one, &parsed.into, format) {
                Ok(ok) => stored.push(ok),
                // **One application's failure does not stop the others.** A
                // machine's backup run that abandoned four applications because
                // the first was down would be a run that quietly did nothing —
                // and the exit status below still says it failed.
                Err(why) => failed.push(why),
            },
            None => deferred.push(one.service.clone()),
        }
    }

    let mut out = String::new();
    for one in &stored {
        out.push_str(&format!(
            "  {}  {} bytes -> {}\n     checked: {}\n",
            one.service,
            one.report.bytes,
            one.file.display(),
            one.report
                .rungs
                .iter()
                .map(|r| r.describe())
                .collect::<Vec<_>>()
                .join("; ")
        ));
        if !one.report.proved_completeness() {
            out.push_str(
                "     NOTE: this format makes no statement about its own size, so nothing here \
                 proves the file is whole beyond the byte count the server declared.\n",
            );
        }
    }
    for service in &deferred {
        out.push_str(&format!(
            "  {service}  NOT FETCHED — it offers a feed, and this build implements the \
             artifact half only (ADR-051).\n"
        ));
    }
    out.push_str(&format!(
        "\n{} stored, {} deferred, {} failed.\n",
        stored.len(),
        deferred.len(),
        failed.len()
    ));
    if failed.is_empty() {
        return Ok(out);
    }
    Err(format!("{out}\n{}", failed.join("\n")))
}

#[cfg(test)]
#[path = "backup_tests.rs"]
mod tests;
