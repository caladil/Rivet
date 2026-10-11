//! **Serving HTTP on a socket a Rivet handed you** — `ADR-040`, behind the
//! optional `serve` feature.
//!
//! # Why this exists, and it is not the line count
//!
//! Three programs in this repository serve HTTP over TLS and each one wrote its
//! own: `rivet-fleetd`, `rivet-proxyd`, and `rivet-mcpd`'s bulk endpoint. Three
//! head parsers, three `rustls::ServerConfig` constructions, two connection
//! caps, and a head deadline that is 30 s in one, 15 s in the next and 10 s in
//! the third for the same operation and no stated reason.
//!
//! **The argument that settled it is the defect record.** The audit of
//! 2026-09-03 found the same class of bug independently in two of them —
//! `rivet-mcpd` had no connection cap and no socket deadlines, `rivet-fleetd`
//! had a 1 KiB read limit that silently truncated a request — and **neither
//! inherited the other's hardening, because there was nothing to inherit
//! from.** This is the thing to inherit from.
//!
//! # It ships the PIECES and never the loop
//!
//! `rivet-fleetd` is one OS thread per connection and `rivet-proxyd` is tokio.
//! **A module that shipped an accept loop could be adopted by one of them.** So
//! there is no loop here, no `spawn`, and nothing that owns a listener: the
//! parser works on a **buffer** the caller filled by whatever means it likes,
//! `Limit` is a counter and not a semaphore that blocks, and `Bounds`
//! carries deadlines as `Duration`s for the caller to apply with
//! `set_read_timeout` or `tokio::time::timeout` as it sees fit.
//!
//! # Every number is the CALLER's
//!
//! One of 30 s, 15 s and 10 s could have been made the rule, which would have
//! made the other two wrong. `Bounds` is the differences, parameterised: the
//! defaults are argued in that type's documentation and a consumer with a
//! reason states its own in one line.
//!
//! ```
//! # use rivet_sdk::serve::Bounds;
//! # use std::time::Duration;
//! let bounds = Bounds { head_patience: Duration::from_secs(15), ..Bounds::default() };
//! ```
//!
//! # What this deliberately does not do
//!
//! **Route, dispatch, or answer.** `Head` hands back a method and a target
//! and stops; what they mean is the application's. **Keep a connection alive.**
//! Every consumer here closes after one exchange, and a shared implementation
//! of something none of them does would be untested by all of them.
//! **Fingerprint a caller's certificate** — `rivet-fleetd` alone wants that and
//! it is eight lines of `sha2`.

use std::time::Duration;

#[cfg(feature = "serve")]
mod tls;

#[cfg(feature = "serve")]
pub use tls::{Tls, provider};

