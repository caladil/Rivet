//! What `get process` answers, and what it must not flatten.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::machine::{MachineError, ProcessInfo, ProcessRead};

struct Fake(Result<ProcessRead, MachineError>);

impl crate::operation::machine::Observed for Fake {
    /// **Refuses rather than answering that this machine trusts nobody.**
    /// An empty trust store is a REAL reading -- it is what `ADR-028` §5.5.5's
    /// `--no-trust` image produces -- and this fake has not taken it. The
    /// direction is the dangerous one: *"trusts nobody"* is the answer
    /// somebody acts on by re-signing an artifact.
    fn read_trust(&mut self) -> Result<crate::operation::machine::TrustRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering an idle machine.** Zeroed counters
    /// are what a machine that has run nothing since it booted looks like,
    /// and a load average of zero is what an idle one looks like — both are
    /// readings, and this fake has taken neither (`AGENTS.md` §3, §5).
    fn read_cpu(&mut self) -> Result<crate::operation::machine::CpuRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering that this machine has no block
    /// devices**, which is not a state a machine that booted off one can
    /// be in.
    fn read_devices(&mut self) -> Result<crate::operation::machine::DeviceRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering an empty network.** No interfaces at
    /// all reads as a machine with none, which is not a state a running
    /// Linux is in.
    fn read_network(&mut self) -> Result<crate::operation::machine::NetworkRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than naming a slot.** Naming the wrong one is how
    /// an update overwrites the running system, so a fake that observes
    /// nothing names none.
    fn read_slots(&mut self) -> Result<crate::operation::machine::SlotRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering an uptime of zero.** `0` would say
    /// this machine booted a moment ago, which is a reading nobody took.
    fn read_boot(&mut self) -> Result<crate::operation::machine::BootRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering with unset flags.** Reporting a
    /// `Throttle` of all-`false` would assert that this machine has never
    /// been under-volted since it booted — the one claim this subject must
    /// never make without a measurement (`AGENTS.md` §3, §5).
    fn read_thermal(
        &mut self,
    ) -> Result<crate::operation::machine::ThermalRead, MachineError> {
        Err(MachineError::Refused(
                "not this fake's port; unread throttle flags are never reported as false"
                    .to_string(),
        ))
    }
    fn read_processes(&mut self) -> Result<ProcessRead, MachineError> {
        match &self.0 {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(MachineError::Refused(e.why().to_string())),
        }
    }
    fn read_memory(&mut self) -> Result<crate::operation::machine::MemoryRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_desired_state(&mut self) -> Result<Option<String>, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_journal(&mut self) -> Result<crate::operation::machine::JournalRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_filesystems(&mut self) -> Result<crate::operation::machine::StorageRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn configuration_schema(&mut self) -> Value {
        Value::str("<not this test's port>")
    }
    fn supervisor_status(&mut self) -> Result<crate::operation::status::Status, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
}

fn one(pid: u32, name: &str, kernel: bool) -> ProcessInfo {
    ProcessInfo {
        pid,
        parent_pid: 1,
        name: name.to_string(),
        state: "S".to_string(),
        threads: 1,
        resident_bytes: if kernel { None } else { Some(4096) },
        kernel,
        // Hand-composed, and deliberately non-round: `pid * 10` gives every
        // fixture process a distinct figure, so a test that swapped two rows
        // would show it rather than passing on two equal numbers.
        user_millis: Some(u64::from(pid) * 10),
        system_millis: Some(u64::from(pid)),
        // **A kernel thread is parked too, and makes no syscall** — which is
        // the pairing the rendering has to get right, since one is present
        // without the other.
        wchan: Some(if kernel { "rescuer_thread" } else { "do_sys_poll" }.to_string()),
        syscall: if kernel {
            None
        } else {
            Some("73 0x7fad5b2f90 0x1d 0x0 0x0".to_string())
        },
    }
}

/// The same process on a machine whose `/proc/<pid>/stat` could not be read —
/// **the case the `Option` exists for**, and the one a zero would erase.
fn without_cpu_time(mut process: ProcessInfo) -> ProcessInfo {
    process.user_millis = None;
    process.system_millis = None;
    process
}

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(fields) => fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v),
        other => panic!("not an object: {other:?}"),
    }
}

fn number(value: &Value, key: &str) -> i64 {
    match field(value, key) {
        Some(Value::Integer(n)) => *n,
        other => panic!("{key} is not an integer: {other:?}"),
    }
}

