//! The journal record — **the format, both directions, in one place.**
//!
//! `ADR-022` §2 is the whole design in a sentence: *a log entry is a structured
//! record; the text an operator reads is a view of it.* §3 makes the on-disk
//! form line-delimited JSON, and §3.1 says that form "sits behind a typed
//! interface so a compact binary format can replace it later without anything
//! above noticing". This module is that interface.
//!
//! # Why it is here rather than in the supervisor — 2026-08-24
//!
//! It was in `cmd/rivet-sysd` until the machine grew a **reader**.
//!
//! One program writes records and a different program reads them back:
//! `rivet log` (`ADR-022` §9) is an operation in the catalogue, so it is
//! reachable from every adapter (`ADR-006` §3.1) and it cannot live inside the
//! process that produced the bytes. Two programs that must agree on a format
//! and each hold their own copy of it is `ADR-011`'s failure exactly — and the
//! failure would be silent, because a reader that disagrees with the writer
//! does not crash. It shows the operator a shorter log.
//!
//! `cmd/rivet-sysd` is a binary crate with no library target, so
//! nothing can import from it. The format therefore moves down into the crate
//! both sides already depend on, which is the same move `ADR-011` prescribed for
//! `/state`'s partition number and for `crate::deploy`'s paths.
//!
//! **What did NOT move** is the supervisor's `Journal` — the thing that holds a
//! boot identifier, mints correlation identifiers, buffers, rotates and writes.
//! That is I/O and policy at a composition root; this is the domain's account of
//! what a record *is*.
//!
//! # The reader is written against bytes a guest actually wrote
//!
//! `AGENTS.md` §4: *"a test that round-trips a reader against its own writer
//! proves the two agree with each other and nothing about the format."* So
//! [`Record::from_jsonl`] is tested against
//! `tests/fixtures/qemu-guest-console.jsonl`, which is a capture of a Rivet
//! guest's console under `qemu-system-aarch64`, kept verbatim. Round-trip tests
//! are here too, and they are the weaker half.
//!
//! # Refusing rather than guessing
//!
//! `AGENTS.md` §5: a line that does not parse is an **error carrying the raw
//! bytes**, never a partial record. A journal is read when something has
//! already gone wrong, and a reader that silently drops what it cannot
//! understand answers "there is nothing about that in the log" — which is the
//! one answer a log must never give falsely.

use std::fmt::Write as _;

// ---------------------------------------------------------------------------
// Severity
// ---------------------------------------------------------------------------

/// Severity, as a closed set.
///
/// `ADR-022` §10 item 2 leaves the exact vocabulary open and `ADR-006` §6.4's
/// closed-vocabulary rule says why it matters: this is precisely the kind of
/// enumeration that grows one convenient value at a time until it means
/// nothing. Four levels are enough to run a supervisor and are what this crate
/// commits to until the maintainer rules otherwise.
///
/// Ordered `Debug < Info < Warn < Error`, which is what makes
/// [`Filter::min_severity`] expressible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Debug,
    Info,
    Warn,
    Error,
}

impl Severity {
    /// Every level, weakest first. The one list — a `rivet log --severity`
    /// argument and this type's parser both read it.
    pub const ALL: [Self; 4] = [Self::Debug, Self::Info, Self::Warn, Self::Error];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    /// Parse the wire form. `None` rather than a default, because a severity
    /// this build does not know is a record from a *newer* Rivet, and
    /// silently calling it `info` would hide an error.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sev| sev.as_str() == s)
    }
}

impl core::fmt::Display for Severity {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Value
// ---------------------------------------------------------------------------

/// A typed field value.
///
/// Deliberately closed. `ADR-022` §3.2 says "arbitrary typed fields follow";
/// arbitrary means the *names* are open, not that the value domain is.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl Value {
    pub fn write_json(&self, out: &mut String) {
        match self {
            Self::Str(s) => write_json_string(s, out),
            Self::Int(i) => out.push_str(&i.to_string()),
            Self::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Self::Float(f) => {
                // JSON has no NaN and no Infinity (RFC 8259 §6). Emitting one
                // would produce a line no parser accepts, so the value is
                // reported as the string it is rather than being dropped or
                // silently turned into null — `AGENTS.md` §3, refuse rather
                // than fake.
                if f.is_finite() {
                    let _ = write!(out, "{f}");
                } else if f.is_nan() {
                    out.push_str("\"nan\"");
                } else if *f > 0.0 {
                    out.push_str("\"inf\"");
                } else {
                    out.push_str("\"-inf\"");
                }
            }
        }
    }

    /// The value as a human would see it in a rendered line. Strings are bare;
    /// everything else is its JSON form.
    pub fn to_display(&self) -> String {
        match self {
            Self::Str(s) => s.clone(),
            other => {
                let mut out = String::new();
                other.write_json(&mut out);
                out
            }
        }
    }
}

/// Escape and quote a string per RFC 8259 §7.
///
/// Non-ASCII passes through as UTF-8, which is valid JSON and keeps the file
/// readable with `cat` — `ADR-022` §3.1's whole argument.
/// Where the kernel publishes this boot's identifier.
///
/// **One spelling, because two produced a real defect.** `cmd/rivet-sysd`
/// read this path and kept the whole UUID; `cmd/rivet-deploy` read the same
/// path and kept the first **eight characters** — so an install's records and
/// the supervisor's records named different boots inside one journal, and
/// `rivet get log previous` selected wrongly (`PLAN.md` §10.8, found
/// 2026-08-24 by running `rivet apply edit` on a guest).
///
/// `ADR-011`: this is a library and can be the authority; both of those are
/// binaries and neither could have been.
pub const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";

/// `ADR-022` §3.2's `boot` field, from the text the kernel published.
///
/// **Never truncated.** The identifier is the whole UUID or nothing: a prefix
/// collides across boots far sooner than a UUID does, and a reader joining
/// records by a truncated id splices two boots together — which is exactly the
/// failure `ADR-022` §9.4 relies on `boot` not having.
///
/// A machine that will not answer gets `"unknown"` rather than a synthesised
/// identifier, because a record whose boot id was invented is worse than one
/// that says it does not know.
pub fn boot_identifier(raw: Result<String, ()>) -> String {
    raw.map_or_else(|()| "unknown".to_string(), |text| text.trim().to_string())
}

pub fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

// ---------------------------------------------------------------------------
// Record
// ---------------------------------------------------------------------------

/// One journal record. `ADR-022` §3.2 is the field list.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// Wall clock, RFC 3339 UTC.
    ///
    /// **Not trustworthy at early boot.** A Pi has no RTC (`ADR-021`), so this
    /// is wrong until something sets the clock and it *jumps* when something
    /// does. `boot` and `mono` are what make ordering recoverable; this field
    /// is for a human reading a record from a machine whose clock is right.
    /// `None` when no wall clock could be read at all.
    pub ts: Option<String>,
    /// Which boot this record belongs to.
    pub boot: String,
    /// Seconds since boot, from a monotonic clock. Never jumps.
    pub mono: f64,
    pub sev: Severity,
    /// The service the record is *about* — the supervisor's own records name
    /// the supervisor.
    pub svc: String,
    /// `ADR-022` §5. Ties the records of one causal chain together.
    pub corr: String,
    /// `ADR-022` §5, composing with `ADR-012` §4's actor provenance.
    pub actor: String,
    pub msg: String,
    pub fields: Vec<(String, Value)>,
}

impl Record {
    /// The column names [`Record::to_jsonl`] always emits.
    ///
    /// A [`Record::fields`] entry using one of these would put the key on the
    /// line **twice**. That is ambiguous — RFC 8259 permits a parser to take
    /// either value or reject the object outright — and `ADR-022` §3 makes this
    /// form the authoritative one, so an ambiguous record is a corrupted record.
    ///
    /// This is a real defect that shipped: the supervisor's own start-up record
    /// added a `boot` field while every record already carried the boot id as a
    /// column, so **every line it wrote had `"boot"` twice.** Found by reading
    /// console output, not by a test — hence
    /// `no_record_can_emit_the_same_key_twice`.
    pub const RESERVED_FIELD_NAMES: [&'static str; 8] =
        ["ts", "boot", "mono", "sev", "svc", "corr", "actor", "msg"];

    /// The on-disk form: one complete, self-describing record on one line.
    ///
    /// The trailing newline is included, because a record that has to be
    /// joined by its caller is a record that can be half-written.
    ///
    /// **A field named in [`Record::RESERVED_FIELD_NAMES`] is dropped rather
    /// than emitted**, and that choice is deliberate: the alternatives are
    /// writing an ambiguous line, or refusing to log at all. The journal is
    /// where a failure explains itself, so it must not be the thing that fails
    /// — but it must also never write a record a parser could read two ways.
    /// Losing a duplicated field loses nothing, because the column carrying
    /// that name is already on the line.
    pub fn to_jsonl(&self) -> String {
        let mut out = String::with_capacity(256);
        out.push('{');
        if let Some(ts) = &self.ts {
            out.push_str("\"ts\":");
            write_json_string(ts, &mut out);
            out.push(',');
        }
        out.push_str("\"boot\":");
        write_json_string(&self.boot, &mut out);
        out.push_str(",\"mono\":");
        let _ = write!(out, "{:.3}", self.mono);
        out.push_str(",\"sev\":");
        write_json_string(self.sev.as_str(), &mut out);
        out.push_str(",\"svc\":");
        write_json_string(&self.svc, &mut out);
        out.push_str(",\"corr\":");
        write_json_string(&self.corr, &mut out);
        out.push_str(",\"actor\":");
        write_json_string(&self.actor, &mut out);
        out.push_str(",\"msg\":");
        write_json_string(&self.msg, &mut out);
        for (name, value) in &self.fields {
            if Self::RESERVED_FIELD_NAMES.contains(&name.as_str()) {
                continue;
            }
            out.push(',');
            write_json_string(name, &mut out);
            out.push(':');
            value.write_json(&mut out);
        }
        out.push_str("}\n");
        out
    }

    /// A one-line human rendering — a *view* of the record, per `ADR-022` §2,
    /// never the thing itself.
    pub fn to_text(&self) -> String {
        let mut out = format!(
            "[{:>10.3}] {:<5} {}: {}",
            self.mono,
            self.sev.as_str(),
            self.svc,
            self.msg
        );
        for (name, value) in &self.fields {
            let mut rendered = String::new();
            value.write_json(&mut rendered);
            let _ = write!(out, " {name}={rendered}");
        }
        out
    }

    /// Read one line back. **The other half of `ADR-022` §3.1's typed
    /// interface**, and the thing that made this module leave the supervisor.
    ///
    /// Strict on purpose. Every departure from what [`Record::to_jsonl`] can
    /// produce is an error naming the position, rather than a best effort:
    ///
    /// * a value that is an array or an object — the encoder cannot emit one,
    ///   so a line carrying one came from something else;
    /// * `null` — likewise, and it is how "I do not know" gets written by
    ///   encoders that guess;
    /// * a missing fixed column, or a duplicated key;
    /// * a severity outside [`Severity::ALL`], which means a **newer** Rivet
    ///   wrote the line and this build must not flatten it to `info`.
    ///
    /// A trailing newline is accepted so a caller may hand over a line as read.
    pub fn from_jsonl(line: &str) -> Result<Self, ParseError> {
        let mut p = Parser::new(line);
        let pairs = p.object()?;
        p.end()?;

        let mut ts = None;
        let mut boot = None;
        let mut mono = None;
        let mut sev = None;
        let mut svc = None;
        let mut corr = None;
        let mut actor = None;
        let mut msg = None;
        let mut fields = Vec::new();

        for (key, value) in pairs {
            // A column stated twice is the ambiguity `RESERVED_FIELD_NAMES`
            // exists to prevent on the way out. Seeing it on the way in means
            // some *other* writer produced the line, and picking a value would
            // be choosing which of two answers to believe.
            match key.as_str() {
                "ts" => take(&mut ts, value, "ts", Self::string)?,
                "boot" => take(&mut boot, value, "boot", Self::string)?,
                "mono" => take(&mut mono, value, "mono", Self::number)?,
                "sev" => take(&mut sev, value, "sev", Self::severity)?,
                "svc" => take(&mut svc, value, "svc", Self::string)?,
                "corr" => take(&mut corr, value, "corr", Self::string)?,
                "actor" => take(&mut actor, value, "actor", Self::string)?,
                "msg" => take(&mut msg, value, "msg", Self::string)?,
                _ => {
                    if fields.iter().any(|(n, _): &(String, Value)| *n == key) {
                        return Err(ParseError::DuplicateKey(key));
                    }
                    fields.push((key, value));
                    continue;
                }
            }
        }

        Ok(Self {
            ts,
            boot: boot.ok_or(ParseError::MissingColumn("boot"))?,
            mono: mono.ok_or(ParseError::MissingColumn("mono"))?,
            sev: sev.ok_or(ParseError::MissingColumn("sev"))?,
            svc: svc.ok_or(ParseError::MissingColumn("svc"))?,
            corr: corr.ok_or(ParseError::MissingColumn("corr"))?,
            actor: actor.ok_or(ParseError::MissingColumn("actor"))?,
            msg: msg.ok_or(ParseError::MissingColumn("msg"))?,
            fields,
        })
    }

    fn string(v: Value, column: &'static str) -> Result<String, ParseError> {
        match v {
            Value::Str(s) => Ok(s),
            other => Err(ParseError::WrongColumnType {
                column,
                found: other,
            }),
        }
    }

    fn number(v: Value, column: &'static str) -> Result<f64, ParseError> {
        match v {
            Value::Float(f) => Ok(f),
            // `"mono":2` is what an encoder that trims trailing zeroes would
            // write. This one does not, but accepting it costs nothing and
            // refusing it would make the reader stricter than the format.
            #[allow(clippy::cast_precision_loss)]
            Value::Int(i) => Ok(i as f64),
            other => Err(ParseError::WrongColumnType {
                column,
                found: other,
            }),
        }
    }

    fn severity(v: Value, column: &'static str) -> Result<Severity, ParseError> {
        let s = Self::string(v, column)?;
        Severity::parse(&s).ok_or(ParseError::UnknownSeverity(s))
    }

    /// One named field, if the record carries it.
    pub fn field(&self, name: &str) -> Option<&Value> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
}

fn take<T>(
    slot: &mut Option<T>,
    value: Value,
    column: &'static str,
    convert: fn(Value, &'static str) -> Result<T, ParseError>,
) -> Result<(), ParseError> {
    if slot.is_some() {
        return Err(ParseError::DuplicateKey(column.to_string()));
    }
    *slot = Some(convert(value, column)?);
    Ok(())
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// What can be wrong with a line. Every variant names where, because a journal
/// is read when something else has already gone wrong.
#[derive(Debug, Clone, PartialEq)]
pub enum ParseError {
    /// The line is empty or whitespace. Not an error a caller has to report —
    /// see [`parse_lines`], which skips these.
    Blank,
    /// Something other than `{` where the object should start.
    NotAnObject { at: usize },
    /// The line ends in the middle of a value.
    Truncated,
    /// A byte that cannot appear there.
    Unexpected { at: usize, found: char },
    /// `\q`, or `\u` with fewer than four hex digits.
    BadEscape { at: usize },
    /// A number this build cannot represent, or that is not a number.
    BadNumber { at: usize, text: String },
    /// An array or object as a field value. The encoder cannot emit one.
    NestedValue { key: String },
    /// `null`. The encoder cannot emit one either — see [`Record::from_jsonl`].
    NullValue { key: String },
    /// A fixed column `ADR-022` §3.2 requires is not on the line.
    MissingColumn(&'static str),
    /// A fixed column carrying the wrong JSON type.
    WrongColumnType { column: &'static str, found: Value },
    /// The same key twice. RFC 8259 permits a parser to take either value; this
    /// one takes neither.
    DuplicateKey(String),
    /// A severity outside [`Severity::ALL`] — a **newer** Rivet wrote this.
    UnknownSeverity(String),
    /// Trailing bytes after the object closed.
    TrailingBytes { at: usize },
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Blank => write!(f, "the line is blank"),
            Self::NotAnObject { at } => {
                write!(
                    f,
                    "a record is a JSON object and this does not start with {{ (at byte {at})"
                )
            }
            Self::Truncated => write!(
                f,
                "the line ends in the middle of a value — a half-written record, \
                 which ADR-022 §8's batched writes make possible after a power cut"
            ),
            Self::Unexpected { at, found } => {
                write!(f, "unexpected {found:?} at byte {at}")
            }
            Self::BadEscape { at } => write!(
                f,
                "a backslash escape at byte {at} is not one RFC 8259 §7 defines"
            ),
            Self::BadNumber { at, text } => {
                write!(
                    f,
                    "{text:?} at byte {at} is not a number this build can read"
                )
            }
            Self::NestedValue { key } => write!(
                f,
                "the field {key:?} holds an array or an object, which ADR-022 §3.2's \
                 closed value domain has no room for — this line was not written by Rivet"
            ),
            Self::NullValue { key } => write!(
                f,
                "the field {key:?} is null. Rivet omits what it does not know \
                 (see Record::ts) rather than writing a null for it"
            ),
            Self::MissingColumn(name) => write!(
                f,
                "the record has no {name:?} — ADR-022 §3.2 makes it a fixed column, \
                 so a line without one is not a Rivet record"
            ),
            Self::WrongColumnType { column, found } => {
                write!(
                    f,
                    "the column {column:?} holds {found:?}, which is the wrong JSON type"
                )
            }
            Self::DuplicateKey(key) => write!(
                f,
                "the key {key:?} appears twice. RFC 8259 lets a parser take either value; \
                 this one takes neither, because choosing would be choosing which of two \
                 answers to believe"
            ),
            Self::UnknownSeverity(s) => write!(
                f,
                "the severity {s:?} is not one this build knows. That means a NEWER Rivet \
                 wrote the line, and reading it as `info` would hide an error"
            ),
            Self::TrailingBytes { at } => write!(
                f,
                "the object closed at byte {at} and the line continues — JSONL is one \
                 complete record per line"
            ),
        }
    }
}

impl std::error::Error for ParseError {}

/// A tiny recursive-descent reader for exactly the subset [`Record::to_jsonl`]
/// emits.
///
/// **Why not `serde_json`.** `ADR-009` §3 requires a survey and a recorded
/// reason before writing something a crate already does, and the encoder half
/// of this module already carries that argument: pulling `serde` into a
/// workspace that deliberately kept it out, to serialise a **closed** set of
/// value types with no derive and no generic data model, uses almost none of
/// what serde exists for. The reader does not change that balance — it is the
/// same closed domain read backwards, it must *refuse* the things a
/// general-purpose parser accepts (nested values, nulls, duplicate keys), and
/// `rivet-core` has an empty `[dependencies]` table by rule (`ADR-014` §3,
/// `AGENTS.md` §10.1) so a dependency here is not merely undesirable but
/// forbidden.
struct Parser<'a> {
    src: &'a [u8],
    text: &'a str,
    at: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            src: text.as_bytes(),
            text,
            at: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.at).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), ParseError> {
        match self.peek() {
            Some(b) if b == byte => {
                self.at += 1;
                Ok(())
            }
            Some(b) => Err(ParseError::Unexpected {
                at: self.at,
                found: char::from(b),
            }),
            None => Err(ParseError::Truncated),
        }
    }