/// The bounds a server puts on a caller it has not yet decided to trust.
///
/// **Defaults, not decrees.** Each is defensible on its own and each is one
/// field of a struct-update away from being something else.
///
/// ```
/// # use rivet_sdk::serve::Bounds;
/// # use std::time::Duration;
/// // `rivet-proxyd`'s number, kept, without forking anything.
/// let bounds = Bounds { head_patience: Duration::from_secs(15), ..Bounds::default() };
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bounds {
    /// The most connections to hold open at once.
    ///
    /// **Sixteen, which is what two of the three consumers already chose
    /// independently**, and both wrote the same reason: one operator is one
    /// session, a fleet page holds one per device it displays, a clone is one
    /// more. Sixteen is several times any of that and small enough that the
    /// thread and descriptor cost is bounded at something a Pi can hold. The
    /// point of the number is that there is one.
    pub connections: usize,

    /// The most bytes of request head to buffer before refusing.
    ///
    /// **16 KiB**, `rivet-fleetd`'s. A request line plus a browser's header
    /// block is a small fraction of it, so this is not a working limit — it is
    /// the point past which the caller is no longer sending a head.
    /// `rivet-proxyd` chose 32 KiB and neither number was measured; the smaller
    /// is the default because the bytes come from whoever connected.
    pub head_bytes: usize,

    /// The largest body to accept, from its declared `Content-Length`.
    ///
    /// **1 MiB**, `rivet-fleetd`'s, because a JSON call is small and an
    /// application that has not thought about it should not be handed a
    /// gigabyte.
    ///
    /// A `u64` and not a `usize` deliberately: `rivet-mcpd`'s bulk endpoint
    /// accepts 1,200,000,000 bytes, which does not fit a 32-bit `usize`, and
    /// the Pi Zero is a deferred compatibility target rather than a dead one.
    pub body_bytes: u64,

    /// How long a caller has to deliver a complete request head.
    ///
    /// **10 s, and this is the one default that overrules a larger number
    /// already in the tree, so here is the argument.** This deadline covers a
    /// peer that has proved nothing: anyone who can reach the port can open a
    /// socket, and without a deadline each one holds a slot until the machine
    /// reboots. [`Bounds::connections`] bounds how many; this bounds how long.
    ///
    /// `rivet-mcpd`'s 10 s is the only one of the three whose choice was
    /// written down — *"before the handshake completes, the peer is nobody"* —
    /// and the other two are larger without a recorded reason. A default has to
    /// be defensible on its own, and *smallest reasoned value* is defensible in
    /// a way that *largest existing value* is not.
    ///
    /// **It is short on purpose and it is not a session timeout.** A caller
    /// that has been admitted may then be given as long as the application
    /// likes; `rivet-mcpd` raises its own to 900 s once a pinned key has been
    /// presented, and that number is not this one.
    pub head_patience: Duration,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            connections: 16,
            head_bytes: 16 * 1024,
            body_bytes: 1024 * 1024,
            head_patience: Duration::from_secs(10),
        }
    }
}

impl Bounds {
    /// Put [`Bounds::head_patience`] on a blocking socket, both directions.
    ///
    /// **The deadline goes on before the handshake**, which is the whole point
    /// of it: until the handshake completes the peer is nobody, and a peer that
    /// opens a socket and says nothing would otherwise hold the connection for
    /// the life of the process.
    ///
    /// For a non-blocking or asynchronous caller there is nothing to apply —
    /// take [`Bounds::head_patience`] and wrap the read in whatever that
    /// runtime uses. `rivet-proxyd` wraps its head read in
    /// `tokio::time::timeout`.
    ///
    /// # Errors
    ///
    /// Whatever the platform said. A socket that will not take a deadline is a
    /// connection to refuse rather than one to serve without one.
    pub fn apply_to(&self, stream: &std::net::TcpStream) -> std::io::Result<()> {
        stream.set_read_timeout(Some(self.head_patience))?;
        stream.set_write_timeout(Some(self.head_patience))
    }
}

/// A request this server will not serve, and whether there is anybody to tell.
///
/// **The split is the honest part.** A body that overran a bound has a caller
/// waiting for an answer; a peer that vanished mid-head has nobody to answer,
/// and a type that pretended otherwise would have every consumer writing a
/// response into a closed socket and logging the second failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// A malformed or oversized request. [`Refusal::response`] is the bytes to
    /// write before closing.
    Answerable {
        /// The HTTP status.
        status: u16,
        /// Its reason phrase.
        reason: &'static str,
        /// A sentence for the caller, and for the journal.
        why: String,
    },
    /// The connection failed, or the peer left mid-request. There is nothing to
    /// write; log it and drop the connection.
    Unanswerable(String),
}

impl Refusal {
    fn answerable(status: u16, reason: &'static str, why: impl Into<String>) -> Self {
        Self::Answerable { status, reason, why: why.into() }
    }

