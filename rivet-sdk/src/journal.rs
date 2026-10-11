//! Saying something a person will find — `ADR-036` §5.1, `ADR-022` §11.
//!
//! # Two paths, and the difference matters
//!
//! 1. **stdout and stderr** are captured for any process the supervisor spawned
//!    (`ADR-022` §4). Zero dependencies, works before anything is initialised,
//!    and carries no severity.
//! 2. **The ingest socket** at [`INGEST_SOCKET_PATH`] takes a *structured*
//!    record with a severity and typed fields, and works for any process at all
//!    — including one the supervisor never started.
//!
//! [`Journal`] uses the second and **falls back to the first**, which is the
//! floor `cmd/rivet` sets for itself: a supervisor that is not listening right
//! now is one that is down or restarting, and a program that blocked retrying a
//! socket would be worse than one that printed the record.
//!
//! # What a sender may claim, and what it cannot
//!
//! The journal stamps `boot`, `mono`, `ts`, `corr` and `actor` itself and demotes
//! a sender's to `emitted_*` fields. A sender **may** claim `svc`. So the
//! strongest thing a hostile sender can do is attribute a record to a service it
//! is not — beside the kernel's own `ingest_pid` and `ingest_uid` contradicting
//! it. `ADR-022` §11.4 records that trade rather than hiding it.
//!
//! # Why the record must be complete
//!
//! `rivet_core::journal::Record::from_jsonl` requires `boot`, `mono`, `sev`,
//! `svc`, `corr`, `actor` and `msg`. **A line missing any of them is not
//! rejected — it becomes a `warn` record carrying the raw bytes.** So an
//! encoder that drifted would not fail; it would quietly turn every record into
//! a complaint about itself. That is the failure `ADR-036` §8.3's round-trip
//! test exists to catch, and the reason this module emits every column even
//! though five of them will be overwritten.

use std::fmt::Write as _;
use std::time::Instant;

/// Where the supervisor listens for records.
///
/// **World-writable by design** (`ADR-022` §11.4) — every application has its
/// own uid, so a socket only root could reach would answer the question for one
/// account. What prevents forgery is not the mode: it is that the supervisor
/// records `SO_PEERCRED`'s kernel-verified uid, gid and pid on every record.
pub const INGEST_SOCKET_PATH: &str = "/run/rivet/journal";

/// The most the supervisor will read from one connection.
///
/// A sender that writes more is cut at the far end and the record says so. This
/// crate refuses **here** instead, and puts the record on stderr — losing it to
/// be tidy would be the worse trade.
pub const INGEST_MAX_BYTES: usize = 16 * 1024;

/// Field names the journal composes itself.
///
/// A record using one would have the key on the line twice, which RFC 8259 lets
/// a parser resolve either way. [`Journal::emit`] drops them and counts the
/// drops rather than emitting an ambiguous line.
pub const RESERVED_FIELD_NAMES: [&str; 18] = [
    "ts",
    "boot",
    "mono",
    "sev",
    "svc",
    "corr",
    "actor",
    "msg",
    "ingest_uid",
    "ingest_gid",
    "ingest_pid",
    "ingest_overrode",
    "ingest_unparsed",
    "ingest_truncated",
    "emitted_boot",
    "emitted_corr",
    "emitted_mono",
    "svc_claimed",
];

/// The field [`Journal::line`] appends itself, counting how many caller
/// fields it dropped.
///
/// **Not in [`RESERVED_FIELD_NAMES`]** — that array is `pub` and fixed-size,
/// so widening it to reserve one more name would change its length from the
/// outside (`[&str; 18]` to `[&str; 19]`), and external applications read
/// that type. Checked alongside the list instead, with the same effect: a
/// caller field of this name is dropped and counted exactly like any other
/// reserved one, so the writer's own key can never appear on a line twice.
const SDK_DROPPED_FIELDS: &str = "sdk_dropped_fields";

