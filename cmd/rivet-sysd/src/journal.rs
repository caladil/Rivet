//! The journal — `ADR-022` §7 puts it here rather than in a service of its own.
//!
//! The reasoning is worth restating because it constrains this module: a
//! journal that were itself a service could fail to start, and the record of
//! *why* would have nowhere to go. So it is a library the supervisor calls, it
//! is running before any service exists, and it cannot depend on anything the
//! supervisor might fail to bring up.
//!
//! **A record is the primary thing; text is a view of it** (`ADR-022` §2).
//! Nothing here formats a sentence and hopes a reader can parse it back —
//! `rivet_core::journal::Record` carries typed fields and renders the on-disk
//! form.
//!
//! # What is here and what moved — 2026-08-24
//!
//! **The record, its encoder, its reader and the calendar are
//! `rivet_core::journal`'s now.** They left because `rivet log` (`ADR-022`
//! §9) reads back what this writes, from a *different process*, and two
//! programs holding their own copy of one format is `ADR-011`'s failure —
//! silent, because a reader that disagrees with the writer just shows a shorter
//! log. That module's own header carries the full argument.
//!
//! What is here is what a *composition root* owns: the boot identifier, the
//! correlation counter, the bounded buffer, the file on `/state`, and rotation.
//!
//! ## `ADR-022` scoreboard
//!
//! | `ADR-022` | State |
//! | :-- | :-- |
//! | §3.2 the record shape, §5 correlation and actor on every record | built — `rivet_core::journal` |
//! | §3 line-delimited JSON as the on-disk form | built, **both directions** since 2026-08-24 |
//! | §7 buffer in memory until `/state` is available | **built** — [`Journal::open_on_state`] drains it |
//! | §8 size caps, rotation, retention | **built** — [`Journal::rotate`], `rivet_core::state`'s caps |
//! | §9 query is an operation | built — `rivet_core::operation::get`, `rivet get log` |
//! | §4 capturing a child's stdout and stderr as records | **built** — `crate::kernel`'s `Capture`; a line that parses as a record is forwarded as one |
//! | §4.1 ingesting `/dev/kmsg` | **built** — `crate::kmsg`. Refuses honestly where there is no `/dev` (the QEMU initramfs; PID 1 mounts it on a board, per the maintainer's 2026-08-24 ruling) |
//!
//! ## Durability: `write(2)` per record, `fsync` never
//!
//! `ADR-022` §8 rules out an `fsync` per record — "a power cut loses the last
//! few seconds of logs, which is an acceptable trade against writing to flash
//! continuously". It does not follow that records should be *buffered in this
//! process*, and they are not.
//!
//! Each record is handed to the kernel as it is emitted and the page cache
//! coalesces them, so flash sees whole blocks rather than lines. What that buys
//! is the case this journal exists for: **the supervisor dying, or the machine
//! being restarted, does not lose the records explaining why.** `ADR-017` §3.1
//! expects this process to be killed and restarted, and `reboot(2)` syncs — so
//! the only loss is a power cut, which is exactly the trade §8 named.
//!
//! Buffering in user space would have made a supervisor crash lose its own last
//! words, which is the one failure `ADR-017` §8 item 2 says a logging system
//! must not have.

use std::collections::VecDeque;

use rivet_core::state::{
    ACTIVE_LOG, LOG_DIR, LOG_DIR_MODE, LOG_FILE_MAX_BYTES, LOG_FILE_MODE, LOG_FILES_KEPT,
    rotated_log,
};

use crate::kernel::{Errno, Kernel};

pub use rivet_core::journal::{Record, Severity, Value, rfc3339};

/// How many records the in-memory buffer holds before it starts dropping the
/// oldest.
///
/// `ADR-022` §8 caps storage because an unbounded log on an SD card is the
/// same hazard as `ADR-017` §3.5's restart loop. The buffer has the same
/// hazard in RAM, on a machine with 4–8 GB and no swap, so it is bounded here
/// too — and drops are **counted and reported**, never silent.
///
/// **It is a pre-`/state` buffer, not a write buffer.** Once
/// [`Journal::open_on_state`] has run, every record goes to the file as it is
/// emitted; the ring keeps holding the most recent ones, which costs nothing
/// and is what a future `rivet log --follow` would read.
pub const BUFFER_CAPACITY: usize = 4096;