    /// The response to write, or `None` when there is nobody to write it to.
    #[must_use]
    pub fn response(&self) -> Option<String> {
        let Self::Answerable { status, reason, why } = self else {
            return None;
        };
        let body = format!("{status} {reason}\n{why}\n");
        Some(format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ))
    }

    /// The sentence, whichever kind this is.
    #[must_use]
    pub fn why(&self) -> &str {
        match self {
            Self::Answerable { why, .. } | Self::Unanswerable(why) => why,
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Answerable { status, why, .. } => write!(f, "{status}: {why}"),
            Self::Unanswerable(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for Refusal {}

/// A request line and its header block, parsed and bounded.
///
/// **Nothing here decides anything.** The method and the target are strings the
/// caller matches on; this type's job is that the bytes were framed correctly
/// and that no bound was passed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    /// The method, upper-cased.
    pub method: String,
    /// The request target, exactly as it arrived.
    pub target: String,
    /// The version token, e.g. `HTTP/1.1`.
    pub version: String,
    /// Where the body starts in the buffer this was parsed from.
    pub body_at: usize,
    /// Field names lower-cased, values trimmed, in the order they arrived.
    fields: Vec<(String, String)>,
    length: Option<u64>,
    chunked: bool,
}

impl Head {
    /// Parse a head out of what has been read so far.
    ///
    /// `Ok(None)` means *not yet* — read more and call again. It is the answer
    /// that makes this usable from a blocking loop, a tokio task or anything
    /// else, because the reading is the caller's.
    ///
    /// # Errors
    ///
    /// A [`Refusal::Answerable`]. Every check here happens before a byte of
    /// body is read, which is the point: a request that was always going to be
    /// refused should not first spend a board's disk and a caller's minutes.
    pub fn parse(buffer: &[u8], bounds: &Bounds) -> Result<Option<Self>, Refusal> {
        let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") else {
            if buffer.len() > bounds.head_bytes {
                return Err(Refusal::answerable(
                    431,
                    "Request Header Fields Too Large",
                    format!(
                        "a request head passed {} bytes with no blank line ending it",
                        bounds.head_bytes
                    ),
                ));
            }
            return Ok(None);
        };

        // **The bound is on the head, not on how it arrived.** Checking only
        // the not-yet-terminated case is what the two hand-written parsers do,
        // and it leaves the bound to be enforced by the size of whatever read
        // buffer the loop happened to use — so a head that arrives complete in
        // one large read passes a limit it exceeds. Here it is one check on the
        // thing the number is about.
        if end + 4 > bounds.head_bytes {
            return Err(Refusal::answerable(
                431,
                "Request Header Fields Too Large",
                format!(
                    "a request head of {} bytes is past the {} this server reads",
                    end + 4,
                    bounds.head_bytes
                ),
            ));
        }

        let text = String::from_utf8_lossy(&buffer[..end]);
        let mut lines = text.split("\r\n");
        let first = lines.next().unwrap_or_default();
        let mut parts = first.split(' ');
        let (Some(method), Some(target), Some(version), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(Refusal::answerable(
                400,
                "Bad Request",
                format!("{first:?} is not a request line: expected `METHOD target HTTP/1.1`"),
            ));
        };
        if !version.starts_with("HTTP/1.") {
            return Err(Refusal::answerable(
                505,
                "HTTP Version Not Supported",
                format!("this server speaks HTTP/1.1 and the request said {version:?}"),
            ));
        }

        let mut fields = Vec::new();
        let mut length: Option<u64> = None;
        let mut chunked = false;
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                return Err(Refusal::answerable(
                    400,
                    "Bad Request",
                    format!("{line:?} is not a header field"),
                ));
            };
            let name = name.trim_end().to_ascii_lowercase();
            let value = value.trim();
            if name == "content-length" {
                let stated = content_length(value)?;
                // **A repeated Content-Length is refused whether or not it
                // agrees with the first** (`.rulings/` R-210 D17). Two that
                // disagree are the classic smuggling shape: a server that
                // honours one and a proxy in front that honours the other
                // disagree about where this request ends and the next one
                // begins. Two that happen to match are refused too, rather
                // than trusted because they matched — nothing downstream of
                // this parser should ever have to decide which of two
                // same-named headers was the real one.
                if length.is_some() {
                    return Err(Refusal::answerable(
                        400,
                        "Bad Request",
                        "two Content-Length fields: this request has no one length",
                    ));
                }
                length = Some(stated);
            } else if name == "transfer-encoding" {
                chunked |= value.to_ascii_lowercase().contains("chunked");
            }
            fields.push((name, value.to_string()));
        }

        // **Both framings at once is the classic smuggling shape.** RFC 9112
        // §6.1 says to refuse it, and refusing costs nothing: no caller in this
        // project sends both.
        if chunked && length.is_some() {
            return Err(Refusal::answerable(
                400,
                "Bad Request",
                "both Transfer-Encoding and Content-Length: two framings is no framing",
            ));
        }

        if let Some(stated) = length
            && stated > bounds.body_bytes
        {
            return Err(Refusal::answerable(
                413,
                "Content Too Large",
                format!(
                    "Content-Length is {stated} and this server accepts at most {}",
                    bounds.body_bytes
                ),
            ));
        }

        Ok(Some(Self {
            method: method.to_ascii_uppercase(),
            target: target.to_string(),
            version: version.to_string(),
            body_at: end + 4,
            fields,
            length,
            chunked,
        }))
    }

    /// One field's value, matched without regard to case.
    ///
    /// **Field names are case-insensitive (RFC 9110 §5.1) and browsers do not
    /// agree on the case they send**, so a comparison that did not fold case
    /// would be a security check that worked in one browser.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&str> {
        let wanted = name.to_ascii_lowercase();
        self.fields.iter().find(|(n, _)| *n == wanted).map(|(_, v)| v.as_str())
    }

    /// Every field, names already lower-cased.
    pub fn fields(&self) -> impl Iterator<Item = (&str, &str)> {
        self.fields.iter().map(|(n, v)| (n.as_str(), v.as_str()))
    }

    /// The declared body length, if the caller declared one.
    #[must_use]
    pub const fn content_length(&self) -> Option<u64> {
        self.length
    }

    /// Whether the caller said the body is chunked.
    ///
    /// **Recorded so it can be REFUSED, not so it can be decoded.** Nothing
    /// here implements chunked transfer, and a chunked body handed on unread
    /// reaches whatever is behind it with its length prefixes still in it —
    /// corruption rather than an error.
    #[must_use]
    pub const fn is_chunked(&self) -> bool {
        self.chunked
    }
}