fn rows(value: &Value) -> &[Value] {
    match field(value, "processes") {
        Some(Value::List(items)) => items,
        other => panic!("processes is not a list: {other:?}"),
    }
}

/// **`userland` counts what is not a kernel thread**, which is the cheapest
/// anomaly check this surface offers: on a machine with no shell, that list
/// should be short and wholly accounted for.
#[test]
fn userland_excludes_kernel_threads() {
    let mut fake = Fake(Ok(ProcessRead {
        processes: vec![
            one(1, "rivet-initd", false),
            one(2, "kthreadd", true),
            one(9, "kworker/0:1", true),
            one(120, "rivet-sysd", false),
        ],
        vanished: 0,
    }));
    let value = perform(&["process"], &mut fake).expect("a table");
    assert_eq!(number(&value, "userland"), 2);
    assert_eq!(rows(&value).len(), 4, "every process is still reported");
}

/// **A kernel thread has no `resident_bytes` field at all**, rather than a
/// zero: it has no user address space, and `0` would be a claim that it
/// occupies none — a different statement, and a false one.
#[test]
fn a_kernel_thread_omits_resident_bytes_rather_than_reporting_zero() {
    let mut fake = Fake(Ok(ProcessRead {
        processes: vec![one(2, "kthreadd", true), one(120, "rivet-sysd", false)],
        vanished: 0,
    }));
    let value = perform(&["process"], &mut fake).expect("a table");
    let all = rows(&value);
    assert!(field(&all[0], "resident_bytes").is_none(), "a zero was invented");
    assert_eq!(number(&all[1], "resident_bytes"), 4096);
}

/// **Vanished processes are counted, not hidden.** `/proc` changes while it is
/// read, and a count that silently shrinks is one nobody can reason about.
#[test]
fn processes_that_vanished_mid_read_are_reported() {
    let mut fake = Fake(Ok(ProcessRead {
        processes: vec![one(1, "rivet-initd", false)],
        vanished: 3,
    }));
    let value = perform(&["process"], &mut fake).expect("a table");
    assert_eq!(number(&value, "vanished"), 3);
}

/// Ascending pid order is carried through, so two readings of an unchanged
/// machine produce the same bytes.
#[test]
fn the_order_the_port_gave_is_preserved() {
    let mut fake = Fake(Ok(ProcessRead {
        processes: vec![one(1, "a", false), one(2, "b", true), one(30, "c", false)],
        vanished: 0,
    }));
    let value = perform(&["process"], &mut fake).expect("a table");
    let pids: Vec<i64> = rows(&value).iter().map(|r| number(r, "pid")).collect();
    assert_eq!(pids, vec![1, 2, 30]);
}

/// A machine that cannot answer says so rather than reporting an empty table,
/// which would be a claim that nothing is running.
#[test]
fn a_port_that_refuses_is_not_a_machine_running_nothing() {
    let mut fake = Fake(Err(MachineError::Refused("no /proc".to_string())));
    let error = perform(&["process"], &mut fake).expect_err("must not report an empty table");
    assert_eq!(error.code(), ErrorCode::MachineRefused);
}

/// A second word is refused rather than ignored — `schema`'s precedent.
#[test]
fn a_second_word_is_refused() {
    let mut fake = Fake(Ok(ProcessRead { processes: Vec::new(), vanished: 0 }));
    let error = perform(&["process", "1"], &mut fake).expect_err("refused");
    assert_eq!(error.code(), ErrorCode::Arguments);
}

/// **The two counters that make a per-process CPU figure possible at all**, and
/// they are counters: cumulative milliseconds since the process started, not a
/// share of anything. Added 2026-08-29 — before them a caller could read this
/// table a thousand times and still have no CPU number, because everything else
/// in a row is a STATE and two readings of a state are two states.
#[test]
fn a_process_row_carries_the_cpu_time_a_rate_is_computed_from() {
    let mut fake = Fake(Ok(ProcessRead {
        processes: vec![one(120, "rivet-sysd", false)],
        vanished: 0,
    }));
    let value = perform(&["process"], &mut fake).expect("a table");
    let row = rows(&value).first().expect("one process");
    assert_eq!(number(row, "user_millis"), 1200);
    assert_eq!(number(row, "system_millis"), 120);
}

