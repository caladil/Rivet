//! `/dev/kmsg` — **`ADR-022` §4.1's third source, and the one worth the most.**
//!
//! > The journal ingests `/dev/kmsg`. This is worth more than completeness: the
//! > firmware-loading failures that `ADR-004` §8 warns will dominate Stage 2 —
//! > `brcmfmac` not finding its `clm_blob`, NVRAM under the wrong
//! > board-specific name — appear only in kernel messages. Ingesting them means
//! > the fiddliest diagnosis in the project is queryable through the same tool
//! > as everything else.
//!
//! Until this existed, `dmesg`'s content was the last thing an HDMI capture
//! showed that `rivet get log` did not — besides PID 1's own lines, which
//! have nowhere to go by construction (`ADR-017` §8 item 2).
//!
//! # The format, read out of the kernel rather than out of documentation
//!
//! `Documentation/ABI/testing/dev-kmsg` defines it and the fixture confirms it.
//! One record per `read(2)`:
//!
//! ```text
//! 6,301,124628,-;pci_bus 5582:00: root bus resource [mem 0x9ffe00000-…]
//!  SUBSYSTEM=pci_bus
//!  DEVICE=+pci_bus:5582:00
//! ```
//!
//! * `6` is **priority and facility together** — `prifac & 7` is the syslog
//!   priority and `prifac >> 3` is the facility. A reader that took the whole
//!   number as a priority would read `46` (facility 5, info) as something off
//!   the end of the scale, and the fixture contains one.
//! * `301` is a sequence number, monotonic across the boot and **not
//!   contiguous**: a reader that assumed no gaps would report loss on every
//!   machine that had any.
//! * `124628` is microseconds since boot — the kernel's own monotonic clock,
//!   which is the same clock `ADR-022` §3.2's `mono` uses and is why a kernel
//!   message can be ordered against a Rivet record at all.
//! * `-` is flags; `c` marks a fragment of a continued line.
//! * Everything after `;` up to the newline is the message. **Lines beginning
//!   with a space are key/value continuations** belonging to the record above,
//!   not records of their own.
//!
//! # Severity: eight levels onto four
//!
//! `ADR-022` §10 item 2 keeps Rivet's severity vocabulary small and closed,
//! so the kernel's eight are mapped rather than adopted. The mapping is in
//! [`severity_of`] and it is deliberately *pessimistic at the top*: `emerg`,
//! `alert`, `crit` and `err` all become `Error`, because every one of them is a
//! thing an operator has to act on and Rivet has no level above `Error` to
//! tell them apart with. The original number is kept as a field, so nothing is
//! lost — `AGENTS.md` §5, report what the system said.

use crate::journal::{Severity, Value};

/// Where the kernel's ring buffer is read from.
pub const KMSG_PATH: &str = rivet_core::state::KMSG;

/// The service name every kernel record carries.
///
/// **`rivet_core`'s constant, not a second spelling** — see
/// [`rivet_core::journal::KERNEL_SVC`] for why the name is not `rivet-sysd`.
pub const SERVICE: &str = rivet_core::journal::KERNEL_SVC;

/// One record, as far as this module is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// The syslog priority, 0–7, exactly as the kernel wrote it.
    pub priority: u8,
    /// The syslog facility. 0 is the kernel itself; anything else came from
    /// userspace through `/dev/kmsg`, which is a thing Rivet does not do and
    /// somebody else's software might.
    pub facility: u8,
    /// The kernel's sequence number. Gaps are ordinary.
    pub sequence: u64,
    /// Microseconds since boot, on the kernel's monotonic clock.
    pub mono_us: u64,
    pub text: String,
    /// The `KEY=VALUE` continuation lines, in order, unparsed.
    ///
    /// Kept whole rather than split into fields: they are the kernel's
    /// vocabulary (`SUBSYSTEM`, `DEVICE`) and turning them into record fields
    /// would put a second naming authority inside Rivet's own records. One
    /// field holding the lot says where they came from.
    pub metadata: Vec<String>,
}