/// A `Content-Length` value, or a refusal.
fn content_length(value: &str) -> Result<u64, Refusal> {
    // `str::parse::<u64>` accepts a leading `+`; a length is digits.
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Refusal::answerable(
            400,
            "Bad Request",
            format!("Content-Length: {value:?} is not a number of bytes"),
        ));
    }
    value.parse::<u64>().map_err(|_| {
        Refusal::answerable(
            400,
            "Bad Request",
            format!("Content-Length: {value:?} is larger than this machine can count"),
        )
    })
}

/// Read one request head from a blocking reader, bounded.
///
/// Returns the head and **whatever body bytes arrived with it**, which is
/// almost always some: a small request usually lands in one segment.
///
/// # This loop is the point, not the parser
///
/// A bound means nothing unless the loop that fills the buffer enforces it, and
/// the loop is where the defect was. `rivet-fleetd` read `[0u8; 1024]` **once**:
/// a request over a kilobyte was silently truncated and a body in a second TCP
/// segment was silently missing. It compiled, it passed, and it forwarded short
/// requests to real machines.
///
/// # It bounds each read, not the time this call can take
///
/// [`Bounds::apply_to`] puts [`Bounds::head_patience`] on the reader as a
/// per-read timeout, and this loop reads more than once. That timeout applies
/// fresh to every read, so a peer that sends one byte just inside it,
/// repeatedly, keeps this call going for as long as it keeps doing that — it
/// is a bound on each read, not a deadline on the call as a whole. Use
/// [`read_head_by_deadline`] for one absolute deadline carried across every
/// read.
///
/// # Errors
///
/// [`Refusal::Answerable`] for a bad or oversized request, and
/// [`Refusal::Unanswerable`] when the peer went away or the socket failed.
pub fn read_head<R: std::io::Read>(
    reader: &mut R,
    bounds: &Bounds,
) -> Result<(Head, Vec<u8>), Refusal> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(head) = Head::parse(&buffer, bounds)? {
            let rest = buffer.split_off(head.body_at);
            return Ok((head, rest));
        }
        let got = reader
            .read(&mut chunk)
            .map_err(|e| Refusal::Unanswerable(format!("reading the request head: {e}")))?;
        if got == 0 {
            return Err(Refusal::Unanswerable(
                "the peer closed before finishing its request head".to_owned(),
            ));
        }
        buffer.extend_from_slice(&chunk[..got]);
    }
}

