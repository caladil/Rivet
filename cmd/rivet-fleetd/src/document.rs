//! **Reading and writing one machine's desired-state document**, over
//! `ADR-035`'s control plane.
//!
//! [`crate::node`] is the socket and the pinned key; this is the two calls
//! `ADR-044`'s poller makes over it, and the reading of what comes back.
//!
//! # Why this is not `rivet_remote::tls`
//!
//! It very nearly is: that crate already composes exactly these calls, with the
//! compare-and-swap and the read-back this module copies. **It is a HOST
//! library** — its own manifest says so, and it links `rustls` and
//! `rivetos-crypto` for a payload digest. `rivet-fleetd` ships on a card, and
//! pulling a host crate's whole dependency graph onto that card to reuse two
//! functions is the wrong trade in a way that would be hard to reverse later.
//!
//! What is shared is the part that must not diverge: the subject's word, the
//! verb's word and the confirmation word are all `rivet-core`'s constants, so a
//! rename in the catalogue breaks this at compile time rather than at runtime
//! against a board.
//!
//! # The write is a compare-and-swap, and that is not a workaround
//!
//! There is no *write this file whole* operation. `apply edit --expect <a>
//! --replace <b>` refuses unless `<a>` occurs exactly once, so replacing the
//! document means stating what you believe is there — and **if the document
//! changed between the read and the write, the machine refuses** rather than
//! clobbering somebody else's edit. For a poller that is exactly the property
//! wanted: the drift check runs against a document read seconds earlier, and
//! this is what stops a change that landed in between being silently undone.

use rivet_core::json::Json;
use rivet_core::journal::write_json_string;
use rivet_core::operation::{CONFIRMATION, DESIRED_STATE, EDIT};
use rivet_core::state::MACHINE_DOCUMENT;

/// How to reach one machine: where it is, and what to trust at the other end.
pub struct Machine<'a> {
    /// `host:port`.
    pub address: &'a str,
    /// The machine's own public key, as the register records it.
    pub pin: &'a str,
}