impl Message {
    /// The fields this record contributes to a journal record.
    ///
    /// `mono_us` is **not** one of them: it is the kernel's reading of the same
    /// monotonic clock `ADR-022` §3.2's `mono` column carries, so putting it in
    /// a field would be the same number twice with no way to tell which was
    /// authoritative (`ADR-011`). It reaches the record as
    /// `kernel_mono`, once, because the two readings are taken at *different
    /// moments* — the kernel's when it logged, the supervisor's when it read —
    /// and the gap between them is exactly what a reader diagnosing a boot
    /// wants to see.
    pub fn fields(&self) -> Vec<(String, Value)> {
        let mut out = vec![
            ("priority".to_string(), Value::Int(i64::from(self.priority))),
            ("sequence".to_string(), Value::Int(seq_as_i64(self.sequence))),
            (
                "kernel_mono".to_string(),
                #[allow(clippy::cast_precision_loss)]
                Value::Float(self.mono_us as f64 / 1_000_000.0),
            ),
        ];
        // Facility 0 is the kernel and is the only one Rivet expects, so
        // saying so on every record would be noise. Anything else is worth
        // seeing, because it means something in userspace wrote to /dev/kmsg.
        if self.facility != 0 {
            out.push(("facility".to_string(), Value::Int(i64::from(self.facility))));
        }
        if !self.metadata.is_empty() {
            out.push((
                "kernel_metadata".to_string(),
                Value::Str(self.metadata.join(" ")),
            ));
        }
        out
    }

    pub fn severity(&self) -> Severity {
        severity_of(self.priority)
    }
}

fn seq_as_i64(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// Map a syslog priority onto `ADR-022`'s four levels.
///
/// | syslog | | Rivet |
/// | :-- | :-- | :-- |
/// | 0 `emerg`, 1 `alert`, 2 `crit`, 3 `err` | | `Error` |
/// | 4 `warning` | | `Warn` |
/// | 5 `notice`, 6 `info` | | `Info` |
/// | 7 `debug` | | `Debug` |
///
/// **Pessimistic at the top, on purpose.** Rivet has no level above `Error`,
/// so the four worst syslog levels collapse into it rather than being spread
/// across `Warn` to make the counts look better. Each of the four is a thing an
/// operator has to act on, and `priority` is kept as a field so the original is
/// never lost.
///
/// A number outside 0–7 cannot come from `prifac & 7`, so the fallthrough is
/// unreachable through [`parse`]. It answers `Error` rather than panicking:
/// this crate denies `panic` and a value nobody can explain is not a reason to
/// under-report.
pub const fn severity_of(priority: u8) -> Severity {
    match priority {
        0..=3 => Severity::Error,
        4 => Severity::Warn,
        5 | 6 => Severity::Info,
        7 => Severity::Debug,
        _ => Severity::Error,
    }
}

/// What can be wrong with a record.
///
/// Refused rather than guessed (`AGENTS.md` §5): a `/dev/kmsg` line this build
/// cannot read is a line from a kernel whose format differs, and inventing a
/// priority for it would put a plausible severity on something nobody parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// No `;`, so there is no message and nothing before it is a header.
    NoSeparator,
    /// The header has fewer than four comma-separated fields.
    ShortHeader { fields: usize },
    /// A header field that is not a number.
    NotANumber { field: &'static str, text: String },
}

impl core::fmt::Display for Fault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoSeparator => write!(
                f,
                "a /dev/kmsg record is <prifac>,<seq>,<usec>,<flags>;<message> and this has no ';'"
            ),
            Self::ShortHeader { fields } => write!(
                f,
                "the header has {fields} comma-separated fields and a /dev/kmsg record has at \
                 least four"
            ),
            Self::NotANumber { field, text } => {
                write!(f, "the {field} field is {text:?}, which is not a number")
            }
        }
    }
}

/// Read one record.
///
/// Takes the whole record including its continuation lines, because that is
/// what one `read(2)` on `/dev/kmsg` returns.
pub fn parse(record: &str) -> Result<Message, Fault> {
    let record = record.trim_end_matches('\n');
    let (header, rest) = record.split_once(';').ok_or(Fault::NoSeparator)?;

    let parts: Vec<&str> = header.split(',').collect();
    if parts.len() < 4 {
        return Err(Fault::ShortHeader {
            fields: parts.len(),
        });
    }
    let prifac = number(parts[0], "priority")?;
    let sequence = number(parts[1], "sequence")?;
    let mono_us = number(parts[2], "timestamp")?;
    // parts[3] is the flags field and parts[4..] are optional extras the ABI
    // reserves. Neither is read: `c` marks a continued fragment, and Rivet
    // records the fragment as it arrives rather than reassembling — a
    // reassembly would hold a partial record in memory waiting for a
    // continuation that a wrapped ring buffer may never deliver.

    let mut lines = rest.split('\n');
    let text = lines.next().unwrap_or("").to_string();
    let metadata: Vec<String> = lines
        .filter(|l| l.starts_with(' '))
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();

    Ok(Message {
        // `& 7` and `>> 3`, which is the whole reason this is not one number.
        priority: u8::try_from(prifac & 7).unwrap_or(7),
        facility: u8::try_from((prifac >> 3) & 0xff).unwrap_or(0),
        sequence,
        mono_us,
        text,
        metadata,
    })
}

