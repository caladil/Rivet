//! `ADR-022` §3.2's record, stamped for this binary.
//!
//! The format is `rivet_core::journal`'s and there is no second copy of it
//! here — that move happened on 2026-08-24 and `cmd/rivet-timed/src/journal.rs`
//! carries the argument. What is here is the stamping: which boot, which
//! service, which actor, and a correlation identifier.
//!
//! # Why this binary emits records rather than prose
//!
//! Because it is supervised. `ADR-017` §7 — *"a Rivet service does not open a
//! log file. It writes structured records, or to standard output and error, and
//! the supervisor captures them"* — and `ADR-022` §4's capture parses `§3`
//! records back out of a child's stdout and forwards them **as records**, with
//! severity and fields intact, rather than wrapping them as text. `rivet-timed`
//! is the only other producer of that path today.
//!
//! It matters more here than it does for the time client, and the reason is
//! the maintainer's 2026-08-24 ruling: an update writes 640 MiB, and the whole point of
//! the installer being its own supervised binary is that **the operator's
//! session does not own the write.** A session that has gone away can still be
//! told what happened, and the journal is how.
//!
//! # One correlation identifier for the whole install
//!
//! `ADR-022` §5 ties the records of one causal chain together, and an install is
//! exactly one chain: verify, write, read back, arm. So [`Journal::next_correlation`]
//! is called once, at the top, and every record of that run carries it. That is
//! the difference from `rivet-timed`, which mints one per poll.

pub use rivet_core::journal::{Record, Severity, Value, rfc3339};

use crate::kernel::Kernel;

/// Stamps records and hands them to the sink.
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
    /// The salt separates *runs* of this binary within one boot. It matters here
    /// for a reason it does not for `rivet-timed`: two installs in one boot is
    /// an ordinary thing to do — install, find the payload was the wrong one,
    /// install again — and a reader following one chain must not splice the two.
    pub fn next_correlation(&mut self) -> String {
        self.counter = self.counter.wrapping_add(1);
        format!("{:08x}{:08x}", self.salt, self.counter)
    }

    /// Stamp and emit one record.
    ///
    /// `mono` and `ts` are read **here**, from the port, at the moment the record
    /// is made — never carried in from the caller. On a 640 MiB write that is
    /// what makes the elapsed time between two records real.
    pub fn emit(
        &mut self,
        k: &mut dyn Kernel,
        sev: Severity,
        corr: &str,
        msg: &str,
        fields: Vec<(String, Value)>,
    ) {
        #[allow(clippy::cast_precision_loss)]
        let mono = k.now_ms() as f64 / 1000.0;
        let ts = k.wall_ms().map(rfc3339);
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
        k.emit(&record);
    }
}

/// A `(name, value)` pair, spelled once rather than at every call site.
pub fn field(name: &str, value: Value) -> (String, Value) {
    (name.to_string(), value)
}

/// A count that has to become an `i64` for [`Value::Int`].
///
/// **Saturating rather than wrapping**, and it is worth a function rather than
/// an `as`: a 640 MiB length that wrapped to a negative number in the journal
/// would make the one record that says how much was written unreadable, and the
/// clippy lint that would have caught it is `deny`d in this crate.
#[must_use]
pub fn count(n: u64) -> Value {
    Value::Int(i64::try_from(n).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::testkernel::FakeKernel;

    /// A record this binary stamps must be one `ADR-022` §9's reader accepts.
    /// The two are a writer and a reader of one authoritative format, and a
    /// disagreement between them does not crash — it shows a shorter log.
    #[test]
    fn a_record_this_binary_stamps_is_one_the_reader_accepts() {
        let mut k = FakeKernel::new();
        k.mono_ms = 184_221;
        k.wall_ms = Some(1_755_820_800_000);
        let mut j = Journal::new("0d3f".to_string(), 1, "update".to_string());
        j.emit(
            &mut k,
            Severity::Info,
            "a91c",
            "the inactive slot is written",
            vec![
                field("bytes", count(671_088_640)),
                field("slot", Value::Int(4)),
            ],
        );
        let line = k
            .records
            .first()
            .map(Record::to_jsonl)
            .expect("the emit produced no record");
        let back = Record::from_jsonl(&line)
            .unwrap_or_else(|e| panic!("rivet get log could not read {line:?}: {e}"));
        assert_eq!(back.svc, crate::SELF_NAME);
        assert_eq!(back.field("bytes"), Some(&Value::Int(671_088_640)));
        assert_eq!(back.ts.as_deref(), Some("2025-08-22T00:00:00.000Z"));
    }

    /// A cold Pi has no wall clock (`ADR-021`), and a record that invented a
    /// plausible-looking 1970 would be worse than one with no `ts` at all.
    #[test]
    fn a_record_with_no_wall_clock_omits_ts_rather_than_inventing_one() {
        let mut k = FakeKernel::new();
        k.wall_ms = None;
        let mut j = Journal::new("b".to_string(), 1, "update".to_string());
        j.emit(&mut k, Severity::Info, "c", "m", vec![]);
        assert_eq!(k.records.first().and_then(|r| r.ts.clone()), None);
    }

    #[test]
    fn every_record_names_this_binary_as_its_service() {
        let mut k = FakeKernel::new();
        let mut j = Journal::new("b".to_string(), 1, "update".to_string());
        j.emit(&mut k, Severity::Info, "c", "m", vec![]);
        assert_eq!(
            k.records.first().map(|r| r.svc.clone()),
            Some(crate::SELF_NAME.to_string())
        );
    }

    /// Two runs in one boot must not mint the same identifier, or a reader
    /// following one install would splice the other into it.
    #[test]
    fn two_runs_in_one_boot_mint_different_correlation_identifiers() {
        let mut first = Journal::new("b".to_string(), 1, "update".to_string());
        let mut second = Journal::new("b".to_string(), 2, "update".to_string());
        assert_ne!(first.next_correlation(), second.next_correlation());
    }

    /// A member length is a `u64` and the record's field is an `i64`. The
    /// conversion must not produce a negative number for the one field that
    /// says how much was written.
    #[test]
    fn a_length_too_large_for_the_record_saturates_rather_than_going_negative() {
        assert_eq!(count(671_088_640), Value::Int(671_088_640));
        assert_eq!(count(u64::MAX), Value::Int(i64::MAX));
    }
}