    fn object(&mut self) -> Result<Vec<(String, Value)>, ParseError> {
        self.skip_ws();
        if self.at >= self.src.len() {
            return Err(ParseError::Blank);
        }
        if self.peek() != Some(b'{') {
            return Err(ParseError::NotAnObject { at: self.at });
        }
        self.at += 1;

        let mut out = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(out);
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            let value = self.value(&key)?;
            out.push((key, value));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b) => {
                    return Err(ParseError::Unexpected {
                        at: self.at,
                        found: char::from(b),
                    });
                }
                None => return Err(ParseError::Truncated),
            }
        }
    }

    /// Anything after the object is a defect rather than slack. JSONL is one
    /// complete record per line, so a second object on the same line means the
    /// writer lost a newline — which is the shape a torn write takes.
    fn end(&mut self) -> Result<(), ParseError> {
        self.skip_ws();
        if self.at < self.src.len() {
            return Err(ParseError::TrailingBytes { at: self.at });
        }
        Ok(())
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let b = self.peek().ok_or(ParseError::Truncated)?;
            match b {
                b'"' => {
                    self.at += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.at += 1;
                    let e = self.peek().ok_or(ParseError::Truncated)?;
                    self.at += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0c}'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return Err(ParseError::BadEscape { at: self.at - 1 }),
                    }
                }
                _ => {
                    // Walk by characters rather than bytes so multi-byte UTF-8
                    // survives — ADR-022 §3.1 keeps the file readable with
                    // `cat`, which means non-ASCII is on it verbatim.
                    let rest = self.text.get(self.at..).ok_or(ParseError::Truncated)?;
                    let c = rest.chars().next().ok_or(ParseError::Truncated)?;
                    self.at += c.len_utf8();
                    out.push(c);
                }
            }
        }
    }

    /// `\uXXXX`. Surrogate pairs are **not** decoded, and that is deliberate:
    /// [`write_json_string`] only ever emits `\u` for control characters below
    /// U+0020, so a surrogate on a line did not come from Rivet.
    fn unicode_escape(&mut self) -> Result<char, ParseError> {
        let start = self.at;
        let hex = self
            .text
            .get(self.at..self.at + 4)
            .ok_or(ParseError::Truncated)?;
        let n = u32::from_str_radix(hex, 16).map_err(|_| ParseError::BadEscape { at: start })?;
        self.at += 4;
        char::from_u32(n).ok_or(ParseError::BadEscape { at: start })
    }

    fn value(&mut self, key: &str) -> Result<Value, ParseError> {
        match self.peek().ok_or(ParseError::Truncated)? {
            b'"' => Ok(Value::Str(self.string()?)),
            b't' => {
                self.literal("true")?;
                Ok(Value::Bool(true))
            }
            b'f' => {
                self.literal("false")?;
                Ok(Value::Bool(false))
            }
            b'n' => {
                self.literal("null")?;
                Err(ParseError::NullValue {
                    key: key.to_string(),
                })
            }
            b'[' | b'{' => Err(ParseError::NestedValue {
                key: key.to_string(),
            }),
            _ => self.number(),
        }
    }

    fn literal(&mut self, word: &str) -> Result<(), ParseError> {
        if self.text.get(self.at..self.at + word.len()) == Some(word) {
            self.at += word.len();
            Ok(())
        } else {
            Err(ParseError::Unexpected {
                at: self.at,
                found: self.peek().map_or('?', char::from),
            })
        }
    }

    fn number(&mut self) -> Result<Value, ParseError> {
        let start = self.at;
        while matches!(
            self.peek(),
            Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
        ) {
            self.at += 1;
        }
        let text = self.text.get(start..self.at).unwrap_or("");
        if text.is_empty() {
            return Err(ParseError::Unexpected {
                at: start,
                found: self.peek().map_or('?', char::from),
            });
        }
        // An integer stays an integer. A `mono` of `2` and a `mono` of `2.0`
        // mean the same thing and the record cares; a *field* of `2` and `2.0`
        // are a count and a measurement, and flattening both to `f64` would
        // print `bytes=251.0` in a log an operator reads.
        let looks_integral = !text.contains(['.', 'e', 'E']);
        if looks_integral && let Ok(i) = text.parse::<i64>() {
            return Ok(Value::Int(i));
        }
        text.parse::<f64>()
            .map(Value::Float)
            .map_err(|_| ParseError::BadNumber {
                at: start,
                text: text.to_string(),
            })
    }
}

/// Read a whole journal: every line, with the ones that would not parse kept as
/// errors rather than dropped.
///
/// **The refusals are returned, not swallowed.** `rivet log` reports how many
/// lines it could not read, because "the log says nothing about that" and "the
/// log has four lines this build cannot read" are different answers and only
/// one of them is safe to act on (`AGENTS.md` §5).
///
/// Blank lines are skipped silently: a file that ends in a newline has one, and
/// so does one that a batched write left short (`ADR-022` §8).
pub fn parse_lines<'a, I>(lines: I) -> (Vec<Record>, Vec<(usize, ParseError)>)
where
    I: IntoIterator<Item = &'a str>,
{
    let mut records = Vec::new();
    let mut refused = Vec::new();
    for (n, line) in lines.into_iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match Record::from_jsonl(line) {
            Ok(r) => records.push(r),
            Err(e) => refused.push((n + 1, e)),
        }
    }
    (records, refused)
}

// ---------------------------------------------------------------------------
// The calendar
// ---------------------------------------------------------------------------

/// Days-to-calendar, for [`rfc3339`].
///
/// Howard Hinnant's `civil_from_days`, which is the standard derivation and is
/// exact for every date in the proleptic Gregorian calendar. It is here rather
/// than in the `time` crate deliberately — `rivet-core` has no dependencies
/// at all by rule (`ADR-014` §3). The tests check it against dates produced by
/// GNU `date`, not against this module's own inverse.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    (
        year,
        u32::try_from(m).unwrap_or(0),
        u32::try_from(d).unwrap_or(0),
    )
}

/// Render Unix epoch milliseconds as RFC 3339 UTC with millisecond precision.
pub fn rfc3339(epoch_ms: u64) -> String {
    let secs = epoch_ms / 1000;
    let ms = epoch_ms % 1000;
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let sod = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{ms:03}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

/// Render Unix epoch milliseconds as `09/06/2026 11:41 UTC`, **for a person**.
///
/// [`rfc3339`] is for the journal, where a machine reads it and sorts by it.
/// This is for a page somebody looks at, where `2026-09-06T11:41:43.812Z` is
/// four pieces of precision nobody wanted and a `T` in the middle of a
/// sentence.
///
/// # `MM/DD/YYYY`, ruled
///
/// It rendered the month as `Sep` for one commit, on the argument that
/// `09/06/2026` is the sixth of September to half the world and the ninth of
/// June to the other half. **The maintainer ruled for the digits**, 2026-09-06,
/// and this is a page on his own machine read by him. The reading is
/// unambiguous to the audience that has one.
///
/// # Why it says `UTC` out loud
///
/// The instant is UTC and a reader in a house is not. Four characters is the
/// whole cost of not being five hours wrong, and a bare `11:41` on a page
/// beside a wall clock reading `06:41` is a discrepancy somebody would go
/// looking for a cause of.
///
/// **Seconds are dropped, not rounded away** — this answers *roughly when* and
/// a value that changes every second on a page nobody reloads is noise.
#[must_use]
pub fn human(epoch_ms: u64) -> String {
    let secs = epoch_ms / 1000;
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let sod = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{m:02}/{d:02}/{y:04} {:02}:{:02} UTC",
        sod / 3600,
        (sod % 3600) / 60
    )
}

// ---------------------------------------------------------------------------
// Selecting records
// ---------------------------------------------------------------------------

/// Which boot to read. `ADR-022` §3.2's whole argument for carrying `boot` and
/// `mono` alongside `ts` is that a Pi's wall clock is wrong at early boot and
/// **jumps** when corrected, so "the last ten minutes" is not a question the
/// clock can answer. "This boot" and "the one before it" are.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum BootSelector {
    /// The most recent boot present in the journal.
    #[default]
    Latest,
    /// The one before that — the boot a reader wants after a board came back.
    Previous,
    /// A named boot identifier.
    Id(String),
    /// Every boot the journal still holds.
    All,
}

/// What `rivet log` is asked for. Pure: it selects, it does not read.
/// A position in the journal, so a caller can ask *what happened after this*.
///
/// **`ADR-035` §4A(b), and it is deliberately not a follow.** §4A is explicit
/// that an operation input meaning *keep answering* describes the **machine**
/// continuing to answer, and a client that keeps asking is doing something
/// else. Naming this `follow` would be an adapter claiming a property it does
/// not have, so it is `since`: repeated calls make a tail, in pure
/// request/response, with no protocol decision at all.
///
/// # Why a boot and a monotonic, and never a time
///
/// `ADR-021`: a Pi has no RTC, so `ts` is wrong until something corrects it and
/// **jumps** when something does. A cursor over wall-clock time would skip
/// every record written before the correction and re-show every record after a
/// clock that went backwards. `boot` and `mono` are what
/// [`Record`]'s own documentation calls *"what make ordering recoverable"*, and
/// they are exactly what this is made of.
///
/// # The wire form is opaque and this one happens to be readable
///
/// `<boot>@<mono>`. A caller is told to pass back what it was given and nothing
/// else; that it is legible is a convenience for a human reading a transcript,
/// not a format anybody may construct. [`Cursor::parse`] is correspondingly
/// strict — a cursor this build did not write is refused rather than
/// interpreted.
#[derive(Debug, Clone)]
pub struct Cursor {
    boot: String,
    mono: f64,
}

/// **Exact, by bits**, because `Filter` is `Eq` and an `f64` is not.
///
/// Two cursors are the same position when they name the same boot and the
/// identical monotonic value. That is the right meaning here even though it is
/// the wrong meaning for arithmetic: a cursor is a token handed back verbatim,
/// so the only `f64`s ever compared are ones that were printed and re-parsed
/// from the same original — never two that were computed separately.
impl PartialEq for Cursor {
    fn eq(&self, other: &Self) -> bool {
        self.boot == other.boot && self.mono.to_bits() == other.mono.to_bits()
    }
}

impl Eq for Cursor {}

impl Cursor {
    /// The position immediately after `record`.
    #[must_use]
    pub fn after(record: &Record) -> Self {
        Self { boot: record.boot.clone(), mono: record.mono }
    }

    /// Read one back. `None` if it is not a cursor this build wrote.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let (boot, mono) = text.rsplit_once('@')?;
        if boot.is_empty() {
            return None;
        }
        let mono: f64 = mono.parse().ok()?;
        if !mono.is_finite() {
            return None;
        }
        Some(Self { boot: boot.to_string(), mono })
    }

    /// Which boot this position is in.
    #[must_use]
    pub fn boot(&self) -> &str {
        &self.boot
    }
}

impl core::fmt::Display for Cursor {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}@{}", self.boot, self.mono)
    }
}

/// What a cursor turned out to mean once it met the journal.
///
/// **Three outcomes, because two would hide the interesting one.** A cursor
/// whose boot the journal no longer holds cannot be placed, and the choice then
/// is between showing nothing (silently skipping every record since) and
/// showing everything (repeating records the caller has already seen).
/// **Everything, and say so** — `AGENTS.md` §5: a log that answers *there is
/// nothing about that* when it could not place the question is the one failure
/// a log must not have. Repetition is visible to the caller; a gap is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placed {
    /// The cursor's boot is held and records were selected after it.
    At,
    /// No cursor was given.
    NotAsked,
    /// The cursor's boot is not in the journal any more, so everything is
    /// returned and the caller is told.
    BootGone,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Filter {
    pub boot: BootSelector,
    /// Exact service name — the `svc` column, not a substring. A prefix match
    /// would make `rivet` select `rivet-netd` and `rivet-timed` too.
    pub service: Option<String>,
    /// Records at this level **or above**. `Severity` is ordered for this.
    pub min_severity: Option<Severity>,
    /// `ADR-022` §5's correlation identifier — one causal chain, across every
    /// service it touched.
    pub actor: Option<String>,
    pub correlation: Option<String>,
    /// A substring of the message. Case-sensitive, because a log is read with
    /// a string somebody copied out of it.
    pub contains: Option<String>,
    /// Keep only the **last** `n`. A journal is read from the end.
    pub limit: Option<usize>,
    /// Keep only what happened strictly **after** this position — `ADR-035`
    /// §4A(b). See [`Cursor`], and note that it composes with `limit` in the
    /// order a caller would expect: `since` selects, then `limit` keeps the
    /// last of what is left.
    pub since: Option<Cursor>,
}

impl Filter {
    /// Apply to records in the order the journal holds them, oldest first.
    ///
    /// The boot selector is resolved against *this* set rather than against a
    /// list held anywhere: the journal's own contents are the authority for
    /// which boots it still has, and a retention pass that dropped the oldest
    /// file must not leave a selector pointing at a boot that is gone.
    pub fn apply(&self, records: Vec<Record>) -> Vec<Record> {
        self.apply_counting(records).0
    }