/// Read one request head under one absolute deadline, resetting the reader's
/// timeout to what remains before every read.
///
/// [`read_head`] bounds each read and not the call — a peer that sends one
/// byte just inside a per-read timeout, repeatedly, can hold the connection
/// well past what [`Bounds::head_patience`] promises. This carries `deadline`
/// through the loop instead: before each read it computes what is left of
/// `deadline` and passes that to `set_read_deadline`, refusing once nothing is
/// left. This is the shape `cmd/rivet-fleetd` wrote for itself before this
/// crate could offer it.
///
/// `set_read_deadline` is whatever the caller's reader takes for a per-read
/// timeout — a real socket's `set_read_timeout`, wrapped to return
/// [`std::io::Result`].
///
/// # Errors
///
/// As [`read_head`], plus [`Refusal::Unanswerable`] when `deadline` passes
/// before a complete head has arrived, or `set_read_deadline` itself fails.
pub fn read_head_by_deadline<R: std::io::Read>(
    reader: &mut R,
    bounds: &Bounds,
    deadline: std::time::Instant,
    mut set_read_deadline: impl FnMut(Duration) -> std::io::Result<()>,
) -> Result<(Head, Vec<u8>), Refusal> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(head) = Head::parse(&buffer, bounds)? {
            let rest = buffer.split_off(head.body_at);
            return Ok((head, rest));
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(Refusal::Unanswerable(
                "the request head did not complete before the deadline".to_owned(),
            ));
        }
        set_read_deadline(remaining)
            .map_err(|e| Refusal::Unanswerable(format!("setting the read deadline: {e}")))?;
        let got = reader
            .read(&mut chunk)
            .map_err(|e| Refusal::Unanswerable(format!("reading the request head: {e}")))?;
        if got == 0 {
            return Err(Refusal::Unanswerable(
                "the peer closed before finishing its request head".to_owned(),
            ));
        }
        buffer.extend_from_slice(&chunk[..got]);
    }
}

/// Read exactly `Content-Length` bytes of body into a sink, and no more.
///
/// `already` is what [`read_head`] handed back. The count returned is what was
/// written.
///
/// **Nothing here holds the body**, which is what lets one function serve both
/// a JSON call and `rivet-mcpd`'s 683 MB payload: the sink can be a `Vec`, a
/// file, or a hasher wrapping a file. On a 926 MB board, buffering that payload
/// would mean the OOM killer choosing between a daemon and the supervisor.
///
/// **Only `Content-Length` bytes are taken**, so a pipelined second request
/// cannot be glued onto the first one's body.
///
/// # Errors
///
/// [`Refusal::Unanswerable`] when the peer promised more than it sent, or the
/// socket failed, or the sink refused the bytes. A caller that promised a body
/// and hung up is an error and not a shorter request.
pub fn read_body_into<R: std::io::Read, W: std::io::Write>(
    reader: &mut R,
    head: &Head,
    already: &[u8],
    sink: &mut W,
) -> Result<u64, Refusal> {
    let length = head.length.unwrap_or(0);
    let mut written: u64 = 0;

    let from_buffer = usize::try_from(length.min(already.len() as u64)).unwrap_or(already.len());
    if from_buffer > 0 {
        sink.write_all(&already[..from_buffer])
            .map_err(|e| Refusal::Unanswerable(format!("writing the request body: {e}")))?;
        written += from_buffer as u64;
    }

    let mut chunk = [0u8; 64 * 1024];
    while written < length {
        let want = usize::try_from((length - written).min(chunk.len() as u64))
            .unwrap_or(chunk.len());
        let got = reader
            .read(&mut chunk[..want])
            .map_err(|e| Refusal::Unanswerable(format!("reading the request body: {e}")))?;
        if got == 0 {
            return Err(Refusal::Unanswerable(format!(
                "the peer promised {length} bytes of body and sent {written}"
            )));
        }
        sink.write_all(&chunk[..got])
            .map_err(|e| Refusal::Unanswerable(format!("writing the request body: {e}")))?;
        written += got as u64;
    }
    Ok(written)
}