fn number(text: &str, field: &'static str) -> Result<u64, Fault> {
    text.trim().parse::<u64>().map_err(|_| Fault::NotANumber {
        field,
        text: text.to_string(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// Records captured verbatim from a running kernel — see the file's own
    /// header for what was selected and why nothing was edited.
    const CAPTURE: &str = include_str!("../tests/fixtures/kmsg-records.txt");

    /// The fixture as `read(2)` would deliver it: one record per element,
    /// continuation lines attached to the record above.
    ///
    /// The `#` lines are the fixture's own commentary and are not part of the
    /// capture; a real `/dev/kmsg` has none.
    fn records() -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for line in CAPTURE.lines() {
            if line.starts_with('#') {
                continue;
            }
            if line.starts_with(' ') {
                if let Some(last) = out.last_mut() {
                    last.push('\n');
                    last.push_str(line);
                }
                continue;
            }
            out.push(line.to_string());
        }
        out
    }

    fn parsed() -> Vec<Message> {
        records()
            .iter()
            .map(|r| parse(r).unwrap_or_else(|e| panic!("the kernel wrote {r:?} and: {e}")))
            .collect()
    }

    #[test]
    fn every_record_a_real_kernel_wrote_parses() {
        let got = parsed();
        assert_eq!(got.len(), 6, "the fixture shrank: {got:#?}");
        assert!(got.iter().all(|m| !m.text.is_empty()));
        assert!(got.iter().all(|m| m.priority <= 7));
    }

    /// **The one a reader gets wrong.** `prifac` is priority *and* facility, so
    /// `46` is facility 5 at priority 6 — info — and not something off the end
    /// of the scale. The fixture carries two records with a non-zero facility
    /// for exactly this.
    #[test]
    fn priority_and_facility_are_separated_rather_than_read_as_one_number() {
        let got = parsed();
        let userspace: Vec<&Message> = got.iter().filter(|m| m.facility != 0).collect();
        assert_eq!(userspace.len(), 2, "{got:#?}");
        for m in userspace {
            assert!(m.priority <= 7, "{m:?}");
            assert_eq!(m.severity(), Severity::Info, "{m:?}");
        }
        // 46 = facility 5, priority 6.
        let journald = got
            .iter()
            .find(|m| m.text.starts_with("systemd-journald"))
            .expect("the fixture carries a facility-5 record");
        assert_eq!((journald.facility, journald.priority), (5, 6));
    }

    #[test]
    fn the_error_and_warning_records_map_onto_rivets_levels() {
        let got = parsed();
        let err = got
            .iter()
            .find(|m| m.text.starts_with("PCI: Fatal"))
            .expect("the fixture carries a priority-3 record");
        assert_eq!(err.severity(), Severity::Error);
        let warn = got
            .iter()
            .find(|m| m.text.contains("Disabling IBT"))
            .expect("the fixture carries a priority-4 record");
        assert_eq!(warn.severity(), Severity::Warn);
        let debug = got
            .iter()
            .find(|m| m.text.starts_with("e820:"))
            .expect("the fixture carries a priority-7 record");
        assert_eq!(debug.severity(), Severity::Debug);
    }

    /// The kernel's own monotonic reading survives, in seconds, because it is
    /// what orders a kernel message against a Rivet record.
    #[test]
    fn the_kernels_timestamp_and_sequence_reach_the_record_as_fields() {
        let got = parsed();
        let m = got
            .iter()
            .find(|m| m.text.starts_with("PCI: Fatal"))
            .expect("present");
        assert_eq!(m.sequence, 224);
        assert_eq!(m.mono_us, 83_932);
        let fields = m.fields();
        assert!(
            fields
                .iter()
                .any(|(n, v)| n == "kernel_mono" && matches!(v, Value::Float(f) if (*f - 0.083_932).abs() < 1e-9)),
            "{fields:?}"
        );
        assert!(
            fields
                .iter()
                .any(|(n, v)| n == "sequence" && *v == Value::Int(224)),
            "{fields:?}"
        );
    }

    /// A device message's `SUBSYSTEM=` and `DEVICE=` lines belong to the record
    /// above them and are not records of their own. A reader that treated them
    /// as records would report three PCI messages where the kernel wrote one.
    #[test]
    fn continuation_lines_belong_to_the_record_above_them() {
        let got = parsed();
        let pci = got
            .iter()
            .find(|m| m.text.starts_with("pci_bus"))
            .expect("the fixture carries a record with continuations");
        assert_eq!(pci.metadata, ["SUBSYSTEM=pci_bus", "DEVICE=+pci_bus:5582:00"]);
        assert!(!pci.text.contains("SUBSYSTEM"), "{pci:?}");
        assert!(
            pci.fields()
                .iter()
                .any(|(n, _)| n == "kernel_metadata"),
            "{:?}",
            pci.fields()
        );
    }

    /// Facility 0 is the kernel and is what almost every record is, so saying
    /// so on each one would be noise. Anything else means userspace wrote to
    /// `/dev/kmsg`, which Rivet does not do and is worth seeing.
    #[test]
    fn only_a_record_that_is_not_the_kernels_carries_a_facility_field() {
        for m in parsed() {
            let named = m.fields().iter().any(|(n, _)| n == "facility");
            assert_eq!(named, m.facility != 0, "{m:?}");
        }
    }

    // -- from the specification, and said to be ------------------------------
    //
    // A healthy machine does not produce priorities 0, 1 or 2, so the fixture
    // has none and these are derived from `Documentation/ABI/testing/dev-kmsg`
    // rather than captured. `AGENTS.md` §4 asks for the difference to be
    // visible, and this comment is it.

    #[test]
    fn the_four_worst_syslog_levels_all_become_error() {
        for priority in 0..=3u8 {
            assert_eq!(severity_of(priority), Severity::Error, "priority {priority}");
        }
        assert_eq!(severity_of(4), Severity::Warn);
        assert_eq!(severity_of(5), Severity::Info);
        assert_eq!(severity_of(6), Severity::Info);
        assert_eq!(severity_of(7), Severity::Debug);
    }

    #[test]
    fn an_emergency_parses_and_is_an_error() {
        let m = parse("0,1,2,-;Kernel panic - not syncing: Attempted to kill init!")
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(m.priority, 0);
        assert_eq!(m.facility, 0);
        assert_eq!(m.severity(), Severity::Error);
    }

    /// A sequence number that skips is ordinary — the kernel drops records
    /// under pressure and the buffer wraps. A reader that reported loss on
    /// every gap would report it on every machine.
    #[test]
    fn sequence_numbers_are_read_and_not_checked_for_gaps() {
        let a = parse("6,10,0,-;one").unwrap_or_else(|e| panic!("{e}"));
        let b = parse("6,900,0,-;two").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((a.sequence, b.sequence), (10, 900));
    }

    /// The `c` flag marks a fragment of a continued line. It is recorded as it
    /// arrives rather than held for reassembly: a reassembly would keep a
    /// partial record in memory waiting for a continuation that a wrapped ring
    /// buffer may never deliver.
    #[test]
    fn a_continued_fragment_is_a_record_rather_than_a_wait() {
        let m = parse("6,7,8,c;half a line").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(m.text, "half a line");
    }

    /// Extra header fields are reserved by the ABI and must not make a record
    /// unreadable on a newer kernel.
    #[test]
    fn a_header_with_more_fields_than_this_build_knows_still_parses() {
        let m = parse("6,7,8,-,caller=T123;something").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(m.text, "something");
        assert_eq!(m.priority, 6);
    }

    // -- and every way a record can be wrong ---------------------------------

    #[test]
    fn a_line_with_no_separator_is_refused_by_name() {
        assert_eq!(parse("no semicolon here"), Err(Fault::NoSeparator));
    }

    #[test]
    fn a_short_header_is_refused_rather_than_padded() {
        assert_eq!(parse("6,7;text"), Err(Fault::ShortHeader { fields: 2 }));
    }

    #[test]
    fn a_header_field_that_is_not_a_number_names_which_one() {
        assert!(matches!(
            parse("six,7,8,-;text"),
            Err(Fault::NotANumber { field: "priority", .. })
        ));
        assert!(matches!(
            parse("6,seven,8,-;text"),
            Err(Fault::NotANumber { field: "sequence", .. })
        ));
        assert!(matches!(
            parse("6,7,soon,-;text"),
            Err(Fault::NotANumber { field: "timestamp", .. })
        ));
    }

    /// An empty message is a record the kernel really writes — a bare newline
    /// in printk output — and it must not be a parse failure.
    #[test]
    fn an_empty_message_is_a_record_rather_than_a_fault() {
        let m = parse("6,7,8,-;").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(m.text, "");
    }
}
