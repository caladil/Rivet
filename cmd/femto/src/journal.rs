//! **What this editor tells the machine it did** — `ADR-022` §11, ruled
//! 2026-08-24.
//!
//! `rivet apply edit` has recorded every change it makes since `ADR-006`
//! §8.9, and `femto` did not — so *"who changed this file, and when"* had an
//! answer for the agent-facing operation and none for the one a human actually
//! uses at 2am. That is the wrong way round: the interactive editor is where an
//! unrecorded change is most likely, because it is where somebody is improvising.
//!
//! # It sends; it does not write
//!
//! `ADR-022` §7 gives the journal exactly one writer and this is not it. What
//! this module does is compose a record and hand it to
//! `rivet_core::journal::INGEST_SOCKET_PATH`, which is the supervisor's, and
//! the supervisor stamps `boot`, `mono`, `ts`, `corr` and `actor` out of its own
//! clocks before writing. So the strongest thing this program can put in the
//! journal is a severity, a message and some fields — see
//! `rivet_core::journal::ingest`, which is the one account of that split.
//!
//! # The floor is stderr, and for this program that means *after* the terminal
//!
//! `ADR-022` §11.5 makes stderr the fallback when nothing is listening, and
//! `cmd/rivet` takes it immediately because its stderr is a scrolling session.
//! **This program cannot**: it is in raw mode on an alternate screen, and a line
//! written to stderr mid-session paints over the buffer the operator is editing
//! and is then thrown away when the alternate screen is left.
//!
//! So an unfiled record is **held** ([`Unfiled`]) and printed by `crate::main`
//! after the terminal has been restored. The operator sees the same JSONL line
//! `cmd/rivet` would have printed, on a screen that is theirs again, and the
//! status line says so at the moment of the write rather than at the end.
//!
//! # The seam
//!
//! [`Journal`] is **an adapter's own I/O seam, not a domain port**
//! (`AGENTS.md` §3.1): its vocabulary is a socket, a clock and a boot
//! identifier. It sits beside `crate::port`'s two for the same reason those
//! exist — a machine with no supervisor listening is a state a development host
//! does not reach on demand, and it is the state that matters most here.

use crate::editor::Wrote;

/// The `svc` every record this program files carries.
///
/// **The program's own name and not `rivet`**, which is the whole point:
/// `rivet get log | where svc == femto` answers *what did somebody change by
/// hand*, and a shared name would fold it into the operation's records.
pub const SELF_NAME: &str = "femto";

/// `ADR-022` §5's actor.
///
/// **`operator`, the same word `rivet apply edit` uses**, and for the reason
/// that module gives: this process cannot tell a human at a terminal from an
/// agent on the control plane, so the honest actor is *the thing that performed the write*.
/// Who is the `principal` field, which is a claim and is labelled as one
/// (`ADR-028` §4.5).
///
/// **This is not what ends up in the record's `actor` column**, and that is the
/// design rather than a loss. The supervisor overrides `actor` with
/// `SO_PEERCRED`'s uid (`ADR-022` §11.4) — the kernel's answer, which is
/// strictly better than this word — and unlike `corr`, `mono` and `boot` it is
/// not kept as an `emitted_*` field, because *"the administration surface did
/// it"* adds nothing to a record that already says `svc: femto`.
///
/// It is set anyway because what goes on the wire is a **complete** `ADR-022`
/// §3.2 record — the same bytes `to_jsonl` writes to a file — and every column
/// of one has to carry something true.
pub const ACTOR: &str = "operator";

/// Where the kernel publishes the boot identifier — `ADR-022` §3.2's `boot`.
///
/// **Re-exported from `rivet-core` rather than spelled again.** `PLAN.md`
/// §10.8 is why: `cmd/rivet-deploy` truncated this value to eight characters
/// where the supervisor wrote it whole, `rivet_core::journal::boots_in_order`
/// counts distinct `boot` fields, and a journal holding both therefore reported
/// two boots where the machine had one. That was fixed by moving the *rule* into
/// the domain, and a new sender that copied the path back out would be the first
/// step towards doing it again.
pub use rivet_core::journal::BOOT_ID_PATH;

