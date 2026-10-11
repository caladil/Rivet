//! A JSON value, parsed just far enough to read what `ConvertTo-Json` prints.
//!
//! # Why this exists rather than a dependency
//!
//! `serde_json` is out for the reason `cmd/rivet/Cargo.toml` records for the
//! same shape of problem: it is a generic data model for a project that reads
//! exactly one JSON producer, on exactly one platform, for exactly one
//! purpose — [`crate::enumerate`]'s disk list. A generic parser earns its
//! dependencies by being reused; this one is called from a single place and
//! is a **closed value enum with no derive**, the same posture
//! `cmd/rivet/src/value.rs` records for the JSON this project writes.
//!
//! **It is not a general-purpose JSON library and does not try to be.** It
//! implements the whole grammar — object, array, string, number, `true`,
//! `false`, `null` — because `ConvertTo-Json`'s output uses all of it (a
//! `null` `MediaType` on a virtual disk is a real thing this has to read),
//! but it is tested against what PowerShell actually prints, not against the
//! JSON specification's edge cases. `AGENTS.md` §4: label fixtures by
//! provenance — the object/array/nesting fixture in the tests below is byte-
//! for-byte what `Get-CimInstance Win32_DiskDrive` produced on this host,
//! captured before this module existed to read it.

/// A parsed JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// Every JSON number, as `f64`. Safe for what this reads: a disk size in
    /// bytes is nowhere near `f64`'s 2^53 exact-integer ceiling — that is
    /// nine **petabytes**, and the devices this enumerates are removable
    /// media.
    Number(f64),
    String(String),
    Array(Vec<Json>),
    /// Insertion order, not a map. There are at most a handful of fields on
    /// any object this reads, so a linear scan in [`Json::get`] costs
    /// nothing and owes no dependency a `HashMap` would not either.
    Object(Vec<(String, Json)>),
}

impl Json {
    /// The value of a field on an object, or `None` for anything else —
    /// including a field that is not there, and including calling this on a
    /// value that is not an object at all.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Self::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Self::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    /// A non-negative, finite number, as `u64` — what every field this reads
    /// actually is (`Size`, a byte count). `None` for a negative number, one
    /// too large to fit, `NaN`, or anything that is not a number at all.
    #[must_use]
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Number(n) if n.is_finite() && *n >= 0.0 && *n <= u64::MAX as f64 => {
                Some(*n as u64)
            }
            _ => None,
        }
    }

    /// A JSON `true` or `false` — `diskutil`'s removability flags on macOS.
    #[must_use]
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

/// Parse a complete JSON document.
///
/// # Errors
///
/// Anything that is not well-formed JSON, or that has trailing content after
/// one complete value — naming the character position, because the only
/// reader of that message is whoever is debugging what PowerShell printed.
pub fn parse(text: &str) -> Result<Json, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0usize;
    let value = value(&chars, &mut at)?;
    skip_ws(&chars, &mut at);
    if at != chars.len() {
        return Err(format!(
            "{} trailing character(s) after the JSON value, starting at position {at}",
            chars.len() - at
        ));
    }
    Ok(value)
}

fn skip_ws(chars: &[char], at: &mut usize) {
    while *at < chars.len() && chars[*at].is_whitespace() {
        *at += 1;
    }
}

fn peek(chars: &[char], at: usize) -> Option<char> {
    chars.get(at).copied()
}

fn expect(chars: &[char], at: &mut usize, c: char) -> Result<(), String> {
    if peek(chars, *at) == Some(c) {
        *at += 1;
        Ok(())
    } else {
        Err(format!(
            "expected {c:?} at position {at}, found {:?}",
            peek(chars, *at)
        ))
    }
}

