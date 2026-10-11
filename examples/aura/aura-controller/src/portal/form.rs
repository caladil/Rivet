//! **`application/x-www-form-urlencoded`, and the query string too.**
//!
//! Go's `r.ParseForm` in about sixty lines, because that is all this portal
//! asks of it: no multipart, no file upload, no JSON. Every form on the page is
//! a `POST` of flat key/value pairs, one of which — `days` — repeats.
//!
//! # Repeated keys are kept, and that is not a detail
//!
//! `days=Mon&days=Fri` is how a set of checkboxes arrives, and a parser that
//! kept only the last value would silently turn *Monday and Friday* into
//! *Friday*. [`Form::all`] is what the schedule editor reads.
//!
//! # `+` is a space, and only here
//!
//! It is the one difference between form encoding and ordinary percent
//! encoding, and it is why this cannot be a generic URL decoder shared with
//! anything that parses a path.

/// The decoded pairs, in the order they arrived.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Form {
    pairs: Vec<(String, String)>,
}

impl Form {
    /// Parse a form body or a query string.
    ///
    /// **Never fails.** A malformed percent escape is left as the literal
    /// characters it was written with, and a pair with no `=` is a key with an
    /// empty value — which is what a browser sends for an empty text input and
    /// is not an error a person could act on.
    pub fn parse(text: &str) -> Self {
        let mut pairs = Vec::new();
        for field in text.split('&') {
            if field.is_empty() {
                continue;
            }
            let (key, value) = field.split_once('=').unwrap_or((field, ""));
            pairs.push((decode(key), decode(value)));
        }
        Self { pairs }
    }

    /// The first value for `key`, or `""`.
    ///
    /// Empty rather than `Option`, matching Go's `FormValue`, because every
    /// caller here treats *absent* and *empty* the same: an unchecked checkbox
    /// is absent and an emptied text field is empty, and both mean *not set*.
    pub fn value(&self, key: &str) -> &str {
        self.pairs
            .iter()
            .find(|(k, _)| k == key)
            .map_or("", |(_, v)| v.as_str())
    }

    /// Every value for `key`, in order. This is what `days` needs.
    pub fn all(&self, key: &str) -> Vec<&str> {
        self.pairs
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

/// Percent-decode, treating `+` as a space.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                match u8::from_str_radix(&text[index + 1..index + 3], 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    // Not an escape after all. Kept as written rather than
                    // dropped: a password may contain a literal `%`, and a
                    // decoder that ate it would shorten it in exactly the way
                    // `crate::credential` exists to have stopped.
                    Err(_) => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    // Lossy, because the bytes came from whoever connected and a refusal here
    // would be a form that cannot be submitted rather than a field that is
    // wrong. A browser sends UTF-8.
    String::from_utf8_lossy(&out).into_owned()
}

/// The query string out of a request target, or `""`.
pub fn query_of(target: &str) -> &str {
    target.split_once('?').map_or("", |(_, query)| query)
}

/// The path out of a request target, without its query string.
pub fn path_of(target: &str) -> &str {
    target.split_once('?').map_or(target, |(path, _)| path)
}

#[cfg(test)]
mod tests;
