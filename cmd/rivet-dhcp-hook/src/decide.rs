//! **What to do about an event, decided without touching anything.**
//!
//! The whole of this program's judgement lives here, as a function from a word
//! and a lookup to an [`Action`]. `main` performs the action and this module
//! performs none — which is the same seam `rivet_adapter_net::lease` draws for
//! the same reason: a decision that reads the environment and writes a file in
//! one step is a decision no test can drive.

use rivet_adapter_net::lease::{Event, Lease, lease_path};

/// What `main` should do about the event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Put this text at this path, atomically.
    Write { path: String, text: String },
    /// Take the lease away — the machine no longer has what it says.
    Remove { path: String },
}

/// Read the event and the environment, and say what should happen.
///
/// # Errors
///
/// A string naming what was wrong, for `main` to report on stderr. Every one of
/// them means **nothing is written**, which is the point: `AGENTS.md` §5's
/// refuse-rather-than-guess, applied to the one fact whose wrong answer is a
/// machine nobody can reach.
pub fn decide(word: &str, lookup: &dyn Fn(&str) -> Option<String>) -> Result<Action, String> {
    if word.trim().is_empty() {
        return Err(
            "no event word. udhcpc passes one as argv[1] — `bound`, `renew`, `deconfig`, `nak` \
             or `leasefail`. Being run with none means something other than udhcpc ran this"
                .to_string(),
        );
    }
    let event = Event::parse(word);

    // **The interface is present for EVERY event**, including the ones carrying
    // no lease — which is what makes it possible to know whose address to
    // withdraw.
    let interface = lookup("interface").unwrap_or_default().trim().to_string();
    if interface.is_empty() {
        return Err(format!(
            "udhcpc said `{word}` and set no `interface`. There is no way to know which interface \
             this is about, so nothing was written"
        ));
    }
    let path = lease_path(&interface);

    if !event.carries_a_lease() {
        // **Removed rather than left to expire.** `deconfig` is said before
        // udhcpc has an answer and again when it gives up; `nak` means the
        // server took the address back; `leasefail` means nobody answered. In
        // all of them the machine no longer has what the file claims, and a
        // stale file would have `rivet-netd` keep configuring an address that
        // is gone.
        return Ok(Action::Remove { path });
    }

    let lease = Lease::from_pairs(lookup).map_err(|why| format!("{interface}: {word} — {why}"))?;

    // The parser read the same `interface` variable, so these cannot disagree
    // — but a mismatch would mean the environment changed underneath, and
    // writing one interface's address into another's file is the kind of wrong
    // that looks right for a long time.
    if lease.interface != interface {
        return Err(format!(
            "udhcpc said `interface={interface}` and the lease names `{}`. Nothing was written",
            lease.interface
        ));
    }

    Ok(Action::Write {
        path,
        text: lease.to_pairs_text(),
    })
}

#[cfg(test)]
#[path = "decide_tests.rs"]
mod tests;