fn value(chars: &[char], at: &mut usize) -> Result<Json, String> {
    skip_ws(chars, at);
    match peek(chars, *at) {
        Some('{') => object(chars, at),
        Some('[') => array(chars, at),
        Some('"') => string(chars, at).map(Json::String),
        Some('t') => literal(chars, at, "true", Json::Bool(true)),
        Some('f') => literal(chars, at, "false", Json::Bool(false)),
        Some('n') => literal(chars, at, "null", Json::Null),
        Some(c) if c == '-' || c.is_ascii_digit() => number(chars, at),
        other => Err(format!("expected a JSON value at position {at}, found {other:?}")),
    }
}

fn literal(chars: &[char], at: &mut usize, word: &str, value: Json) -> Result<Json, String> {
    for expected in word.chars() {
        expect(chars, at, expected)?;
    }
    Ok(value)
}

fn object(chars: &[char], at: &mut usize) -> Result<Json, String> {
    expect(chars, at, '{')?;
    let mut fields = Vec::new();
    skip_ws(chars, at);
    if peek(chars, *at) == Some('}') {
        *at += 1;
        return Ok(Json::Object(fields));
    }
    loop {
        skip_ws(chars, at);
        let key = string(chars, at)?;
        skip_ws(chars, at);
        expect(chars, at, ':')?;
        let val = value(chars, at)?;
        fields.push((key, val));
        skip_ws(chars, at);
        match peek(chars, *at) {
            Some(',') => {
                *at += 1;
            }
            Some('}') => {
                *at += 1;
                break;
            }
            other => return Err(format!("expected ',' or '}}' at position {at}, found {other:?}")),
        }
    }
    Ok(Json::Object(fields))
}

fn array(chars: &[char], at: &mut usize) -> Result<Json, String> {
    expect(chars, at, '[')?;
    let mut items = Vec::new();
    skip_ws(chars, at);
    if peek(chars, *at) == Some(']') {
        *at += 1;
        return Ok(Json::Array(items));
    }
    loop {
        let item = value(chars, at)?;
        items.push(item);
        skip_ws(chars, at);
        match peek(chars, *at) {
            Some(',') => {
                *at += 1;
            }
            Some(']') => {
                *at += 1;
                break;
            }
            other => return Err(format!("expected ',' or ']' at position {at}, found {other:?}")),
        }
    }
    Ok(Json::Array(items))
}

fn string(chars: &[char], at: &mut usize) -> Result<String, String> {
    expect(chars, at, '"')?;
    let mut out = String::new();
    loop {
        match peek(chars, *at) {
            None => return Err(format!("an unterminated string starting before position {at}")),
            Some('"') => {
                *at += 1;
                break;
            }
            Some('\\') => {
                *at += 1;
                match peek(chars, *at) {
                    Some('"') => {
                        out.push('"');
                        *at += 1;
                    }
                    Some('\\') => {
                        out.push('\\');
                        *at += 1;
                    }
                    Some('/') => {
                        out.push('/');
                        *at += 1;
                    }
                    Some('b') => {
                        out.push('\u{8}');
                        *at += 1;
                    }
                    Some('f') => {
                        out.push('\u{c}');
                        *at += 1;
                    }
                    Some('n') => {
                        out.push('\n');
                        *at += 1;
                    }
                    Some('r') => {
                        out.push('\r');
                        *at += 1;
                    }
                    Some('t') => {
                        out.push('\t');
                        *at += 1;
                    }
                    Some('u') => {
                        *at += 1;
                        let high = unicode_escape(chars, at)?;
                        // A surrogate pair: `\uD8xx\uDCxx`-shaped, needed for
                        // anything outside the Basic Multilingual Plane. A
                        // disk model string is not going to carry one, but a
                        // parser that silently mangled it would be worse than
                        // one that never had to.
                        if (0xD800..=0xDBFF).contains(&high) {
                            expect(chars, at, '\\')?;
                            expect(chars, at, 'u')?;
                            let low = unicode_escape(chars, at)?;
                            if !(0xDC00..=0xDFFF).contains(&low) {
                                return Err(format!(
                                    "a high surrogate \\u{high:04x} was not followed by a low \
                                     surrogate, at position {at}"
                                ));
                            }
                            let code = 0x10000
                                + (u32::from(high) - 0xD800) * 0x400
                                + (u32::from(low) - 0xDC00);
                            out.push(char::from_u32(code).ok_or_else(|| {
                                format!("\\u{high:04x}\\u{low:04x} is not a valid character")
                            })?);
                        } else {
                            out.push(char::from_u32(u32::from(high)).ok_or_else(|| {
                                format!("\\u{high:04x} is not a valid character on its own")
                            })?);
                        }
                    }
                    other => {
                        return Err(format!(
                            "{other:?} is not a valid escape at position {at}"
                        ));
                    }
                }
            }
            Some(c) => {
                out.push(c);
                *at += 1;
            }
        }
    }
    Ok(out)
}