/// What a record needs from the machine that only the machine can say.
///
/// `cmd/rivet`'s `edit::Stamp`, in the crate that cannot import it — two
/// binary crates have no library target between them, which is the same
/// constraint `rivet_core::deploy` was created for. What is *shared* is the
/// part that could be: the record type, the encoder, and
/// `rivet_core::journal::boot_identifier`'s rule about an unreadable file.
#[derive(Debug, Clone, PartialEq)]
pub struct Stamp {
    /// `/proc/sys/kernel/random/boot_id`, whole, or `unknown` on a machine that
    /// will not answer — never a synthesised identifier, which would splice two
    /// boots together for a reader.
    pub boot: String,
    /// Seconds since boot, monotonic.
    pub mono: f64,
    /// Milliseconds since the Unix epoch, or `None` on a machine with no
    /// readable wall clock — the ordinary state of a cold Pi (`ADR-021`).
    pub wall_ms: Option<u64>,
}

/// Everything this program needs from the machine's journal.
pub trait Journal {
    /// Which boot this is and what time it is.
    fn stamp(&mut self) -> Stamp;

    /// File one record, in the authoritative form
    /// (`rivet_core::journal::Record::to_jsonl`). Returns why it could not.
    fn send(&mut self, line: &str) -> Result<(), String>;
}

/// A record that did not reach the journal, kept so that it can.
///
/// **The line itself, not a summary.** An operator can paste it back, and an
/// agent reading the session gets the same structure `rivet get log` would
/// have given it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unfiled {
    pub line: String,
    pub why: String,
}

/// Compose the record for one write.
///
/// **Pure**, and that is what makes every decision in it reachable from a test
/// with no socket and no `/proc`: which fields a write is described by, that the
/// principal is a claim, and that a count is a number rather than a sentence.
#[must_use]
pub fn record(wrote: &Wrote, stamp: &Stamp) -> rivet_core::journal::Record {
    use rivet_core::journal::{Record, Severity, Value, rfc3339};

    // One write, one chain. Minted from the monotonic clock, as
    // `cmd/rivet`'s edit and `cmd/rivet-deploy`'s salt are and for the same
    // reason: monotone within a boot, so two runs that are not simultaneous
    // cannot collide. The supervisor mints its own for the record it writes and
    // keeps this one as `emitted_corr`.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let corr = format!("{:08x}{:08x}", (stamp.mono * 1000.0) as u32, 1u32);

    let principal = this_process();
    let mut fields = vec![
        ("path".to_string(), Value::Str(wrote.path.clone())),
        ("after".to_string(), Value::Int(count(wrote.bytes))),
        ("lines".to_string(), Value::Int(count(wrote.lines))),
        (
            "principal".to_string(),
            Value::Str(principal.origin().as_token().to_string()),
        ),
    ];
    // **Only when it happened**, so an ordinary `^O` does not carry a field
    // whose value is always the same word.
    if wrote.renamed {
        fields.push((
            "opened_as".to_string(),
            Value::Str(wrote.opened_as.clone()),
        ));
    }

    Record {
        ts: stamp.wall_ms.map(rfc3339),
        boot: stamp.boot.clone(),
        mono: stamp.mono,
        sev: Severity::Info,
        svc: SELF_NAME.to_string(),
        corr,
        actor: ACTOR.to_string(),
        msg: format!(
            "wrote {path}: {bytes} bytes, {lines} line{s}{from}",
            path = wrote.path,
            bytes = wrote.bytes,
            lines = wrote.lines,
            s = if wrote.lines == 1 { "" } else { "s" },
            from = if wrote.renamed {
                format!(", opened as {}", wrote.opened_as)
            } else {
                String::new()
            },
        ),
        fields,
    }
}

/// A count as a field value, saturating rather than wrapping.
///
/// `usize` to `i64` cannot wrap on any target this ships for, and the cast that
/// would is the one this crate denies the lint for — so it is written as a
/// saturation, which is what `cmd/rivet` does with the same number.
fn count(n: usize) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// What this process can honestly say about its own invocation.
///
/// `rivet_core::deploy::Principal::from_environment`'s rule, reached the same
/// way `cmd/rivet::deploy::this_process` reaches it — **the same function,
/// not a second reading**, so an edit made with `femto` and an install asked for
/// in the same session name one client.
///
/// **It is self-reported and says so.** The pinned peer key arrives in an
/// environment variable and this process's own environment is writable by
/// whoever started it; `ADR-028` §4.5 is careful that this is attribution and
/// not authorisation. The record's kernel-verified half is the supervisor's
/// `ingest_uid`/`ingest_pid`, and the two answer different questions —
/// `rivet_core::journal::Peer` carries the comparison.
fn this_process() -> rivet_core::deploy::Principal {
    rivet_core::deploy::Principal::from_environment(
        std::env::var(rivet_core::deploy::PEER_KEY_ENV)
            .ok()
            .as_deref(),
    )
}

