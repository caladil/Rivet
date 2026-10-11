//! What `get cpu` answers, and the one thing it must never answer.
//!
//! **Every fixture here is hand-composed and labelled as such** (`AGENTS.md`
//! §4). The counters are invented; what they prove is that this subject carries
//! the shape it declares and refuses the percentage it cannot compute. They do
//! not prove anything about a Raspberry Pi's `/proc/stat`, which is
//! `cmd/rivet/src/cpu.rs`'s tests and, past those, a board.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::machine::{CpuRead, CpuTimes, LoadAverage, MachineError};

struct Fake(Result<CpuRead, MachineError>);

impl crate::operation::machine::Observed for Fake {
    /// **Refuses rather than answering that this machine trusts nobody.**
    /// An empty trust store is a REAL reading -- it is what `ADR-028` §5.5.5's
    /// `--no-trust` image produces -- and this fake has not taken it. The
    /// direction is the dangerous one: *"trusts nobody"* is the answer
    /// somebody acts on by re-signing an artifact.
    fn read_trust(&mut self) -> Result<crate::operation::machine::TrustRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_cpu(&mut self) -> Result<CpuRead, MachineError> {
        match &self.0 {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(MachineError::Refused(e.why().to_string())),
        }
    }
    /// **Refuses rather than answering that this machine has no block
    /// devices**, which is not a state a machine that booted off one can be in.
    fn read_devices(&mut self) -> Result<crate::operation::machine::DeviceRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering an empty network.**
    fn read_network(&mut self) -> Result<crate::operation::machine::NetworkRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than naming a slot.**
    fn read_slots(&mut self) -> Result<crate::operation::machine::SlotRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering an uptime of zero.**
    fn read_boot(&mut self) -> Result<crate::operation::machine::BootRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    /// **Refuses rather than answering with unset throttle flags.**
    fn read_thermal(&mut self) -> Result<crate::operation::machine::ThermalRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_processes(&mut self) -> Result<crate::operation::machine::ProcessRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_memory(&mut self) -> Result<crate::operation::machine::MemoryRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
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

/// Hand-composed. One core has done work, three have mostly idled — the shape
/// this subject reports per-CPU in order to make visible.
fn one(name: &str, user: u64, idle: u64) -> CpuTimes {
    CpuTimes {
        name: name.to_string(),
        user_millis: user,
        nice_millis: 0,
        system_millis: user / 4,
        idle_millis: idle,
        iowait_millis: 1_000,
        irq_millis: 0,
        softirq_millis: 0,
        steal_millis: 0,
    }
}

fn reading(load: Option<LoadAverage>) -> CpuRead {
    CpuRead {
        cpus: vec![
            one("cpu0", 400_000, 100_000),
            one("cpu1", 1_000, 499_000),
            one("cpu2", 1_000, 499_000),
            one("cpu3", 1_000, 499_000),
        ],
        total: one("cpu", 403_000, 1_597_000),
        processors: 4,
        load,
        load_unmeasured: load
            .is_none()
            .then(|| "/proc/loadavg could not be read: No such file".to_string()),
        source: "/proc/stat".to_string(),
    }
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

fn text(value: &Value, key: &str) -> String {
    match field(value, key) {
        Some(Value::Str(s)) => s.clone(),
        other => panic!("{key} is not text: {other:?}"),
    }
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    match field(value, key) {
        Some(Value::List(items)) => items,
        other => panic!("{key} is not a list: {other:?}"),
    }
}

/// **The assertion this whole subject exists to make.** A percentage is a rate,
/// a rate needs two readings, and this one takes one — so nothing in the result
/// may be a percentage, and the prose has to say so rather than leaving it to be
/// noticed.
#[test]
fn no_field_is_a_utilisation_and_the_answer_says_why_not() {
    let mut fake = Fake(Ok(reading(None)));
    let value = perform(&["cpu"], &mut fake).expect("a reading");

    let Value::Object(fields) = &value else {
        panic!("not an object");
    };
    for (key, _) in fields {
        assert!(
            !key.contains("percent") && !key.contains("utilisation") && !key.contains("busy_percent"),
            "`{key}` reads as a percentage, which a single reading cannot compute"
        );
    }
    let means = text(&value, "means");
    assert!(means.contains("NO UTILISATION IS REPORTED HERE"), "{means}");
    assert!(means.contains("0%"), "the reader is told what NOT to render: {means}");
}

/// Every counter arrives in milliseconds and the two sums are made here, so that
/// no caller computes `busy` for itself and gets `iowait` on the wrong side of
/// it.
#[test]
fn the_counters_are_milliseconds_and_busy_excludes_iowait_as_well_as_idle() {
    let mut fake = Fake(Ok(reading(None)));
    let value = perform(&["cpu"], &mut fake).expect("a reading");
    let total = field(&value, "total").expect("the kernel's aggregate");

    assert_eq!(number(total, "user_millis"), 403_000);
    assert_eq!(number(total, "idle_millis"), 1_597_000);
    assert_eq!(number(total, "iowait_millis"), 1_000);

    let whole = number(total, "total_millis");
    let busy = number(total, "busy_millis");
    assert_eq!(
        whole,
        403_000 + 100_750 + 1_597_000 + 1_000,
        "total_millis must be every counter summed"
    );
    assert_eq!(
        busy,
        whole - 1_597_000 - 1_000,
        "iowait is WAITING, not work — a machine blocked on a dying card is not busy"
    );
}

/// **Per-CPU, because one core pinned while three idle is what an aggregate
/// hides.** The fixture is that machine, and the test is that the shape survives
/// to the answer.
#[test]
fn every_processor_is_reported_separately_so_one_pinned_core_is_visible() {
    let mut fake = Fake(Ok(reading(None)));
    let value = perform(&["cpu"], &mut fake).expect("a reading");
    assert_eq!(number(&value, "processors"), 4);

    let cpus = list(&value, "cpus");
    assert_eq!(cpus.len(), 4);
    let busy: Vec<i64> = cpus.iter().map(|c| number(c, "busy_millis")).collect();
    let hottest = busy.iter().max().copied().unwrap_or(0);
    let coolest = busy.iter().min().copied().unwrap_or(0);
    assert!(
        hottest > coolest * 100,
        "the fixture must exercise the disparity this field exists to show: {busy:?}"
    );
}

/// **A machine that states no load average reports none, not zero.** Zero is a
/// real reading of an idle machine, and it is the figure a reader is likeliest
/// to act on without checking — `get memory`'s rule for PSI, one subject over.
#[test]
fn an_unreadable_load_average_is_absent_rather_than_three_zeroes() {
    let mut fake = Fake(Ok(reading(None)));
    let value = perform(&["cpu"], &mut fake).expect("a reading");
    assert!(field(&value, "load").is_none(), "absence was reported as zero");
    assert!(text(&value, "load_unmeasured").contains("/proc/loadavg"));
    assert!(
        text(&value, "means").contains("NOT READ"),
        "the prose must say the load was not measured"
    );
}

/// And when it is there it arrives in hundredths, with the caveat that stops it
/// being read as a utilisation.
#[test]
fn a_load_average_is_hundredths_and_is_told_apart_from_a_utilisation() {
    let mut fake = Fake(Ok(reading(Some(LoadAverage {
        avg1: 412,
        avg5: 130,
        avg15: 40,
        runnable: 2,
        existing: 97,
    }))));
    let value = perform(&["cpu"], &mut fake).expect("a reading");
    let load = field(&value, "load").expect("a load average");
    assert_eq!(number(load, "avg1"), 412, "4.12 arrives as 412");
    assert_eq!(number(load, "avg15"), 40, "0.40 arrives as 40, not 4");
    assert_eq!(number(load, "runnable"), 2);

    let means = text(&value, "means");
    assert!(means.contains("NOT A UTILISATION"), "{means}");
    assert!(
        means.contains("uninterruptible sleep"),
        "the Linux-specific caveat has to travel with the number: {means}"
    );
    assert!(means.contains("4.12"), "the sentence names the reading: {means}");
}

/// A machine that cannot answer refuses, rather than reporting zeroed counters —
/// which would say this machine has done nothing since it booted.
#[test]
fn a_port_that_refuses_is_an_error_and_not_a_machine_that_has_run_nothing() {
    let mut fake = Fake(Err(MachineError::Refused("no /proc/stat".to_string())));
    let error = perform(&["cpu"], &mut fake).expect_err("must not report zeroes");
    assert_eq!(error.code(), ErrorCode::MachineRefused);
}

/// A second word is refused rather than ignored — and the refusal answers the
/// guess a person is most likely to have made, which is that the word was an
/// interval.
#[test]
fn a_second_word_is_refused_and_the_refusal_names_the_interval_guess() {
    let mut fake = Fake(Ok(reading(None)));
    let error = perform(&["cpu", "2"], &mut fake).expect_err("refused");
    assert_eq!(error.code(), ErrorCode::Arguments);
    assert!(error.message().contains("rate"), "{}", error.message());
}