/// How bad it is. Ordered `Debug < Info < Warn < Error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Debug,
    Info,
    Warn,
    Error,
}

impl Severity {
    /// Every level, weakest first.
    pub const ALL: [Self; 4] = [Self::Debug, Self::Info, Self::Warn, Self::Error];

    /// The wire form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    /// Parse the wire form.
    ///
    /// `None` rather than a default: a severity this build does not know is a
    /// record from a *newer* Rivet, and silently calling it `info` would hide an
    /// error.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sev| sev.as_str() == s)
    }
}

/// A typed field value. The four JSON scalars a record may carry.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl Value {
    fn write_json(&self, out: &mut String) {
        match self {
            Self::Str(s) => write_json_string(s, out),
            Self::Int(i) => {
                let _ = write!(out, "{i}");
            }
            Self::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Self::Float(f) => {
                // JSON has no NaN and no Infinity (RFC 8259 §6). Emitting one
                // produces a line no parser accepts, so it goes out as the
                // string it is — refuse rather than fake.
                if f.is_finite() {
                    let _ = write!(out, "{f}");
                } else {
                    write_json_string(&f.to_string(), out);
                }
            }
        }
    }
}

/// Escape one string as a JSON string, quotes included.
fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// An application's connection to the journal.
///
/// Cheap to construct and cheap to hold. It opens no socket until something is
/// emitted, because a supervisor that restarts should not leave an application
/// holding a dead descriptor.
#[derive(Debug, Clone)]
pub struct Journal {
    svc: String,
    boot: String,
    corr: String,
    started: Instant,
}

impl Journal {
    /// A journal for records attributed to `service`.
    ///
    /// The name should be the one in the manifest. A name the machine does not
    /// recognise is not rejected — it is recorded as `svc_claimed` beside the
    /// kernel's own view of who sent it, which is `ADR-022` §11.4's trade.
    #[must_use]
    pub fn for_service(service: &str) -> Self {
        Self {
            svc: service.to_string(),
            boot: boot_id(),
            corr: String::new(),
            started: Instant::now(),
        }
    }

    /// Tie every record from this journal to one causal chain.
    #[must_use]
    pub fn with_correlation(mut self, corr: &str) -> Self {
        self.corr = corr.to_string();
        self
    }

    /// Emit one record. Never fails, and never silently loses the record.
    ///
    /// The ingest socket first; **stderr when it cannot be reached**, which the
    /// supervisor captures anyway for a process it spawned. A record that could
    /// not be delivered either way is one this process could not have reported
    /// in any case.
    pub fn emit(&self, sev: Severity, msg: &str, fields: &[(&str, Value)]) {
        let line = self.line(sev, msg, fields);
        if line.len() > INGEST_MAX_BYTES || send(&line).is_err() {
            // stderr keeps the whole line rather than a summary: it is the same
            // record, and a reader who has to reconstruct it from a truncation
            // has been given a puzzle instead of an answer.
            eprint!("{line}");
        }
    }

    /// `Severity::Debug`, with no fields.
    pub fn debug(&self, msg: &str) {
        self.emit(Severity::Debug, msg, &[]);
    }
    /// `Severity::Info`, with no fields.
    pub fn info(&self, msg: &str) {
        self.emit(Severity::Info, msg, &[]);
    }
    /// `Severity::Warn`, with no fields.
    pub fn warn(&self, msg: &str) {
        self.emit(Severity::Warn, msg, &[]);
    }
    /// `Severity::Error`, with no fields.
    pub fn error(&self, msg: &str) {
        self.emit(Severity::Error, msg, &[]);
    }