/// What happened when the journal tried to reach `/state`.
///
/// **Never an `Err`.** The journal cannot be the thing that fails: a supervisor
/// that gave up because its log destination was missing would take the
/// explanation down with it, which is the failure `ADR-017` §8 item 2 rules
/// against. So this is a verdict the caller journals — to the console, which is
/// still there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskOutcome {
    /// Records are being written to [`ACTIVE_LOG`]. `flushed` is how many the
    /// in-memory buffer held and has now put on the card — the boot's records
    /// from before `/state` was reachable.
    Open { flushed: usize, existing_bytes: u64 },
    /// `/state` is not usable, so this boot's records are on the console and
    /// nowhere else. **Normal under QEMU** (`ADR-026` §4: there is no
    /// `/dev/mmcblk0`), and a real fault on a board.
    Refused { detail: String },
}

/// The journal: it mints correlation identifiers, stamps records, buffers them,
/// hands each to the console, and — once `/state` is reachable — appends each to
/// the card.
///
/// **What it does not do yet, stated so it is not mistaken for done:**
///
/// * ~~**It does not capture a service's stdout or stderr.**~~ **Built
///   2026-08-24** — `crate::kernel`'s `Capture`, and it went exactly the way
///   this paragraph predicted: *"spawn with piped stdio and put the read ends in
///   the same `poll(2)` set the signal descriptor is already in, which is why
///   [`crate::kernel::Kernel`]'s wait is expressed as 'return the events that
///   are ready' rather than as 'wait for a signal'."* What it did **not**
///   predict is that a dead child's last words matter most, so `reap_ready`
///   drains to EOF into a queue `wait` hands over before it polls again.
///   Closed 2026-08-24.
/// * ~~**Nothing drains [`Journal::pending`] to disk.**~~ **Built 2026-08-24.**
///   See [`Journal::open_on_state`]. The sentence this replaces had already been
///   corrected once for a stale *reason* while the gap itself was real; the gap
///   is now closed and the correction is left visible because `AGENTS.md` §7 is
///   about exactly this file's history.
/// * ~~**`/dev/kmsg` is not ingested.**~~ **Built 2026-08-24** — `crate::kmsg`,
///   and it is the last thing a console showed that `rivet get log` did not.
///   **It cannot be exercised under the emulator**: `tools/qemu/run.sh` boots
///   an initramfs, the kernel does not auto-mount devtmpfs onto one, and
///   `/dev` is therefore empty. It is proven against the build host's own
///   kernel instead — `crate::kernel`'s
///   `on_a_host_with_dev_kmsg_the_records_arrive_whole_and_parse`. Whether
///   to teach the loop a `/dev` of its own is still open, and it is the
///   maintainer's.
///
/// What is left on a console and nowhere else is **PID 1's own lines**. It
/// predates the journal, is not a service, and has nowhere to write — the
/// journal is the supervisor's and PID 1 runs before it (`ADR-017` §8 item 2).
pub struct Journal {
    ring: VecDeque<Record>,
    capacity: usize,
    dropped: u64,
    boot: String,
    /// `ADR-022` §5's actor, composing with `ADR-012` §4's provenance. It is a
    /// property of the journal rather than a parameter on every call because
    /// today there is exactly one actor: the supervisor itself. When operations
    /// arrive through an adapter (`ADR-006` §3.1) they will carry their own,
    /// and this becomes a per-record argument.
    actor: String,
    /// Distinguishes correlation identifiers minted by different supervisor
    /// runs within one boot. `ADR-017` §3.1 expects this process to be
    /// restarted, so "within one boot" is not "once".
    salt: u32,
    counter: u32,
    /// Bytes in [`ACTIVE_LOG`], once it is open. `None` before
    /// [`Journal::open_on_state`] has succeeded, and it stays `None` if that
    /// refused — so a machine with no `/state` writes to the console and never
    /// pretends otherwise.
    ///
    /// **Tracked rather than `stat`ed per record.** A `stat` per record is a
    /// syscall per record for a number this process is the only writer of.
    on_disk: Option<u64>,
    /// Counted, and reported once per episode — see
    /// [`Journal::take_disk_trouble`].
    write_failures: u64,
    unreported_failure: Option<String>,
}