/// File a record of one write, or hand back the record that could not be filed.
pub fn file(journal: &mut dyn Journal, wrote: &Wrote) -> Option<Unfiled> {
    let stamp = journal.stamp();
    let line = record(wrote, &stamp).to_jsonl();
    let why = journal.send(&line).err()?;
    Some(Unfiled { line, why })
}

// ---------------------------------------------------------------------------
// The machine's own journal
// ---------------------------------------------------------------------------

/// [`Journal`] over the socket the supervisor listens on.
///
/// Stateless: every fact it deals in is read at the point of use
/// (`AGENTS.md` §5), and it holds no connection open — one write, one
/// connection, closed.
pub struct HostJournal;

impl Journal for HostJournal {
    fn stamp(&mut self) -> Stamp {
        Stamp {
            boot: rivet_core::journal::boot_identifier(
                std::fs::read_to_string(BOOT_ID_PATH).map_err(|_| ()),
            ),
            mono: monotonic_seconds(),
            wall_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()
                .and_then(|d| u64::try_from(d.as_millis()).ok()),
        }
    }

    fn send(&mut self, line: &str) -> Result<(), String> {
        #[cfg(unix)]
        {
            use std::io::Write as _;

            if line.len() > rivet_core::journal::INGEST_MAX_BYTES {
                return Err(format!(
                    "the record is {} bytes and the ingest socket takes at most {}",
                    line.len(),
                    rivet_core::journal::INGEST_MAX_BYTES
                ));
            }
            let mut socket =
                std::os::unix::net::UnixStream::connect(rivet_core::journal::INGEST_SOCKET_PATH)
                    .map_err(|e| e.to_string())?;
            socket.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
            drop(socket);
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = line;
            Err("this platform has no AF_UNIX sockets, and no Rivet journal to reach".to_string())
        }
    }
}