    /// The encoded record, newline included.
    ///
    /// Public because it is the thing `ADR-036` §8.3's agreement test
    /// round-trips through `rivet-core`, and because a caller that wants to
    /// write records somewhere else should write the same bytes.
    ///
    /// **Every column is emitted**, including the five the journal overwrites —
    /// see the module note on why a partial record is worse than no record.
    #[must_use]
    pub fn line(&self, sev: Severity, msg: &str, fields: &[(&str, Value)]) -> String {
        let mut out = String::with_capacity(256);
        out.push('{');
        out.push_str("\"boot\":");
        write_json_string(&self.boot, &mut out);
        out.push_str(",\"mono\":");
        let _ = write!(out, "{:.3}", self.started.elapsed().as_secs_f64());
        out.push_str(",\"sev\":");
        write_json_string(sev.as_str(), &mut out);
        out.push_str(",\"svc\":");
        write_json_string(&self.svc, &mut out);
        out.push_str(",\"corr\":");
        write_json_string(&self.corr, &mut out);
        out.push_str(",\"actor\":");
        write_json_string(&self.svc, &mut out);
        out.push_str(",\"msg\":");
        write_json_string(msg, &mut out);

        let mut dropped = 0i64;
        let mut seen: Vec<&str> = Vec::with_capacity(fields.len());
        for (name, value) in fields {
            // A reserved name would put the key on the line twice; a repeated
            // name is the same ambiguity one door over; the writer's OWN
            // appended field is the same ambiguity a third way — a caller
            // claiming `sdk_dropped_fields` for itself would otherwise
            // collide with the count below. All three are dropped and
            // counted, never emitted.
            if RESERVED_FIELD_NAMES.contains(name) || *name == SDK_DROPPED_FIELDS || seen.contains(name) {
                dropped += 1;
                continue;
            }
            seen.push(name);
            out.push(',');
            write_json_string(name, &mut out);
            out.push(':');
            value.write_json(&mut out);
        }
        if dropped > 0 {
            let _ = write!(out, ",\"{SDK_DROPPED_FIELDS}\":{dropped}");
        }
        out.push_str("}\n");
        out
    }
}