impl Journal {
    pub fn new(boot: String, salt: u32, actor: String) -> Self {
        Self {
            ring: VecDeque::new(),
            capacity: BUFFER_CAPACITY,
            dropped: 0,
            boot,
            actor,
            salt,
            counter: 0,
            on_disk: None,
            write_failures: 0,
            unreported_failure: None,
        }
    }

    /// A smaller buffer, for tests that need to reach the eviction path without
    /// writing four thousand records.
    #[cfg(test)]
    pub fn with_capacity(boot: String, salt: u32, capacity: usize) -> Self {
        let mut j = Self::new(boot, salt, "test".to_string());
        j.capacity = capacity;
        j
    }

    /// Mint a correlation identifier.
    ///
    /// Unique within a boot: the salt separates supervisor runs and the counter
    /// separates causal chains within a run. `boot` on every record separates
    /// boots, so no global uniqueness is claimed or needed.
    pub fn next_correlation(&mut self) -> String {
        self.counter = self.counter.wrapping_add(1);
        format!("{:08x}{:08x}", self.salt, self.counter)
    }

    /// Records held in memory, oldest first.
    #[allow(dead_code)]
    pub fn pending(&self) -> &VecDeque<Record> {
        &self.ring
    }

    /// How many records the buffer has discarded. Reported, never silent.
    #[allow(dead_code)]
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Whether records are reaching the card.
    ///
    /// Read by the tests and by nothing else yet. `run` acts on
    /// [`DiskOutcome`] instead, which carries the same fact plus what went
    /// wrong — so this is the cheap form of the question, kept because every
    /// test about the card asks it and reaching into `on_disk` from outside
    /// would be worse.
    #[allow(dead_code)]
    pub fn writing_to_disk(&self) -> bool {
        self.on_disk.is_some()
    }

    /// Start writing to `/state`, and put everything buffered so far on the card.
    ///
    /// **Call this as early as the boot allows**, and after PID 1 has mounted
    /// `/state` (`ADR-017` §3.1.7). Everything emitted before it is in the ring
    /// and is written here, in order — which is `ADR-022` §7's
    /// "records produced before `/state` is mounted are buffered in memory and
    /// flushed once it is available", and the reason the ring exists.
    ///
    /// **Appends to whatever is there rather than truncating.** The previous
    /// boot's records are the ones worth having: `ADR-018` §5.3.3's rollback was
    /// diagnosed from a boot that had already ended, and a journal that started
    /// each boot by erasing the last one would have had nothing to say about it.
    pub fn open_on_state(&mut self, k: &mut dyn Kernel) -> DiskOutcome {
        if let Err(e) = k.mkdir_mode(LOG_DIR, LOG_DIR_MODE) {
            return DiskOutcome::Refused {
                detail: format!("{LOG_DIR}: {e}"),
            };
        }
        // A missing active file is not a fault — it is a machine whose journal
        // has never been written. `size` refuses only when the path exists and
        // cannot be measured, which is a card that is wrong in some other way.
        let existing = match k.file_size(ACTIVE_LOG) {
            Ok(n) => n,
            Err(Errno::ENOENT) => 0,
            Err(e) => {
                return DiskOutcome::Refused {
                    detail: format!("{ACTIVE_LOG}: {e}"),
                };
            }
        };
        self.on_disk = Some(existing);

        let buffered: Vec<Record> = self.ring.iter().cloned().collect();
        let mut flushed = 0;
        for record in &buffered {
            if self.append(k, record) {
                flushed += 1;
            }
        }
        DiskOutcome::Open {
            flushed,
            existing_bytes: existing,
        }
    }

    /// Say something about a failed write, once per episode.
    ///
    /// **Not emitted from inside [`Journal::emit`]**, which would recurse: a
    /// failing write reporting itself is a record, and writing that record
    /// fails. So the failure is remembered and the supervisor's loop asks for it
    /// — at which point the report goes to the console whatever the card is
    /// doing.
    ///
    /// Returns the record's fields if there was something to say, so the caller
    /// journals it in its own voice rather than this module inventing a service
    /// name.
    pub fn take_disk_trouble(&mut self) -> Option<(String, u64)> {
        self.unreported_failure
            .take()
            .map(|detail| (detail, self.write_failures))
    }

