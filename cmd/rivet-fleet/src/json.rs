//! **Reading JSON**, because the monitor pane asks a machine for a table and
//! gets one back as bytes somebody else wrote.
//!
//! # Why this exists at all, and why it is the second copy
//!
//! `cmd/rivet/src/json.rs` is the same reader, written for the MCP adapter on
//! the device, and its header carries the argument for writing rather than
//! depending — `ADR-009` §3, and the fact that the on-device binary is `root`'s
//! login shell. **That crate is binary-only** (`ADR-014` makes `cmd/*`
//! composition roots), so nothing can import it, and `ADR-035` §3 already names
//! the fix and declines to do it:
//!
//! > *"the right end state if the spawn ever becomes the wrong shape … a large
//! > mechanical refactor to perform before the transport it is meant to serve
//! > has ever carried a byte."*
//!
//! So this file is a **known duplicate**, said out loud rather than hidden
//! behind a different shape. `ADR-011` is about two things that are both
//! authoritative for the same question; these two are authoritative for
//! *reading JSON*, which is a format neither of them owns, and the failure mode
//! `ADR-011` guards against — one of them being right — cannot occur. What can
//! occur is the two drifting, so:
//!
//! **When `cmd/rivet`'s `json` module moves into a library, delete this file
//! and depend on it.** That is the whole of the debt.
//!
//! # Why not a hand-written scan, which is what this crate did before
//!
//! `crate::tui::log` lifts three fields out of a journal record with a string
//! search, and `rivet_remote::deploy::parse_measure_blake3` lifts one digest the
//! same way. Both are defensible: one field, one shape, and a loud failure when
//! the shape changes.
//!
//! **A process table is not that.** `get process` answers a list of objects
//! with optional members, and a scanner over it would have to guess where one
//! record ends — which is exactly `AGENTS.md` §5's *refuse rather than guess*,
//! in the place where guessing wrong produces a plausible number instead of an
//! error. A monitor screen that renders a plausible wrong number is worse than
//! one that renders nothing.
//!
//! # Why not `serde_json`
//!
//! This workspace carries no serialisation framework and every wire format in it
//! is a function that writes and a function that reads, both visible
//! (`ADR-009` §4). `rivet-fleet` is a host tool and `ADR-001`'s austerity does
//! not bind it, but the reader is a page of code and the posture is the
//! project's rather than the device's.
//!
//! # This is a boundary and it is written like one
//!
//! The bytes came off a network. So: **depth is capped** ([`MAX_DEPTH`]),
//! because recursive descent over `[[[[[…` is an abort rather than a parse
//! error; **nothing is guessed** — no trailing commas, no comments, no unquoted
//! keys; and a refusal quotes what it saw.

/// How deeply a document may nest.
///
/// **32**, which is far more than any `get` result — the deepest is
/// `filesystems[].capacity-percent`, three levels — and far less than a stack
/// can take.
pub const MAX_DEPTH: usize = 32;

/// A JSON document, as read.
///
/// **Not `rivet_core::value::Value`**, for `cmd/rivet/src/json.rs`'s reason:
/// that type keys objects on `&'static str` deliberately, so that a value this
/// system *produces* has field names that were written in the source. These keys
/// were written by a machine at the other end of a socket.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// A number that fits in an `i64`. Every count, byte total and pid a `get`
    /// subject answers is one — `rivet_core::value::Value` has no float at all,
    /// which is why `memory`'s pressure is in hundredths of a percent.
    Integer(i64),
    /// A number that does not fit. **Kept rather than refused**: refusing valid
    /// JSON because this program has no use for it would make the reader a worse
    /// citizen than the format.
    Float(f64),
    Str(String),
    Array(Vec<Json>),
    /// Ordered and not deduplicated — the first of a repeated key wins in
    /// [`Json::get`], and the duplicate is visible to anyone who walks the list.
    Object(Vec<(String, Json)>),
}

impl Json {
    /// The value of `key`, if this is an object that has one.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Self::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// This value as an integer, if it is one.
    ///
    /// **A float is not silently truncated.** A byte count that arrived as
    /// `1.0e9` is a shape this build does not expect, and rounding it would
    /// hide that.
    #[must_use]
    pub const fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(n) => Some(*n),
            _ => None,
        }
    }

    /// The integer at `key`, if there is one.
    #[must_use]
    pub fn i64_at(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Json::as_i64)
    }

    /// The unsigned integer at `key`. **A negative is `None`**, not a wrap: a
    /// negative byte count is a machine saying something this screen cannot
    /// render, and `AGENTS.md` §5 makes that an absence rather than a guess.
    #[must_use]
    pub fn u64_at(&self, key: &str) -> Option<u64> {
        self.i64_at(key).and_then(|n| u64::try_from(n).ok())
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The string at `key`, if there is one.
    #[must_use]
    pub fn str_at(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Json::as_str)
    }

    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The boolean at `key`, if there is one.
    #[must_use]
    pub fn bool_at(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(Json::as_bool)
    }

    /// The array at `key`. **An empty slice when the key is absent** is
    /// deliberately *not* what this returns — absence and emptiness are
    /// different answers and the caller decides which it can live with.
    #[must_use]
    pub fn array_at(&self, key: &str) -> Option<&[Json]> {
        match self.get(key) {
            Some(Self::Array(items)) => Some(items),
            _ => None,
        }
    }

}