/// This machine's boot id, or `unknown`.
///
/// The journal stamps its own `boot` and demotes this to `emitted_boot`, so it
/// is informational — which is exactly why it must not be invented. `unknown`
/// is an honest answer; a random string would be a lie that looked like data.
fn boot_id() -> String {
    std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

/// How long a record may take to reach the supervisor before it is given up on
/// and written to stderr instead.
///
/// **A budget, not a guess at how long the supervisor needs.** `ADR-017`'s
/// supervisor is single-threaded, so *every* blocking call in it stops the whole
/// machine; the ingest listener is in that one loop, and this module's only
/// consumers are applications with frame budgets — `aura-renderer` draws to a
/// 33.3 ms one. A journal call that waits on a busy supervisor is a dropped
/// frame at best and a frozen screen at worst, and losing the record to stderr
/// costs almost nothing: the supervisor captures stdout and stderr as well
/// (`ADR-022` §4), so the words still reach the journal — with five columns
/// demoted, which this module's header already explains.
///
/// 250 ms is far longer than any healthy hand-over and far shorter than a
/// person notices.
#[cfg(unix)]
const INGEST_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

/// Write one line to the ingest socket.
///
/// # Bounded, since 2026-09-04
///
/// The audit of 2026-09-03's LOW list: *"`Journal::emit` blocks with no timeout
/// on ingest"*. Every step here could wait for ever on a supervisor that was not
/// reading — the `connect` when the listen backlog is full, and the write when
/// the socket buffer is. `Journal::emit`'s contract is that it *cannot fail*, so
/// there was no way for a caller to notice, and nothing above it was written to
/// survive a call that simply does not return.
///
/// Both are bounded now. The write and read timeouts are `SO_SNDTIMEO` and
/// `SO_RCVTIMEO`; the connect is bounded by making the socket non-blocking for
/// the length of it, which is what `std` has no API for and is why this is the
/// one function in this crate that needs `libc`.
#[cfg(unix)]
fn send(line: &str) -> Result<(), std::io::Error> {
    send_to(INGEST_SOCKET_PATH, line)
}

/// [`send`], with the socket named — so the timeouts above can be TESTED.
///
/// The path is a constant on a running machine and a test cannot bind
/// `/run/rivet/journal`; without this seam the only way to exercise a bound that
/// exists to stop a hang would be to hang.
#[cfg(unix)]
#[allow(unsafe_code)]
fn send_to(socket_path: &str, line: &str) -> Result<(), std::io::Error> {
    use std::io::Write as _;
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};

    let path = socket_path.as_bytes();
    // SAFETY: a plain C struct with no invariants; every field is set below.
    let mut address: libc::sockaddr_un = unsafe { core::mem::zeroed() };
    if path.len() >= core::mem::size_of_val(&address.sun_path) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "the journal socket path does not fit in a sockaddr_un",
        ));
    }
    address.sun_family = libc::AF_UNIX as _;
    for (slot, byte) in address.sun_path.iter_mut().zip(path) {
        *slot = *byte as libc::c_char;
    }

    // SAFETY: an ordinary socket call with constant arguments.
    let raw = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: a fresh descriptor the kernel just returned, owned only here — so
    // every path out of this function closes it.
    let owned = unsafe { OwnedFd::from_raw_fd(raw) };

    // SAFETY: `owned` is an open socket and `address` is a live, fully
    // initialised `sockaddr_un` of the length given.
    let connected = unsafe {
        libc::connect(
            owned.as_raw_fd(),
            (&raw const address).cast::<libc::sockaddr>(),
            core::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
        )
    };
    if connected < 0 {
        let why = std::io::Error::last_os_error();
        // Anything but "in progress" is a real refusal — no socket there, no
        // permission, or **`EAGAIN` for an accept queue that is full**, which is
        // what a stalled supervisor with other applications talking to it looks
        // like. Answered at once rather than waited on.
        if why.raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(why);
        }
        let mut fds = libc::pollfd {
            fd: owned.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        // SAFETY: one live `pollfd` describing a descriptor this process owns.
        let ready = unsafe {
            libc::poll(
                &raw mut fds,
                1,
                INGEST_TIMEOUT.as_millis().min(i32::MAX as u128) as libc::c_int,
            )
        };
        if ready == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "the supervisor did not accept a journal record in time",
            ));
        }
        if ready < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // **`POLLOUT` alone does not mean it worked.** A failed connect is also
        // reported as writable, and the reason is only in `SO_ERROR`.
        let mut error: libc::c_int = 0;
        let mut len = core::mem::size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: `error` and `len` are live and correctly sized for
        // `SO_ERROR`, which is an `int`.
        let asked = unsafe {
            libc::getsockopt(
                owned.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&raw mut error).cast::<libc::c_void>(),
                &raw mut len,
            )
        };
        if asked < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if error != 0 {
            return Err(std::io::Error::from_raw_os_error(error));
        }
    }

    let mut socket = std::os::unix::net::UnixStream::from(owned);
    // Back to blocking, with a deadline on it: a non-blocking `write_all` would
    // report a partial record as an error and this one waits, briefly.
    socket.set_nonblocking(false)?;
    socket.set_write_timeout(Some(INGEST_TIMEOUT))?;
    socket.set_read_timeout(Some(INGEST_TIMEOUT))?;
    socket.write_all(line.as_bytes())?;
    socket.flush()
}