/// **Unread CPU time is absent, never zero** — and the reason is sharper here
/// than for `resident_bytes`: zero milliseconds of CPU is a REAL reading of a
/// process that has been scheduled and done nothing, so a zero standing in for
/// *not measured* would be indistinguishable from a measurement. The two
/// figures come from a different file in `/proc` than the rest of the row and
/// can go missing on their own, which is what this fixture is.
#[test]
fn a_process_whose_cpu_time_was_not_read_omits_it_rather_than_reporting_zero() {
    let mut fake = Fake(Ok(ProcessRead {
        processes: vec![without_cpu_time(one(120, "rivet-sysd", false))],
        vanished: 0,
    }));
    let value = perform(&["process"], &mut fake).expect("a table");
    let row = rows(&value).first().expect("one process");
    assert!(
        field(row, "user_millis").is_none() && field(row, "system_millis").is_none(),
        "an unread counter was reported as a measurement of zero: {row:?}"
    );
    // The rest of the row is unaffected: one figure going missing is not the
    // answer going missing.
    assert_eq!(number(row, "pid"), 120);
    assert_eq!(number(row, "resident_bytes"), 4096);
}

/// **What the counters cannot say** — added 2026-09-16 with `wchan` and
/// `syscall`, and the reason is an incident rather than a wish.
///
/// On 2026-09-16 `rivet-sysd` stopped with `state` `S`, one thread, and both CPU
/// counters frozen. Everything on the machine that did not need it kept
/// answering for eleven minutes. Two hypotheses survived the audit — blocked
/// writing an answer nobody was reading, and blocked in a filesystem read — and
/// **nothing in this row could tell them apart**, because a frozen counter says
/// a process is doing nothing and never what it is waiting for.
#[test]
fn the_row_says_where_a_process_is_parked_and_what_it_is_stopped_in() {
    let mut fake = Fake(Ok(ProcessRead {
        processes: vec![one(120, "rivet-sysd", false)],
        vanished: 0,
    }));
    let value = perform(&["process"], &mut fake).expect("a table");
    let row = rows(&value).first().expect("one process");
    assert_eq!(
        text(row, "wchan"),
        Some("do_sys_poll"),
        "the kernel function is what separates a blocked write from a blocked read"
    );
    assert!(
        text(row, "syscall").is_some_and(|s| s.starts_with("73 ")),
        "the syscall line is kept whole: 73 is ppoll on aarch64 and the arguments \
         are what would name the descriptor a blocked write is blocked on, {row:?}"
    );
}

/// **A kernel thread is parked and made no syscall**, and the two travel
/// separately — one present without the other is the ordinary case here, not an
/// inconsistency to paper over.
#[test]
fn a_kernel_thread_is_parked_somewhere_and_has_no_syscall_at_all() {
    let mut fake = Fake(Ok(ProcessRead {
        processes: vec![one(7, "kworker/R-rcu_gp", true)],
        vanished: 0,
    }));
    let value = perform(&["process"], &mut fake).expect("a table");
    let row = rows(&value).first().expect("one process");
    assert_eq!(
        text(row, "wchan"),
        Some("rescuer_thread")
    );
    assert!(
        field(row, "syscall").is_none(),
        "a kernel thread makes no syscall, and an empty string in its place would \
         read as one it made: {row:?}"
    );
}

/// **Absent, never a placeholder** — the rule every other field in this row
/// already obeys. A `wchan` the kernel declined to answer about arrives as the
/// literal `0`, and `cmd/rivet`'s reader turns that into an absence before it
/// gets here; this asserts the rendering does not invent one either.
#[test]
fn a_process_with_no_wchan_omits_it_rather_than_naming_a_symbol() {
    let mut process = one(120, "rivet-sysd", false);
    process.wchan = None;
    process.syscall = None;
    let mut fake = Fake(Ok(ProcessRead {
        processes: vec![process],
        vanished: 0,
    }));
    let value = perform(&["process"], &mut fake).expect("a table");
    let row = rows(&value).first().expect("one process");
    assert!(field(row, "wchan").is_none() && field(row, "syscall").is_none());
    // And the rest of the row is unaffected, for the reason above it.
    assert_eq!(number(row, "pid"), 120);
    assert_eq!(number(row, "user_millis"), 1200);
}

/// A string field, or `None` when the row does not carry it.
fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    match field(value, key) {
        Some(Value::Str(s)) => Some(s.as_str()),
        None => None,
        other => panic!("{key} is not a string: {other:?}"),
    }
}