    /// Stamp, buffer and emit one record in the journal's own actor.
    pub fn emit(
        &mut self,
        k: &mut dyn Kernel,
        sev: Severity,
        svc: &str,
        corr: &str,
        msg: &str,
        fields: Vec<(String, Value)>,
    ) {
        let actor = self.actor.clone();
        self.emit_as(k, sev, svc, corr, &actor, msg, fields);
    }

    /// The same, for a record whose actor is **not** this process.
    ///
    /// [`Journal::actor`]'s own documentation predicted this call: *"it is a
    /// property of the journal rather than a parameter on every call because
    /// today there is exactly one actor … when operations arrive through an
    /// adapter they will carry their own, and this becomes a per-record
    /// argument."* `ADR-022` §11's ingest socket is that arrival, and the actor
    /// it carries is `SO_PEERCRED`'s uid — the kernel's answer rather than the
    /// sender's.
    ///
    /// **This is the only way a record with a foreign actor is made**, which is
    /// what makes the guarantee checkable: every other path through this type
    /// stamps [`Journal::actor`], so no ingested record can be dressed as one
    /// the supervisor produced.
    ///
    /// **Eight arguments**, which is one past clippy's threshold and is allowed
    /// deliberately: they are `ADR-022` §3.2's columns, and the alternative — a
    /// struct every caller builds — would be a second spelling of `Record` whose
    /// only difference is the four fields this method fills in. That is the
    /// duplication `rivet_core::journal`'s own header is about.
    #[allow(clippy::too_many_arguments)]
    pub fn emit_as(
        &mut self,
        k: &mut dyn Kernel,
        sev: Severity,
        svc: &str,
        corr: &str,
        actor: &str,
        msg: &str,
        fields: Vec<(String, Value)>,
    ) {
        let mono = k.now_ms() as f64 / 1000.0;
        let ts = k.wall_ms().map(rfc3339);
        let record = Record {
            ts,
            boot: self.boot.clone(),
            mono,
            sev,
            svc: svc.to_string(),
            corr: corr.to_string(),
            actor: actor.to_string(),
            msg: msg.to_string(),
            fields,
        };
        if self.ring.len() >= self.capacity {
            self.ring.pop_front();
            self.dropped += 1;
        }
        self.ring.push_back(record.clone());
        k.emit(&record);
        if self.on_disk.is_some() {
            self.append(k, &record);
        }
    }

    /// Append one record to the card, rotating first if it would not fit.
    ///
    /// Returns whether it landed. A failure is counted and remembered rather
    /// than propagated: `ADR-017` §8 item 2's rule is that the journal must not
    /// be the thing that fails, and a full or read-only `/state` must not stop
    /// the supervisor supervising.
    fn append(&mut self, k: &mut dyn Kernel, record: &Record) -> bool {
        let Some(size) = self.on_disk else {
            return false;
        };
        let line = record.to_jsonl();
        let bytes = line.len() as u64;

        // Rotate *before* the write that would cross the cap, not after — so
        // the cap is a cap rather than a threshold the last record overshoots.
        // A single record larger than the whole cap still gets written, into a
        // freshly rotated file: refusing it would silently lose the record that
        // was interesting enough to be that long.
        let size = if size > 0 && size + bytes > LOG_FILE_MAX_BYTES {
            match self.rotate(k) {
                Ok(()) => 0,
                Err(e) => {
                    self.note_failure(&format!("rotating {ACTIVE_LOG}: {e}"));
                    size
                }
            }
        } else {
            size
        };

        match k.append_file(ACTIVE_LOG, line.as_bytes(), LOG_FILE_MODE) {
            Ok(()) => {
                self.on_disk = Some(size + bytes);
                true
            }
            Err(e) => {
                self.note_failure(&format!("{ACTIVE_LOG}: {e}"));
                self.on_disk = Some(size);
                false
            }
        }
    }