/// There is no ingest socket on a platform without unix sockets.
///
/// Not a stub for something unfinished: `rivet-sdk` compiles on a development
/// host so that the encoding and the constants stay testable there, and on such
/// a host the honest behaviour is the stderr floor.
#[cfg(not(unix))]
fn send(_line: &str) -> Result<(), std::io::Error> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "the journal ingest socket is a unix socket",
    ))
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test — the same block every
    // inline test module in this workspace carries.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn journal() -> Journal {
        Journal {
            svc: "demo".to_string(),
            boot: "b0".to_string(),
            corr: "c0".to_string(),
            started: Instant::now(),
        }
    }

    /// The module's central claim: a record missing a column is not rejected,
    /// it is demoted. So every column must be present.
    #[test]
    fn every_required_column_is_emitted() {
        let line = journal().line(Severity::Info, "hello", &[]);
        for column in ["boot", "mono", "sev", "svc", "corr", "actor", "msg"] {
            assert!(
                line.contains(&format!("\"{column}\":")),
                "{column} missing from {line}"
            );
        }
        assert!(line.ends_with("}\n"));
    }

    #[test]
    fn a_field_reaches_the_line_with_its_type() {
        let line = journal().line(
            Severity::Warn,
            "m",
            &[
                ("count", Value::Int(3)),
                ("ok", Value::Bool(false)),
                ("name", Value::Str("x".to_string())),
            ],
        );
        assert!(line.contains("\"count\":3"));
        assert!(line.contains("\"ok\":false"));
        assert!(line.contains("\"name\":\"x\""));
    }

    #[test]
    fn a_reserved_name_is_dropped_and_counted_rather_than_duplicating_a_column() {
        let line = journal().line(Severity::Info, "m", &[("svc", Value::Int(1))]);
        assert!(!line.contains("\"svc\":1"));
        assert!(line.contains("\"sdk_dropped_fields\":1"));
    }

    #[test]
    fn a_repeated_field_name_is_the_same_ambiguity_and_is_dropped_too() {
        let line = journal().line(
            Severity::Info,
            "m",
            &[("k", Value::Int(1)), ("k", Value::Int(2))],
        );
        assert!(line.contains("\"k\":1"));
        assert!(!line.contains("\"k\":2"));
        assert!(line.contains("\"sdk_dropped_fields\":1"));
    }

    /// **`sdk_dropped_fields` is the name THIS WRITER appends for its own
    /// drop count, and it was not itself reserved** — so a caller field of
    /// that name survived the reserved-name check, and the line carried the
    /// key twice: once from the caller, once from the writer's own count.
    /// `rivet-core`'s ingestion refuses a duplicate custom key, so this
    /// collision turned a parseable record into an ingestion warning
    /// (`rivet_core::journal::ingest`).
    #[test]
    fn a_caller_field_named_like_the_counter_is_dropped_and_folded_into_the_count() {
        let line = journal().line(
            Severity::Error,
            "m",
            &[("sdk_dropped_fields", Value::Int(99)), ("svc", Value::Int(1))],
        );
        assert_eq!(
            line.matches("\"sdk_dropped_fields\":").count(),
            1,
            "the key appears more than once: {line}"
        );
        // Both fields were reserved (`svc` already was; `sdk_dropped_fields`
        // is now), so the writer's own count is the whole of what is left.
        assert!(line.contains("\"sdk_dropped_fields\":2"), "{line}");
    }

    /// Two copies of the counter's own name collide with EACH OTHER, not
    /// just with the writer — the second is a repeat of the first, and both
    /// are dropped by the same rule.
    #[test]
    fn two_copies_of_the_counter_name_are_both_dropped_and_counted_once() {
        let line = journal().line(
            Severity::Info,
            "m",
            &[
                ("sdk_dropped_fields", Value::Int(1)),
                ("sdk_dropped_fields", Value::Int(2)),
            ],
        );
        assert_eq!(line.matches("\"sdk_dropped_fields\":").count(), 1, "{line}");
        assert!(line.contains("\"sdk_dropped_fields\":2"), "{line}");
    }

    /// **Nothing else reserved.** Before this fix `dropped` would have stayed
    /// `0` here and the field would have been kept verbatim — no collision
    /// yet, but only because nothing else happened to collide. This is the
    /// narrowest case: the name is reserved on its own, not as a side effect
    /// of colliding with something else.
    #[test]
    fn the_counter_name_alone_with_nothing_else_reserved_is_still_dropped() {
        let line = journal().line(Severity::Info, "m", &[("sdk_dropped_fields", Value::Int(5))]);
        assert_eq!(line.matches("\"sdk_dropped_fields\":").count(), 1, "{line}");
        assert!(line.contains("\"sdk_dropped_fields\":1"), "{line}");
    }

    #[test]
    fn quotes_newlines_and_control_characters_are_escaped() {
        let line = journal().line(Severity::Error, "a\"b\nc\td\u{1}e", &[]);
        // Each escape is checked on its own. One combined expected string
        // has to carry four backslash sequences, and two attempts at
        // writing it put LITERAL control characters where the text of the
        // escape belonged - asserting a condition that could never hold
        // rather than the one intended, while still compiling.
        assert!(line.contains("\\\""), "quote not escaped: {line}");
        assert!(line.contains("\\n"), "newline not escaped: {line}");
        assert!(line.contains("\\t"), "tab not escaped: {line}");
        assert!(line.contains("\\u0001"), "control character not escaped: {line}");
        // Exactly one newline, and it is the terminator.
        assert_eq!(line.matches('\n').count(), 1);
    }

    /// RFC 8259 has no NaN. A line carrying one is a line no parser accepts, so
    /// the value goes out as the string it is.
    #[test]
    fn a_non_finite_float_becomes_a_string_rather_than_invalid_json() {
        let line = journal().line(Severity::Info, "m", &[("f", Value::Float(f64::NAN))]);
        assert!(line.contains("\"f\":\"NaN\""));
    }

    #[test]
    fn severity_round_trips_through_the_wire_form() {
        for sev in Severity::ALL {
            assert_eq!(Severity::parse(sev.as_str()), Some(sev));
        }
        assert_eq!(Severity::parse("catastrophe"), None);
    }

    #[test]
    fn severity_is_ordered_weakest_first() {
        assert!(Severity::Debug < Severity::Info);
        assert!(Severity::Info < Severity::Warn);
        assert!(Severity::Warn < Severity::Error);
    }

    /// An unknown boot id is said, not invented — see `boot_id`.
    #[test]
    fn a_machine_with_no_boot_id_says_unknown() {
        if std::path::Path::new("/proc/sys/kernel/random/boot_id").exists() {
            assert_ne!(boot_id(), "unknown");
        } else {
            assert_eq!(boot_id(), "unknown");
        }
    }

    /// **A supervisor that is not accepting cannot stop the application** — the
    /// audit of 2026-09-03's LOW list, *"`Journal::emit` blocks with no timeout
    /// on ingest"*.
    ///
    /// A listener that is bound and never accepts is exactly the state a
    /// single-threaded supervisor is in while it does anything else
    /// (`ADR-017`), and `emit`'s contract is that it cannot fail — so a caller
    /// had no way to notice a call that simply never returned. On the frame
    /// loop this crate is written for, that is a frozen screen.
    ///
    /// **The send runs on another thread and the result is collected with a
    /// deadline**, so the unfixed code FAILS this test rather than hanging it:
    /// a test that proves a bound by waiting for ever is not a test.
    ///
    /// # Producing the state, and what it took to produce it HONESTLY
    ///
    /// A first version bound an ordinary `UnixListener`, never accepted, and
    /// made four thousand connections. **All four thousand succeeded**, against
    /// the UNFIXED code, in twenty milliseconds — because each one closed
    /// immediately and Linux takes a closed embryo back out of the accept queue,
    /// so the queue never fills. That version proved nothing, and it is recorded
    /// here because the reachable form of this defect is narrower than the audit
    /// line suggests and a reader deserves to know which form was closed.
    ///
    /// What blocks is an accept queue full of connections that are still OPEN —
    /// a stalled supervisor with other applications journalling past it.
    /// `listen(fd, 0)` reaches that with one held connection instead of a
    /// hundred and twenty-eight, which is why the listener here is built by hand
    /// rather than with `UnixListener::bind`.
    #[cfg(unix)]
    #[test]
    #[allow(unsafe_code)]
    fn a_journal_write_to_a_supervisor_that_never_accepts_gives_up() {
        use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};

        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sdk-{}-ingest", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("journal").to_string_lossy().into_owned();
        let _ = std::fs::remove_file(&path);

        // SAFETY: constant arguments to an ordinary socket call.
        let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
        assert!(raw >= 0, "socket: {}", std::io::Error::last_os_error());
        // SAFETY: a fresh descriptor owned only here.
        let listener = unsafe { OwnedFd::from_raw_fd(raw) };
        // SAFETY: a zeroed `sockaddr_un`, filled in immediately below.
        let mut address: libc::sockaddr_un = unsafe { core::mem::zeroed() };
        address.sun_family = libc::AF_UNIX as _;
        assert!(path.len() < address.sun_path.len(), "the temp path is too long");
        for (slot, byte) in address.sun_path.iter_mut().zip(path.as_bytes()) {
            *slot = *byte as libc::c_char;
        }
        // SAFETY: an open socket and a live, fully initialised address.
        let bound = unsafe {
            libc::bind(
                listener.as_raw_fd(),
                (&raw const address).cast::<libc::sockaddr>(),
                core::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
            )
        };
        assert_eq!(bound, 0, "bind: {}", std::io::Error::last_os_error());
        // SAFETY: an open, bound socket. Nothing ever calls `accept` on it.
        assert_eq!(unsafe { libc::listen(listener.as_raw_fd(), 0) }, 0);

        // Connections that stay OPEN, until one is refused — at which point the
        // queue is full and the next caller is the one under test.
        //
        // **Made NON-BLOCKING, and that is not a detail.** A blocking
        // `UnixStream::connect` here is the very thing under test: the first
        // attempt past a full queue would wait for ever, in this thread, before
        // the deadline below exists — and a test that hangs while setting up a
        // test about hanging is worse than no test.
        let mut held: Vec<OwnedFd> = Vec::new();
        for _ in 0..8 {
            // SAFETY: constant arguments to an ordinary socket call.
            let fd = unsafe {
                libc::socket(
                    libc::AF_UNIX,
                    libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
                    0,
                )
            };
            assert!(fd >= 0, "socket: {}", std::io::Error::last_os_error());
            // SAFETY: a fresh descriptor owned only here.
            let client = unsafe { OwnedFd::from_raw_fd(fd) };
            // SAFETY: an open socket and the live address bound above.
            let joined = unsafe {
                libc::connect(
                    client.as_raw_fd(),
                    (&raw const address).cast::<libc::sockaddr>(),
                    core::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
                )
            };
            if joined < 0 && std::io::Error::last_os_error().raw_os_error() != Some(libc::EINPROGRESS)
            {
                // `EAGAIN`: the accept queue is full, which is the state wanted.
                break;
            }
            held.push(client);
        }

        let (done, heard) = std::sync::mpsc::channel();
        let sending = std::thread::spawn(move || {
            let _ = done.send(send_to(&path, "{\"msg\":\"probe\"}\n").is_err());
        });

        // Generous against the 250 ms budget, and finite — which is the whole
        // property under test.
        let answered = heard.recv_timeout(std::time::Duration::from_secs(30));
        let Ok(gave_up) = answered else {
            panic!(
                "a journal write to a listener that never accepts did not return: this is \
                 the hang the timeouts exist to stop"
            )
        };
        assert!(
            gave_up,
            "the write reported success against a listener that never accepted it, so this \
             test did not reach the state it is about ({} connections were held)",
            held.len()
        );
        let _ = sending.join();

        drop(held);
        drop(listener);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