    /// [`Self::apply`], and **how many records the LIMIT dropped**.
    ///
    /// # Why the count is worth returning at all
    ///
    /// Because `returned < held` has two completely different causes and a
    /// caller cannot tell them apart. Selecting one boot out of thirty is
    /// working as asked; the limit silently cutting the older half of the boot
    /// you asked for is the thing that ends an investigation early — and it is
    /// not hypothetical. `PLAN.md` §104.10: a search for `vc4` came back empty
    /// against the 200-record default, **because a boot's kernel records are at
    /// its start**, and the wrong conclusion drawn from that empty answer cost
    /// a whole subject before it was found.
    ///
    /// The count is of the LIMIT'S cut only. Records that `boot`, `since` or
    /// any of the other predicates excluded are not in it: those were asked
    /// for, and this number exists to report what was **not**.
    pub fn apply_counting(&self, records: Vec<Record>) -> (Vec<Record>, usize) {
        let wanted = self.wanted_boot(&records);
        if wanted == BootChoice::Nothing {
            // Nothing selected, and so nothing the LIMIT cut. The zero is the
            // honest count: an empty answer here is about the boot that was
            // named, and saying the limit dropped records would send a reader
            // after records that were never candidates.
            return (Vec::new(), 0);
        }
        let placed = self.placed(&records);
        // Boot chronology, and the cursor's place in it, come from the
        // UNFILTERED records. The selector and the predicates below can
        // legitimately remove every record of the cursor's own boot — `latest`
        // right after a reboot is exactly this — and that boot still has a
        // position in history even when nothing of it survives the filter. An
        // order computed from what survived cannot place a boot the filter
        // just erased, and every one of its later records then falls into the
        // `_ => false` arm below instead of being kept.
        let order = if self.since.is_some() {
            boots_in_order(&records)
        } else {
            Vec::new()
        };
        let at = self
            .since
            .as_ref()
            .and_then(|cursor| order.iter().position(|b| b == cursor.boot()));
        let mut out: Vec<Record> = records
            .into_iter()
            .filter(|r| match &wanted {
                BootChoice::One(boot) => r.boot == *boot,
                BootChoice::Any => true,
                BootChoice::Nothing => false,
            })
            .filter(|r| self.service.as_ref().is_none_or(|s| r.svc == *s))
            .filter(|r| self.min_severity.is_none_or(|min| r.sev >= min))
            .filter(|r| self.actor.as_ref().is_none_or(|a| r.actor == *a))
            .filter(|r| self.correlation.as_ref().is_none_or(|c| r.corr == *c))
            .filter(|r| {
                self.contains
                    .as_ref()
                    .is_none_or(|needle| r.msg.contains(needle.as_str()))
            })
            .collect();
        if let Some(cursor) = &self.since
            && placed == Placed::At
        {
            out.retain(|r| match (at, order.iter().position(|b| *b == r.boot)) {
                // A later boot is unambiguously after, whatever its `mono`
                // says — every boot's monotonic clock starts again at zero,
                // which is precisely why a cursor carries both.
                (Some(cut), Some(here)) if here > cut => true,
                (Some(cut), Some(here)) if here == cut => r.mono > cursor.mono,
                _ => false,
            });
        }
        // **The OLDEST go.** A journal is read from the end (`ADR-022` §9.6),
        // and the count of what went is returned rather than left for the
        // caller to compute from `held`, which cannot separate this cut from
        // the ones the caller asked for.
        let mut truncated = 0;
        if let Some(n) = self.limit
            && out.len() > n
        {
            truncated = out.len() - n;
            out.drain(..truncated);
        }
        (out, truncated)
    }

    /// Whether a `since` cursor could be placed in this journal at all.
    ///
    /// Reported beside the records rather than kept private, because a caller
    /// polling for a tail has to be able to tell *"nothing new"* from
    /// *"I lost your place and started again"*.
    #[must_use]
    pub fn placed(&self, records: &[Record]) -> Placed {
        match &self.since {
            None => Placed::NotAsked,
            Some(cursor) => {
                if records.iter().any(|r| r.boot == cursor.boot) {
                    Placed::At
                } else {
                    Placed::BootGone
                }
            }
        }
    }

