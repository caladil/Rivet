//! What `get boot` answers, and the two distinctions it must not collapse.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::machine::{BootRead, JournalRead, MachineError};

/// A machine that answers the kernel's half and the journal's half separately,
/// **because the whole point of this subject is that either can fail alone.**
struct Fake {
    kernel: Result<BootRead, MachineError>,
    journal: Result<JournalRead, MachineError>,
}

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
    fn read_boot(&mut self) -> Result<BootRead, MachineError> {
        match &self.kernel {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(MachineError::Refused(e.why().to_string())),
        }
    }
    fn read_journal(&mut self) -> Result<JournalRead, MachineError> {
        match &self.journal {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(MachineError::Refused(e.why().to_string())),
        }
    }
    fn read_thermal(&mut self) -> Result<crate::operation::machine::ThermalRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_processes(&mut self) -> Result<crate::operation::machine::ProcessRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_memory(&mut self) -> Result<crate::operation::machine::MemoryRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_desired_state(&mut self) -> Result<Option<String>, MachineError> {
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

const THIS_BOOT: &str = "11111111-1111-4111-8111-111111111111";
const LAST_BOOT: &str = "22222222-2222-4222-8222-222222222222";

fn kernel_says(boot_id: Option<&str>) -> BootRead {
    BootRead {
        boot_id: boot_id.map(str::to_string),
        boot_id_unmeasured: match boot_id {
            Some(_) => None,
            None => Some("no boot_id on this kernel".to_string()),
        },
        uptime_seconds: 13_415,
        idle_seconds: Some(52_340),
        source: "/proc/uptime".to_string(),
    }
}

/// A journal holding one record per boot, **oldest first** — which is the order
/// `boots_in_order` takes as authoritative.
///
/// The lines are produced by [`crate::journal::Record::to_jsonl`] rather than
/// hand-written, so that a change to the record format breaks this fixture
/// loudly instead of leaving it silently unparseable — which is exactly what a
/// hand-written line did on the first run of these tests, and it read as *this
/// machine has recorded no boots*.
fn journal_of(boots: &[&str]) -> JournalRead {
    JournalRead {
        lines: boots
            .iter()
            .map(|boot| {
                crate::journal::Record {
                    // `None`, and deliberately: `ADR-021` says a Pi's clock is
                    // wrong until something sets it, and this subject's whole
                    // argument is that the order is recoverable without one.
                    ts: None,
                    boot: (*boot).to_string(),
                    mono: 1.0,
                    sev: crate::journal::Severity::Info,
                    svc: "test".to_string(),
                    corr: String::new(),
                    actor: String::new(),
                    msg: "x".to_string(),
                    fields: Vec::new(),
                }
                .to_jsonl()
            })
            .collect(),
        files: vec!["/state/log/journal.jsonl".to_string()],
        source: "/state/log".to_string(),
    }
}

fn healthy() -> Fake {
    Fake {
        kernel: Ok(kernel_says(Some(THIS_BOOT))),
        journal: Ok(journal_of(&[LAST_BOOT, THIS_BOOT])),
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
        other => panic!("{key} is not a string: {other:?}"),
    }
}

fn boolean(value: &Value, key: &str) -> bool {
    match field(value, key) {
        Some(Value::Bool(b)) => *b,
        other => panic!("{key} is not a boolean: {other:?}"),
    }
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    match field(value, key) {
        Some(Value::List(items)) => items
            .iter()
            .map(|item| match item {
                Value::Str(s) => s.clone(),
                other => panic!("not a string: {other:?}"),
            })
            .collect(),
        other => panic!("{key} is not a list: {other:?}"),
    }
}

/// The three facts the survey asked for, in one answer.
#[test]
fn the_boot_id_the_uptime_and_the_boots_are_all_answered() {
    let mut fake = healthy();
    let value = perform(&["boot"], &mut fake).expect("a reading");
    assert_eq!(text(&value, "boot_id"), THIS_BOOT);
    assert_eq!(number(&value, "uptime_seconds"), 13_415);
    assert_eq!(number(&value, "idle_seconds"), 52_340);
    assert_eq!(strings(&value, "boots"), vec![LAST_BOOT, THIS_BOOT]);
    assert!(boolean(&value, "recorded"));
}

/// **The order is the journal's, and it is carried through untouched.**
/// `boots_in_order` takes first appearance as authoritative because a boot
/// identifier is a random UUID and a machine with no RTC starts every boot at
/// the epoch — so a subject that sorted this list would make `--boot previous`
/// name the wrong boot.
#[test]
fn the_boots_keep_the_journals_own_order_rather_than_being_sorted() {
    // Deliberately in an order that sorting would change.
    let mut fake = Fake {
        kernel: Ok(kernel_says(Some(THIS_BOOT))),
        journal: Ok(journal_of(&[THIS_BOOT, LAST_BOOT])),
    };
    let value = perform(&["boot"], &mut fake).expect("a reading");
    assert_eq!(
        strings(&value, "boots"),
        vec![THIS_BOOT, LAST_BOOT],
        "the list was reordered; --boot previous would now name the wrong boot"
    );
}

/// **A journal that could not be read is reported as unmeasured, never as a
/// machine that has recorded no boots.** The uptime still answers, because a
/// machine whose `/state` never mounted still knows how long it has been up.
#[test]
fn an_unreadable_journal_leaves_the_boots_unmeasured_and_the_uptime_answered() {
    let mut fake = Fake {
        kernel: Ok(kernel_says(Some(THIS_BOOT))),
        journal: Err(MachineError::Refused("/state is not mounted".to_string())),
    };
    let value = perform(&["boot"], &mut fake).expect("the uptime still answers");
    assert!(
        field(&value, "boots").is_none(),
        "an empty list was reported for a journal nobody could read"
    );
    assert!(field(&value, "recorded").is_none(), "a cross-check was invented");
    assert!(text(&value, "boots_unmeasured").contains("/state is not mounted"));
    assert_eq!(number(&value, "uptime_seconds"), 13_415);
    assert!(text(&value, "means").contains("could NOT be read"));
}

/// **A machine that is not in its own journal is a finding, and the prose says
/// so.** On a booted Rivet it means nothing has been written during this boot.
#[test]
fn a_boot_absent_from_its_own_journal_is_reported_and_explained() {
    let mut fake = Fake {
        kernel: Ok(kernel_says(Some(THIS_BOOT))),
        journal: Ok(journal_of(&[LAST_BOOT])),
    };
    let value = perform(&["boot"], &mut fake).expect("a reading");
    assert!(!boolean(&value, "recorded"));
    let means = text(&value, "means");
    assert!(means.contains("NOT AMONG THEM"), "{means}");
    assert!(means.contains("/state"), "{means}");
}

/// A kernel that cannot name its own boot still has a real uptime, and the
/// cross-check is **absent rather than guessed** when half of it is missing.
#[test]
fn a_machine_that_cannot_name_its_boot_still_reports_its_uptime() {
    let mut fake = Fake {
        kernel: Ok(kernel_says(None)),
        journal: Ok(journal_of(&[LAST_BOOT])),
    };
    let value = perform(&["boot"], &mut fake).expect("a reading");
    assert!(field(&value, "boot_id").is_none());
    assert!(text(&value, "boot_id_unmeasured").contains("boot_id"));
    assert!(field(&value, "recorded").is_none(), "a cross-check was invented");
    assert_eq!(number(&value, "uptime_seconds"), 13_415);
}

/// **Absent idle seconds are absent, not zero.** Zero idle seconds is a real
/// reading of a machine that has been busy since it booted.
#[test]
fn a_kernel_that_states_no_idle_figure_reports_none_rather_than_zero() {
    let mut kernel = kernel_says(Some(THIS_BOOT));
    kernel.idle_seconds = None;
    let mut fake = Fake {
        kernel: Ok(kernel),
        journal: Ok(journal_of(&[THIS_BOOT])),
    };
    let value = perform(&["boot"], &mut fake).expect("a reading");
    assert!(field(&value, "idle_seconds").is_none(), "a zero was invented");
}

/// **The clock caveat is always stated**, whatever else the answer says,
/// because every timestamp a reader goes on to look at inherits it.
#[test]
fn the_prose_always_says_that_nothing_here_is_a_wall_clock_time() {
    for mut fake in [
        healthy(),
        Fake {
            kernel: Ok(kernel_says(None)),
            journal: Err(MachineError::Refused("no journal".to_string())),
        },
    ] {
        let value = perform(&["boot"], &mut fake).expect("a reading");
        let means = text(&value, "means");
        assert!(means.contains("NOTHING HERE SAYS WHEN IT BOOTED"), "{means}");
        assert!(means.contains("ADR-021"), "{means}");
    }
}

/// A machine that cannot answer says so rather than reporting an uptime of
/// zero, which would say it booted a moment ago.
#[test]
fn a_port_that_refuses_is_not_a_machine_that_just_booted() {
    let mut fake = Fake {
        kernel: Err(MachineError::Refused("no /proc/uptime".to_string())),
        journal: Ok(journal_of(&[THIS_BOOT])),
    };
    let error = perform(&["boot"], &mut fake).expect_err("must not report zero seconds");
    assert_eq!(error.code(), ErrorCode::MachineRefused);
}

/// A second word is refused — and the refusal points at the subject that *does*
/// take one, because `rivet get boot previous` is the natural mistake.
#[test]
fn a_second_word_is_refused_and_names_the_subject_that_takes_one() {
    let mut fake = healthy();
    let error = perform(&["boot", "previous"], &mut fake).expect_err("refused");
    assert_eq!(error.code(), ErrorCode::Arguments);
    assert!(error.message().contains("log"), "{}", error.message());
}
