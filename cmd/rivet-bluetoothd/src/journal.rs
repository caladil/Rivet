//! `ADR-022` §3.2's record, stamped for this binary.
//!
//! The same shape `cmd/rivet-timed/src/journal.rs` carries, and for the same
//! reason its header gives at length: the **record** lives in `rivet-core`
//! because a format with one authoritative form cannot have several encoders,
//! and what is duplicated here is the four lines of stamping — a binary crate
//! has no library target, so nothing can `use` a struct out of `rivet-timed`.
//!
//! Smaller than the supervisor's by everything `ADR-022` §7 and §8 are about:
//! no ring buffer and no file on `/state`. This process is started by the
//! supervisor, which owns both.

use std::io::Write as _;

pub use rivet_core::journal::{Record, Severity, Value, rfc3339};

/// Stamps records and writes them to stdout, one JSON object per line.
pub struct Journal {
    boot: String,
    actor: String,
    salt: u32,
    counter: u32,
}

impl Journal {
    pub fn new(boot: String, salt: u32, actor: String) -> Self {
        Self {
            boot,
            actor,
            salt,
            counter: 0,
        }
    }

    /// Mint a correlation identifier (`ADR-022` §5).
    ///
    /// The salt is the pid, which separates runs of this binary within one
    /// boot. `ADR-017` §3.5 makes several runs the expected case on a machine
    /// whose adapter is not there yet.
    pub fn next_correlation(&mut self) -> String {
        self.counter = self.counter.wrapping_add(1);
        format!("{:08x}{:08x}", self.salt, self.counter)
    }

    /// Stamp and write one record.
    ///
    /// `mono` and `ts` are read here, at the moment the record is made, never
    /// carried in — `ADR-022` §3.2's pair is what makes ordering recoverable
    /// whatever the wall clock did.
    pub fn emit(&mut self, sev: Severity, corr: &str, msg: &str, fields: Vec<(String, Value)>) {
        // The same monotonic clock the advertisements are stamped from, so a
        // record and a record's subject are on one timeline.
        #[allow(clippy::cast_precision_loss)]
        let mono = crate::sockets::monotonic_ms() as f64 / 1000.0;
        let ts = wall_ms().map(rfc3339);
        let record = Record {
            ts,
            boot: self.boot.clone(),
            mono,
            sev,
            svc: crate::SELF_NAME.to_string(),
            corr: corr.to_string(),
            actor: self.actor.clone(),
            msg: msg.to_string(),
            fields,
        };
        let mut out = std::io::stdout().lock();
        // A journal write that fails has nowhere to report the failure, which
        // is why this is the one place a Result is deliberately dropped. The
        // supervisor notices a service whose stdout has gone.
        let _ = writeln!(out, "{}", record.to_jsonl());
        let _ = out.flush();
    }
}

/// Milliseconds since the epoch, or `None` when the clock is not set.
///
/// `ADR-021`: a Rivet's wall clock may be unset, and a record with no `ts` is
/// the honest form of that. `mono` is always there.
fn wall_ms() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// A field, spelled once.
pub fn field(name: &str, value: Value) -> (String, Value) {
    (name.to_string(), value)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_record_this_binary_stamps_is_one_the_reader_accepts() {
        let record = Record {
            ts: Some("2026-09-05T00:00:00.000Z".to_string()),
            boot: "0d3f".to_string(),
            mono: 1.5,
            sev: Severity::Info,
            svc: crate::SELF_NAME.to_string(),
            corr: "a91c".to_string(),
            actor: crate::ACTOR.to_string(),
            msg: "scanning".to_string(),
            fields: vec![field("index", Value::Int(0))],
        };
        let line = record.to_jsonl();
        let back = Record::from_jsonl(&line)
            .unwrap_or_else(|e| panic!("rivet log could not read {line:?}: {e}"));
        assert_eq!(back.svc, "rivet-bluetoothd");
        assert_eq!(back.field("index"), Some(&Value::Int(0)));
    }

    #[test]
    fn a_correlation_id_changes_between_records() {
        let mut j = Journal::new("0d3f".to_string(), 7, ACTOR_FOR_TEST.to_string());
        assert_ne!(j.next_correlation(), j.next_correlation());
    }

    const ACTOR_FOR_TEST: &str = "bluetoothd";
}