    /// Which boot identifier [`Filter::apply`] should keep.
    ///
    /// **Three answers, not two, and the third is the one that took a failing
    /// test to find.** "Every boot" and "no boot at all" are different, and
    /// collapsing them into `Option::None` made
    /// `BootSelector::Previous` on a journal holding **one** boot answer with
    /// that boot — a machine that had never rebooted claiming to show you the
    /// boot before this one. That is the shape of wrongness `ADR-018` §5's
    /// withheld commit had: a plausible answer to a question the system could
    /// not actually answer.
    fn wanted_boot(&self, records: &[Record]) -> BootChoice {
        match &self.boot {
            BootSelector::All => BootChoice::Any,
            BootSelector::Id(id) => BootChoice::One(id.clone()),
            BootSelector::Latest => boots_in_order(records)
                .pop()
                .map_or(BootChoice::Nothing, BootChoice::One),
            BootSelector::Previous => {
                let mut seen = boots_in_order(records);
                seen.pop();
                seen.pop().map_or(BootChoice::Nothing, BootChoice::One)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BootChoice {
    Any,
    One(String),
    Nothing,
}

/// The distinct boot identifiers, in the order they first appear.
///
/// **First appearance, not sorted.** A boot identifier is
/// `/proc/sys/kernel/random/boot_id` — a random UUID — so it does not sort, and
/// `ts` cannot order them either because a machine with no RTC starts every
/// boot at the epoch until something corrects it (`ADR-021`). The file's own
/// order is the only reliable one, and it is reliable because the journal only
/// ever appends.
pub fn boots_in_order(records: &[Record]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in records {
        if !out.contains(&r.boot) {
            out.push(r.boot.clone());
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Following — the cursor a streaming adapter holds between two reads
// ---------------------------------------------------------------------------

/// How often a follower asks the card whether anything new was written —
/// **500 ms**.
///
/// **The number is a trade between two costs, and neither is the card's
/// lifetime:** reads do not wear flash, so this is not `ADR-022` §8's hazard
/// arriving by another route. What it spends is CPU on a machine whose whole
/// point is to be doing something else, and what it buys is how long an
/// operator waits to see a line that has already been written.
///
/// Half a second is under the threshold at which a human reading a terminal
/// perceives a lag, and — because a follower reads only the bytes appended
/// since its last read (`PLAN.md` §21.2) — a poll that finds nothing costs one
/// `stat` and no read at all.
pub const FOLLOW_POLL_MS: u64 = 500;

/// How long a follower runs when the caller does not say — **300 seconds**.
///
/// **A default that never ends is the defect this constant exists to fix.**
/// `PLAN.md` §20.6: every question an operator or an agent asks this machine
/// costs a connection to it, and a command that does not return holds one open
/// until something else kills it. An agent that runs `rivet get log --follow`
/// to see what an install did must get its terminal back.
///
/// Five minutes is longer than any single act of watching — a boot, a deploy, a
/// service's restart loop declaring itself — and short enough that a forgotten
/// session is gone before it matters. [`FOLLOW_FOREVER`] is the word for the
/// operator who genuinely wants a permanent tail, because *deciding* to hold a
/// connection open indefinitely is a different act from *forgetting* to close
/// one.
pub const FOLLOW_SECONDS_DEFAULT: u64 = 300;

/// The word that means *until something stops me* — `--for forever`.
///
/// The same shape as `crate::operation::log::LIMIT_ALL`: the unbounded case is
/// still reachable, and it is reachable only by naming it.
pub const FOLLOW_FOREVER: &str = "forever";

/// How many records a follower emits before it stops — **5000**.
///
/// The second half of the bound, and it is the one that catches the case the
/// clock does not: a service in a restart loop can write faster than anything
/// reads, and `ADR-017` §3.5 has that failure mode by design. A deadline alone
/// would let one produce megabytes inside it.
///
/// 5000 records is on the order of a megabyte — an eighth of everything
/// `ADR-022` §8 lets the journal hold — and reaching it is itself a finding
/// worth reporting, which is why a follower that stops here says so rather than
/// closing quietly.
pub const FOLLOW_RECORDS_MAX: usize = 5_000;

/// The cursor an adapter holds between two reads of the journal.
///
/// # Why this is here and the loop is not
///
/// **Following is a property of the transport; *which records are new* is a
/// property of the journal.** `crate::operation::log` answers once, from what is
/// on the card at the moment it is asked (`ADR-022` §9.5), and an adapter that
/// wants a stream calls it — or reads the file itself — and calls again. What
/// it must not do is decide for itself what "again" means, because that
/// decision is the one with the boot identifiers in it.
///
/// So this type owns exactly two things, and both of them are `ADR-022` §3.2's
/// argument about what a boot identifier is:
///
/// 1. **Which boot is being followed**, resolved **once** and then held. A
///    selector of `latest` re-resolved on every read would silently follow a
///    different boot the moment the machine restarted, which is the shape of
///    wrongness `Filter::wanted_boot`'s three-way answer was written to
///    prevent.
/// 2. **That the machine restarted underneath the stream**, which is not an
///    error and must not be silence either.
///
/// Everything the loop needs beyond that — a clock, a file offset, a sleep — is
/// the adapter's, and none of it can be named here (`AGENTS.md` §3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Follow {
    /// Boot pinning aside, this is the caller's filter with `limit` cleared:
    /// a limit counts from the end of a finite answer, and a stream has no end.
    filter: Filter,
    /// Whether [`Self::filter`]'s boot is a concrete identifier yet.
    ///
    /// `false` on a machine whose journal is empty when the follow starts —
    /// there is no boot to pin until a record arrives.
    pinned: bool,
    /// Whether the selector was `latest`, which is the only one that follows a
    /// restart across to the new boot. A caller who named an identifier gets
    /// the boot they named, for as long as they wait.
    tracks_latest: bool,
    /// Every boot identifier this follower has been shown, in the order it
    /// first saw them.
    ///
    /// **It is what tells a restart apart from a re-read, and it took a failing
    /// test to find.** Without it, *any* record whose boot is not the pinned one
    /// reads as "the machine restarted" — so a follower shown a record from an
    /// OLDER boot, which is what a rotated file's tail carries
    /// (`PLAN.md` §21.2), would report a restart and walk backwards onto a boot
    /// that ended days ago. A boot identifier is a random UUID (`ADR-022`
    /// §3.2), so one that has been seen before is a re-read by construction and
    /// one that has not is new.
    seen: Vec<String>,
}

/// What one read of new lines produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Followed {
    /// The records to emit, oldest first. Empty is the ordinary case.
    pub records: Vec<Record>,
    /// Something a reader must be told about, or `None`.
    pub note: Option<FollowNote>,
}

/// The one thing that can happen to a follower that is not a record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FollowNote {
    /// A boot identifier appeared that is not the one being followed.
    ///
    /// **On a machine following its own journal this means it restarted**, and
    /// the follower will not have survived it — the connection went with the
    /// machine. It is reachable when a journal is being followed from
    /// somewhere else, and it is reported rather than assumed impossible
    /// because a stream that goes permanently silent looks exactly like a quiet
    /// machine (`AGENTS.md` §5).
    BootChanged {
        from: String,
        to: String,
        /// Whether the follower moved to the new boot. `latest` does; a named
        /// identifier does not, because the caller named it.
        switched: bool,
    },
}

impl Follow {
    /// Start following, with the selector the caller gave.
    ///
    /// # Errors
    ///
    /// [`BootSelector::Previous`] names a boot that has already ended, so
    /// nothing will ever be appended to it. Refused with the reason rather than
    /// accepted into a stream that is silent by construction.
    pub fn start(boot: BootSelector) -> Result<Self, &'static str> {
        if boot == BootSelector::Previous {
            return Err(
                "`previous` names the boot BEFORE this one, which has ended — nothing will \
                 ever be appended to it, so following it would wait for ever in silence. Ask \
                 for it without following, or follow `latest`.",
            );
        }
        let tracks_latest = boot == BootSelector::Latest;
        let pinned = matches!(boot, BootSelector::Id(_) | BootSelector::All);
        Ok(Self {
            filter: Filter {
                boot,
                ..Filter::default()
            },
            pinned,
            tracks_latest,
            seen: Vec::new(),
        })
    }

    /// The records already on the card that the stream opens with, **and the
    /// act that pins the boot.**
    ///
    /// `limit` is the caller's, applied here and nowhere else: it is the last
    /// question with an end to count back from.
    pub fn backfill(&mut self, records: Vec<Record>, limit: Option<usize>) -> Vec<Record> {
        self.pin(&records);
        self.remember(&records);
        let filter = Filter {
            limit,
            ..self.filter.clone()
        };
        filter.apply(records)
    }

    /// The records from a read that happened after [`Self::backfill`].
    ///
    /// `records` is whatever the adapter parsed out of the bytes appended since
    /// its last read. Everything about *which* of them to emit is decided here.
    pub fn more(&mut self, records: Vec<Record>) -> Followed {
        self.pin(&records);
        let note = self.notice_a_restart(&records);
        self.remember(&records);
        Followed {
            records: self.filter.apply(records),
            note,
        }
    }

    /// The boot this follower is pinned to, once it has one.
    #[must_use]
    pub fn boot(&self) -> Option<&str> {
        match &self.filter.boot {
            BootSelector::Id(id) => Some(id.as_str()),
            _ => None,
        }
    }

    /// Resolve `latest` against the first records that arrive, then stop.
    fn pin(&mut self, records: &[Record]) {
        if self.pinned {
            return;
        }
        if let Some(latest) = boots_in_order(records).pop() {
            self.filter.boot = BootSelector::Id(latest);
            self.pinned = true;
        }
    }

    /// Add these records' boots to [`Self::seen`].
    fn remember(&mut self, records: &[Record]) {
        for boot in boots_in_order(records) {
            if !self.seen.contains(&boot) {
                self.seen.push(boot);
            }
        }
    }

    /// Whether these records carry a boot that is not the pinned one, and what
    /// to do about it.
    fn notice_a_restart(&mut self, records: &[Record]) -> Option<FollowNote> {
        let BootSelector::Id(current) = &self.filter.boot else {
            // `all` follows every boot, so a new one is not a change of subject.
            return None;
        };
        // A boot this follower has already been shown is a re-read, not a
        // restart — see `seen`.
        let newest = boots_in_order(records)
            .into_iter()
            .rfind(|b| !self.seen.contains(b))?;
        if newest == *current {
            return None;
        }
        let from = current.clone();
        if self.tracks_latest {
            self.filter.boot = BootSelector::Id(newest.clone());
        }
        Some(FollowNote::BootChanged {
            from,
            to: newest,
            switched: self.tracks_latest,
        })
    }
}

// ---------------------------------------------------------------------------
// Ingest — how a process the supervisor did not spawn files a record
// ---------------------------------------------------------------------------

/// Where the supervisor listens for records from processes it did not start.
///
/// **`ADR-022` §11**, ruled 2026-08-24. `ADR-022` §7 gives the journal exactly
/// one writer and §4 already has that writer ingesting other processes' records
/// — through pipes it owns **because it spawned them**. `cmd/rivet` cannot use
/// that route: it is the login shell, so its stdout is the
/// operator's session. This socket is the general answer to *how does a process
/// the supervisor did not spawn get a record into the journal*, and it is what
/// kept §7 unamended.
///
/// **Beside `/run/rivet/notify` rather than inside it.** That directory holds
/// one `sd_notify` datagram socket per service, created when the supervisor
/// starts that service and named after it; a process the supervisor never
/// started has no entry there, and `sd_notify`'s convention is that an
/// unrecognised `KEY=VALUE` is **ignored** — so a record sent that way would be
/// dropped in silence, which is the one thing a journal must not do. See
/// `ADR-022` §11.2.
///
/// It is named here rather than in `cmd/rivet-sysd` for [`crate::deploy`]'s
/// reason: three binary crates have to agree about it — the supervisor that
/// binds it and the CLI and the editor that connect to it — and a binary crate
/// has no library target for the others to import from. `ADR-011`.
pub const INGEST_SOCKET_PATH: &str = "/run/rivet/journal";

/// The mode the supervisor gives the socket.
///
/// **World-writable, deliberately, and it is the widest permission Rivet
/// grants anywhere** — `ADR-022` §11.4 is the argument and the honest account of
/// what it costs. The short form: `ADR-008` §7.3 rules full Unix multi-user and
/// `ADR-015` §5.1 gives every application its own uid, so a socket only uid 0
/// can reach is a socket that answers the question for one account. What stops
/// it being a forgery channel is not the mode — it is that the supervisor
/// records `SO_PEERCRED`'s kernel-verified uid, gid and pid on every record and
/// stamps `boot`, `mono`, `ts`, `corr` and `actor` itself.
///
/// **It authorises nothing.** A credential says who sent something; it does not
/// make what they sent true.
pub const INGEST_SOCKET_MODE: u32 = 0o666;

/// The most the supervisor will read from one connection.
///
/// Twice `cmd/rivet-sysd`'s `MAX_CAPTURED_LINE`, so anything a service could
/// have said on a pipe fits, and small enough that
/// [`INGEST_MAX_IN_FLIGHT`] connections at their limit is a quarter of a
/// megabyte in the process `ADR-017` §8 item 2 says must not fail.
///
/// A sender that writes more is cut here and the record says so — never
/// silently, which is `cmd/rivet-sysd`'s rule for a truncated captured line
/// and the same rule for the same reason.
pub const INGEST_MAX_BYTES: usize = 16 * 1024;

/// How many accepted-but-unfinished connections the supervisor will hold.
///
/// A connection that is accepted and never written to costs a descriptor and a
/// buffer until [`INGEST_DEADLINE_MS`] expires. Bounding the count is what stops
/// a process opening a thousand of them and exhausting PID 1's only child of
/// file descriptors; bounding the time is what stops one being held for ever.
/// Both are needed — either alone is a way to hold the resource.
pub const INGEST_MAX_IN_FLIGHT: usize = 16;

/// How long a connection may stay open without delivering a whole record.
///
/// A sender writes one line and closes; two seconds is several orders of
/// magnitude more than that takes on a machine that is working, and a sender
/// slower than this has already fallen back to stderr on its own side.
pub const INGEST_DEADLINE_MS: u64 = 2_000;

/// The `svc` an ingested record carries when the sender did not supply a usable
/// one.
///
/// A real service name would be a lie about which service spoke; this says
/// exactly what is known — that it arrived on the ingest socket.
pub const INGEST_SVC: &str = "ingest";

/// The prefix of [`Peer::actor`].
///
/// `ADR-022` §5's actor for an ingested record is the **kernel's** answer to who
/// sent it, which is the only part of a sender's identity nothing had to be
/// taken on trust for. `ADR-008` §7.3 rules full Unix multi-user, so this is
/// written to distinguish accounts from the first record rather than to be
/// widened when it has to.
pub const INGEST_ACTOR_PREFIX: &str = "uid:";

/// The most of an unparsable line that becomes a record's message.
///
/// The bytes are evidence and are kept rather than discarded (`AGENTS.md` §5);
/// a kilobyte of them is enough to see what a broken sender emitted, and the
/// whole [`INGEST_MAX_BYTES`] would let one sender write sixteen ordinary
/// records' worth of storage per mistake.
pub const INGEST_UNPARSED_MAX_BYTES: usize = 1024;

/// The `svc` the supervisor's own records carry, and **the one name an ingested
/// record may not claim.**
///
/// `ADR-022` §11.4 keeps a sender's claimed `svc` on purpose — refusing an
/// unknown name would lose the records of `rivet` and `femto`, which are the
/// senders this socket exists for — and the trade it takes is that a hostile
/// process **can** write a record saying `svc: rivet-netd`, contradicted by the
/// kernel's `ingest_pid` sitting beside it.
///
/// **The supervisor's own name is the one case where that trade does not
/// hold**, and it is excluded here rather than being left to the reader.
/// §11.4's security claim is *"no ingested record can be made to read as one the
/// supervisor produced"*, and it rests on the actor column: an ingested record
/// carries `uid:<n>` and the supervisor's carries `supervisor`. That is true and
/// it is not enough on its own, because **`svc` is the column a reader
/// filters** — §9.2 names `rivet get log | where svc == rivet-netd` as the query
/// the column exists for, and the same query with `rivet-sysd` would return a
/// local process's chosen text mixed into the supervisor's own account of the
/// boot. A reader who then checked `actor` would see it; the point is that they
/// should not have to.
///
/// So a claim of this name is treated exactly as a claim of a malformed one:
/// [`INGEST_SVC`], with the claim kept as `svc_claimed`. Nothing is dropped.
///
/// **Named here rather than in `cmd/rivet-sysd`** so that the supervisor's own
/// `SELF_NAME` and this exclusion cannot drift apart — the crate that binds the
/// socket takes its name from this constant.
pub const SUPERVISOR_SVC: &str = "rivet-sysd";

/// The `actor` the supervisor stamps on every record it produces itself —
/// its own notes, and everything captured from a service's stdout or stderr
/// and re-emitted as a record (`ADR-022` §4). An ingested record never
/// carries this: [`Peer::actor`] stamps [`INGEST_ACTOR_PREFIX`] plus the
/// sender's uid, unconditionally, whatever the sender's own line claimed.
///
/// So this is the column a reader — or an operation reading the journal,
/// such as `operation::install`'s install report — checks to tell
/// "the supervisor said this" from "a sender claimed this service's name
/// over the ingest socket", which [`SUPERVISOR_SVC`]'s own documentation
/// already argues `svc` alone cannot do.
///
/// **Named here for [`SUPERVISOR_SVC`]'s reason**: `cmd/rivet-sysd` stamps
/// this exact value, and a second spelling in the crate that reads it back is
/// one rename away from being silently wrong.
pub const SUPERVISOR_ACTOR: &str = "supervisor";

/// The `svc` every kernel record carries — `cmd/rivet-sysd`'s `kmsg` module
/// reads `/dev/kmsg` and stamps this on everything it relays.
///
/// **Not `rivet-sysd`: the supervisor is the *transport*, and a record saying
/// the supervisor reported a PCI error would attribute the kernel's words to
/// the wrong speaker.** `rivet get log | where svc == kernel` is the query
/// this name exists for.
///
/// **Named here for [`SUPERVISOR_SVC`]'s reason**: `cmd/rivet-sysd` stamps
/// this exact value, and a second spelling in the crate that reads it back is
/// one rename away from being silently wrong.
pub const KERNEL_SVC: &str = "kernel";

/// How many records one uid may file in a burst — [`IngestRate`]'s bucket size.
///
/// Enough that an application starting up and filing a run of records is never
/// throttled: `ADR-022` §12's seven boot events are the supervisor's own and do
/// not come through here, and the largest sender in the tree today —
/// `cmd/rivet`'s `apply edit` — files **one** record per operation.
pub const INGEST_BURST: u32 = 64;

/// The sustained rate one uid may file at, in records per second.
///
/// **Chosen against `ADR-022` §8's cap rather than measured**, and it is worth
/// being exact about that: §8 gives the journal a rolling ~1 MiB, a record is a
/// few hundred bytes, and eight per second is therefore in the region of an hour
/// to turn the journal over — long enough that a flood is a thing an operator
/// can catch a running machine doing, rather than something that has already
/// finished by the time anybody looks. `ADR-022` §11.7 records that nothing has
/// ever sent this socket more than a handful of records, so this is a bound
/// derived from the storage it protects and **not** a rate anything has been
/// observed to need.
pub const INGEST_PER_SECOND: u32 = 8;

/// How many uids [`IngestRate`] keeps a bucket for.
///
/// Bounded because this lives in the process `ADR-017` §8 item 2 says must not
/// fail, and an unbounded map keyed by anything a caller influences is the shape
/// of the next resource exhaustion. `ADR-015` §5.1 gives each application its own
/// uid, so the real population is the installed applications plus the system's
/// own accounts.
///
/// **When it is full the least recently seen bucket is evicted**, and a new uid
/// starts with a full bucket. That is a way to reset a bucket — and it costs the
/// attacker control of thirty-two distinct uids, which needs root, and root can
/// append to `/state/log/journal.jsonl` directly. Stated rather than left as a
/// gap for somebody to find.
pub const INGEST_TRACKED_UIDS: usize = 32;

/// How often a uid over its rate gets one `warn` record saying so.
///
/// **A record per dropped record would be the flood**, filed by the supervisor
/// against its own name, which is worse than the attack. One per minute per uid,
/// carrying the count since the last one, says the same thing in bounded space —
/// and saying it at all is the requirement: a journal that quietly discarded
/// records would be giving the answer `AGENTS.md` §5 forbids.
///
/// **What this costs, stated rather than left to be found:** the first drop in a
/// window is reported immediately and carries a count of one, so an operator
/// learns *that* a uid is over its share at the moment it happens. The rest of
/// that window is counted and carried on the **next** note — so a flood that
/// stops inside a window leaves a tail counted and never written down. Nothing
/// here ticks on its own; [`IngestRate::admit`] is only reached when a record
/// arrives, and a timer in PID 1's only child to flush a counter would be more
/// machinery than the fact is worth. The journal therefore says *"records from
/// this uid were dropped"* accurately and its running total can lag.
pub const INGEST_REPORT_EVERY_MS: u64 = 60_000;

/// The relationships between the ingest caps, checked **at compile time**.
///
/// A runtime test of a comparison between two constants is a test the optimiser
/// can see through and clippy calls out as always true, which is fair — so it is
/// written as the stronger thing it wanted to be. A build where the wire cap
/// dropped below the message cap would truncate every unparsable line at the
/// wire, and a build with no in-flight slots would accept nothing at all.
const _: () = assert!(INGEST_MAX_BYTES > INGEST_UNPARSED_MAX_BYTES);
const _: () = assert!(INGEST_MAX_IN_FLIGHT > 0);
// A bucket that cannot hold one record admits nothing, and a bucket that never
// refills turns the rate limit into a lifetime quota after the first burst.
const _: () = assert!(INGEST_BURST > 0);
const _: () = assert!(INGEST_PER_SECOND > 0);
const _: () = assert!(INGEST_TRACKED_UIDS > 0);

/// The field names the journal supplies for an ingested record.
///
/// **A sender's own field with one of these names is dropped**, for
/// [`Record::RESERVED_FIELD_NAMES`]' reason one level out: the key would be on
/// the line twice and RFC 8259 lets a parser take either value. Here it is also
/// a forgery: a sender that could set `ingest_uid` could put somebody else's
/// account on its own record.
pub const INGEST_FIELD_NAMES: [&str; 10] = [
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

/// What the kernel says about the process on the other end of an ingest
/// connection.
///
/// **`SO_PEERCRED`, and therefore not a claim.** The kernel fills these in at
/// `connect(2)` from the connecting process's own credentials; the sender
/// cannot write them and cannot choose them.
///
/// # How this relates to [`crate::deploy::Principal`], because they are not the
/// same thing
///
/// [`crate::deploy::Principal`] reads the pinned peer key out of a process's
/// **environment**, which the process itself can set. It is self-reported, it
/// says so, and `ADR-028` §4.5 is careful that it is attribution rather than
/// authorisation.
///
/// This is not a stronger version of that. It answers a **different question**,
/// and both are worth having:
///
/// | | [`Peer`] | [`crate::deploy::Principal`] |
/// | :-- | :-- | :-- |
/// | Answers | *which process on this machine sent this* | *which caller out there asked for it* |
/// | Supplied by | the kernel, at `connect(2)` | the sender, from its own environment |
/// | Forgeable by the sender | no | yes |
/// | Survives across a machine boundary | no — a uid is local | yes — a pinned key is the same key `ADR-025` §2's map names |
/// | Distinguishes two people | **eventually** (`ADR-008` §7.3) | today |
///
/// Today `rivet` runs as root for every operator, so [`Peer::uid`] is 0 and
/// says nothing about *who*; the pinned key in a `Principal` does. Conversely a
/// `Principal` naming a remote caller proves nothing about which process wrote
/// it, and [`Peer::pid`] does. **Neither authorises anything.** They are two
/// observations that a reader can put beside each other, which is the whole of
/// the chain of custody `ADR-028` §4.5 claims and is why the ingested record
/// carries both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peer {
    pub uid: u32,
    pub gid: u32,
    pub pid: i32,
}

impl Peer {
    /// `ADR-022` §5's actor for a record this peer sent.
    ///
    /// **The journal overrides the sender's own `actor` with this**, and that is
    /// the property that makes an ingested record impossible to disguise: every
    /// record the supervisor produces itself carries the supervisor's actor, and
    /// no value of `uid` can produce that string.
    #[must_use]
    pub fn actor(&self) -> String {
        format!("{INGEST_ACTOR_PREFIX}{}", self.uid)
    }
}

/// What the journal should do with one record off the ingest socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub enum Admission {
    /// File it.
    Accept,
    /// **Drop it, and say so**, naming this many records dropped for this uid
    /// since the last time it was said — including this one, so the counts add
    /// up to every record that was refused.
    Report { dropped: u64 },
    /// Drop it. The sentence has already been written for this uid inside
    /// [`INGEST_REPORT_EVERY_MS`]; writing it again is the flood.
    Drop,
}

/// One key's token bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bucket<K> {
    key: K,
    /// Tokens, in **thousandths of an event**, so the refill is integer
    /// arithmetic — a per-second rate of N is exactly N thousandths per
    /// millisecond, with no rounding to accumulate and no float in the process
    /// `ADR-017` §8 item 2 says must not fail.
    milli_tokens: u64,
    /// When [`Bucket::milli_tokens`] was last brought up to date.
    last_ms: u64,
    /// Events refused since the last [`Admission::Report`].
    dropped: u64,
    /// When the last [`Admission::Report`] was made, or `None` if none has been.
    reported_ms: Option<u64>,
}

const MILLI: u64 = 1_000;

/// A bounded set of token buckets keyed by `K` — the **shared core** of every
/// per-source rate limit in this project, extracted so the proven refill,
/// eviction and collapse-into-one-report logic exists once (`ADR-011`).
///
/// [`IngestRate`] wraps one keyed by uid for the journal ingest socket;
/// `cmd/rivet-mcpd`'s control plane wraps one keyed by source address. The
/// caller owns the policy the key does not carry — the journal exempts uid 0,
/// the control plane exempts nobody — so no exemption lives here.
///
/// # Why a token bucket
///
/// A sender's traffic is bursty by nature — a program starting up files several
/// records and then goes quiet for hours, an operator runs a clone and then
/// sits idle — so a flat events-per-second window would refuse the ordinary
/// case while permitting a steady drip. A bucket admits the burst and bounds
/// the average, which is the shape the traffic actually has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenBuckets<K: Copy + Eq> {
    /// Bounded by [`TokenBuckets::tracked`]; eviction is least-recently-seen.
    buckets: Vec<Bucket<K>>,
    /// The ceiling a key's token count refills to — the largest burst admitted
    /// at once, in events.
    burst: u32,
    /// The sustained rate one key may run at, in events per second.
    per_second: u32,
    /// How many keys a bucket is kept for. Bounded because this can live in the
    /// process `ADR-017` §8 item 2 says must not fail, and an unbounded map
    /// keyed by anything a caller influences is the shape of the next
    /// exhaustion. When it is full the least recently seen key is evicted and a
    /// new key starts with a full bucket.
    tracked: usize,
    /// How often a key over its rate gets one [`Admission::Report`] saying so.
    /// An event per dropped event would be the flood; one per window carrying
    /// the count since the last says the same in bounded space.
    report_every_ms: u64,
}

impl<K: Copy + Eq> TokenBuckets<K> {
    /// A limiter admitting a `burst` of events at once, `per_second` sustained,
    /// tracking at most `tracked` keys, reporting a flooding key at most once
    /// per `report_every_ms`.
    #[must_use]
    pub const fn new(burst: u32, per_second: u32, tracked: usize, report_every_ms: u64) -> Self {
        Self {
            buckets: Vec::new(),
            burst,
            per_second,
            tracked,
            report_every_ms,
        }
    }

    /// Decide what to do with one event from `key`, at monotonic `now_ms`.
    ///
    /// **`now_ms` is `CLOCK_MONOTONIC`** (`ADR-021` §5). A wall clock here would
    /// let a machine whose time was corrected backwards refill every bucket, or
    /// stall every bucket for the length of a forward jump — and `ADR-021` §3
    /// makes exactly that correction on a machine with no RTC, which is every
    /// Raspberry Pi.
    pub fn admit(&mut self, key: K, now_ms: u64) -> Admission {
        let capacity = u64::from(self.burst) * MILLI;
        let per_second = self.per_second;
        let report_every_ms = self.report_every_ms;
        let index = self.bucket_for(key, now_ms, capacity);
        let bucket = &mut self.buckets[index];

        // `saturating_sub`, because a monotonic clock that appears to go
        // backwards is a defect somewhere else and must not become a refill
        // here — nor a panic in PID 1's only child.
        let elapsed = now_ms.saturating_sub(bucket.last_ms);
        bucket.milli_tokens = bucket
            .milli_tokens
            .saturating_add(elapsed.saturating_mul(u64::from(per_second)))
            .min(capacity);
        bucket.last_ms = now_ms;

        if bucket.milli_tokens >= MILLI {
            bucket.milli_tokens -= MILLI;
            return Admission::Accept;
        }

        self.note_refused(index, now_ms, report_every_ms)
    }