impl Machine<'_> {
    /// One `tools/call`, and the text of its result.
    ///
    /// # Errors
    ///
    /// The transport's words, or the machine's refusal, or a sentence about an
    /// answer that is not the shape MCP promises.
    fn call(&self, tool: &str, arguments: &str) -> Result<String, String> {
        let mut request = String::from(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"",
        );
        request.push_str(tool);
        request.push_str("\",\"arguments\":");
        request.push_str(arguments);
        request.push_str("}}");

        let answer = crate::node::ask(self.address, self.pin, &request)?;
        let parsed = rivet_core::json::parse(&answer)
            .map_err(|e| format!("{}: the machine's answer is not JSON: {e:?}", self.address))?;

        // **A JSON-RPC error is not a refusal and must not be read as one.** A
        // refusal arrives as a SUCCESSFUL call carrying `isError`, which is the
        // distinction `cmd/rivet`'s MCP adapter draws so that an agent can tell
        // *Rivet is broken* from *Rivet said no*.
        if let Some(error) = parsed.get("error") {
            let said = error
                .get("message")
                .and_then(Json::as_str)
                .unwrap_or("the machine sent an error with no message");
            return Err(format!("{}: {said}", self.address));
        }

        let text = parsed
            .get("result")
            .and_then(|result| result.get("content"))
            .and_then(Json::as_array)
            .and_then(|items| items.first())
            .and_then(|first| first.get("text"))
            .and_then(Json::as_str)
            .ok_or_else(|| {
                format!(
                    "{}: the machine's answer to `{tool}` carries no result content. It said: {}",
                    self.address,
                    shortened(&answer)
                )
            })?;

        let refused = parsed
            .get("result")
            .and_then(|result| result.get("isError"))
            .is_some_and(|flag| *flag == Json::Bool(true));
        if refused {
            return Err(format!("{}: {}", self.address, text.trim()));
        }
        Ok(text.to_owned())
    }

    /// **The machine's desired-state document**, or `None` if it has none.
    ///
    /// An absent document is an ordinary answer and not an error — a machine
    /// boots from the provisioning document on the boot partition and writes
    /// `/state/config` only when it has something to write. What it means for
    /// a poller is decided by the caller, not here.
    ///
    /// # Errors
    ///
    /// The transport's, the machine's refusal, or an answer whose shape is not
    /// this subject's.
    pub fn read(&self) -> Result<Option<String>, String> {
        let text = self.call("rivet_get", &format!("{{\"subject\":\"{DESIRED_STATE}\"}}"))?;
        let answer = rivet_core::json::parse(&text).map_err(|e| {
            format!("{}: `get {DESIRED_STATE}` did not answer with JSON: {e:?}", self.address)
        })?;

        let subject = answer.get("subject").and_then(Json::as_str).unwrap_or("");
        if subject != DESIRED_STATE {
            return Err(format!(
                "{}: this asked for `{DESIRED_STATE}` and the machine answered about \
                 `{subject}`. That is a different question and its answer is not the \
                 document, so nothing is read from it.",
                self.address
            ));
        }
        if answer.get("present") != Some(&Json::Bool(true)) {
            return Ok(None);
        }
        let contents = answer.get("contents").and_then(Json::as_str).ok_or_else(|| {
            format!("{}: the machine reported a document and sent no contents", self.address)
        })?;

        // **The length is checked against what the machine said**, and this is
        // the reason the subject reports it. Two layers of JSON escaping stand
        // between the file and this string, and a mistake in either produces
        // text that is *plausible* — a lost backslash, a newline that became
        // the letter `n`. A plausible document is worse than none, because it
        // is the one that gets written back.
        let said = answer.get("bytes").and_then(Json::as_integer).ok_or_else(|| {
            format!("{}: the machine reported no length for the document", self.address)
        })?;
        let decoded = i64::try_from(contents.len()).unwrap_or(i64::MAX);
        if said != decoded {
            return Err(format!(
                "{}: the machine says its document is {said} bytes and this end decoded \
                 {decoded}. Nothing is read from this answer.",
                self.address
            ));
        }
        Ok(Some(contents.to_owned()))
    }

    /// **Write `text` as this machine's document**, believing `current` is
    /// what is there now.
    ///
    /// # Errors
    ///
    /// The machine's refusal — which includes `--expect` matching zero times
    /// or more than once, and that is the compare-and-swap doing its job: the
    /// document changed between the read and this call, and nothing was
    /// written.
    pub fn write(&self, current: &str, text: &str) -> Result<(), String> {
        if current == text {
            return Ok(());
        }
        // An empty document is not an absent one, and `--expect ""` would match
        // everywhere — which the machine refuses as ambiguous. Appending to an
        // empty file leaves precisely the new text.
        let change = if current.is_empty() {
            vec!["--append", text]
        } else {
            vec!["--expect", current, "--replace", text]
        };
        let words = change
            .iter()
            .map(|word| {
                let mut out = String::new();
                write_json_string(word, &mut out);
                out
            })
            .collect::<Vec<_>>()
            .join(",");

        let mut slot = String::new();
        write_json_string(MACHINE_DOCUMENT, &mut slot);
        self.call(
            "rivet_apply",
            &format!(
                "{{\"subject\":\"{EDIT}\",\"slot\":{slot},\"change\":[{words}],\
                 \"confirmation\":\"{CONFIRMATION}\"}}"
            ),
        )?;

        // **Read it back.** `rivet_remote::tls` does the same and says why: one
        // end of this channel escapes bytes it cannot read, and a document that
        // is nearly right is the one that gets left in place. A document is
        // small and a second call is cheap.
        match self.read()? {
            Some(now) if now == text => Ok(()),
            Some(_) => Err(format!(
                "{}: {MACHINE_DOCUMENT} is NOT the document that was sent, and the machine \
                 ACCEPTED the edit - so this is a fault in the channel and not in the \
                 document. What is on the machine now is neither the old document nor the \
                 new one; read it with `rivet get {DESIRED_STATE}` before anything reloads.",
                self.address
            )),
            None => Err(format!(
                "{}: the machine accepted the edit and now reports NO document at all. \
                 Nothing here can say what is on it.",
                self.address
            )),
        }
    }
}

/// Enough of a machine's answer to recognise it in a journal, and no more.
fn shortened(text: &str) -> String {
    const MOST: usize = 200;
    let trimmed = text.trim();
    if trimmed.len() <= MOST {
        return trimmed.to_owned();
    }
    // On a character boundary, because an answer may carry a hostname somebody
    // typed and a slice through a multi-byte one panics.
    let mut at = MOST;
    while at > 0 && !trimmed.is_char_boundary(at) {
        at -= 1;
    }
    format!("{}...", &trimmed[..at])
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_long_answer_is_shortened_on_a_character_boundary() {
        let answer = "\u{e9}".repeat(300);
        let said = shortened(&answer);
        assert!(said.ends_with("..."), "{said}");
        assert!(said.len() <= 204, "{}", said.len());
    }

    #[test]
    fn a_short_answer_is_left_alone() {
        assert_eq!(shortened("  {\"a\":1}  "), "{\"a\":1}");
    }

    /// The three words this transport and the machine both have to agree on
    /// come from the catalogue's own constants, so a rename breaks the build
    /// rather than a board.
    #[test]
    fn the_words_are_the_catalogues_own() {
        assert_eq!(DESIRED_STATE, "desired-state");
        assert_eq!(EDIT, "edit");
        assert_eq!(CONFIRMATION, "confirm");
    }
}
