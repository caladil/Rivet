//! **`peers` reloaded while this daemon runs** — `ADR-063` §3.
//!
//! The pins are read once at startup to build the listener, and afterwards
//! again whenever a control-plane session answers. A changed file that parses,
//! is not empty, and still names the key of the session that observed the
//! change replaces the set in force; the next handshake is decided by it, and
//! every session already open carries on under the set it was admitted by.
//!
//! # Why after each answer, and not a file watcher
//!
//! The change this exists for arrives through this daemon: `apply
//! restore-secret peers` runs in the `rivet mcp` child of one of its own
//! connections, writes the file by rename, and then answers. Checking the file
//! as each answer passes through on its way to the caller means the new set is
//! in force **before the caller reads that it was written**, and it means the
//! reload knows whose session made the change — which is the key it must not
//! remove. A watcher would need `inotify`, a thread of its own, and would see a
//! change with no caller attached to it.
//!
//! Reading a file of a few hundred bytes per answer costs less than the answer.
//! A change made some other way — the serial console, a program running as
//! root — is picked up when the next session answers, or at the next start.
//!
//! # What a reload refuses, and what refusing means
//!
//! A file that does not parse, one naming no keys, and one that omits the key
//! of the session that observed it are refused. **The set already in force
//! stays in force**, and the refusal is written to this daemon's stderr, which
//! is the journal. `apply restore-secret` refuses the first and third before
//! writing, so reaching either here means the file was written another way.
//!
//! A refused text is judged once per caller, not once per answer, so a bad
//! file does not flood the journal. A caller-dependent refusal is not a verdict
//! on the text: the session whose key the file does name loads it when it
//! answers.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use rivetos_tls::{PinnedPeer, Pin};

/// What one look at the file found.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The text is the one already loaded, or one already refused for this
    /// caller. Nothing is journalled.
    Unchanged,
    /// The file changed and its set is now in force.
    Loaded {
        /// Keys in the new set that were not in the old.
        added: usize,
        /// Keys in the old set that are not in the new.
        removed: usize,
        /// How many keys may connect now.
        now: usize,
    },
    /// The file changed and was not loaded; the previous set is in force.
    Refused(String),
}

/// **The decision, with no file and no lock** — what a new text becomes, or
/// why it is refused.
///
/// # Errors
///
/// A sentence for the journal when `text` does not parse, names no keys, or
/// omits `caller`.
pub fn judge(text: &str, caller: Option<&Pin>) -> Result<Vec<Pin>, String> {
    let pins = rivetos_tls::parse_pins(text).map_err(|e| format!("it does not parse: {e}"))?;
    if pins.is_empty() {
        return Err("it names no keys, so no caller could ever connect again".to_string());
    }
    if let Some(key) = caller
        && !pins.contains(key)
    {
        return Err(format!(
            "it does not name {}, the key of the session that observed the change, so loading \
             it would lock that operator out",
            rivetos_tls::encode_hex(key)
        ));
    }
    Ok(pins)
}

/// The keys in `new` that are not in `old`, and the keys in `old` that are
/// not in `new`.
fn difference(old: &[Pin], new: &[Pin]) -> (usize, usize) {
    let added = new.iter().filter(|key| !old.contains(key)).count();
    let removed = old.iter().filter(|key| !new.contains(key)).count();
    (added, removed)
}

/// What has been looked at, so that nothing is judged or journalled twice.
#[derive(Debug)]
struct Seen {
    /// The text whose keys are in force.
    loaded: String,
    /// The last refusal: what was read (a text, or why it could not be) and
    /// the caller it was judged for.
    refused: Option<(Result<String, String>, Option<Pin>)>,
}

/// The pin file and the verifier it feeds.
#[derive(Debug)]
pub struct LivePeers {
    path: PathBuf,
    verifier: Arc<PinnedPeer>,
    seen: Mutex<Seen>,
}

impl LivePeers {
    /// `loaded` is the text the verifier's pins were read from at startup.
    pub fn new(path: &Path, verifier: Arc<PinnedPeer>, loaded: String) -> Self {
        Self {
            path: path.to_path_buf(),
            verifier,
            seen: Mutex::new(Seen { loaded, refused: None }),
        }
    }

    /// The file this reads.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Look at the file on behalf of the session whose key is `caller` (hex,
    /// as the transport names it), and load it if it changed and may be.
    ///
    /// Held under one lock from the read to the replacement, so two sessions
    /// answering at once cannot interleave a stale read with a fresh load.
    pub fn look(&self, caller: Option<&str>) -> Outcome {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        let read = std::fs::read_to_string(&self.path).map_err(|e| e.to_string());
        if read.as_deref() == Ok(seen.loaded.as_str()) {
            seen.refused = None;
            return Outcome::Unchanged;
        }
        let caller = caller.and_then(|hex| rivetos_tls::parse_pins(hex).ok()?.first().copied());
        let judged = match &read {
            Ok(text) => judge(text, caller.as_ref()),
            Err(e) => Err(format!("it cannot be read: {e}")),
        };
        match judged {
            Ok(pins) => {
                let old = self.verifier.pins();
                let (added, removed) = difference(&old, &pins);
                let now = pins.len();
                if let Err(e) = self.verifier.replace(pins) {
                    return Outcome::Refused(e.to_string());
                }
                seen.loaded = read.unwrap_or_default();
                seen.refused = None;
                Outcome::Loaded { added, removed, now }
            }
            Err(why) => {
                let key = (read, caller);
                if seen.refused.as_ref() == Some(&key) {
                    return Outcome::Unchanged;
                }
                seen.refused = Some(key);
                Outcome::Refused(why)
            }
        }
    }

    /// [`LivePeers::look`], with what it found written to the journal. `who`
    /// names the session for the reader.
    pub fn look_and_journal(&self, caller: Option<&str>, who: &str) {
        match self.look(caller) {
            Outcome::Unchanged => {}
            Outcome::Loaded { added, removed, now } => eprintln!(
                "rivet-mcpd: {path} changed during {who}'s session and is in force for new \
                 connections: {added} key(s) added, {removed} removed, {now} may connect. \
                 Sessions already open are unchanged.",
                path = self.path().display(),
            ),
            Outcome::Refused(why) => eprintln!(
                "rivet-mcpd: {path} changed and was NOT loaded, observed during {who}'s session: \
                 {why}. The {n} key(s) in force before still decide who may connect.",
                path = self.path().display(),
                n = self.verifier.pinned(),
            ),
        }
    }
}

#[cfg(test)]
#[path = "peers_tests.rs"]
mod tests;