// ~~`Json::scalars` — every `(key, scalar)` this object carries, rendered as
// text — stood here until 2026-08-29, for a result whose shape this build did
// not know.
//
// **Removed with `crate::telemetry::Reading::Verbatim`, which was its only
// caller**, and recorded rather than dropped silently. It existed for `get
// thermal` before that subject existed; the subject landed and has a reader
// written against its real field names. Reinstating it is a dozen lines if a
// pane ever again needs to show a shape nobody has seen — what must not come
// back is a renderer written against invented names.~~

/// Read one JSON document.
///
/// # Errors
///
/// A sentence naming what was wrong and quoting the text around it, because the
/// caller's only other move is to show it to a person.
pub fn parse(text: &str) -> Result<Json, String> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        at: 0,
    };
    parser.skip_space();
    let value = parser.value(0)?;
    parser.skip_space();
    if parser.at < parser.bytes.len() {
        return Err(parser.complain("trailing text after the document"));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn complain(&self, what: &str) -> String {
        let from = self.at.saturating_sub(20);
        let to = (self.at + 20).min(self.bytes.len());
        let around = String::from_utf8_lossy(self.bytes.get(from..to).unwrap_or_default());
        format!("{what}, at byte {}: …{around}…", self.at)
    }

    fn skip_space(&mut self) {
        while matches!(
            self.bytes.get(self.at),
            Some(b' ' | b'\t' | b'\n' | b'\r')
        ) {
            self.at += 1;
        }
    }

    fn eat(&mut self, word: &str) -> bool {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_DEPTH {
            return Err(self.complain(&format!("nested deeper than {MAX_DEPTH}")));
        }
        match self.bytes.get(self.at) {
            None => Err(self.complain("the document ended where a value was expected")),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Json::Str),
            Some(b't') if self.eat("true") => Ok(Json::Bool(true)),
            Some(b'f') if self.eat("false") => Ok(Json::Bool(false)),
            Some(b'n') if self.eat("null") => Ok(Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.complain("not a value this reader accepts")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, String> {
        self.at += 1; // `{`
        let mut fields = Vec::new();
        self.skip_space();
        if self.bytes.get(self.at) == Some(&b'}') {
            self.at += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_space();
            if self.bytes.get(self.at) != Some(&b'"') {
                return Err(self.complain("a key must be a quoted string"));
            }
            let key = self.string()?;
            self.skip_space();
            if self.bytes.get(self.at) != Some(&b':') {
                return Err(self.complain("a key must be followed by `:`"));
            }
            self.at += 1;
            self.skip_space();
            let value = self.value(depth + 1)?;
            fields.push((key, value));
            self.skip_space();
            match self.bytes.get(self.at) {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Object(fields));
                }
                _ => return Err(self.complain("expected `,` or `}` after a member")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, String> {
        self.at += 1; // `[`
        let mut items = Vec::new();
        self.skip_space();
        if self.bytes.get(self.at) == Some(&b']') {
            self.at += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_space();
            items.push(self.value(depth + 1)?);
            self.skip_space();
            match self.bytes.get(self.at) {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Json::Array(items));
                }
                _ => return Err(self.complain("expected `,` or `]` after an element")),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.at += 1; // the opening quote
        let mut out = String::new();
        loop {
            let Some(byte) = self.bytes.get(self.at).copied() else {
                return Err(self.complain("the document ended inside a string"));
            };
            self.at += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let Some(escape) = self.bytes.get(self.at).copied() else {
                        return Err(self.complain("the document ended inside an escape"));
                    };
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{08}'),
                        b'f' => out.push('\u{0c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.escape_u()?),
                        _ => return Err(self.complain("not an escape JSON defines")),
                    }
                }
                // A raw control character is not legal JSON. Refused rather
                // than passed through, because a terminal renders some of them
                // as cursor movement and this text goes straight to a screen.
                0x00..=0x1f => return Err(self.complain("a raw control character in a string")),
                other => {
                    // Multi-byte UTF-8 arrives a byte at a time; collect the
                    // continuation bytes and decode once.
                    let extra = match other {
                        0xc0..=0xdf => 1,
                        0xe0..=0xef => 2,
                        0xf0..=0xf7 => 3,
                        _ => 0,
                    };
                    let from = self.at - 1;
                    self.at += extra;
                    let slice = self
                        .bytes
                        .get(from..self.at)
                        .ok_or_else(|| self.complain("the document ended mid-character"))?;
                    match std::str::from_utf8(slice) {
                        Ok(text) => out.push_str(text),
                        Err(_) => return Err(self.complain("not valid UTF-8")),
                    }
                }
            }
        }
    }

    /// One `\uXXXX`, and the surrogate pair it may be half of.
    fn escape_u(&mut self) -> Result<char, String> {
        let first = self.hex4()?;
        // A high surrogate is only half a character. JSON writes an astral
        // codepoint as a pair, and decoding the halves separately would produce
        // two replacement characters rather than the emoji somebody sent.
        if (0xd800..0xdc00).contains(&first) {
            if !self.eat("\\u") {
                return Err(self.complain("a high surrogate with no pair after it"));
            }
            let second = self.hex4()?;
            if !(0xdc00..0xe000).contains(&second) {
                return Err(self.complain("a high surrogate followed by something else"));
            }
            let combined = 0x1_0000 + ((first - 0xd800) << 10) + (second - 0xdc00);
            return char::from_u32(combined)
                .ok_or_else(|| self.complain("a surrogate pair naming no character"));
        }
        char::from_u32(first).ok_or_else(|| self.complain("an escape naming no character"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .bytes
            .get(self.at..self.at + 4)
            .ok_or_else(|| self.complain("a `\\u` escape needs four hex digits"))?;
        let text = std::str::from_utf8(digits)
            .map_err(|_| self.complain("a `\\u` escape with non-ASCII digits"))?;
        let value = u32::from_str_radix(text, 16)
            .map_err(|_| self.complain("a `\\u` escape whose digits are not hex"))?;
        self.at += 4;
        Ok(value)
    }

    fn number(&mut self) -> Result<Json, String> {
        let from = self.at;
        if self.bytes.get(self.at) == Some(&b'-') {
            self.at += 1;
        }
        let mut floating = false;
        while let Some(byte) = self.bytes.get(self.at) {
            match byte {
                b'0'..=b'9' => self.at += 1,
                b'.' | b'e' | b'E' | b'+' | b'-' => {
                    floating = true;
                    self.at += 1;
                }
                _ => break,
            }
        }
        let text = std::str::from_utf8(self.bytes.get(from..self.at).unwrap_or_default())
            .map_err(|_| self.complain("a number that is not ASCII"))?;
        if floating {
            return text
                .parse::<f64>()
                .map(Json::Float)
                .map_err(|_| self.complain("not a number"));
        }
        // An integer too large for `i64` is kept as a float rather than
        // refused — see `Json::Float`.
        text.parse::<i64>().map_or_else(
            |_| {
                text.parse::<f64>()
                    .map(Json::Float)
                    .map_err(|_| self.complain("not a number"))
            },
            |n| Ok(Json::Integer(n)),
        )
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn an_object_reads_back_field_by_field() {
        let value = parse(
            r#"{"subject":"memory","total_bytes":971505664,"available_bytes":612345678,
                "pressure":{"some_avg10":42,"full_avg60":0}}"#,
        )
        .expect("a document");
        assert_eq!(value.str_at("subject"), Some("memory"));
        assert_eq!(value.u64_at("total_bytes"), Some(971_505_664));
        assert_eq!(
            value.get("pressure").and_then(|p| p.i64_at("some_avg10")),
            Some(42)
        );
    }

    /// **An absent field is absent, not zero.** The whole of `AGENTS.md` §5 for
    /// this reader: a kernel with no PSI omits `pressure`, and a caller that got
    /// `Some(0)` back would render *nothing is stalling*.
    #[test]
    fn a_missing_field_is_none_and_never_a_zero() {
        let value = parse(r#"{"subject":"memory"}"#).expect("a document");
        assert_eq!(value.u64_at("total_bytes"), None);
        assert_eq!(value.get("pressure"), None);
        assert_eq!(value.array_at("processes"), None);
    }

    /// A process table, which is the shape a hand-written scanner could not
    /// read: a list of objects with an optional member.
    #[test]
    fn a_list_of_objects_with_optional_members_reads_correctly() {
        let value = parse(
            r#"{"subject":"process","processes":[
                 {"pid":1,"parent_pid":0,"name":"rivet-initd","state":"S","threads":1,
                  "kernel":false,"resident_bytes":2162688},
                 {"pid":2,"parent_pid":0,"name":"kthreadd","state":"S","threads":1,
                  "kernel":true}],
               "vanished":0,"userland":1}"#,
        )
        .expect("a document");
        let rows = value.array_at("processes").expect("a list");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].str_at("name"), Some("rivet-initd"));
        assert_eq!(rows[0].u64_at("resident_bytes"), Some(2_162_688));
        // The kernel thread has no resident size at all, and that is the point.
        assert_eq!(rows[1].bool_at("kernel"), Some(true));
        assert_eq!(rows[1].u64_at("resident_bytes"), None);
    }

    /// **Hyphenated keys**, which `get storage` uses throughout.
    #[test]
    fn a_hyphenated_key_is_a_key_like_any_other() {
        let value =
            parse(r#"{"filesystems":[{"mount-point":"/state","capacity-percent":31}]}"#)
                .expect("a document");
        let rows = value.array_at("filesystems").expect("a list");
        assert_eq!(rows[0].str_at("mount-point"), Some("/state"));
        assert_eq!(rows[0].i64_at("capacity-percent"), Some(31));
    }

    /// **Nothing is guessed.** Every one of these is a place where two parsers
    /// disagree, and a monitor screen that rendered a plausible wrong number
    /// would be worse than one that rendered nothing.
    #[test]
    fn the_shapes_a_generous_parser_would_accept_are_refused() {
        for bad in [
            r#"{"a":1,}"#,
            r"{a:1}",
            r"{'a':1}",
            r"[1,2,]",
            r#"{"a":1} trailing"#,
            r"{",
            r#"{"a":}"#,
            r#"{"a" 1}"#,
            "",
            "// a comment",
            r#"{"a":NaN}"#,
        ] {
            assert!(parse(bad).is_err(), "{bad:?} was accepted");
        }
    }

    /// **Depth is capped**, because recursive descent over `[[[[[…` is an abort
    /// rather than a parse error, and these bytes came off a network.
    #[test]
    fn nesting_past_the_cap_is_refused_rather_than_overflowing_the_stack() {
        let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
        let refusal = parse(&deep).expect_err("200 deep must refuse");
        assert!(refusal.contains("nested deeper"), "{refusal}");
        // And a document at a sane depth still reads.
        let fine = format!("{}1{}", "[".repeat(8), "]".repeat(8));
        assert!(parse(&fine).is_ok());
    }

    /// Escapes, including the surrogate pair a naive reader turns into two
    /// replacement characters.
    #[test]
    fn escapes_are_decoded_including_a_surrogate_pair() {
        let value = parse(r#"{"a":"line\nbreak \"quoted\" µ 😀"}"#)
            .expect("a document");
        let text = value.str_at("a").expect("a string");
        assert!(text.contains('\n'));
        assert!(text.contains('"'));
        assert!(text.contains('\u{b5}'), "{text:?}");
        assert!(text.contains('\u{1f600}'), "{text:?}");
    }

    /// A raw control character goes straight to a terminal if it is passed
    /// through, and some of them move the cursor. It is not legal JSON either.
    #[test]
    fn a_raw_control_character_in_a_string_is_refused() {
        assert!(parse("{\"a\":\"be\u{1b}[2Jfore\"}").is_err());
    }

    /// UTF-8 that arrived as bytes comes back as the character it is.
    #[test]
    fn multibyte_utf8_survives_the_read() {
        let value = parse(r#"{"a":"µ§ — ✓"}"#).expect("a document");
        assert_eq!(value.str_at("a"), Some("µ§ — ✓"));
    }

    /// **A float is not an integer.** A byte count that arrived as `1.0e9` is a
    /// shape this build does not expect; rounding it would hide that.
    #[test]
    fn a_float_is_not_offered_as_an_integer() {
        let value = parse(r#"{"a":1.5,"b":2}"#).expect("a document");
        assert_eq!(value.i64_at("a"), None);
        assert_eq!(value.i64_at("b"), Some(2));
    }

    /// **A negative is not a byte count.** `u64_at` refuses rather than
    /// wrapping, which would turn `-1` into eighteen exabytes on screen.
    #[test]
    fn a_negative_never_becomes_an_enormous_unsigned_number() {
        let value = parse(r#"{"a":-1}"#).expect("a document");
        assert_eq!(value.i64_at("a"), Some(-1));
        assert_eq!(value.u64_at("a"), None);
    }

    /// The refusal quotes what it saw, because the caller's only other move is
    /// to put it on a screen.
    #[test]
    fn a_refusal_says_where_and_shows_the_text() {
        let refusal = parse(r#"{"subject":"memory","total":}"#).expect_err("must refuse");
        assert!(refusal.contains("byte"), "{refusal}");
        assert!(refusal.contains("total"), "{refusal}");
    }
}
