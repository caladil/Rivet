//! **Reading JSON** — the half this workspace did not have.
//!
//! `crate::value::json` has written JSON since `ADR-006` §4 made the structured
//! value primary. Nothing has ever *read* any, because until now nothing
//! received any: an operator types words and the machine answers.
//!
//! `ADR-032` §4 changes that. An MCP adapter is a JSON-RPC server, and JSON-RPC
//! arrives as bytes somebody else wrote.
//!
//! # Why this is written rather than depended on
//!
//! `ADR-009` §3 asks the question and `ADR-014` answers it for the on-device
//! binaries: **`serde_json` is 3 crates and a derive macro**, and this workspace
//! carries no serialisation framework on purpose — every wire format in it is a
//! function that writes and a function that reads, both visible. The two
//! `rivet-config` needs are `toml_edit`'s, which is the one exception `ADR-009`
//! §6 already argued.
//!
//! What is actually needed here is small and closed: JSON-RPC carries objects,
//! strings, integers, booleans and null, nested a few deep. That is a page of
//! code, and the alternative is a dependency in the binary that is `root`'s
//! login shell.
//!
//! # This is a boundary, and it is written like one
//!
//! **The bytes reaching this function were written by something else** — a
//! client over the control plane, or a person on the serial console. So it is
//! strict
//! rather than generous, and it is bounded in three ways a hand-written parser
//! usually is not:
//!
//! * **Depth is capped** ([`MAX_DEPTH`]). Recursive descent over `[[[[[…`
//!   overflows the stack, and a stack overflow is not a parse error — it is an
//!   abort with no message. **In `cmd/rivet-initd` it would be the machine**,
//!   which is why `ADR-035` §8A.1.2 requires exactly this kind of bound.
//! * **Length is capped** by the caller, which reads a line at a time.
//! * **Nothing is guessed.** No trailing commas, no comments, no unquoted keys,
//!   no `NaN` — every one of those is a place two parsers disagree, and
//!   disagreement is what `cmd/rivet`'s invocation layer exists to prevent.
//!
//! # Why not [`crate::value::Value`]
//!
//! Because its objects key on `&'static str` — deliberately, so that a value
//! this system *produces* has field names that were written in the source.
//! Incoming JSON has keys that were written by somebody else at runtime, and
//! forcing them into `&'static str` would mean leaking them. The asymmetry is
//! real and the two types are kept apart because of it: [`Json`] is what
//! arrives, [`crate::value::Value`] is what this machine says.
//!
//! # Why it lives in `rivet-core`
//!
//! **Moved here 2026-08-30, from `cmd/rivet`.** `ADR-035` §8A.1.2 puts a
//! break-glass MCP reader in PID 1, and PID 1 cannot depend on a binary crate.
//! A second copy of a parser for untrusted input is the thing `ADR-011` exists
//! to forbid, and the bound above is the reason this one is the right copy.

use std::fmt;

/// How deeply a document may nest.
///
/// **32 is far more than JSON-RPC needs** — a `tools/call` is three levels — and
/// far less than a stack can take. It exists because recursive descent turns
/// nesting into stack frames, and a hostile `[[[[[[…` is otherwise an abort
/// rather than a refusal.
pub const MAX_DEPTH: usize = 32;

/// A JSON document, as read.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// A number that fits in an `i64`, which is every number this protocol
    /// carries: request ids and array indices.
    Integer(i64),
    /// A number that does not. **Kept rather than refused**: refusing valid
    /// JSON because this program has no use for it would make the parser a
    /// worse citizen than the format, and a caller that wants an integer asks
    /// for one and gets a refusal it can act on.
    Float(f64),
    Str(String),
    Array(Vec<Json>),
    /// Ordered, and not deduplicated. `crate::value::Value`'s reason for the
    /// first; for the second, a document with two `"id"` keys is a document
    /// somebody should be told about rather than one this parser silently
    /// picks a winner from — [`Json::get`] returns the first and
    /// [`Json::duplicate_keys`] is how a caller finds out.
    Object(Vec<(String, Json)>),
}

