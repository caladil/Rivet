//! `ADR-022` §3.2's record, stamped for this binary.
//!
//! # ~~This is the second copy of this code~~ — **it is not, since 2026-08-24**
//!
//! What this file said, kept because `AGENTS.md` §2 asks the record to show its
//! own history and because the paragraph called its own ending correctly:
//!
//! > ~~`cmd/rivet-sysd/src/journal.rs` is the first. Every line here is
//! > the same idea and most of them are the same code — the field list, the RFC
//! > 8259 escaping, Howard Hinnant's `civil_from_days`. It is duplicated for the
//! > reason `cmd/rivet-sysd/src/kernel.rs` states about its own third
//! > copy of `Errno`: **a binary crate has no library target, so nothing can
//! > `use` a type out of it**, and `ADR-013` §7 ruled that the shared platform
//! > crate emerges at Stage 3 out of accumulated evidence rather than being
//! > built speculatively.~~
//! >
//! > ~~**So this file is evidence, and it is a different shape from the last
//! > piece.** `Errno` and `ExitStatus` wanted sharing while the *ports* did not.
//! > Here the whole module wants sharing and nothing about it is
//! > binary-specific: a record is a value, `to_jsonl` is a pure function, and
//! > `rfc3339` is arithmetic. **`ADR-022` §3.2's record is the strongest
//! > candidate yet for the first thing that shared crate should hold** —
//! > stronger than `Errno`, because a second *encoder* can disagree with the
//! > first about the format itself, and `ADR-022` §3 makes that format
//! > authoritative.~~
//!
//! **The record went to `rivet-core`**, not to a new platform crate, and the
//! thing that forced it was not tidiness: `ADR-022` §9's `rivet log` is a
//! **reader**, in a third process, and a format with one authoritative form
//! (`ADR-022` §3) cannot have three encoders and a decoder that each believe
//! their own copy. `rivet_core::journal`'s header carries the full argument.
//!
//! **`ADR-013` §7's platform crate is not pre-empted by this.** The record is a
//! *domain* value — `ADR-022` §2 makes it the primary thing and the text a view
//! of it — and `rivet-core` is where Rivet's domain values live. What
//! `ADR-013` §7 is about is *"what it means to be a program on Rivet"*: the
//! emitting API, routing an application's records into the journal, the
//! platform's side of `ADR-022` §6's table. None of that is here.
//!
//! **What this crate's manifest said, and why it changed.** `[dependencies]`
//! carried a deliberate note that `rivet-core` is *not* here because "this
//! binary names no Rivet domain type". That was true and stopped being true
//! the moment the record became one. The same note used "this crate already
//! needed a copy of the journal record" as an argument for not taking `chrono`;
//! that argument is now stronger rather than weaker, because the copy is gone
//! and `rfc3339` comes from the same place the supervisor's does.
//!
//! `the_column_list_matches_the_supervisors` is deleted with the duplication it
//! guarded. A tripwire between two copies is worth exactly as much as the
//! copies are, and it read the supervisor's *source text* — which stopped
//! declaring `RESERVED_FIELD_NAMES` when the record moved, and the tripwire
//! failed loudly and correctly rather than going quiet. That is what it was
//! written to do.
//!
//! # What is deliberately smaller than the supervisor's `Journal`
//!
//! * **No in-memory ring buffer and no file on `/state`.** `ADR-022` §7's
//!   buffer-until-`/state` and §8's rotation are the supervisor's, because it is
//!   the process running before anything else and the one that owns the file.
//!   This binary is started by the supervisor, long after that.
//! * **No capture of its own.** This binary does not open `/state/log` itself
//!   — `ADR-022` §4's capture of a child's stdout as records closed 2026-08-24
//!   in the supervisor, which is the process that owns the file. This
//!   binary's own records are valid `ADR-022` §3 records on stdout either
//!   way, so the supervisor's capture reaches them without this binary
//!   doing anything extra.

pub use rivet_core::journal::{Record, Severity, Value, rfc3339};