/// The monotonic clock in seconds, or `0.0` where it cannot be read.
///
/// `0.0` rather than a refusal, on `cmd/rivet`'s reasoning: a record with a
/// wrong `mono` is worse than one with a zero, because zero is visibly not a
/// time since boot and a plausible number is not. The supervisor stamps its own
/// `mono` on the record it writes, so this only reaches `emitted_mono`.
#[allow(clippy::cast_precision_loss)]
fn monotonic_seconds() -> f64 {
    #[cfg(target_os = "linux")]
    {
        match crate::syscall::monotonic_nanos() {
            Ok(nanos) => (nanos as f64) / 1_000_000_000.0,
            Err(_) => 0.0,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        0.0
    }
}

#[cfg(test)]
pub mod testing {
    //! A journal in a `Vec`, which can also be told to refuse. Compiled only
    //! under `cfg(test)`.

    use super::{Journal, Stamp};

    #[derive(Debug, Default)]
    pub struct MemoryJournal {
        /// Every line handed to [`Journal::send`], in order — **including the
        /// refused ones**, because "it did not try to file a record" and "it
        /// tried and nothing was listening" are different defects.
        pub sent: Vec<String>,
        /// What every `send` answers with, so the supervisor-is-down case is
        /// driven rather than described.
        pub refuses: Option<String>,
    }

    impl MemoryJournal {
        /// A machine whose supervisor is not listening.
        pub fn refusing() -> MemoryJournal {
            MemoryJournal {
                sent: Vec::new(),
                refuses: Some("No such file or directory".to_owned()),
            }
        }

        /// The lines that actually reached the journal.
        pub fn filed(&self) -> Vec<String> {
            if self.refuses.is_some() {
                return Vec::new();
            }
            self.sent.clone()
        }
    }

    impl Journal for MemoryJournal {
        fn stamp(&mut self) -> Stamp {
            Stamp {
                boot: "0d3f2a11-0000-0000-0000-000000000000".to_owned(),
                mono: 12.5,
                wall_ms: Some(1_755_820_800_000),
            }
        }

        fn send(&mut self, line: &str) -> Result<(), String> {
            self.sent.push(line.to_owned());
            match &self.refuses {
                Some(why) => Err(why.clone()),
                None => Ok(()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::testing::MemoryJournal;
    use super::*;
    use rivet_core::journal::{Record, Severity, Value};

    fn wrote() -> Wrote {
        Wrote {
            path: "/state/notes.txt".to_owned(),
            opened_as: "/state/notes.txt".to_owned(),
            renamed: false,
            bytes: 18,
            lines: 2,
        }
    }

    fn stamp() -> Stamp {
        MemoryJournal::default().stamp()
    }

    /// **The record is one the journal's own reader accepts**, which is what
    /// makes this a record rather than a hope — and it is the same parser
    /// `rivet get log` uses, not a second one.
    #[test]
    fn a_write_becomes_a_record_the_journals_own_reader_reads_back() {
        let line = record(&wrote(), &stamp()).to_jsonl();
        let read = Record::from_jsonl(&line).expect("the journal's reader must accept it");
        assert_eq!(read.svc, SELF_NAME);
        assert_eq!(read.actor, ACTOR);
        assert_eq!(read.sev, Severity::Info);
        assert_eq!(
            read.field("path"),
            Some(&Value::Str("/state/notes.txt".to_owned()))
        );
        assert_eq!(read.field("after"), Some(&Value::Int(18)));
        assert_eq!(read.field("lines"), Some(&Value::Int(2)));
    }

    /// The boot identifier is the whole value, never truncated — two spellings
    /// of one boot make `rivet get log --boot previous` select the wrong
    /// records (`PLAN.md` §10.8).
    #[test]
    fn the_boot_identifier_is_carried_whole() {
        let read = record(&wrote(), &stamp());
        assert_eq!(read.boot, "0d3f2a11-0000-0000-0000-000000000000");
    }

    /// `^O` to a different name says so, and an ordinary write does not carry a
    /// field whose value would always be the same.
    #[test]
    fn writing_under_a_new_name_is_in_the_record_and_an_ordinary_write_is_not() {
        let plain = record(&wrote(), &stamp());
        assert_eq!(plain.field("opened_as"), None);
        assert!(!plain.msg.contains("opened as"), "{}", plain.msg);

        let mut renamed = wrote();
        renamed.path = "/state/copy.txt".to_owned();
        renamed.renamed = true;
        let record = record(&renamed, &stamp());
        assert_eq!(
            record.field("opened_as"),
            Some(&Value::Str("/state/notes.txt".to_owned()))
        );
        assert!(record.msg.contains("opened as /state/notes.txt"), "{}", record.msg);
    }

    /// A machine that is listening takes the record and there is nothing held.
    #[test]
    fn a_filed_record_leaves_nothing_to_report() {
        let mut journal = MemoryJournal::default();
        assert_eq!(file(&mut journal, &wrote()), None);
        assert_eq!(journal.filed().len(), 1);
    }

    /// **A machine whose supervisor is not listening keeps the record**, with
    /// the reason — which is what `crate::main` prints once the terminal is the
    /// operator's again. Losing it here would be the one failure `ADR-022` §11.5
    /// exists to prevent.
    #[test]
    fn a_record_that_could_not_be_filed_is_held_whole_with_the_reason() {
        let mut journal = MemoryJournal::refusing();
        let held = file(&mut journal, &wrote()).expect("an unfiled record must be handed back");
        assert!(held.why.contains("No such file or directory"), "{}", held.why);
        Record::from_jsonl(&held.line).expect("what is held is still a record");
        assert_eq!(journal.sent.len(), 1, "it never tried");
        assert!(journal.filed().is_empty());
    }

    /// A record bigger than the socket takes is refused by the real journal
    /// before anything is sent, so the line reaches the operator rather than
    /// producing a refusal at the far end about bytes nobody kept.
    #[test]
    fn the_real_journal_refuses_a_record_larger_than_the_socket_takes() {
        let huge = "x".repeat(rivet_core::journal::INGEST_MAX_BYTES + 1);
        match HostJournal.send(&huge) {
            Err(why) => assert!(why.contains("at most") || why.contains("AF_UNIX"), "{why}"),
            Ok(()) => panic!("an oversized record was sent"),
        }
    }

    /// **The real journal answers honestly on a workstation.** There is no
    /// supervisor here, so `send` must report that rather than succeeding —
    /// a stub that returned `Ok(())` would report a record filed on a machine
    /// with no journal at all (`AGENTS.md` §3).
    #[test]
    fn the_real_journal_refuses_where_there_is_no_supervisor_listening() {
        assert!(
            !std::path::Path::new(rivet_core::journal::INGEST_SOCKET_PATH).exists(),
            "something on this host is listening on the Rivet journal socket"
        );
        assert!(HostJournal.send("{}\n").is_err());
    }

    /// And its stamp is read from the machine rather than invented.
    #[test]
    fn the_real_stamp_is_read_from_the_machine_and_never_synthesised() {
        let stamp = HostJournal.stamp();
        assert!(!stamp.boot.is_empty());
        assert!(stamp.mono >= 0.0);
        let from_kernel = std::fs::read_to_string(BOOT_ID_PATH).is_ok();
        assert_eq!(stamp.boot == "unknown", !from_kernel);
    }
}
