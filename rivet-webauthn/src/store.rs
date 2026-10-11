//! **The enrolled credentials, as text** — one line per passkey.
//!
//! ```text
//! # rivet-proxyd passkeys: passkey <id> <algorithm> <public key> <counter> <enrolled> [<rp id>]
//! passkey 8R85HbTJsv3g6nAYnLo_tj9Xm6YSKzOtlP8-wzAIS-Q es256 BK_voW-X… 0 1790000000 dev-01.home-fleet.home.arpa
//! passkey fAbOmZ85O5HvF6gegifR4w es256 BGNEoOCg… 0 1790000000
//! ```
//!
//! `<id>` and `<public key>` are base64url without padding, `<algorithm>` is
//! [`crate::Algorithm::word`], `<counter>` the last signature counter seen and
//! `<enrolled>` seconds since the Unix epoch by the board's clock. Nothing in it
//! is secret: a public key verifies and cannot sign.
//!
//! **`<rp id>` is the name the passkey was enrolled at**, because a passkey
//! belongs to one relying party and a machine answers at every name its
//! certificate carries (`ADR-063` §4). A line without it was written before
//! the name was recorded, when the machine had one relying party; the reader
//! says so with [`Stored::rp_id`] `None`, the caller supplies the name it meant,
//! and the writer renders it back without one — so such a line is never
//! rewritten into a claim nobody made.
//!
//! **Both halves are here** and the round trip is tested, so the writer and the
//! reader cannot disagree. **A line that does not read refuses the whole
//! file**: a store with a line skipped would be a store that silently lost a
//! credential, or kept one an operator thought was removed.

use crate::{Algorithm, Credential, b64url, from_b64url};

/// The first line every rendering carries.
pub const HEADER: &str =
    "# rivet-proxyd passkeys: passkey <id> <algorithm> <public key> <counter> <enrolled> [<rp id>]";

/// The longest RP ID a line may carry — a DNS name's limit.
const MAX_RP_ID: usize = 253;

/// One stored passkey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stored {
    pub credential: Credential,
    /// Seconds since the Unix epoch when it was enrolled, by the board's clock.
    pub enrolled: u64,
    /// The RP ID it was enrolled under. `None` for a line written before the
    /// name was recorded: see the module's own documentation.
    pub rp_id: Option<String>,
}

impl Stored {
    /// **The name this passkey belongs to**: its own, or `legacy` — the one
    /// relying party the machine had when a line without a name was written.
    #[must_use]
    pub fn enrolled_at<'a>(&'a self, legacy: &'a str) -> &'a str {
        self.rp_id.as_deref().unwrap_or(legacy)
    }
}

/// Whether `text` may be stored as an RP ID: a lowercase DNS name — letters,
/// digits, `-` and `.` — and nothing that would split the line.
#[must_use]
pub fn is_rp_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_RP_ID
        && text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

/// Why a store could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreError {
    /// 1-based.
    pub line: usize,
    pub why: &'static str,
}

impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "line {} of the passkey store: {}", self.line, self.why)
    }
}

impl std::error::Error for StoreError {}

/// Render every passkey, header first.
#[must_use]
pub fn render(stored: &[Stored]) -> String {
    let mut out = String::from(HEADER);
    out.push('\n');
    for one in stored {
        out.push_str(&format!(
            "passkey {} {} {} {} {}",
            b64url(&one.credential.id),
            one.credential.algorithm.word(),
            b64url(&one.credential.public_key),
            one.credential.sign_count,
            one.enrolled,
        ));
        if let Some(rp_id) = &one.rp_id {
            out.push(' ');
            out.push_str(rp_id);
        }
        out.push('\n');
    }
    out
}

/// [`render`]'s inverse.
///
/// # Errors
///
/// [`StoreError`] for the first line that does not read, for an RP ID that is
/// not a lowercase DNS name, and for a credential ID that appears twice.
pub fn parse(text: &str) -> Result<Vec<Stored>, StoreError> {
    let mut out: Vec<Stored> = Vec::new();
    for (at, line) in text.lines().enumerate() {
        let line_no = at + 1;
        let refuse = |why| StoreError { line: line_no, why };
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = trimmed.split(' ').collect();
        let (kind, id, algorithm, key, counter, enrolled, rp_id) = match words.as_slice() {
            [kind, id, algorithm, key, counter, enrolled] => {
                (kind, id, algorithm, key, counter, enrolled, None)
            }
            [kind, id, algorithm, key, counter, enrolled, rp_id] => {
                (kind, id, algorithm, key, counter, enrolled, Some(*rp_id))
            }
            _ => return Err(refuse("not six or seven words separated by single spaces")),
        };
        if rp_id.is_some_and(|rp_id| !is_rp_id(rp_id)) {
            return Err(refuse("the RP ID is not a lowercase DNS name"));
        }
        if *kind != "passkey" {
            return Err(refuse("does not begin with `passkey`"));
        }
        let id = from_b64url(id).ok_or_else(|| refuse("the ID is not base64url"))?;
        if id.is_empty() || id.len() > crate::MAX_CREDENTIAL_ID {
            return Err(refuse("the ID is empty or longer than 1023 bytes"));
        }
        let algorithm = Algorithm::from_word(algorithm).ok_or_else(|| refuse("an unknown algorithm"))?;
        let public_key = from_b64url(key).ok_or_else(|| refuse("the public key is not base64url"))?;
        let expected = match algorithm {
            Algorithm::Es256 => 65,
            Algorithm::Ed25519 => 32,
        };
        if public_key.len() != expected {
            return Err(refuse("the public key is the wrong length for its algorithm"));
        }
        let sign_count = counter.parse().map_err(|_| refuse("the counter is not a number"))?;
        let enrolled = enrolled.parse().map_err(|_| refuse("the enrollment time is not a number"))?;
        if out.iter().any(|s| s.credential.id == id) {
            return Err(refuse("this credential ID is already on an earlier line"));
        }
        out.push(Stored {
            credential: Credential {
                id,
                algorithm,
                public_key,
                sign_count,
            },
            enrolled,
            rp_id: rp_id.map(str::to_string),
        });
    }
    Ok(out)
}