    /// Record one refusal for the bucket at `index` and decide whether to
    /// report it. Separated from [`TokenBuckets::admit`] so a refusal decided
    /// elsewhere — a connection the rate let through but a concurrency cap did
    /// not — collapses into the **same** one-line-per-window cadence rather
    /// than opening a second channel of journal spam.
    fn note_refused(&mut self, index: usize, now_ms: u64, report_every_ms: u64) -> Admission {
        let bucket = &mut self.buckets[index];
        bucket.dropped = bucket.dropped.saturating_add(1);
        let due = match bucket.reported_ms {
            None => true,
            Some(at) => now_ms.saturating_sub(at) >= report_every_ms,
        };
        if due {
            let dropped = bucket.dropped;
            bucket.dropped = 0;
            bucket.reported_ms = Some(now_ms);
            return Admission::Report { dropped };
        }
        Admission::Drop
    }

    /// Record a refusal for `key` that this limiter's rate did not itself
    /// decide, collapsed on the same cadence as a rate refusal. Used by a
    /// caller whose own gate (a concurrency cap) turned the event away.
    pub fn refuse(&mut self, key: K, now_ms: u64) -> Admission {
        let capacity = u64::from(self.burst) * MILLI;
        let report_every_ms = self.report_every_ms;
        let index = self.bucket_for(key, now_ms, capacity);
        self.note_refused(index, now_ms, report_every_ms)
    }

    /// The index of `key`'s bucket, creating or evicting one if it has none.
    fn bucket_for(&mut self, key: K, now_ms: u64, capacity: u64) -> usize {
        if let Some(index) = self.buckets.iter().position(|b| b.key == key) {
            return index;
        }

        let fresh = Bucket {
            key,
            milli_tokens: capacity,
            last_ms: now_ms,
            dropped: 0,
            reported_ms: None,
        };

        if self.buckets.len() < self.tracked {
            self.buckets.push(fresh);
            return self.buckets.len() - 1;
        }

        // Least recently seen. `last_ms` is touched on every admission, so this
        // is the key that has been quiet longest — the one whose bucket has most
        // likely refilled to full anyway, which is what makes the eviction cheap
        // rather than merely arbitrary.
        let mut oldest = 0;
        for (index, bucket) in self.buckets.iter().enumerate() {
            if bucket.last_ms < self.buckets[oldest].last_ms {
                oldest = index;
            }
        }
        self.buckets[oldest] = fresh;
        oldest
    }

    /// How many keys are tracked right now. For the bound test and a diagnostic.
    #[must_use]
    pub fn tracked_now(&self) -> usize {
        self.buckets.len()
    }
}

/// A per-uid rate limit on the journal ingest socket.
///
/// # What this is, and what it is emphatically not
///
/// **It is a rate limit, not authentication.** `ADR-022` §11.3 is explicit that
/// the credential on an ingest connection *authorises nothing* — it says which
/// process sent a record, and nothing about whether the record is true. That is
/// unchanged. This adds one thing on top of it: a bound on **how much** of
/// `ADR-022` §8's rolling storage any one uid can consume.
///
/// So, plainly, **what it does not stop**:
///
/// * A local process can still file records, attributed to its own uid, saying
///   anything it likes. That is the ruling (§11.1) and not a defect.
/// * A process that stays under the rate can still fill the journal, given
///   time. [`INGEST_PER_SECOND`] makes that slow enough to notice; it does not
///   make it impossible.
/// * Several uids cooperating get several buckets. The limit is per uid because
///   the uid is the only thing about a sender the kernel vouches for.
/// * **Nothing here stops uid 0**, which is exempt — see [`IngestRate::admit`].
/// * It does nothing about the *content* of a record, and a hostile process can
///   still claim a `svc` that is not its own (§11.4, and [`SUPERVISOR_SVC`] for
///   the one exception).
///
/// `PLAN.md` §11.2 put four options to the maintainer on 2026-08-24 and
/// recommended *"(a) now, (c) when the first application ships"*. This is (c),
/// the per-uid rate limit, built on 2026-09-04 after the audit of 2026-09-03
/// raised the same finding and after applications shipped.
///
/// # Why a token bucket, and why the core is shared
///
/// A sender's traffic is bursty by nature — a program starting up files several
/// records and then goes quiet for hours — so a flat records-per-second window
/// would refuse the ordinary case while permitting a steady drip that empties
/// the journal just as thoroughly. A bucket admits the burst and bounds the
/// average, which is the shape the traffic actually has.
///
/// The bucket math itself is [`TokenBuckets`], shared with the control plane's
/// per-source limit (`cmd/rivet-mcpd`). This type adds the one thing specific
/// to the journal: **uid 0 is exempt.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestRate {
    /// The uid 0 exemption lives here, in [`IngestRate::admit`]; everything else
    /// is the shared core, bounded by [`INGEST_TRACKED_UIDS`].
    inner: TokenBuckets<u32>,
}

impl Default for IngestRate {
    fn default() -> Self {
        Self::new()
    }
}

impl IngestRate {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inner: TokenBuckets::new(
                INGEST_BURST,
                INGEST_PER_SECOND,
                INGEST_TRACKED_UIDS,
                INGEST_REPORT_EVERY_MS,
            ),
        }
    }

    /// Decide what to do with one record from `uid`, at monotonic `now_ms`.
    ///
    /// **uid 0 is always accepted**, and that is a decision rather than an
    /// oversight. Root can append to `/state/log/journal.jsonl` directly —
    /// `ADR-022` §8's file is `0600` and owned by it — so a limit on root buys
    /// nothing against a hostile root and costs the operator's own `apply edit`
    /// records the moment a script files a few in a row. `PLAN.md` §11.2 makes
    /// the same argument against option (b) from the other direction. The
    /// control plane shares the bucket math but not this exemption — there is no
    /// privileged source address.
    ///
    /// **`now_ms` is `CLOCK_MONOTONIC`** (`ADR-021` §5). A wall clock here would
    /// let a machine whose time was corrected backwards refill every bucket, or
    /// stall every bucket for the length of a forward jump — and `ADR-021` §3
    /// makes exactly that correction on a machine with no RTC, which is every
    /// Raspberry Pi.
    pub fn admit(&mut self, uid: u32, now_ms: u64) -> Admission {
        if uid == 0 {
            return Admission::Accept;
        }
        self.inner.admit(uid, now_ms)
    }
}

/// What one ingested line becomes, before the journal stamps it.
///
/// The three-way split this type exists to make visible:
///
/// * **the sender** keeps what only it knows — severity, message, fields, and
///   the service name it claims to be;
/// * **the journal** supplies what only it knows — `boot`, `mono`, `ts` and a
///   fresh `corr`, none of which appear here because [`crate::journal`]'s writer
///   stamps them;
/// * **the kernel** supplies who — [`Ingested::actor`] and the `ingest_*`
///   fields.
#[derive(Debug, Clone, PartialEq)]
pub struct Ingested {
    pub sev: Severity,
    pub svc: String,
    pub actor: String,
    pub msg: String,
    pub fields: Vec<(String, Value)>,
}

/// Turn one line off the ingest socket into the parts of a record.
///
/// **Pure, and here rather than in the supervisor**, for the reason this
/// module's header gives about the format itself: what an ingested record
/// becomes is a decision, every branch of it is reachable from a test with no
/// socket, and a second copy of the rule would be `ADR-011`'s failure.
///
/// # It never refuses
///
/// A line that is not a record becomes a `warn` record carrying the bytes,
/// because the alternative is the answer `AGENTS.md` §5 forbids — *"there is
/// nothing about that in the log"* said falsely. A sender emitting rubbish is
/// itself a defect worth seeing, which is why it is `warn` and not `info`.
///
/// # What a sender may and may not claim
///
/// It may claim a `svc`, **except [`SUPERVISOR_SVC`]** — added 2026-09-04 for
/// the audit of 2026-09-03, and argued at that constant. It cannot claim
/// `boot`, `mono`, `ts`, `corr` or `actor` — the journal stamps those — and it
/// cannot claim any name in [`INGEST_FIELD_NAMES`]. So the strongest thing a
/// hostile sender can do is write a record attributed to some **other** service
/// it is not, **beside** the kernel's own `ingest_pid` and `ingest_uid`
/// contradicting it. That is weaker than what `ADR-022` §9.2 gives a spawned
/// service, which cannot claim a `svc` at all, and §11.4 records the trade
/// rather than hiding it.
#[must_use]
pub fn ingest(line: &str, peer: &Peer) -> Ingested {
    let mut fields: Vec<(String, Value)> = Vec::new();
    let (sev, svc, msg);

    match Record::from_jsonl(line) {
        Ok(record) => {
            sev = record.sev;
            msg = record.msg;
            // A claimed `svc` has to be a name a service could actually have,
            // or a reader sorting the journal by service gets a column with a
            // kilobyte of attacker-chosen bytes in it. The rule is the one that
            // already governs every real name on this machine.
            //
            // **And it may not be the supervisor's own name**, which is the one
            // claim §11.4's trade does not cover — [`SUPERVISOR_SVC`] carries
            // the argument. Both refusals land in the same place: the name
            // becomes `ingest` and the claim is kept as `svc_claimed`, so
            // nothing is lost and the attempt is visible.
            svc = if record.svc != SUPERVISOR_SVC
                && crate::service::ServiceName::new(&record.svc).is_some()
            {
                record.svc
            } else {
                fields.push((
                    "svc_claimed".to_string(),
                    Value::Str(truncate_utf8(&record.svc, 128).0.to_string()),
                ));
                INGEST_SVC.to_string()
            };
            // The sender's own chain and its own clock. Kept as fields rather
            // than dropped — they are the only written record of what it
            // thought it was doing — and never as the columns, which are the
            // journal's. `ADR-022` §9.2 makes the same split for a captured
            // record and this is that rule, one door over.
            fields.push(("emitted_corr".to_string(), Value::Str(record.corr)));
            fields.push(("emitted_mono".to_string(), Value::Float(record.mono)));
            fields.push(("emitted_boot".to_string(), Value::Str(record.boot)));
            let mut overrode = 0i64;
            for (name, value) in record.fields {
                if INGEST_FIELD_NAMES.contains(&name.as_str()) {
                    overrode += 1;
                    continue;
                }
                fields.push((name, value));
            }
            if overrode > 0 {
                fields.push(("ingest_overrode".to_string(), Value::Int(overrode)));
            }
        }
        Err(problem) => {
            sev = Severity::Warn;
            svc = INGEST_SVC.to_string();
            let (kept, cut) = truncate_utf8(line, INGEST_UNPARSED_MAX_BYTES);
            msg = kept.to_string();
            fields.push(("ingest_unparsed".to_string(), Value::Str(problem.to_string())));
            if cut {
                fields.push(("ingest_truncated".to_string(), Value::Bool(true)));
            }
        }
    }

    fields.push(("ingest_uid".to_string(), Value::Int(i64::from(peer.uid))));
    fields.push(("ingest_gid".to_string(), Value::Int(i64::from(peer.gid))));
    fields.push(("ingest_pid".to_string(), Value::Int(i64::from(peer.pid))));

    Ingested {
        sev,
        svc,
        actor: peer.actor(),
        msg,
        fields,
    }
}