/// The same, buffered, for a body small enough to hold.
///
/// **Bounded by [`Bounds::body_bytes`], which [`Head::parse`] already refused
/// past** — so the size of what this allocates is a number the application
/// chose. For anything payload-sized use [`read_body_into`] and give it a file.
///
/// # Errors
///
/// As [`read_body_into`].
pub fn read_body<R: std::io::Read>(
    reader: &mut R,
    head: &Head,
    already: &[u8],
) -> Result<Vec<u8>, Refusal> {
    let hint = usize::try_from(head.length.unwrap_or(0).min(64 * 1024)).unwrap_or(0);
    let mut body = Vec::with_capacity(hint);
    read_body_into(reader, head, already, &mut body)?;
    Ok(body)
}

/// How many connections are open, and a refusal when there are too many.
///
/// **Cheap to clone and safe to share.** A `Limit` is a handle: clone it into
/// each thread or task, or hold one and claim from the accept loop before the
/// thread exists — which is the order that matters, because claiming after
/// spawning bounds nothing.
///
/// It never blocks and it never waits. Refusing is the behaviour: a caller told
/// *no* retries, and a caller queued behind fifteen others holds a descriptor.
#[derive(Debug, Clone)]
pub struct Limit(std::sync::Arc<Counter>);

#[derive(Debug)]
struct Counter {
    live: std::sync::atomic::AtomicUsize,
    most: usize,
}

impl Limit {
    /// A limit of `most` connections.
    #[must_use]
    pub fn of(most: usize) -> Self {
        Self(std::sync::Arc::new(Counter {
            live: std::sync::atomic::AtomicUsize::new(0),
            most,
        }))
    }

    /// Take a slot, or `None` when they are all taken.
    ///
    /// **Claim it before the thread or task exists.** A slot taken inside the
    /// thread is taken after the descriptor was accepted, which is a limit on
    /// nothing.
    #[must_use]
    pub fn claim(&self) -> Option<Slot> {
        use std::sync::atomic::Ordering;
        // `try_update` and not `fetch_add` then check: adding first would let
        // the count exceed the limit between the two operations, and a limit
        // that is briefly wrong is a limit an attacker can time.
        self.0
            .live
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.0.most).then_some(n + 1)
            })
            .ok()
            .map(|_| Slot(std::sync::Arc::clone(&self.0)))
    }

    /// How many are open right now.
    #[must_use]
    pub fn live(&self) -> usize {
        self.0.live.load(std::sync::atomic::Ordering::Acquire)
    }

    /// The most this will hold at once.
    #[must_use]
    pub fn most(&self) -> usize {
        self.0.most
    }
}

/// One connection's claim on a [`Limit`], released when it is dropped.
///
/// **A guard rather than a decrement at the end of the handler**, because there
/// are four ways out of a connection — a refused handshake, an expired
/// deadline, the ordinary end, and a panic — and every one of them runs a
/// destructor while only one of them would have run a line at the bottom.
#[derive(Debug)]
#[must_use = "dropping the slot immediately releases it, which bounds nothing"]
pub struct Slot(std::sync::Arc<Counter>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.live.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests;