    /// `journal.jsonl` → `journal.1.jsonl` → … → dropped.
    ///
    /// `ADR-022` §8: "size-capped files, capped total, oldest deleted first".
    /// The oldest is removed **first**, before anything is renamed onto its
    /// name, so a rotation interrupted half way leaves fewer files rather than
    /// two files claiming the same one.
    fn rotate(&mut self, k: &mut dyn Kernel) -> Result<(), Errno> {
        let oldest = rotated_log(LOG_FILES_KEPT - 1);
        match k.remove_file(&oldest) {
            Ok(()) | Err(Errno::ENOENT) => {}
            Err(e) => return Err(e),
        }
        for n in (1..LOG_FILES_KEPT - 1).rev() {
            let from = rotated_log(n);
            let to = rotated_log(n + 1);
            match k.rename(&from, &to) {
                Ok(()) | Err(Errno::ENOENT) => {}
                Err(e) => return Err(e),
            }
        }
        k.rename(ACTIVE_LOG, &rotated_log(1))?;
        self.on_disk = Some(0);
        Ok(())
    }

    fn note_failure(&mut self, detail: &str) {
        self.write_failures += 1;
        if self.unreported_failure.is_none() {
            self.unreported_failure = Some(detail.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkernel::FakeKernel;

    fn field(name: &str, v: Value) -> (String, Value) {
        (name.to_string(), v)
    }

    // -- the buffer ---------------------------------------------------------

    #[test]
    fn an_emitted_record_reaches_both_the_buffer_and_the_sink() {
        let mut k = FakeKernel::new();
        let mut j = Journal::new("boot0".to_string(), 1, "test-actor".to_string());
        j.emit(&mut k, Severity::Info, "svc", "corr", "hello", vec![]);
        assert_eq!(j.pending().len(), 1);
        assert_eq!(k.records.len(), 1);
        assert_eq!(k.records[0].msg, "hello");
    }

    #[test]
    fn a_record_is_stamped_from_the_clocks_rather_than_by_its_caller() {
        let mut k = FakeKernel::new();
        k.mono_ms = 184_221;
        k.wall_ms = Some(1_755_820_800_000);
        let mut j = Journal::new("0d3f".to_string(), 1, "test-actor".to_string());
        j.emit(&mut k, Severity::Info, "s", "c", "m", vec![]);
        let r = &k.records[0];
        assert!((r.mono - 184.221).abs() < 1e-9, "{}", r.mono);
        assert_eq!(r.ts.as_deref(), Some("2025-08-22T00:00:00.000Z"));
        assert_eq!(r.boot, "0d3f");
    }

    #[test]
    fn a_machine_with_no_readable_wall_clock_still_produces_records() {
        let mut k = FakeKernel::new();
        k.wall_ms = None;
        let mut j = Journal::new("b".to_string(), 1, "test-actor".to_string());
        j.emit(&mut k, Severity::Info, "s", "c", "m", vec![]);
        assert_eq!(k.records[0].ts, None);
    }

    #[test]
    fn the_buffer_is_bounded_and_counts_what_it_dropped() {
        let mut k = FakeKernel::new();
        let mut j = Journal::with_capacity("b".to_string(), 1, 3);
        for i in 0..5 {
            j.emit(&mut k, Severity::Info, "s", "c", &i.to_string(), vec![]);
        }
        assert_eq!(j.pending().len(), 3);
        assert_eq!(j.dropped(), 2);
        // Oldest first is what a flush would write, and the oldest two are gone.
        let kept: Vec<_> = j.pending().iter().map(|r| r.msg.clone()).collect();
        assert_eq!(kept, ["2", "3", "4"]);
    }

    #[test]
    fn correlation_identifiers_do_not_repeat_within_a_run() {
        let mut j = Journal::new("b".to_string(), 0xdead_beef, "test-actor".to_string());
        let a = j.next_correlation();
        let b = j.next_correlation();
        assert_ne!(a, b);
        assert!(a.starts_with("deadbeef"), "{a}");
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn two_supervisor_runs_in_one_boot_mint_different_identifiers() {
        // ADR-017 §3.1 expects this process to be restarted. If the salt did
        // not separate runs, the second run's correlation ids would collide
        // with the first's inside the same boot.
        let mut first = Journal::new("b".to_string(), 1, "test-actor".to_string());
        let mut second = Journal::new("b".to_string(), 2, "test-actor".to_string());
        assert_ne!(first.next_correlation(), second.next_correlation());
    }

    #[test]
    fn the_text_view_carries_severity_service_and_fields() {
        let r = Record {
            ts: None,
            boot: "b".to_string(),
            mono: 1.25,
            sev: Severity::Error,
            svc: "rivet-netd".to_string(),
            corr: "c".to_string(),
            actor: "a".to_string(),
            msg: "exited".to_string(),
            fields: vec![field("code", Value::Int(1))],
        };
        let text = r.to_text();
        assert!(text.contains("error"), "{text}");
        assert!(text.contains("rivet-netd"), "{text}");
        assert!(text.contains("code=1"), "{text}");
        assert!(text.contains("1.250"), "{text}");
    }

    // -- the card ------------------------------------------------------------

    /// Every line the fake kernel has been asked to append, as one string.
    fn on_card(k: &FakeKernel) -> String {
        k.appends
            .iter()
            .filter(|(path, _)| path == ACTIVE_LOG)
            .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
            .collect()
    }

    #[test]
    fn nothing_reaches_the_card_before_state_is_open() {
        let mut k = FakeKernel::new();
        let mut j = Journal::new("b".to_string(), 1, "supervisor".to_string());
        j.emit(&mut k, Severity::Info, "s", "c", "early", vec![]);
        assert!(!j.writing_to_disk());
        assert!(on_card(&k).is_empty());
        // And it is not lost: the console has it, and so does the ring.
        assert_eq!(k.records.len(), 1);
        assert_eq!(j.pending().len(), 1);
    }

    /// `ADR-022` §7's whole promise: the records from before `/state` existed
    /// are the ones explaining how the boot got there, and they end up on the
    /// card in order.
    #[test]
    fn opening_state_flushes_everything_buffered_before_it_in_order() {
        let mut k = FakeKernel::new();
        let mut j = Journal::new("b".to_string(), 1, "supervisor".to_string());
        for msg in ["one", "two", "three"] {
            j.emit(&mut k, Severity::Info, "s", "c", msg, vec![]);
        }
        let got = j.open_on_state(&mut k);
        assert_eq!(
            got,
            DiskOutcome::Open {
                flushed: 3,
                existing_bytes: 0
            }
        );
        let text = on_card(&k);
        let (records, refused) = rivet_core::journal::parse_lines(text.lines());
        assert!(refused.is_empty(), "{refused:?}");
        let msgs: Vec<&str> = records.iter().map(|r| r.msg.as_str()).collect();
        assert_eq!(msgs, ["one", "two", "three"]);
    }

    #[test]
    fn after_opening_every_record_goes_to_the_card_as_it_is_emitted() {
        let mut k = FakeKernel::new();
        let mut j = Journal::new("b".to_string(), 1, "supervisor".to_string());
        let _ = j.open_on_state(&mut k);
        assert!(j.writing_to_disk());
        j.emit(&mut k, Severity::Warn, "rivet-netd", "c", "late", vec![]);
        let (records, _) = rivet_core::journal::parse_lines(on_card(&k).lines());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].msg, "late");
        assert_eq!(records[0].svc, "rivet-netd");
        assert_eq!(records[0].sev, Severity::Warn);
    }

    /// The one that makes `--boot previous` possible. A journal that truncated
    /// on open would have had nothing to say about `ADR-018` §5.3.3's rollback,
    /// which was diagnosed from a boot that had already ended.
    #[test]
    fn opening_appends_to_the_previous_boots_records_rather_than_truncating() {
        let mut k = FakeKernel::new();
        k.sizes_mut().insert(ACTIVE_LOG.to_string(), 4096);
        let mut j = Journal::new("b".to_string(), 1, "supervisor".to_string());
        let got = j.open_on_state(&mut k);
        assert_eq!(
            got,
            DiskOutcome::Open {
                flushed: 0,
                existing_bytes: 4096
            }
        );
        assert!(
            !k.truncations.contains(&ACTIVE_LOG.to_string()),
            "the journal truncated the file it was meant to append to"
        );
    }

    /// **Normal under QEMU and a real fault on a board**, and the difference is
    /// the caller's to report. What matters here is that the supervisor keeps
    /// running and the console keeps getting records.
    #[test]
    fn a_state_that_will_not_take_a_directory_is_refused_and_survived() {
        let mut k = FakeKernel::new();
        k.mkdir_mode_results = crate::testkernel::Script::always(Err(Errno::EROFS));
        let mut j = Journal::new("b".to_string(), 1, "supervisor".to_string());
        let got = j.open_on_state(&mut k);
        assert!(matches!(got, DiskOutcome::Refused { .. }), "{got:?}");
        assert!(!j.writing_to_disk());

        j.emit(&mut k, Severity::Error, "s", "c", "still speaking", vec![]);
        assert_eq!(k.records.len(), 1, "the console must still get records");
        assert!(on_card(&k).is_empty());
    }

    /// A `/state` that fills mid-boot must not take the supervisor with it —
    /// `ADR-017` §8 item 2. The failure is counted and surfaced once, and the
    /// console keeps working.
    #[test]
    fn a_failing_write_is_counted_and_reported_once_rather_than_propagated() {
        let mut k = FakeKernel::new();
        let mut j = Journal::new("b".to_string(), 1, "supervisor".to_string());
        let _ = j.open_on_state(&mut k);
        k.append_results = crate::testkernel::Script::always(Err(Errno::ENOSPC));

        for i in 0..3 {
            j.emit(&mut k, Severity::Info, "s", "c", &i.to_string(), vec![]);
        }
        assert_eq!(k.records.len(), 3, "the console kept every record");

        let trouble = j.take_disk_trouble();
        assert!(trouble.is_some(), "a failed write said nothing");
        assert_eq!(trouble.map(|(_, n)| n), Some(3));
        assert_eq!(
            j.take_disk_trouble(),
            None,
            "the same episode was reported twice"
        );
    }

    /// Reporting from inside `emit` would recurse: the record describing a
    /// failed write is itself a record, whose write fails. Asserted rather than
    /// left to be discovered as a stack overflow on a full card.
    #[test]
    fn a_failing_write_does_not_emit_a_record_about_itself() {
        let mut k = FakeKernel::new();
        let mut j = Journal::new("b".to_string(), 1, "supervisor".to_string());
        let _ = j.open_on_state(&mut k);
        k.append_results = crate::testkernel::Script::always(Err(Errno::ENOSPC));
        j.emit(&mut k, Severity::Info, "s", "c", "one", vec![]);
        assert_eq!(k.records.len(), 1, "the failure emitted a record of its own");
    }

    // -- rotation ------------------------------------------------------------

    /// `ADR-022` §8, driven by writing past the cap rather than by calling
    /// `rotate` directly — a rotation nothing triggers is a rotation that never
    /// happens on a card.
    #[test]
    fn crossing_the_cap_rotates_and_the_new_file_starts_empty() {
        let mut k = FakeKernel::new();
        let mut j = Journal::new("b".to_string(), 1, "supervisor".to_string());
        k.sizes_mut()
            .insert(ACTIVE_LOG.to_string(), LOG_FILE_MAX_BYTES - 10);
        let _ = j.open_on_state(&mut k);
        j.emit(&mut k, Severity::Info, "s", "c", "the record that tips it", vec![]);

        assert!(
            k.renames
                .contains(&(ACTIVE_LOG.to_string(), rotated_log(1))),
            "{:?}",
            k.renames
        );
        // And the record still landed — rotation is not a reason to lose one.
        let (records, _) = rivet_core::journal::parse_lines(on_card(&k).lines());
        assert_eq!(records.len(), 1);
    }

    /// Oldest first, or a rename lands on a name something else still holds.
    #[test]
    fn rotation_drops_the_oldest_before_it_renames_anything_onto_that_name() {
        let mut k = FakeKernel::new();
        let mut j = Journal::new("b".to_string(), 1, "supervisor".to_string());
        k.sizes_mut()
            .insert(ACTIVE_LOG.to_string(), LOG_FILE_MAX_BYTES);
        let _ = j.open_on_state(&mut k);
        j.emit(&mut k, Severity::Info, "s", "c", "tip", vec![]);

        assert_eq!(
            k.removed.first().map(String::as_str),
            Some(rotated_log(LOG_FILES_KEPT - 1).as_str()),
            "the oldest file was not the first thing removed: {:?}",
            k.removed
        );
        // And every rename moves a file one number older, in descending order,
        // so nothing overwrites a file that has not been moved yet.
        let expected: Vec<(String, String)> = (1..LOG_FILES_KEPT - 1)
            .rev()
            .map(|n| (rotated_log(n), rotated_log(n + 1)))
            .chain(std::iter::once((
                ACTIVE_LOG.to_string(),
                rotated_log(1),
            )))
            .collect();
        assert_eq!(k.renames, expected);
    }

    /// Never more than `LOG_FILES_KEPT` files, which is what makes the total
    /// cap a cap. Driven by rotating repeatedly rather than by reading the
    /// constant back.
    #[test]
    fn the_journal_never_holds_more_files_than_it_is_allowed() {
        let mut k = FakeKernel::new();
        let mut j = Journal::new("b".to_string(), 1, "supervisor".to_string());
        let _ = j.open_on_state(&mut k);
        for i in 0..LOG_FILES_KEPT * 2 {
            k.sizes_mut()
                .insert(ACTIVE_LOG.to_string(), LOG_FILE_MAX_BYTES);
            // Force the tracked size up to the cap, as a boot's worth of
            // records would.
            j.on_disk = Some(LOG_FILE_MAX_BYTES);
            j.emit(&mut k, Severity::Info, "s", "c", &i.to_string(), vec![]);
        }
        let mut live: Vec<String> = k.live_files();
        live.retain(|p| p.starts_with(LOG_DIR));
        assert!(
            live.len() <= LOG_FILES_KEPT,
            "{} files under {LOG_DIR}: {live:?}",
            live.len()
        );
    }

    /// A rotation that cannot happen must not lose the record that triggered
    /// it. The cap is exceeded for one file, which is a smaller harm than the
    /// alternative and is the same judgement `Record::to_jsonl` makes about a
    /// duplicated field.
    #[test]
    fn a_rotation_that_fails_still_writes_the_record_and_says_so() {
        let mut k = FakeKernel::new();
        let mut j = Journal::new("b".to_string(), 1, "supervisor".to_string());
        k.sizes_mut()
            .insert(ACTIVE_LOG.to_string(), LOG_FILE_MAX_BYTES);
        let _ = j.open_on_state(&mut k);
        k.rename_results = crate::testkernel::Script::always(Err(Errno::EROFS));
        j.emit(&mut k, Severity::Info, "s", "c", "kept anyway", vec![]);

        let (records, _) = rivet_core::journal::parse_lines(on_card(&k).lines());
        assert_eq!(records.len(), 1, "the record was lost to a failed rotation");
        assert!(j.take_disk_trouble().is_some(), "the failure said nothing");
    }

    /// The round trip that matters: what this writes is what
    /// `rivet_core::journal` reads, byte for byte, including the awkward
    /// characters a message can carry.
    #[test]
    fn what_the_supervisor_writes_is_what_the_reader_reads() {
        let mut k = FakeKernel::new();
        k.wall_ms = Some(1_755_820_800_000);
        let mut j = Journal::new("boot-1".to_string(), 7, "supervisor".to_string());
        let _ = j.open_on_state(&mut k);
        j.emit(
            &mut k,
            Severity::Error,
            "rivet-netd",
            "corr-1",
            "REFUSED the document could not be applied: \"unknown key\"\nat line 15",
            vec![
                field("errno", Value::Int(2)),
                field("path", Value::Str("/state/config/machine.toml".to_string())),
                field("retried", Value::Bool(false)),
            ],
        );
        let (records, refused) = rivet_core::journal::parse_lines(on_card(&k).lines());
        assert!(refused.is_empty(), "{refused:?}");
        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r.boot, "boot-1");
        assert_eq!(r.svc, "rivet-netd");
        assert_eq!(r.sev, Severity::Error);
        assert!(r.msg.contains('\n') && r.msg.contains('"'), "{:?}", r.msg);
        assert_eq!(r.field("errno"), Some(&Value::Int(2)));
        assert_eq!(r.field("retried"), Some(&Value::Bool(false)));
    }
}