/// The longest prefix of `text` that is at most `max` bytes and ends on a
/// character boundary, and whether anything was cut.
///
/// Byte-counted rather than character-counted because the cap is about how much
/// of the card one record may take, and a character is one to four bytes.
fn truncate_utf8(text: &str, max: usize) -> (&str, bool) {
    if text.len() <= max {
        return (text, false);
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

#[cfg(test)]
mod tests {



    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    /// The defect this function exists to prevent, written as a test.
    ///
    /// `cmd/rivet-deploy` truncated to eight characters while
    /// `cmd/rivet-sysd` kept the whole UUID, so one journal named one boot
    /// two ways and `rivet get log previous` selected wrongly.
    #[test]
    fn a_boot_identifier_is_never_truncated() {
        let uuid = "05319ec6-c005-422e-857e-9604ca13d971";
        assert_eq!(super::boot_identifier(Ok(format!("{uuid}
"))), uuid);
        assert_eq!(super::boot_identifier(Ok(format!("  {uuid}  "))), uuid);
    }

    /// A machine that will not answer says so rather than inventing one.
    #[test]
    fn an_unreadable_boot_id_is_unknown_rather_than_synthesised() {
        assert_eq!(super::boot_identifier(Err(())), "unknown");
    }

    /// The two boots in one journal that started this: a prefix collides where
    /// the whole identifier does not.
    #[test]
    fn two_boots_sharing_a_prefix_stay_distinct() {
        let a = "05319ec6-c005-422e-857e-9604ca13d971";
        let b = "05319ec6-0f0d-4fdc-81c0-3d19240047c8";
        assert_eq!(&a[..8], &b[..8], "the fixtures must share a prefix");
        assert_ne!(
            super::boot_identifier(Ok(a.to_string())),
            super::boot_identifier(Ok(b.to_string())),
            "truncation would splice these two boots together"
        );
    }

    use super::*;

    fn field(name: &str, v: Value) -> (String, Value) {
        (name.to_string(), v)
    }

    fn sample() -> Record {
        Record {
            ts: Some("2026-08-22T14:02:11.412Z".to_string()),
            boot: "0d3f".to_string(),
            mono: 184.221,
            sev: Severity::Warn,
            svc: "rivet-netd".to_string(),
            corr: "a91c".to_string(),
            actor: "agent:claude".to_string(),
            msg: "association failed".to_string(),
            fields: vec![field("ssid_len", Value::Int(11))],
        }
    }

    fn at(mono: f64, boot: &str, sev: Severity, svc: &str, msg: &str) -> Record {
        Record {
            ts: None,
            boot: boot.to_string(),
            mono,
            sev,
            svc: svc.to_string(),
            corr: "c".to_string(),
            actor: "supervisor".to_string(),
            msg: msg.to_string(),
            fields: Vec::new(),
        }
    }

    // -- JSON encoding ------------------------------------------------------

    #[test]
    fn a_record_renders_every_fixed_field_adr_022_3_2_names() {
        assert_eq!(
            sample().to_jsonl(),
            concat!(
                r#"{"ts":"2026-08-22T14:02:11.412Z","boot":"0d3f","mono":184.221,"#,
                r#""sev":"warn","svc":"rivet-netd","corr":"a91c","#,
                r#""actor":"agent:claude","msg":"association failed","ssid_len":11}"#,
                "\n"
            )
        );
    }

    /// The defect this exists for actually shipped.
    ///
    /// The supervisor's start-up record added a `boot` field while every record
    /// already carried the boot id as a column, so **every line it wrote had
    /// `"boot"` twice** — ambiguous under RFC 8259, and `ADR-022` §3 makes this
    /// form authoritative. It was found by a human reading console output. This
    /// test is deliberately written against *every* reserved name rather than
    /// against `boot`, because fixing the one that was noticed is how the other
    /// seven survive (`AGENTS.md` §7).
    #[test]
    fn no_record_can_emit_the_same_key_twice() {
        let r = Record {
            fields: Record::RESERVED_FIELD_NAMES
                .iter()
                .map(|n| field(n, Value::Str("collision".to_string())))
                .chain(std::iter::once(field("kept", Value::Int(7))))
                .collect(),
            ..sample()
        };
        let line = r.to_jsonl();

        for name in Record::RESERVED_FIELD_NAMES {
            let key = format!("\"{name}\":");
            assert_eq!(
                line.matches(&key).count(),
                1,
                "{name} appears more than once in {line}"
            );
        }
        // The collision is dropped, not the record, and not the innocent field.
        assert!(
            !line.contains("collision"),
            "a reserved field leaked: {line}"
        );
        assert!(
            line.contains(r#""kept":7"#),
            "a good field was lost: {line}"
        );
    }

    #[test]
    fn a_record_without_a_wall_clock_omits_ts_rather_than_inventing_one() {
        let r = Record {
            ts: None,
            fields: Vec::new(),
            ..sample()
        };
        let line = r.to_jsonl();
        assert!(!line.contains("\"ts\""), "{line}");
        assert!(line.starts_with("{\"boot\":"), "{line}");
    }

    /// The escaping is the only place this encoder can be quietly wrong, and a
    /// wrong escape produces a line no parser will accept — which is exactly
    /// when the logs are being read.
    #[test]
    fn json_strings_escape_what_rfc_8259_requires() {
        let mut out = String::new();
        write_json_string("a\"b\\c\nd\re\tf\u{08}g\u{0c}h\u{1}i", &mut out);
        assert_eq!(out, "\"a\\\"b\\\\c\\nd\\re\\tf\\bg\\fh\\u0001i\"");
    }

    #[test]
    fn json_strings_pass_utf8_through_so_the_file_stays_readable() {
        let mut out = String::new();
        write_json_string("café — 日本語", &mut out);
        assert_eq!(out, "\"café — 日本語\"");
    }

    #[test]
    fn del_is_not_escaped_because_rfc_8259_does_not_require_it() {
        let mut out = String::new();
        write_json_string("\u{7f}", &mut out);
        assert_eq!(out, "\"\u{7f}\"");
    }

    #[test]
    fn every_control_character_below_space_is_escaped() {
        for c in 0u32..0x20 {
            let mut out = String::new();
            let s = char::from_u32(c).unwrap_or('?').to_string();
            write_json_string(&s, &mut out);
            assert!(
                !out.contains(char::from_u32(c).unwrap_or('?')),
                "U+{c:04X} reached the output unescaped as {out:?}"
            );
        }
    }

    #[test]
    fn a_field_name_is_escaped_exactly_as_a_value_is() {
        let r = Record {
            fields: vec![field("a\"b", Value::Str("c\"d".to_string()))],
            ..sample()
        };
        assert!(
            r.to_jsonl().contains(r#""a\"b":"c\"d""#),
            "{}",
            r.to_jsonl()
        );
    }

    #[test]
    fn the_four_value_kinds_render_as_their_json_types() {
        let mut out = String::new();
        Value::Int(-7).write_json(&mut out);
        out.push(' ');
        Value::Bool(true).write_json(&mut out);
        out.push(' ');
        Value::Bool(false).write_json(&mut out);
        out.push(' ');
        Value::Float(1.5).write_json(&mut out);
        assert_eq!(out, "-7 true false 1.5");
    }

    /// JSON has no NaN and no Infinity. Emitting one would make the line
    /// unparseable, so the encoder says what the value was instead.
    #[test]
    fn non_finite_floats_become_strings_rather_than_invalid_json() {
        for (v, expected) in [
            (f64::NAN, "\"nan\""),
            (f64::INFINITY, "\"inf\""),
            (f64::NEG_INFINITY, "\"-inf\""),
        ] {
            let mut out = String::new();
            Value::Float(v).write_json(&mut out);
            assert_eq!(out, expected);
        }
    }

    #[test]
    fn mono_is_rendered_to_milliseconds_and_not_in_exponent_form() {
        let r = Record {
            mono: 0.000_12,
            ..sample()
        };
        assert!(r.to_jsonl().contains("\"mono\":0.000"), "{}", r.to_jsonl());
    }

    #[test]
    fn a_record_ends_in_exactly_one_newline_because_jsonl_is_one_per_line() {
        let r = Record {
            ts: None,
            msg: "line\nbreak".to_string(),
            fields: Vec::new(),
            ..sample()
        };
        let line = r.to_jsonl();
        assert_eq!(line.matches('\n').count(), 1, "{line:?}");
        assert!(line.ends_with("}\n"));
    }

    // -- reading it back ----------------------------------------------------
    //
    // The fixture is the load-bearing half. Round-trip tests come after it, and
    // `AGENTS.md` §4 is explicit that they prove the two halves agree with each
    // other and nothing about the format.

    /// Provenance: captured verbatim from the console of a Rivet guest booted
    /// under `qemu-system-aarch64 -M virt` on `ADR-024`'s own kernel, 33 records
    /// from one boot. Nothing was edited; the guest's RTC was pinned to the
    /// epoch at boot so the file carries no real time (`AGENTS.md` §8.3).
    ///
    /// **Its `svc` is `rivet-supervisor`, which no process is called any
    /// more.** The supervisor became `rivet-sysd` on 2026-08-24, after this
    /// capture was taken. That string is **not** rewritten and the assertion
    /// below still names it: the bytes of a line are what a machine wrote, and
    /// editing them would make the fixture agree with this repository instead
    /// of with a guest — which is the one property it exists to have
    /// (`AGENTS.md` §4, §5). A guest today writes `rivet-sysd`.
    ///
    /// **Lines have been removed and none has been edited**, which is the only
    /// change this file admits. A record naming a service the image no longer
    /// carries describes a boot no Rivet can have; removing the whole line
    /// leaves every surviving line exactly as the guest wrote it, where an edit
    /// would leave a line no guest ever wrote. The count assertion below is what
    /// stops the file being whittled down to something that proves nothing.
    const GUEST_CAPTURE: &str = include_str!("../tests/fixtures/qemu-guest-console.jsonl");

    #[test]
    fn every_line_a_real_guest_wrote_reads_back() {
        let (records, refused) = parse_lines(GUEST_CAPTURE.lines());
        assert!(
            refused.is_empty(),
            "the reader refused {} of a real guest's own lines: {refused:?}",
            refused.len()
        );
        assert!(
            records.len() >= 30,
            "the fixture shrank to {} records; it is meant to be a whole boot",
            records.len()
        );
    }

    /// Not "it parsed" but "it parsed into the right things". A reader that
    /// returned every record with an empty `msg` would pass the test above.
    #[test]
    fn the_guest_capture_reads_back_as_one_boot_of_a_supervisor() {
        let (records, _) = parse_lines(GUEST_CAPTURE.lines());
        assert_eq!(
            boots_in_order(&records).len(),
            1,
            "the fixture is one boot and should carry one boot id"
        );
        assert!(records.iter().any(|r| r.svc == "rivet-supervisor"));
        assert!(records.iter().any(|r| r.svc == "rivet-netd"));
        assert!(records.iter().any(|r| r.sev == Severity::Error));
        assert!(records.iter().all(|r| !r.msg.is_empty()));
        assert!(records.iter().all(|r| !r.corr.is_empty()));
        // Monotonic time is monotonic, which is the property ADR-022 §3.2 says
        // makes ordering recoverable whatever the wall clock did.
        for pair in records.windows(2) {
            assert!(pair[0].mono <= pair[1].mono, "{:?}", pair);
        }
    }

    /// The typed fields survive, including the distinction between a count and
    /// a measurement — `"bytes":251` must not come back as `251.0`.
    #[test]
    fn a_guest_record_keeps_its_field_types() {
        let (records, _) = parse_lines(GUEST_CAPTURE.lines());
        let rendered = records
            .iter()
            .find(|r| r.msg == "rendered")
            .expect("the fixture contains the /etc projection's records");
        assert!(
            matches!(rendered.field("bytes"), Some(Value::Int(_))),
            "{rendered:?}"
        );
        assert!(
            matches!(rendered.field("path"), Some(Value::Str(_))),
            "{rendered:?}"
        );

        let mounted = records
            .iter()
            .find(|r| r.field("kernel_says_read_only").is_some())
            .expect("the fixture contains the mount-table readback");
        assert_eq!(
            mounted.field("kernel_says_read_only"),
            Some(&Value::Bool(true))
        );
    }

    /// And the weaker half, stated as such.
    #[test]
    fn a_record_survives_its_own_encoder_and_reader() {
        for r in [
            sample(),
            Record {
                ts: None,
                ..sample()
            },
            Record {
                msg: "a \"quoted\" \\ path\nwith a newline\tand a tab".to_string(),
                fields: vec![
                    field("f", Value::Float(-0.5)),
                    field("b", Value::Bool(false)),
                    field("i", Value::Int(i64::MIN)),
                    field("unicode", Value::Str("café — 日本語 \u{1}".to_string())),
                ],
                ..sample()
            },
        ] {
            let line = r.to_jsonl();
            let back = Record::from_jsonl(&line)
                .unwrap_or_else(|e| panic!("could not read back {line:?}: {e}"));
            // `mono` is written to three decimals, so it is the one field the
            // round trip cannot preserve exactly. Compared at the precision the
            // format actually carries rather than exempted.
            assert!(
                (back.mono - r.mono).abs() < 5e-4,
                "{} vs {}",
                back.mono,
                r.mono
            );
            assert_eq!(
                Record {
                    mono: r.mono,
                    ..back
                },
                r
            );
        }
    }

    #[test]
    fn a_line_with_a_trailing_newline_reads_the_same_as_one_without() {
        let line = sample().to_jsonl();
        assert_eq!(
            Record::from_jsonl(&line),
            Record::from_jsonl(line.trim_end())
        );
    }

    // -- and every way a line can be wrong ----------------------------------

    #[test]
    fn a_torn_line_is_an_error_rather_than_a_short_record() {
        let full = sample().to_jsonl();
        // Every prefix of a real record, which is what a power cut during
        // ADR-022 §8's batched write can leave behind.
        for cut in 1..full.len() - 1 {
            let prefix = &full[..cut];
            assert!(
                Record::from_jsonl(prefix).is_err(),
                "the prefix {prefix:?} was accepted as a record"
            );
        }
    }

    #[test]
    fn a_nested_value_is_refused_because_the_encoder_cannot_emit_one() {
        let line = r#"{"boot":"b","mono":1.0,"sev":"info","svc":"s","corr":"c","actor":"a","msg":"m","nested":{"x":1}}"#;
        assert_eq!(
            Record::from_jsonl(line),
            Err(ParseError::NestedValue {
                key: "nested".to_string()
            })
        );
        let line = line.replace(r#"{"x":1}"#, "[1,2]");
        assert!(matches!(
            Record::from_jsonl(&line),
            Err(ParseError::NestedValue { .. })
        ));
    }

    #[test]
    fn a_null_is_refused_because_rivet_omits_what_it_does_not_know() {
        let line = r#"{"boot":"b","mono":1.0,"sev":"info","svc":"s","corr":"c","actor":"a","msg":"m","x":null}"#;
        assert_eq!(
            Record::from_jsonl(line),
            Err(ParseError::NullValue {
                key: "x".to_string()
            })
        );
    }

    /// The one that matters most for `AGENTS.md` §7: a record from a build that
    /// knows more than this one must be reported, not flattened.
    #[test]
    fn a_severity_from_a_newer_rivet_is_refused_rather_than_read_as_info() {
        let line = r#"{"boot":"b","mono":1.0,"sev":"critical","svc":"s","corr":"c","actor":"a","msg":"m"}"#;
        assert_eq!(
            Record::from_jsonl(line),
            Err(ParseError::UnknownSeverity("critical".to_string()))
        );
    }

    #[test]
    fn every_fixed_column_is_required_and_says_which_is_missing() {
        for column in ["boot", "mono", "sev", "svc", "corr", "actor", "msg"] {
            let full = sample().to_jsonl();
            // Drop just that column, by rebuilding the line without it.
            let (pairs, _) = parse_lines([full.as_str()]);
            let _ = pairs;
            let stripped: String = full
                .trim_end()
                .trim_start_matches('{')
                .trim_end_matches('}')
                .split(',')
                .filter(|part| !part.starts_with(&format!("\"{column}\":")))
                .collect::<Vec<_>>()
                .join(",");
            let line = format!("{{{stripped}}}");
            assert_eq!(
                Record::from_jsonl(&line),
                Err(ParseError::MissingColumn(column)),
                "dropping {column} produced {line}"
            );
        }
    }

    #[test]
    fn ts_is_the_one_column_that_may_be_absent() {
        let line = Record {
            ts: None,
            ..sample()
        }
        .to_jsonl();
        assert!(Record::from_jsonl(&line).is_ok());
    }

    #[test]
    fn a_duplicated_key_is_refused_rather_than_resolved() {
        let line = r#"{"boot":"b","boot":"c","mono":1.0,"sev":"info","svc":"s","corr":"c","actor":"a","msg":"m"}"#;
        assert_eq!(
            Record::from_jsonl(line),
            Err(ParseError::DuplicateKey("boot".to_string()))
        );
        let line = r#"{"boot":"b","mono":1.0,"sev":"info","svc":"s","corr":"c","actor":"a","msg":"m","x":1,"x":2}"#;
        assert_eq!(
            Record::from_jsonl(line),
            Err(ParseError::DuplicateKey("x".to_string()))
        );
    }

    #[test]
    fn a_column_of_the_wrong_type_is_refused() {
        let line =
            r#"{"boot":"b","mono":"soon","sev":"info","svc":"s","corr":"c","actor":"a","msg":"m"}"#;
        assert!(matches!(
            Record::from_jsonl(line),
            Err(ParseError::WrongColumnType { column: "mono", .. })
        ));
    }

    /// An integer `mono` is accepted, because refusing it would make the reader
    /// stricter than the format rather than stricter than this encoder.
    #[test]
    fn an_integer_mono_is_read_as_a_number() {
        let line =
            r#"{"boot":"b","mono":2,"sev":"info","svc":"s","corr":"c","actor":"a","msg":"m"}"#;
        let r = Record::from_jsonl(line);
        assert!(
            matches!(r, Ok(ref rec) if (rec.mono - 2.0).abs() < f64::EPSILON),
            "{r:?}"
        );
    }

    #[test]
    fn two_records_on_one_line_are_refused_because_jsonl_is_one_per_line() {
        let two = format!("{}{}", sample().to_jsonl().trim_end(), sample().to_jsonl());
        assert!(matches!(
            Record::from_jsonl(&two),
            Err(ParseError::TrailingBytes { .. })
        ));
    }

    #[test]
    fn something_that_is_not_json_at_all_is_refused_by_name() {
        assert!(matches!(
            Record::from_jsonl("rivet-initd: starting as pid 1"),
            Err(ParseError::NotAnObject { .. })
        ));
    }

    /// PID 1's own console lines are NOT records — they are plain text on the
    /// same console (`cmd/rivet-initd` predates the journal). A reader handed a
    /// whole console capture must report them rather than pretend the boot had
    /// no beginning.
    #[test]
    fn a_mixed_console_capture_yields_records_and_named_refusals() {
        let record = sample().to_jsonl();
        let mixed = [
            "rivet-initd: starting as pid 1",
            record.trim_end(),
            "",
            "rivet-initd: reaped pid 53 (exit 7) — direct child",
        ];
        let (records, refused) = parse_lines(mixed);
        assert_eq!(records.len(), 1);
        assert_eq!(refused.len(), 2);
        assert_eq!(refused[0].0, 1, "the line number is 1-based");
        assert_eq!(refused[1].0, 4, "the blank line must not be counted");
    }

    // -- the calendar -------------------------------------------------------
    //
    // Provenance: every expectation below was produced by GNU `date -u -d @N
    // +%Y-%m-%dT%H:%M:%S` in the build environment, not by this module's own
    // inverse. `AGENTS.md` §4 — a reader tested against its own writer proves
    // the two agree and nothing about the format.

    #[test]
    fn rfc3339_matches_gnu_date_on_vectors_captured_from_it() {
        for (epoch_s, expected) in [
            (0u64, "1970-01-01T00:00:00.000Z"),
            (1, "1970-01-01T00:00:01.000Z"),
            (951_782_400, "2000-02-29T00:00:00.000Z"), // a century leap year
            (951_868_799, "2000-02-29T23:59:59.000Z"),
            (1_078_012_800, "2004-02-29T00:00:00.000Z"), // an ordinary leap year
            (1_583_020_800, "2020-03-01T00:00:00.000Z"),
            (1_709_164_800, "2024-02-29T00:00:00.000Z"),
            (2_147_483_647, "2038-01-19T03:14:07.000Z"), // the 32-bit cliff
            (4_102_444_800, "2100-01-01T00:00:00.000Z"), // 2100 is not a leap year
            (1_755_820_800, "2025-08-22T00:00:00.000Z"),
        ] {
            assert_eq!(rfc3339(epoch_s * 1000), expected, "at epoch {epoch_s}");
        }
    }

    /// The same vectors, in the shape a person reads. **Independently stated**
    /// rather than derived from [`rfc3339`]'s output — a reader tested against
    /// its own writer proves the two agree and nothing about the format.
    #[test]
    fn human_matches_gnu_date_on_the_same_vectors() {
        for (epoch_s, expected) in [
            (0u64, "01/01/1970 00:00 UTC"),
            (951_782_400, "02/29/2000 00:00 UTC"), // a century leap year
            (951_868_799, "02/29/2000 23:59 UTC"),
            (1_078_012_800, "02/29/2004 00:00 UTC"), // an ordinary leap year
            (1_583_020_800, "03/01/2020 00:00 UTC"),
            (2_147_483_647, "01/19/2038 03:14 UTC"), // the 32-bit cliff
            (4_102_444_800, "01/01/2100 00:00 UTC"), // 2100 is not a leap year
            (1_788_694_903, "09/06/2026 11:41 UTC"),
        ] {
            assert_eq!(human(epoch_s * 1000), expected, "at epoch {epoch_s}");
        }
    }

    /// **Every month is two digits**, including the single-digit ones — an
    /// unpadded `9/06/2026` would not line up in a column and would read as a
    /// different width every ninth of the year.
    #[test]
    fn every_month_is_zero_padded_and_in_the_right_position() {
        // The first of each month of 2026, from GNU `date -u -d 2026-MM-01`.
        for (epoch_s, month) in [
            (1_767_225_600u64, "01"),
            (1_769_904_000, "02"),
            (1_772_323_200, "03"),
            (1_775_001_600, "04"),
            (1_777_593_600, "05"),
            (1_780_272_000, "06"),
            (1_782_864_000, "07"),
            (1_785_542_400, "08"),
            (1_788_220_800, "09"),
            (1_790_812_800, "10"),
            (1_793_491_200, "11"),
            (1_796_083_200, "12"),
        ] {
            let rendered = human(epoch_s * 1000);
            assert_eq!(
                rendered,
                format!("{month}/01/2026 00:00 UTC"),
                "at epoch {epoch_s}"
            );
        }
    }

    /// It says which zone out loud: the instant is UTC and the reader is not.
    #[test]
    fn human_drops_the_seconds_and_names_the_zone() {
        assert_eq!(human(1_788_694_903_812), "09/06/2026 11:41 UTC");
        // The same minute, a different second — the page must not churn.
        assert_eq!(human(1_788_694_940_000), "09/06/2026 11:42 UTC");
    }

    #[test]
    fn rfc3339_carries_milliseconds_and_pads_them() {
        assert_eq!(rfc3339(1_500), "1970-01-01T00:00:01.500Z");
        assert_eq!(rfc3339(1_007), "1970-01-01T00:00:01.007Z");
        assert_eq!(rfc3339(1_070), "1970-01-01T00:00:01.070Z");
    }

    #[test]
    fn the_day_before_a_leap_day_is_not_the_leap_day() {
        // A one-off-by-one in `civil_from_days` shows up here and nowhere else.
        assert_eq!(rfc3339(951_696_000_000), "2000-02-28T00:00:00.000Z");
        assert_eq!(rfc3339(951_868_800_000), "2000-03-01T00:00:00.000Z");
    }

    #[test]
    fn severity_orders_from_debug_to_error_so_a_filter_can_be_written() {
        assert!(Severity::Debug < Severity::Info);
        assert!(Severity::Info < Severity::Warn);
        assert!(Severity::Warn < Severity::Error);
    }

    #[test]
    fn severity_parses_exactly_the_names_it_renders() {
        for sev in Severity::ALL {
            assert_eq!(Severity::parse(sev.as_str()), Some(sev));
        }
        assert_eq!(Severity::parse("INFO"), None);
        assert_eq!(Severity::parse(""), None);
    }

    // -- selecting ----------------------------------------------------------

    fn three_boots() -> Vec<Record> {
        vec![
            at(1.0, "boot-a", Severity::Info, "rivet-sysd", "one"),
            at(2.0, "boot-a", Severity::Error, "rivet-timed", "two"),
            at(1.0, "boot-b", Severity::Info, "rivet-sysd", "three"),
            at(2.0, "boot-b", Severity::Warn, "rivet-netd", "four"),
            at(1.0, "boot-c", Severity::Info, "rivet-sysd", "five"),
        ]
    }

    fn msgs(rs: &[Record]) -> Vec<&str> {
        rs.iter().map(|r| r.msg.as_str()).collect()
    }

    #[test]
    fn the_default_filter_is_this_boot_and_nothing_else() {
        let got = Filter::default().apply(three_boots());
        assert_eq!(msgs(&got), ["five"]);
    }

    /// The one an agent reaches for after a board came back. It cannot be
    /// answered from the wall clock — `ADR-021` — so it is answered from the
    /// order the journal appended in.
    #[test]
    fn previous_selects_the_boot_before_the_last_one() {
        let f = Filter {
            boot: BootSelector::Previous,
            ..Filter::default()
        };
        assert_eq!(msgs(&f.apply(three_boots())), ["three", "four"]);
    }

    #[test]
    fn previous_on_a_journal_with_one_boot_selects_nothing_rather_than_that_boot() {
        let f = Filter {
            boot: BootSelector::Previous,
            ..Filter::default()
        };
        let one = vec![at(1.0, "boot-a", Severity::Info, "s", "only")];
        assert!(
            f.apply(one).is_empty(),
            "a first boot has no predecessor, and answering with itself would be a lie"
        );
    }

    #[test]
    fn all_keeps_every_boot_in_the_order_the_journal_holds_them() {
        let f = Filter {
            boot: BootSelector::All,
            ..Filter::default()
        };
        assert_eq!(
            msgs(&f.apply(three_boots())),
            ["one", "two", "three", "four", "five"]
        );
    }

    #[test]
    fn a_named_boot_that_the_journal_no_longer_holds_selects_nothing() {
        let f = Filter {
            boot: BootSelector::Id("boot-z".to_string()),
            ..Filter::default()
        };
        assert!(f.apply(three_boots()).is_empty());
    }

    #[test]
    fn severity_selects_that_level_and_above() {
        let f = Filter {
            boot: BootSelector::All,
            min_severity: Some(Severity::Warn),
            ..Filter::default()
        };
        assert_eq!(msgs(&f.apply(three_boots())), ["two", "four"]);
    }

    /// Exact, not a prefix — otherwise `--service rivet` would quietly
    /// include `rivet-netd` and `rivet-timed`, and an operator narrowing a
    /// search would get a wider one.
    #[test]
    fn a_service_name_matches_exactly() {
        let f = Filter {
            boot: BootSelector::All,
            service: Some("rivet".to_string()),
            ..Filter::default()
        };
        assert!(f.apply(three_boots()).is_empty());

        let f = Filter {
            boot: BootSelector::All,
            service: Some("rivet-netd".to_string()),
            ..Filter::default()
        };
        assert_eq!(msgs(&f.apply(three_boots())), ["four"]);
    }

    #[test]
    fn contains_matches_the_message_and_a_limit_keeps_the_last_n() {
        let f = Filter {
            boot: BootSelector::All,
            contains: Some("e".to_string()),
            ..Filter::default()
        };
        assert_eq!(msgs(&f.apply(three_boots())), ["one", "three", "five"]);

        let f = Filter {
            boot: BootSelector::All,
            limit: Some(2),
            ..Filter::default()
        };
        assert_eq!(msgs(&f.apply(three_boots())), ["four", "five"]);
    }

    #[test]
    fn a_limit_larger_than_the_journal_is_not_an_error() {
        let f = Filter {
            boot: BootSelector::All,
            limit: Some(1000),
            ..Filter::default()
        };
        assert_eq!(f.apply(three_boots()).len(), 5);
    }

    // -- Following ----------------------------------------------------------

    #[test]
    fn a_follower_opens_with_the_last_n_of_the_boot_it_is_following() {
        let mut f = Follow::start(BootSelector::Latest).unwrap();
        let opened = f.backfill(three_boots(), Some(200));
        assert_eq!(msgs(&opened), ["five"], "`latest` is boot-c");
        assert_eq!(f.boot(), Some("boot-c"));
    }

    /// **The property the whole type exists for.** A stream that re-resolved
    /// `latest` on every read would follow whichever boot was newest at that
    /// moment — so a machine that restarted would splice two boots into one
    /// stream with nothing saying it had.
    #[test]
    fn the_boot_is_resolved_once_and_then_held() {
        let mut f = Follow::start(BootSelector::Latest).unwrap();
        f.backfill(three_boots(), Some(200));
        assert_eq!(f.boot(), Some("boot-c"));

        // A read that carries records of an older boot — a rotated file being
        // re-read, say. The follower does not go backwards, and it does not
        // report a restart either: it has seen boot-a already.
        let got = f.more(vec![at(9.0, "boot-a", Severity::Info, "s", "stale")]);
        assert!(got.records.is_empty(), "{got:?}");
        assert_eq!(got.note, None, "a boot already seen is a re-read");
        assert_eq!(f.boot(), Some("boot-c"));
    }

    #[test]
    fn new_records_of_the_followed_boot_come_through_in_order() {
        let mut f = Follow::start(BootSelector::Latest).unwrap();
        f.backfill(three_boots(), Some(200));
        let got = f.more(vec![
            at(2.0, "boot-c", Severity::Info, "s", "six"),
            at(3.0, "boot-c", Severity::Error, "s", "seven"),
        ]);
        assert_eq!(msgs(&got.records), ["six", "seven"]);
        assert_eq!(got.note, None);
    }

    /// An empty journal has no boot to pin, and the first record that arrives
    /// is the one that decides — not an error, and not a stream that silently
    /// discards everything because it resolved `latest` against nothing.
    #[test]
    fn a_follower_that_started_on_an_empty_journal_pins_on_the_first_record() {
        let mut f = Follow::start(BootSelector::Latest).unwrap();
        assert!(f.backfill(Vec::new(), Some(200)).is_empty());
        assert_eq!(f.boot(), None, "nothing has been written to pin to");

        let got = f.more(vec![at(1.0, "boot-z", Severity::Info, "s", "first")]);
        assert_eq!(msgs(&got.records), ["first"]);
        assert_eq!(f.boot(), Some("boot-z"));
        assert_eq!(
            got.note, None,
            "pinning for the first time is not a boot CHANGE"
        );
    }

    /// A stream that goes permanently silent looks exactly like a quiet
    /// machine, so the one event that would cause it is reported —
    /// `AGENTS.md` §5.
    #[test]
    fn a_restart_under_a_latest_follower_is_reported_and_followed() {
        let mut f = Follow::start(BootSelector::Latest).unwrap();
        f.backfill(three_boots(), Some(200));
        let got = f.more(vec![at(0.1, "boot-d", Severity::Info, "s", "after")]);
        assert_eq!(
            got.note,
            Some(FollowNote::BootChanged {
                from: "boot-c".to_string(),
                to: "boot-d".to_string(),
                switched: true,
            })
        );
        assert_eq!(msgs(&got.records), ["after"]);
        assert_eq!(f.boot(), Some("boot-d"));
    }

    /// A caller who NAMED a boot gets that boot. The restart is still reported
    /// — it is why their stream just went quiet — but it is not silently
    /// swapped for a different subject.
    #[test]
    fn a_restart_under_a_named_follower_is_reported_and_not_followed() {
        let mut f = Follow::start(BootSelector::Id("boot-c".to_string())).unwrap();
        f.backfill(three_boots(), Some(200));
        let got = f.more(vec![at(0.1, "boot-d", Severity::Info, "s", "after")]);
        assert_eq!(
            got.note,
            Some(FollowNote::BootChanged {
                from: "boot-c".to_string(),
                to: "boot-d".to_string(),
                switched: false,
            })
        );
        assert!(got.records.is_empty());
        assert_eq!(f.boot(), Some("boot-c"));
    }

    #[test]
    fn following_all_takes_every_boot_and_reports_no_change_of_subject() {
        let mut f = Follow::start(BootSelector::All).unwrap();
        assert_eq!(
            msgs(&f.backfill(three_boots(), Some(2))),
            ["four", "five"],
            "the limit still counts back from the end of the backfill"
        );
        let got = f.more(vec![at(0.1, "boot-d", Severity::Info, "s", "after")]);
        assert_eq!(msgs(&got.records), ["after"]);
        assert_eq!(got.note, None);
    }

    /// `previous` names a boot that has ended. Following it would be a stream
    /// that is silent by construction, which is indistinguishable from a
    /// machine that is doing nothing.
    #[test]
    fn following_the_previous_boot_is_refused_with_the_reason() {
        let refused = Follow::start(BootSelector::Previous);
        let Err(why) = refused else {
            panic!("`previous` was accepted for following");
        };
        assert!(why.contains("has ended"), "{why}");
    }

    /// The backfill is the last question with an end to count back from; after
    /// it, a limit would mean dropping records the stream had already been
    /// asked for.
    #[test]
    fn the_limit_applies_to_the_backfill_and_never_to_the_stream() {
        let mut f = Follow::start(BootSelector::All).unwrap();
        assert_eq!(msgs(&f.backfill(three_boots(), Some(1))), ["five"]);
        let got = f.more(vec![
            at(2.0, "boot-c", Severity::Info, "s", "six"),
            at(3.0, "boot-c", Severity::Info, "s", "seven"),
        ]);
        assert_eq!(msgs(&got.records), ["six", "seven"]);
    }

    #[test]
    fn filters_compose_rather_than_override_each_other() {
        let f = Filter {
            boot: BootSelector::All,
            service: Some("rivet-sysd".to_string()),
            min_severity: Some(Severity::Info),
            limit: Some(2),
            ..Filter::default()
        };
        assert_eq!(msgs(&f.apply(three_boots())), ["three", "five"]);
    }

    /// **A predicate can erase the cursor's own boot from what survives, and
    /// the cut must still work.** Every `boot-a` record here is `rivet-sysd`,
    /// so a `service` filter for `rivet-netd` removes the whole boot the
    /// cursor sits in — the same shape of loss a `latest` selector produces
    /// right after a reboot. The boot chronology used to place the cursor has
    /// to come from every boot the journal holds, not from what the service
    /// predicate left behind, or `boot-b`'s matching record is wrongly cut
    /// too.
    #[test]
    fn a_service_filter_that_erases_the_cursor_boot_still_places_the_cut() {
        let records = vec![
            at(1.0, "boot-a", Severity::Info, "rivet-sysd", "old one"),
            at(2.0, "boot-a", Severity::Info, "rivet-sysd", "old two"),
            at(1.0, "boot-b", Severity::Info, "rivet-netd", "new one"),
        ];
        let cursor = Cursor::after(&records[1]);
        let f = Filter {
            boot: BootSelector::All,
            service: Some("rivet-netd".to_string()),
            since: Some(cursor),
            ..Filter::default()
        };
        assert_eq!(msgs(&f.apply(records)), ["new one"]);
    }

    #[test]
    fn boots_in_order_is_first_appearance_and_not_sorted() {
        let records = vec![
            at(1.0, "zzz", Severity::Info, "s", "a"),
            at(2.0, "aaa", Severity::Info, "s", "b"),
            at(3.0, "zzz", Severity::Info, "s", "c"),
        ];
        assert_eq!(boots_in_order(&records), ["zzz", "aaa"]);
    }

    #[test]
    fn an_empty_journal_answers_every_selector_with_nothing() {
        for boot in [
            BootSelector::Latest,
            BootSelector::Previous,
            BootSelector::All,
            BootSelector::Id("x".to_string()),
        ] {
            let f = Filter {
                boot,
                ..Filter::default()
            };
            assert!(f.apply(Vec::new()).is_empty());
        }
    }

    // -- ingest (`ADR-022` §11) ---------------------------------------------

    fn peer() -> Peer {
        Peer {
            uid: 1000,
            gid: 1000,
            pid: 4242,
        }
    }

    fn sent(record: &Record) -> String {
        record.to_jsonl()
    }

    fn a_record() -> Record {
        Record {
            ts: Some("2026-08-24T14:02:11.412Z".to_string()),
            boot: "the-senders-idea-of-a-boot".to_string(),
            mono: 99.5,
            sev: Severity::Error,
            svc: "rivet".to_string(),
            corr: "senders-chain".to_string(),
            actor: "operator".to_string(),
            msg: "replace /state/x: 7 bytes to 6".to_string(),
            fields: vec![("path".to_string(), Value::Str("/state/x".to_string()))],
        }
    }

    fn field_of<'a>(i: &'a Ingested, name: &str) -> Option<&'a Value> {
        i.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// The sender keeps what only it knows.
    #[test]
    fn an_ingested_record_keeps_the_senders_severity_message_and_fields() {
        let got = ingest(&sent(&a_record()), &peer());
        assert_eq!(got.sev, Severity::Error);
        assert_eq!(got.msg, "replace /state/x: 7 bytes to 6");
        assert_eq!(
            field_of(&got, "path"),
            Some(&Value::Str("/state/x".to_string()))
        );
    }

    /// **The property a hostile sender cannot get round.** `actor` is the
    /// kernel's answer, so no ingested record can be dressed as one the
    /// supervisor produced — whatever it puts in its own `actor` column.
    #[test]
    fn the_actor_is_the_kernels_uid_and_never_the_senders_claim() {
        let mut claim = a_record();
        claim.actor = "supervisor".to_string();
        let got = ingest(&sent(&claim), &peer());
        assert_eq!(got.actor, "uid:1000");
        assert!(got.actor.starts_with(INGEST_ACTOR_PREFIX));
        assert_ne!(got.actor, "supervisor");
    }

    /// The kernel's three numbers are on every ingested record, whether it
    /// parsed or not.
    #[test]
    fn every_ingested_record_carries_the_peer_credentials() {
        for line in [sent(&a_record()), "not a record at all".to_string()] {
            let got = ingest(&line, &peer());
            assert_eq!(field_of(&got, "ingest_uid"), Some(&Value::Int(1000)));
            assert_eq!(field_of(&got, "ingest_gid"), Some(&Value::Int(1000)));
            assert_eq!(field_of(&got, "ingest_pid"), Some(&Value::Int(4242)));
        }
    }

    /// The sender's own chain and clock survive as fields; the columns are the
    /// journal's. `ADR-022` §9.2's split, one door over.
    #[test]
    fn the_senders_own_correlation_boot_and_clock_become_fields() {
        let got = ingest(&sent(&a_record()), &peer());
        assert_eq!(
            field_of(&got, "emitted_corr"),
            Some(&Value::Str("senders-chain".to_string()))
        );
        assert_eq!(field_of(&got, "emitted_mono"), Some(&Value::Float(99.5)));
        assert_eq!(
            field_of(&got, "emitted_boot"),
            Some(&Value::Str("the-senders-idea-of-a-boot".to_string()))
        );
    }

    /// **A sender cannot forge its own credentials.** A field named
    /// `ingest_uid` is dropped and the kernel's value is what lands — and the
    /// count of what was dropped is reported rather than silent.
    #[test]
    fn a_sender_cannot_supply_a_field_the_kernel_supplies() {
        let mut claim = a_record();
        claim.fields = vec![
            ("ingest_uid".to_string(), Value::Int(0)),
            ("ingest_pid".to_string(), Value::Int(1)),
            ("emitted_corr".to_string(), Value::Str("mine".to_string())),
            ("path".to_string(), Value::Str("/state/x".to_string())),
        ];
        let got = ingest(&sent(&claim), &peer());
        assert_eq!(field_of(&got, "ingest_uid"), Some(&Value::Int(1000)));
        assert_eq!(field_of(&got, "ingest_pid"), Some(&Value::Int(4242)));
        assert_eq!(
            field_of(&got, "emitted_corr"),
            Some(&Value::Str("senders-chain".to_string()))
        );
        assert_eq!(field_of(&got, "ingest_overrode"), Some(&Value::Int(3)));
        // And the field that was not a forgery is untouched.
        assert_eq!(
            field_of(&got, "path"),
            Some(&Value::Str("/state/x".to_string()))
        );
        // The result must be encodable without a duplicate key, which is the
        // failure `Record::RESERVED_FIELD_NAMES` exists for.
        let mut seen: Vec<&str> = got.fields.iter().map(|(n, _)| n.as_str()).collect();
        let count = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), count, "a key appears twice: {seen:?}");
    }

    /// A `svc` that no service could have is replaced and the claim is kept
    /// beside it — never taken as a service name, and never thrown away.
    #[test]
    fn a_svc_that_is_not_a_service_name_is_replaced_and_the_claim_kept() {
        let mut claim = a_record();
        claim.svc = "../../etc/passwd".to_string();
        let got = ingest(&sent(&claim), &peer());
        assert_eq!(got.svc, INGEST_SVC);
        assert_eq!(
            field_of(&got, "svc_claimed"),
            Some(&Value::Str("../../etc/passwd".to_string()))
        );
    }

    /// An enormous `svc` cannot put a kilobyte of attacker-chosen bytes into the
    /// column a reader sorts by.
    #[test]
    fn an_enormous_svc_is_cut_rather_than_carried() {
        let mut claim = a_record();
        claim.svc = "x".repeat(4096);
        let got = ingest(&sent(&claim), &peer());
        assert_eq!(got.svc, INGEST_SVC);
        match field_of(&got, "svc_claimed") {
            Some(Value::Str(kept)) => assert_eq!(kept.len(), 128),
            other => panic!("{other:?}"),
        }
    }

    /// **The regression test for the audit of 2026-09-03's forged-`svc`
    /// finding.** `ADR-022` §11.4 keeps a claimed service name on purpose; the
    /// supervisor's own name is the one exclusion, because `svc` is the column
    /// a reader filters by and `where svc == rivet-sysd` must not return a
    /// local process's chosen text.
    #[test]
    fn a_sender_cannot_claim_the_supervisors_own_service_name() {
        let mut claim = a_record();
        claim.svc = SUPERVISOR_SVC.to_string();
        let got = ingest(&sent(&claim), &peer());
        assert_eq!(got.svc, INGEST_SVC);
        assert_eq!(
            field_of(&got, "svc_claimed"),
            Some(&Value::Str(SUPERVISOR_SVC.to_string())),
            "the claim was discarded rather than recorded"
        );
    }

    /// The other half of §11.4, which did **not** change: any other name is
    /// still kept. Without this the exclusion above could quietly become the
    /// blanket refusal §11.4 rejected — which would lose the records of
    /// `rivet` and `femto`, the senders this socket exists for.
    #[test]
    fn a_sender_may_still_claim_any_other_service_name() {
        for name in ["rivet-apid", "rivet-netd", "rivet", "femto"] {
            let mut claim = a_record();
            claim.svc = name.to_string();
            let got = ingest(&sent(&claim), &peer());
            assert_eq!(got.svc, name);
            assert_eq!(field_of(&got, "svc_claimed"), None, "{name}");
        }
    }

    // -- the per-uid rate limit (`ADR-022` §11.4, `PLAN.md` §11.2 option (c))

    /// Take records for `uid` at `now_ms` until one is refused, and answer with
    /// the refusal.
    ///
    /// **Bounded, and the bound is the point.** The obvious spelling is
    /// `while rate.admit(..) == Accept {}` — and a build where the limit had
    /// been removed would make every test using it spin for ever instead of
    /// failing. A test that hangs when the fix is broken is barely better than
    /// one that cannot fail (`AGENTS.md` §4), so the loop refuses instead.
    fn drain(rate: &mut IngestRate, uid: u32, now_ms: u64) -> Admission {
        let ceiling = usize::try_from(INGEST_BURST).unwrap_or(64) * 4 + 8;
        for _ in 0..ceiling {
            let outcome = rate.admit(uid, now_ms);
            if outcome != Admission::Accept {
                return outcome;
            }
        }
        panic!("uid {uid} was still being accepted after {ceiling} records: the limit is not on");
    }

    /// The ordinary sender is never touched. `cmd/rivet`'s `apply edit` files
    /// one record per operation, so a test that throttled that would be
    /// describing a broken machine.
    #[test]
    fn a_sender_inside_its_share_is_always_accepted() {
        let mut rate = IngestRate::new();
        for n in 0..INGEST_BURST {
            assert_eq!(rate.admit(1000, 1_000), Admission::Accept, "record {n}");
        }
    }

    /// **The bound.** Past the burst, inside one millisecond, nothing more is
    /// accepted — this is the denial-of-evidence attack failing.
    #[test]
    fn a_flood_inside_one_millisecond_is_bounded_at_the_burst() {
        let mut rate = IngestRate::new();
        let mut accepted = 0;
        for _ in 0..(INGEST_BURST * 10) {
            if rate.admit(1000, 1_000) == Admission::Accept {
                accepted += 1;
            }
        }
        assert_eq!(accepted, INGEST_BURST);
    }

    /// **And it is said, once, with the count.** A limit that dropped records
    /// in silence would be `ADR-022` §11.5's forbidden answer arriving through
    /// the mechanism meant to prevent it.
    #[test]
    fn the_first_drop_is_reported_and_the_rest_of_the_window_is_not() {
        let mut rate = IngestRate::new();
        for _ in 0..INGEST_BURST {
            assert_eq!(rate.admit(1000, 1_000), Admission::Accept);
        }
        assert_eq!(rate.admit(1000, 1_000), Admission::Report { dropped: 1 });
        for _ in 0..50 {
            assert_eq!(rate.admit(1000, 1_000), Admission::Drop);
        }
        // The window elapses, and the next drop carries everything since the
        // last report — the 50 above plus the one that triggers it. The first
        // report reset the count, which is what makes the sum of every
        // `dropped` equal the number of records actually refused.
        let later = 1_000 + INGEST_REPORT_EVERY_MS;
        // The bucket refilled over that minute, so it has to be drained first;
        // the value that ends the drain IS the next drop.
        assert_eq!(drain(&mut rate, 1000, later), Admission::Report { dropped: 51 });
    }

    /// The bucket refills, so a sender that waits is served again — a rate
    /// limit and not a lifetime quota.
    #[test]
    fn a_bucket_refills_at_the_stated_rate_and_no_faster() {
        let mut rate = IngestRate::new();
        let _ = drain(&mut rate, 1000, 1_000);
        // One second later: exactly `INGEST_PER_SECOND` more, and not one more
        // than that.
        let mut accepted = 0;
        for _ in 0..(INGEST_PER_SECOND * 4) {
            if rate.admit(1000, 2_000) == Admission::Accept {
                accepted += 1;
            }
        }
        assert_eq!(accepted, INGEST_PER_SECOND);
    }

    /// **Per uid, and that is the whole point of the shape.** A shared bucket
    /// would let one hostile application silence the operator's own tools,
    /// which is the attack rather than the defence.
    #[test]
    fn one_uid_over_its_share_leaves_every_other_uid_alone() {
        let mut rate = IngestRate::new();
        let _ = drain(&mut rate, 1000, 1_000);
        assert!(matches!(
            rate.admit(1000, 1_000),
            Admission::Drop | Admission::Report { .. }
        ));
        assert_eq!(rate.admit(1001, 1_000), Admission::Accept);
        assert_eq!(rate.admit(1002, 1_000), Admission::Accept);
    }

    /// uid 0 is exempt — [`IngestRate::admit`] carries the argument. Asserted
    /// so that closing the hole by accident fails loudly rather than quietly
    /// throttling the operator.
    #[test]
    fn root_is_never_limited() {
        let mut rate = IngestRate::new();
        for _ in 0..(INGEST_BURST * 100) {
            assert_eq!(rate.admit(0, 1_000), Admission::Accept);
        }
    }

    /// The table is bounded, so PID 1's only child cannot be made to allocate
    /// without bound by a caller. Reaching the eviction path takes control of
    /// more uids than [`INGEST_TRACKED_UIDS`], which takes root.
    #[test]
    fn the_number_of_tracked_uids_is_bounded() {
        let mut rate = IngestRate::new();
        for uid in 1..=(u32::try_from(INGEST_TRACKED_UIDS).unwrap_or(32) * 4) {
            assert_eq!(rate.admit(uid, u64::from(uid)), Admission::Accept);
        }
        assert_eq!(rate.inner.tracked_now(), INGEST_TRACKED_UIDS);
    }

    /// A monotonic clock that appears to move backwards is a defect somewhere
    /// else; here it must not become a free refill, and it must not panic in
    /// the process `ADR-017` §8 item 2 says must not fail.
    #[test]
    fn a_clock_that_goes_backwards_refills_nothing() {
        let mut rate = IngestRate::new();
        let _ = drain(&mut rate, 1000, 10_000);
        for _ in 0..10 {
            assert!(matches!(
                rate.admit(1000, 1),
                Admission::Drop | Admission::Report { .. }
            ));
        }
    }

    // -- the shared generic core, on its own terms (control plane uses it)

    /// **The generic core exempts nobody.** `IngestRate` carries the uid-0
    /// exemption itself; the core must not, or the control plane would grant a
    /// free pass to a source address of 0.
    #[test]
    fn the_token_bucket_core_has_no_privileged_key() {
        let mut rate = TokenBuckets::<u32>::new(4, 1, 8, 60_000);
        let mut accepted = 0;
        for _ in 0..40 {
            if rate.admit(0, 1_000) == Admission::Accept {
                accepted += 1;
            }
        }
        assert_eq!(accepted, 4, "key 0 was treated as privileged in the shared core");
    }

    /// **`refuse` collapses on the same cadence as a rate drop.** A caller whose
    /// own gate (the control plane's per-source slot cap) turns an event away
    /// feeds it here so the journal sees one line per window, not one per event.
    #[test]
    fn refuse_reports_once_then_stays_quiet_for_the_window() {
        let mut rate = TokenBuckets::<u32>::new(4, 1, 8, 60_000);
        assert_eq!(rate.refuse(7, 1_000), Admission::Report { dropped: 1 });
        for _ in 0..50 {
            assert_eq!(rate.refuse(7, 1_000), Admission::Drop);
        }
        assert_eq!(rate.refuse(7, 1_000 + 60_000), Admission::Report { dropped: 51 });
    }

    /// A refusal fed through `refuse` shares the key's bucket with `admit`, so a
    /// source already reported for rate is not reported again for a slot refusal
    /// inside the same window — one key, one voice.
    #[test]
    fn refuse_and_admit_share_one_window_per_key() {
        let mut rate = TokenBuckets::<u32>::new(4, 1, 8, 60_000);
        for _ in 0..4 {
            assert_eq!(rate.admit(7, 1_000), Admission::Accept);
        }
        // The rate drop is the first refusal and reports.
        assert_eq!(rate.admit(7, 1_000), Admission::Report { dropped: 1 });
        // A slot refusal for the same key in the same window stays quiet.
        assert_eq!(rate.refuse(7, 1_000), Admission::Drop);
    }

    /// **A sender that speaks nonsense still gets a record**, because the other
    /// answer is "there is nothing about that in the log" said falsely.
    #[test]
    fn a_line_that_is_not_a_record_becomes_a_warning_carrying_the_bytes() {
        let got = ingest("Segmentation fault", &peer());
        assert_eq!(got.sev, Severity::Warn);
        assert_eq!(got.svc, INGEST_SVC);
        assert_eq!(got.msg, "Segmentation fault");
        assert!(field_of(&got, "ingest_unparsed").is_some());
    }

    /// And an enormous one is cut at a character boundary, with the cut
    /// reported. A multi-byte character straddling the cap must not produce
    /// bytes that are not UTF-8.
    #[test]
    fn an_enormous_unparsable_line_is_cut_on_a_character_boundary_and_says_so() {
        let line = "é".repeat(INGEST_UNPARSED_MAX_BYTES);
        let got = ingest(&line, &peer());
        assert!(got.msg.len() <= INGEST_UNPARSED_MAX_BYTES);
        assert!(line.starts_with(&got.msg));
        assert_eq!(field_of(&got, "ingest_truncated"), Some(&Value::Bool(true)));
    }

    /// Whatever a sender sends, the result encodes to one line the journal's own
    /// reader accepts — the round trip that makes this a record rather than a
    /// hope. Driven with the awkward cases rather than a tidy one.
    #[test]
    fn anything_ingested_encodes_to_a_line_the_reader_reads_back() {
        let hostile = [
            sent(&a_record()),
            "not json".to_string(),
            String::new(),
            "{\"sev\":\"error\"".to_string(),
            "\u{1}\u{2}\"\\\n\t".to_string(),
        ];
        for line in hostile {
            let got = ingest(&line, &peer());
            let record = Record {
                ts: None,
                boot: "b".to_string(),
                mono: 1.0,
                sev: got.sev,
                svc: got.svc.clone(),
                corr: "c".to_string(),
                actor: got.actor.clone(),
                msg: got.msg.clone(),
                fields: got.fields.clone(),
            };
            let encoded = record.to_jsonl();
            let read = Record::from_jsonl(&encoded)
                .unwrap_or_else(|e| panic!("{line:?} became a line that will not parse: {e}"));
            assert_eq!(read.actor, "uid:1000");
            assert_eq!(read.field("ingest_uid"), Some(&Value::Int(1000)));
        }
    }

    /// The socket lives beside the notify directory rather than inside it, and
    /// both facts are pinned so a rename cannot quietly put a record socket
    /// where the supervisor scans for service names.
    #[test]
    fn the_ingest_socket_is_not_inside_the_notify_directory() {
        assert!(INGEST_SOCKET_PATH.starts_with("/run/rivet/"));
        assert!(!INGEST_SOCKET_PATH.starts_with("/run/rivet/notify"));
    }
}