/// Why a document could not be read.
///
/// Every variant carries **where**, in bytes from the start, because a JSON-RPC
/// client that sent something unreadable needs to find it in a line it
/// generated rather than typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub at: usize,
    pub why: Reason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reason {
    /// Nothing but whitespace.
    Empty,
    /// A byte where a value should start.
    NotAValue { found: char },
    /// More text after the document ended. **Refused rather than ignored**: two
    /// JSON documents on one line is a framing bug in the sender, and reading
    /// the first and dropping the second would hide it.
    TrailingText,
    /// The document ended mid-value.
    Truncated,
    /// Nesting past [`MAX_DEPTH`].
    TooDeep,
    /// A string that never closed, or one containing a raw control character —
    /// which RFC 8259 forbids and which is how a newline gets into a
    /// line-delimited protocol.
    BadString { detail: &'static str },
    /// An escape this parser does not know, or a broken `\uXXXX`.
    BadEscape { detail: &'static str },
    /// A number that is not one.
    BadNumber,
    /// A literal that starts like `true`, `false` or `null` and is not.
    BadLiteral,
    /// A key that is not a string, or a missing `:` or `,`.
    BadObject { detail: &'static str },
    /// A missing `,` or `]`.
    BadArray { detail: &'static str },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match &self.why {
            Reason::Empty => "it is empty".to_string(),
            Reason::NotAValue { found } => {
                format!("a value cannot start with {found:?}")
            }
            Reason::TrailingText => {
                "there is more text after the document ends; two documents in one message is a \
                 framing error in the sender and this will not read the first and drop the \
                 second"
                    .to_string()
            }
            Reason::Truncated => "it ends in the middle of a value".to_string(),
            Reason::TooDeep => format!("it nests deeper than {MAX_DEPTH}"),
            Reason::BadString { detail } => format!("a string {detail}"),
            Reason::BadEscape { detail } => format!("an escape {detail}"),
            Reason::BadNumber => "that is not a number this build reads".to_string(),
            Reason::BadLiteral => "that is not `true`, `false` or `null`".to_string(),
            Reason::BadObject { detail } => format!("an object {detail}"),
            Reason::BadArray { detail } => format!("an array {detail}"),
        };
        write!(f, "at byte {}: {what}", self.at)
    }
}

impl Json {
    /// The value at `key`, if this is an object that has one.
    ///
    /// **The first, when a key repeats.** See [`Json::Object`].
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Self::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Keys this object carries more than once.
    ///
    /// Not an error here, because RFC 8259 does not make it one — but a caller
    /// that cares can refuse, and one that does not is at least not silently
    /// choosing.
    #[must_use]
    pub fn duplicate_keys(&self) -> Vec<&str> {
        let Self::Object(fields) = self else {
            return Vec::new();
        };
        let mut seen: Vec<&str> = Vec::new();
        let mut twice: Vec<&str> = Vec::new();
        for (key, _) in fields {
            if seen.contains(&key.as_str()) {
                if !twice.contains(&key.as_str()) {
                    twice.push(key);
                }
            } else {
                seen.push(key);
            }
        }
        twice
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Integer(n) => Some(*n),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_array(&self) -> Option<&Vec<Json>> {
        match self {
            Self::Array(items) => Some(items),
            _ => None,
        }
    }

    /// What this is, for a refusal that names the shape it got.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "a boolean",
            Self::Integer(_) | Self::Float(_) => "a number",
            Self::Str(_) => "a string",
            Self::Array(_) => "an array",
            Self::Object(_) => "an object",
        }
    }
}

/// Read one JSON document, or say why not.
///
/// **One document, and nothing after it.** A line-delimited protocol depends on
/// that, and a parser that read the first of two and returned would turn a
/// sender's framing bug into this program's silent data loss.
///
/// # Errors
///
/// [`ParseError`], which carries the byte offset.
pub fn parse(text: &str) -> Result<Json, ParseError> {
    let bytes = text.as_bytes();
    let mut p = Parser { text, bytes, at: 0 };
    p.skip_whitespace();
    if p.at >= bytes.len() {
        return Err(p.error(Reason::Empty));
    }
    let value = p.value(0)?;
    p.skip_whitespace();
    if p.at < bytes.len() {
        return Err(p.error(Reason::TrailingText));
    }
    Ok(value)
}

struct Parser<'a> {
    /// The input as text, which a string's ordinary characters are sliced from.
    text: &'a str,
    /// The same input as bytes, which everything else steps through.
    bytes: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    const fn error(&self, why: Reason) -> ParseError {
        ParseError { at: self.at, why }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn skip_whitespace(&mut self) {
        // The four RFC 8259 permits, and no others. A vertical tab is not
        // whitespace in JSON however much it looks like it.
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.error(Reason::TooDeep));
        }
        self.skip_whitespace();
        match self.peek() {
            None => Err(self.error(Reason::Truncated)),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Json::Str),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(other) => Err(self.error(Reason::NotAValue {
                found: char::from(other),
            })),
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, ParseError> {
        if self.bytes[self.at..].starts_with(word.as_bytes()) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(self.error(Reason::BadLiteral))
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, ParseError> {
        self.at += 1; // `{`
        let mut fields = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Json::Object(fields));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(self.error(Reason::BadObject {
                    detail: "key must be a quoted string",
                }));
            }
            let key = self.string()?;
            self.skip_whitespace();
            if self.peek() != Some(b':') {
                return Err(self.error(Reason::BadObject {
                    detail: "key must be followed by `:`",
                }));
            }
            self.at += 1;
            let value = self.value(depth + 1)?;
            fields.push((key, value));
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Object(fields));
                }
                None => return Err(self.error(Reason::Truncated)),
                Some(_) => {
                    return Err(self.error(Reason::BadObject {
                        detail: "member must be followed by `,` or `}`",
                    }));
                }
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, ParseError> {
        self.at += 1; // `[`
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Json::Array(items));
                }
                None => return Err(self.error(Reason::Truncated)),
                Some(_) => {
                    return Err(self.error(Reason::BadArray {
                        detail: "element must be followed by `,` or `]`",
                    }));
                }
            }
        }
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.at += 1; // the opening quote
        let mut out = String::new();
        loop {
            let Some(byte) = self.peek() else {
                return Err(self.error(Reason::BadString {
                    detail: "never closed",
                }));
            };
            match byte {
                b'"' => {
                    self.at += 1;
                    return Ok(out);
                }
                // **RFC 8259 forbids these unescaped**, and the rule earns its
                // keep in a line-delimited protocol: a raw newline inside a
                // string would end the message early for the framing layer and
                // not for the parser, which is two readers disagreeing about
                // where a message ends.
                0x00..=0x1F => {
                    return Err(self.error(Reason::BadString {
                        detail: "carries a raw control character, which JSON forbids",
                    }));
                }
                b'\\' => {
                    self.at += 1;
                    self.escape(&mut out)?;
                }
                _ => {
                    // A run of ordinary characters is copied in one step, as
                    // text. Every byte that ends a run — a quote, a backslash, a
                    // control character — is ASCII, and no byte of a multi-byte
                    // UTF-8 character is, so a run always ends on a character
                    // boundary. The input is a `&str`, so the run is already
                    // valid UTF-8 and nothing is decoded here: the work is
                    // proportional to the string, never to the rest of the input.
                    let start = self.at;
                    while matches!(self.peek(), Some(b) if b != b'"' && b != b'\\' && b >= 0x20) {
                        self.at += 1;
                    }
                    let run = self.text.get(start..self.at).ok_or(ParseError {
                        at: start,
                        why: Reason::BadString {
                            detail: "is not valid UTF-8",
                        },
                    })?;
                    out.push_str(run);
                }
            }
        }
    }

    fn escape(&mut self, out: &mut String) -> Result<(), ParseError> {
        let Some(byte) = self.peek() else {
            return Err(self.error(Reason::Truncated));
        };
        self.at += 1;
        let c = match byte {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{0008}',
            b'f' => '\u{000C}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => return self.unicode_escape(out),
            _ => {
                return Err(self.error(Reason::BadEscape {
                    detail: "is not one JSON defines",
                }));
            }
        };
        out.push(c);
        Ok(())
    }

    /// `\uXXXX`, including the surrogate pair that carries anything above the
    /// basic plane.
    fn unicode_escape(&mut self, out: &mut String) -> Result<(), ParseError> {
        let first = self.hex4()?;
        // A high surrogate is only half a character, and the other half must be
        // the very next escape. Emitting a lone surrogate would produce a
        // `String` that is not valid UTF-8, which Rust will not have anyway.
        if (0xD800..0xDC00).contains(&first) {
            if !self.bytes[self.at..].starts_with(b"\\u") {
                return Err(self.error(Reason::BadEscape {
                    detail: "is half of a surrogate pair and the other half is missing",
                }));
            }
            self.at += 2;
            let second = self.hex4()?;
            if !(0xDC00..0xE000).contains(&second) {
                return Err(self.error(Reason::BadEscape {
                    detail: "is a surrogate pair whose second half is not a low surrogate",
                }));
            }
            let combined = 0x1_0000 + ((first - 0xD800) << 10) + (second - 0xDC00);
            let c = char::from_u32(combined).ok_or_else(|| {
                self.error(Reason::BadEscape {
                    detail: "names no character",
                })
            })?;
            out.push(c);
            return Ok(());
        }
        if (0xDC00..0xE000).contains(&first) {
            return Err(self.error(Reason::BadEscape {
                detail: "is a low surrogate with no high surrogate before it",
            }));
        }
        let c = char::from_u32(first).ok_or_else(|| {
            self.error(Reason::BadEscape {
                detail: "names no character",
            })
        })?;
        out.push(c);
        Ok(())
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let end = self.at + 4;
        if end > self.bytes.len() {
            return Err(self.error(Reason::Truncated));
        }
        let mut value: u32 = 0;
        for byte in &self.bytes[self.at..end] {
            let digit = char::from(*byte).to_digit(16).ok_or_else(|| {
                self.error(Reason::BadEscape {
                    detail: "`\\u` must be followed by four hexadecimal digits",
                })
            })?;
            value = value * 16 + digit;
        }
        self.at = end;
        Ok(value)
    }

    fn number(&mut self) -> Result<Json, ParseError> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        // An integer part is required, and a leading zero may not be followed
        // by another digit — `01` is not a number, which is the rule that stops
        // an octal-looking string being read as decimal.
        match self.peek() {
            Some(b'0') => self.at += 1,
            Some(b'1'..=b'9') => {
                while matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.at += 1;
                }
            }
            _ => return Err(self.error(Reason::BadNumber)),
        }
        let mut floating = false;
        if self.peek() == Some(b'.') {
            floating = true;
            self.at += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error(Reason::BadNumber));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.at += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            floating = true;
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(self.error(Reason::BadNumber));
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.at += 1;
            }
        }
        let text = core::str::from_utf8(&self.bytes[start..self.at])
            .map_err(|_| ParseError { at: start, why: Reason::BadNumber })?;
        if floating {
            text.parse::<f64>()
                .map(Json::Float)
                .map_err(|_| ParseError { at: start, why: Reason::BadNumber })
        } else {
            // An integer too large for `i64` is kept as a float rather than
            // refused, for `Json::Float`'s reason: the format permits it and
            // this program's lack of a use for it is not the sender's problem.
            text.parse::<i64>().map_or_else(
                |_| {
                    text.parse::<f64>()
                        .map(Json::Float)
                        .map_err(|_| ParseError { at: start, why: Reason::BadNumber })
                },
                |n| Ok(Json::Integer(n)),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    fn ok(text: &str) -> Json {
        parse(text).unwrap_or_else(|e| panic!("{text:?} was refused: {e}"))
    }

    fn why(text: &str) -> Reason {
        parse(text).expect_err(text).why
    }

    // ---- the shapes JSON-RPC actually carries ------------------------------

    /// **A real `tools/call`**, which is the document this parser exists for.
    #[test]
    fn a_tools_call_reads_back_field_for_field() {
        let call = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call",
                       "params":{"name":"rivet_get","arguments":{"subject":"status"}}}"#;
        let value = ok(call);
        assert_eq!(value.get("jsonrpc").and_then(Json::as_str), Some("2.0"));
        assert_eq!(value.get("id").and_then(Json::as_integer), Some(3));
        assert_eq!(value.get("method").and_then(Json::as_str), Some("tools/call"));
        let params = value.get("params").expect("params");
        assert_eq!(params.get("name").and_then(Json::as_str), Some("rivet_get"));
        assert_eq!(
            params
                .get("arguments")
                .and_then(|a| a.get("subject"))
                .and_then(Json::as_str),
            Some("status")
        );
    }

    /// An id may be a string as well as a number, and a notification has none.
    #[test]
    fn every_shape_an_id_takes_is_read() {
        assert_eq!(ok(r#"{"id":1}"#).get("id"), Some(&Json::Integer(1)));
        assert_eq!(ok(r#"{"id":"a"}"#).get("id"), Some(&Json::Str("a".into())));
        assert_eq!(ok(r#"{"id":null}"#).get("id"), Some(&Json::Null));
        assert_eq!(ok(r#"{"method":"x"}"#).get("id"), None);
    }

    #[test]
    fn the_scalars_read_as_themselves() {
        assert_eq!(ok("true"), Json::Bool(true));
        assert_eq!(ok("false"), Json::Bool(false));
        assert_eq!(ok("null"), Json::Null);
        assert_eq!(ok("0"), Json::Integer(0));
        assert_eq!(ok("-7"), Json::Integer(-7));
        assert_eq!(ok(r#""x""#), Json::Str("x".into()));
        assert_eq!(ok("[]"), Json::Array(vec![]));
        assert_eq!(ok("{}"), Json::Object(vec![]));
    }

    /// `as_bool`, [`Json::as_str`] and [`Json::as_integer`]'s own sibling: the
    /// value if the shape matches, `None` for every other shape rather than a
    /// coercion — `0` and `"true"` are not booleans.
    #[test]
    fn as_bool_reads_only_a_boolean() {
        assert_eq!(ok("true").as_bool(), Some(true));
        assert_eq!(ok("false").as_bool(), Some(false));
        assert_eq!(ok("0").as_bool(), None);
        assert_eq!(ok(r#""true""#).as_bool(), None);
        assert_eq!(ok("null").as_bool(), None);
    }

    #[test]
    fn whitespace_between_anything_is_ignored() {
        assert_eq!(
            ok(" \t\r\n { \"a\" : [ 1 , 2 ] } \n"),
            Json::Object(vec![(
                "a".into(),
                Json::Array(vec![Json::Integer(1), Json::Integer(2)])
            )])
        );
    }

    // ---- the bounds, which are the point ----------------------------------

    /// **Recursive descent turns nesting into stack frames**, and an abort in
    /// `root`'s login shell is not a parse error — it is silence.
    #[test]
    fn nesting_past_the_cap_is_refused_rather_than_overflowing_the_stack() {
        let deep = "[".repeat(MAX_DEPTH + 5) + &"]".repeat(MAX_DEPTH + 5);
        assert_eq!(why(&deep), Reason::TooDeep);
        // And a document at the limit still reads, so the cap is a cap and not
        // an off-by-one that refuses ordinary work.
        let fine = "[".repeat(8) + &"]".repeat(8);
        assert!(parse(&fine).is_ok());
    }

    /// **Two documents on one line is a framing bug in the sender**, and
    /// reading the first would hide it.
    #[test]
    fn a_second_document_after_the_first_is_refused_not_ignored() {
        assert_eq!(why(r#"{"a":1} {"b":2}"#), Reason::TrailingText);
        assert_eq!(why("1 2"), Reason::TrailingText);
    }

    /// A raw newline inside a string would end the message for the framing
    /// layer and not for the parser — two readers disagreeing about where a
    /// message ends, which is the shape of the bug this whole file avoids.
    #[test]
    fn a_raw_control_character_in_a_string_is_refused() {
        assert!(matches!(why("\"a\nb\""), Reason::BadString { .. }));
        assert!(matches!(why("\"a\tb\""), Reason::BadString { .. }));
        // Escaped, both are ordinary.
        assert_eq!(ok(r#""a\nb""#), Json::Str("a\nb".into()));
        assert_eq!(ok(r#""a\tb""#), Json::Str("a\tb".into()));
    }

    // ---- nothing is guessed ------------------------------------------------

    #[test]
    fn the_generosities_other_parsers_allow_are_all_refused() {
        for (text, what) in [
            (r#"{"a":1,}"#, "a trailing comma in an object"),
            ("[1,]", "a trailing comma in an array"),
            (r#"{a:1}"#, "an unquoted key"),
            ("'a'", "single quotes"),
            ("01", "a leading zero"),
            (".5", "a bare decimal point"),
            ("1.", "a trailing decimal point"),
            ("+1", "a leading plus"),
            ("NaN", "NaN"),
            ("Infinity", "Infinity"),
            ("// a comment\n1", "a comment"),
            ("", "nothing at all"),
        ] {
            assert!(parse(text).is_err(), "{what} was accepted: {text:?}");
        }
    }

    #[test]
    fn a_truncated_document_says_so_rather_than_returning_what_it_had() {
        for text in [r#"{"a":"#, r#"{"a":1"#, "[1", r#""abc"#, "tru"] {
            assert!(parse(text).is_err(), "{text:?}");
        }
    }

    // ---- strings, which is where parsers go wrong -------------------------

    #[test]
    fn every_escape_json_defines_is_read_and_no_others() {
        assert_eq!(
            ok(r#""\" \\ \/ \b \f \n \r \t""#),
            Json::Str("\" \\ / \u{8} \u{c} \n \r \t".into())
        );
        assert!(matches!(why(r#""\q""#), Reason::BadEscape { .. }));
        assert!(matches!(why(r#""\x41""#), Reason::BadEscape { .. }));
    }

    #[test]
    fn a_unicode_escape_becomes_the_character_it_names() {
        assert_eq!(ok(r#""A""#), Json::Str("A".into()));
        assert_eq!(ok(r#""é""#), Json::Str("é".into()));
        // A surrogate pair, which is the only way JSON carries an emoji.
        assert_eq!(ok(r#""😀""#), Json::Str("\u{1f600}".into()));
    }

    /// **A lone surrogate is not a character**, and Rust's `String` cannot hold
    /// one — so this is a refusal rather than a lossy substitution.
    #[test]
    fn a_lone_surrogate_is_refused_rather_than_replaced() {
        assert!(matches!(why(r#""\ud83d""#), Reason::BadEscape { .. }));
        assert!(matches!(why(r#""\ude00""#), Reason::BadEscape { .. }));
        assert!(matches!(why(r#""\ud83dA""#), Reason::BadEscape { .. }));
        assert!(matches!(why(r#""\uZZZZ""#), Reason::BadEscape { .. }));
    }

    #[test]
    fn text_outside_the_basic_plane_survives_unescaped_too() {
        assert_eq!(ok("\"héllo \u{1f600}\""), Json::Str("héllo \u{1f600}".into()));
    }

    /// Escapes directly against multi-byte characters, on both sides of each,
    /// so a run of ordinary characters that stops at an escape stops on a
    /// character and not inside one.
    #[test]
    fn escapes_beside_multi_byte_characters_read_back_exactly() {
        assert_eq!(
            ok("\"é\\\"ü\\\\\u{1f600}\\nb\\u00e9é\""),
            Json::Str("é\"ü\\\u{1f600}\nbéé".into())
        );
    }

    /// **A long string costs time in proportion to its length.** `cargo rivet
    /// run` receives a whole `get log` answer as one string inside one JSON-RPC
    /// line, which on a running board is megabytes, so a reader that does work
    /// proportional to the REST of the input for every character never returns.
    ///
    /// The bound is wide on both sides: reading a mebibyte linearly takes
    /// milliseconds even unoptimized, and rescanning the remainder per character
    /// takes tens of seconds.
    #[test]
    fn a_mebibyte_string_reads_in_time_proportional_to_its_length() {
        let body = "abcdé\u{1f600}".repeat(1 << 17);
        let text = format!("\"{body}\"");
        let started = std::time::Instant::now();
        let read = ok(&text);
        let took = started.elapsed();
        assert_eq!(read, Json::Str(body));
        assert!(
            took < std::time::Duration::from_secs(2),
            "a {} byte string took {took:?} to read",
            text.len()
        );
    }

    // ---- numbers -----------------------------------------------------------

    #[test]
    fn a_number_too_large_for_an_integer_is_kept_rather_than_refused() {
        // The format permits it; this program's lack of a use for it is not the
        // sender's problem.
        assert!(matches!(ok("123456789012345678901234567890"), Json::Float(_)));
        assert!(matches!(ok("1.5"), Json::Float(_)));
        assert!(matches!(ok("1e3"), Json::Float(_)));
        assert_eq!(ok("-9223372036854775808"), Json::Integer(i64::MIN));
    }

    // ---- objects -----------------------------------------------------------

    /// A repeated key is not an error in the format, so it is not one here —
    /// but a caller can find out rather than silently getting one of them.
    #[test]
    fn a_repeated_key_is_findable_rather_than_silently_resolved() {
        let value = ok(r#"{"a":1,"b":2,"a":3}"#);
        assert_eq!(value.get("a").and_then(Json::as_integer), Some(1));
        assert_eq!(value.duplicate_keys(), ["a"]);
        assert!(ok(r#"{"a":1}"#).duplicate_keys().is_empty());
    }

    /// The error says where, because a client that sent this generated it
    /// rather than typed it.
    #[test]
    fn a_refusal_names_the_byte_it_stopped_at() {
        let e = parse(r#"{"a":1,}"#).expect_err("trailing comma");
        assert!(e.to_string().contains("byte 7"), "{e}");
    }
}