/// Stamps records and hands them to the sink.
///
/// Smaller than the supervisor's `Journal` by everything `ADR-022` §7 and §8 are
/// about; see this module's documentation for what was left out and why.
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
    /// The salt separates *runs* of this binary within one boot, which matters
    /// here more than it does for the supervisor: `ADR-017` §3.5 restarts a
    /// failed service, and a time client whose network is not up yet is
    /// expected to fail and be restarted several times before it succeeds.
    pub fn next_correlation(&mut self) -> String {
        self.counter = self.counter.wrapping_add(1);
        format!("{:08x}{:08x}", self.salt, self.counter)
    }

    /// Stamp and emit one record.
    ///
    /// `mono` and `ts` are read **here**, from the port, at the moment the
    /// record is made — never carried in from the caller. That is what makes a
    /// record written immediately after a step carry the *new* wall clock and
    /// the monotonic instant it happened at, which is the pair `ADR-022` §3.2
    /// says makes ordering recoverable whatever the wall clock did.
    pub fn emit(
        &mut self,
        k: &mut dyn crate::kernel::Kernel,
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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use crate::testkernel::FakeKernel;

    fn field(name: &str, v: Value) -> (String, Value) {
        (name.to_string(), v)
    }

    /// **The property the deleted tripwire was reaching for, now free.**
    ///
    /// It used to read `RESERVED_FIELD_NAMES` out of the supervisor's source
    /// text and compare it with this crate's copy. There is one copy now, so
    /// the two cannot disagree — and what is worth asserting instead is that a
    /// record *this* binary stamps is one `ADR-022` §9's reader accepts, which
    /// is the thing the duplication actually endangered.
    #[test]
    fn a_record_this_binary_stamps_is_one_the_reader_accepts() {
        let mut k = FakeKernel::new();
        k.mono_ms = 184_221;
        k.wall_ms = Some(1_755_820_800_000);
        let mut j = Journal::new("0d3f".to_string(), 1, "timed".to_string());
        j.emit(
            &mut k,
            Severity::Warn,
            "a91c",
            "the clock is stepped",
            vec![
                field("offset_ms", Value::Int(-4200)),
                field("round_trip_s", Value::Float(0.031)),
            ],
        );
        let line = k
            .records
            .first()
            .map(Record::to_jsonl)
            .expect("the emit produced no record");
        let back = rivet_core::journal::Record::from_jsonl(&line)
            .unwrap_or_else(|e| panic!("rivet log could not read {line:?}: {e}"));
        assert_eq!(back.svc, crate::SELF_NAME);
        assert_eq!(back.sev, Severity::Warn);
        assert_eq!(back.field("offset_ms"), Some(&Value::Int(-4200)));
        assert_eq!(back.ts.as_deref(), Some("2025-08-22T00:00:00.000Z"));
    }

    #[test]
    fn a_record_renders_every_fixed_field_adr_022_3_2_names() {
        let r = Record {
            ts: Some("2026-08-22T14:02:11.412Z".to_string()),
            boot: "0d3f".to_string(),
            mono: 184.221,
            sev: Severity::Warn,
            svc: "rivet-timed".to_string(),
            corr: "a91c".to_string(),
            actor: "timed".to_string(),
            msg: "the clock is stepped".to_string(),
            fields: vec![field("offset_ms", Value::Int(-4200))],
        };
        assert_eq!(
            r.to_jsonl(),
            concat!(
                r#"{"ts":"2026-08-22T14:02:11.412Z","boot":"0d3f","mono":184.221,"#,
                r#""sev":"warn","svc":"rivet-timed","corr":"a91c","#,
                r#""actor":"timed","msg":"the clock is stepped","offset_ms":-4200}"#,
                "\n"
            )
        );
    }

    /// A record with no readable wall clock omits `ts` entirely rather than
    /// writing a plausible-looking 1970. **This is the ordinary state of this
    /// binary's first records on a cold Pi.**
    #[test]
    fn a_record_with_no_wall_clock_omits_ts_rather_than_inventing_one() {
        let mut k = FakeKernel::new();
        k.wall_ms = None;
        let mut j = Journal::new("b".to_string(), 1, "timed".to_string());
        j.emit(&mut k, Severity::Info, "c", "m", vec![]);
        assert_eq!(k.records.first().and_then(|r| r.ts.clone()), None);
    }

    /// The service name is this binary's own and is not a parameter, so every
    /// record it writes is attributable without a caller having to remember.
    #[test]
    fn every_record_names_this_binary_as_its_service() {
        let mut k = FakeKernel::new();
        let mut j = Journal::new("b".to_string(), 1, "timed".to_string());
        j.emit(&mut k, Severity::Info, "c", "m", vec![]);
        assert_eq!(k.records.first().map(|r| r.svc.clone()), Some(crate::SELF_NAME.to_string()));
    }

    /// `ADR-017` §3.5 restarts a failed service, and a time client whose
    /// network is not up is expected to fail several times before it succeeds.
    /// Correlation identifiers from two runs in one boot must not collide, or a
    /// reader following one chain would splice two.
    #[test]
    fn two_runs_in_one_boot_mint_different_correlation_identifiers() {
        let mut first = Journal::new("b".to_string(), 1, "timed".to_string());
        let mut second = Journal::new("b".to_string(), 2, "timed".to_string());
        assert_ne!(first.next_correlation(), second.next_correlation());
    }

    /// The stamps come from the port at emit time, not from the caller. A step
    /// moves the wall clock, so a record written just after one must carry the
    /// **new** time and the monotonic instant it happened at (`ADR-022` §3.2).
    #[test]
    fn a_record_is_stamped_from_the_clocks_at_the_moment_it_is_made() {
        let mut k = FakeKernel::new();
        k.mono_ms = 5_000;
        k.wall_ms = Some(1_000);
        let mut j = Journal::new("b".to_string(), 1, "timed".to_string());
        j.emit(&mut k, Severity::Info, "c", "before", vec![]);
        k.wall_ms = Some(1_755_820_800_000);
        k.mono_ms = 5_100;
        j.emit(&mut k, Severity::Info, "c", "after", vec![]);

        assert_eq!(
            k.records.first().and_then(|r| r.ts.clone()).as_deref(),
            Some("1970-01-01T00:00:01.000Z")
        );
        assert_eq!(
            k.records.get(1).and_then(|r| r.ts.clone()).as_deref(),
            Some("2025-08-22T00:00:00.000Z")
        );
        assert!(
            k.records.get(1).is_some_and(|r| (r.mono - 5.1).abs() < 1e-9),
            "{:?}",
            k.records.get(1).map(|r| r.mono)
        );
    }
}