/// Four hex digits after a `\u` already consumed.
fn unicode_escape(chars: &[char], at: &mut usize) -> Result<u16, String> {
    let mut digits = String::with_capacity(4);
    for _ in 0..4 {
        let c = peek(chars, *at)
            .ok_or_else(|| format!("an unterminated \\u escape at position {at}"))?;
        digits.push(c);
        *at += 1;
    }
    u16::from_str_radix(&digits, 16)
        .map_err(|_| format!("{digits:?} is not four hex digits, at position {at}"))
}

fn number(chars: &[char], at: &mut usize) -> Result<Json, String> {
    let start = *at;
    if peek(chars, *at) == Some('-') {
        *at += 1;
    }
    while peek(chars, *at).is_some_and(|c| c.is_ascii_digit()) {
        *at += 1;
    }
    if peek(chars, *at) == Some('.') {
        *at += 1;
        while peek(chars, *at).is_some_and(|c| c.is_ascii_digit()) {
            *at += 1;
        }
    }
    if matches!(peek(chars, *at), Some('e' | 'E')) {
        *at += 1;
        if matches!(peek(chars, *at), Some('+' | '-')) {
            *at += 1;
        }
        while peek(chars, *at).is_some_and(|c| c.is_ascii_digit()) {
            *at += 1;
        }
    }
    let text: String = chars[start..*at].iter().collect();
    text.parse::<f64>()
        .map(Json::Number)
        .map_err(|e| format!("{text:?} is not a number: {e}"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn an_empty_object_parses() {
        assert_eq!(parse("{}").expect("parses"), Json::Object(Vec::new()));
    }

    #[test]
    fn an_empty_array_parses() {
        assert_eq!(parse("[]").expect("parses"), Json::Array(Vec::new()));
    }

    /// Whitespace exactly as `ConvertTo-Json` produces it — aligned with
    /// spaces, not tabs, and with a blank line inside an empty array. Both
    /// were seen in real output before this test was written.
    #[test]
    fn convert_to_jsons_own_whitespace_style_parses() {
        let text = "{\n    \"disks\":  [\n\n              ]\n}";
        let parsed = parse(text).expect("parses");
        assert_eq!(parsed.get("disks").and_then(Json::as_array), Some([].as_slice()));
    }

    /// **The real fixture.** Captured 2026-09-09 from `Get-CimInstance
    /// Win32_DiskDrive` on the development host, through the exact script
    /// `crate::enumerate` runs — two fixed NVMe disks, not removable, each
    /// with one drive letter. `AGENTS.md` §4: label fixtures by provenance.
    #[test]
    fn a_real_cim_shaped_payload_parses() {
        let text = r#"{
    "disks":  [
                  {
                      "DeviceID":  "\\\\.\\PHYSICALDRIVE1",
                      "Model":  "NVMe MTFDKBA1T0QFM-1BD1AABGB",
                      "Size":  1024203640320,
                      "InterfaceType":  "SCSI",
                      "MediaType":  "Fixed hard disk media",
                      "Letters":  [
                                      "D:"
                                  ]
                  },
                  {
                      "DeviceID":  "\\\\.\\PHYSICALDRIVE0",
                      "Model":  "NVMe MTFDKBA1T0QFM-1BD1AABGB",
                      "Size":  1024203640320,
                      "InterfaceType":  "SCSI",
                      "MediaType":  "Fixed hard disk media",
                      "Letters":  [
                                      "C:"
                                  ]
                  }
              ]
}"#;
        let parsed = parse(text).expect("real PowerShell output must parse");
        let disks = parsed.get("disks").and_then(Json::as_array).expect("a disks array");
        assert_eq!(disks.len(), 2);
        assert_eq!(
            disks[0].get("DeviceID").and_then(Json::as_str),
            Some(r"\\.\PHYSICALDRIVE1"),
            "the doubled backslashes in the JSON escaping must decode to one each"
        );
        assert_eq!(disks[0].get("Size").and_then(Json::as_u64), Some(1_024_203_640_320));
        assert_eq!(
            disks[0].get("Letters").and_then(Json::as_array).and_then(|a| a[0].as_str()),
            Some("D:")
        );
    }

    /// A `null` field — a virtual or exotic disk really does answer this way,
    /// and `crate::enumerate` must read past it rather than fail the whole
    /// enumeration for one disk's missing `MediaType`.
    #[test]
    fn a_null_field_parses_as_null_and_not_as_absent() {
        let parsed = parse(r#"{"MediaType": null, "Model": "x"}"#).expect("parses");
        assert_eq!(parsed.get("MediaType"), Some(&Json::Null));
        assert_eq!(parsed.get("MediaType").and_then(Json::as_str), None);
    }

    #[test]
    fn booleans_and_nesting_parse() {
        let parsed = parse(r#"{"a": [true, false, {"b": 1.5e2}]}"#).expect("parses");
        let a = parsed.get("a").and_then(Json::as_array).expect("an array");
        assert_eq!(a[0], Json::Bool(true));
        assert_eq!(a[1], Json::Bool(false));
        assert_eq!(a[2].get("b"), Some(&Json::Number(150.0)), "1.5e2 is exponent notation for 150");
    }

    #[test]
    fn a_negative_number_parses() {
        assert_eq!(parse("-42").expect("parses"), Json::Number(-42.0));
    }

    #[test]
    fn as_u64_reads_a_plain_integer() {
        assert_eq!(parse("1024203640320").expect("parses").as_u64(), Some(1_024_203_640_320));
    }

    #[test]
    fn escaped_characters_decode() {
        let parsed = parse(r#""a\"b\\c\nd\te""#).expect("parses");
        assert_eq!(parsed, Json::String("a\"b\\c\nd\te".to_owned()));
    }

    #[test]
    fn a_unicode_escape_decodes() {
        let parsed = parse(r#""\u00e9""#).expect("parses");
        assert_eq!(parsed, Json::String("é".to_owned()));
    }

    #[test]
    fn malformed_json_is_refused_by_position_not_silently_truncated() {
        let why = parse("{\"a\": 1,}").expect_err("a trailing comma before '}' is not valid JSON");
        assert!(why.contains("position"), "{why}");

        let why = parse("{\"a\": ").expect_err("an unterminated object is not valid JSON");
        assert!(why.contains("position"), "{why}");
    }

    #[test]
    fn trailing_content_after_a_complete_value_is_refused() {
        let why = parse("{} {}").expect_err("two documents is not one document");
        assert!(why.contains("trailing"), "{why}");
    }

    #[test]
    fn get_on_a_non_object_is_none_rather_than_a_panic() {
        assert_eq!(Json::Array(Vec::new()).get("x"), None);
        assert_eq!(Json::Null.get("x"), None);
    }

    #[test]
    fn as_u64_refuses_a_negative_or_non_finite_number() {
        assert_eq!(Json::Number(-1.0).as_u64(), None);
        assert_eq!(Json::Number(f64::NAN).as_u64(), None);
        assert_eq!(Json::Number(3.0).as_u64(), Some(3));
    }
}
