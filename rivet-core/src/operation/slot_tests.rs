//! What `get slot` answers, and the case where it must answer nothing.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::machine::{MachineError, SlotRead};

struct Fake(Result<SlotRead, MachineError>);

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
    fn read_slots(&mut self) -> Result<SlotRead, MachineError> {
        match &self.0 {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(MachineError::Refused(e.why().to_string())),
        }
    }
    fn read_boot(&mut self) -> Result<crate::operation::machine::BootRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
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

/// **The reading the live board produced on 2026-08-29** — slot A, booted from
/// `p1`, with `root=PARTLABEL=system-a` on the command line and no trial
/// pending. Labelled by provenance, per `AGENTS.md` §4.
fn board_says(booted: Option<Slot>, root: Option<&str>, trial: Option<bool>) -> SlotRead {
    SlotRead {
        booted,
        booted_unmeasured: match booted {
            Some(_) => None,
            None => Some("no device tree on this host".to_string()),
        },
        booted_from: "/proc/device-tree/chosen/bootloader/partition".to_string(),
        root_argument: root.map(str::to_string),
        root_unmeasured: match root {
            Some(_) => None,
            None => Some("/proc/cmdline carries no root= argument".to_string()),
        },
        root_from: "/proc/cmdline".to_string(),
        in_trial_boot: trial,
        trial_unmeasured: match trial {
            Some(_) => None,
            None => Some("no tryboot property on this host".to_string()),
        },
        // **The default is the booted slot unless a case says otherwise**,
        // which is what a committed machine looks like and is what the
        // 2026-08-29 reading this fixture is named for actually was. The
        // uncommitted-trial case builds its own reading — see
        // `an_entered_trial_that_was_confirmed_does_not_warn_about_rolling_back`.
        default_slot: booted,
        default_unmeasured: match booted {
            Some(_) => None,
            None => Some("no autoboot.txt on this host".to_string()),
        },
    }
}

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(fields) => fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v),
        other => panic!("not an object: {other:?}"),
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

/// The board's own reading: two authorities, agreeing, and the update target
/// computed from them.
#[test]
fn two_agreeing_authorities_name_the_running_slot_and_the_one_an_update_writes() {
    let mut fake = Fake(Ok(board_says(
        Some(Slot::A),
        Some("PARTLABEL=system-a"),
        Some(false),
    )));
    let value = perform(&["slot"], &mut fake).expect("a reading");
    assert_eq!(text(&value, "running"), "a");
    assert_eq!(text(&value, "inactive"), "b");
    assert_eq!(text(&value, "booted"), "a");
    assert_eq!(text(&value, "root"), "a");
    assert_eq!(text(&value, "root_argument"), "PARTLABEL=system-a");
    assert!(boolean(&value, "agree"));
    assert!(!boolean(&value, "in_trial_boot"));
}

/// **The property this subject exists for.** When the firmware and the kernel
/// name different slots, NO running slot is reported — because naming one would
/// send the next update into the wrong partition, and *"whichever I looked at
/// first"* is a coin toss that reads like a measurement.
#[test]
fn two_disagreeing_authorities_name_no_running_slot_at_all() {
    let mut fake = Fake(Ok(board_says(
        Some(Slot::A),
        Some("PARTLABEL=system-b"),
        Some(false),
    )));
    let value = perform(&["slot"], &mut fake).expect("a reading");
    assert!(
        field(&value, "running").is_none(),
        "a slot was named on a machine that has two answers"
    );
    assert!(
        field(&value, "inactive").is_none(),
        "an update target was named from an answer that does not exist"
    );
    assert!(!boolean(&value, "agree"));
    // Both readings survive, so the operator can see WHICH two disagree.
    assert_eq!(text(&value, "booted"), "a");
    assert_eq!(text(&value, "root"), "b");
    let means = text(&value, "means");
    assert!(means.contains("DISAGREE"), "{means}");
    assert!(means.contains("NOTHING SHOULD BE ARMED"), "{means}");
}

/// One authority is still an answer, and the prose says it is a single reading
/// rather than a corroborated one — which is what a detached build produces.
#[test]
fn one_authority_answers_and_the_prose_says_it_was_not_corroborated() {
    let mut fake = Fake(Ok(board_says(None, Some("PARTLABEL=system-b"), None)));
    let value = perform(&["slot"], &mut fake).expect("a reading");
    assert_eq!(text(&value, "running"), "b");
    assert_eq!(text(&value, "inactive"), "a");
    assert!(
        field(&value, "agree").is_none(),
        "an agreement was reported between one reading and nothing"
    );
    assert!(text(&value, "booted_unmeasured").contains("device tree"));
    assert!(text(&value, "means").contains("single reading"));
}

/// **A `root=` this layout does not describe names no slot, and is still
/// reported whole.** A machine booted by `root=UUID=…` is a real machine that
/// is not running an `ADR-018` §4 slot.
#[test]
fn a_root_argument_that_names_no_slot_is_carried_through_and_resolved_to_nothing() {
    for argument in ["UUID=1234-5678", "/dev/mmcblk0p3", "PARTLABEL=rootfs"] {
        let mut fake = Fake(Ok(board_says(None, Some(argument), None)));
        let value = perform(&["slot"], &mut fake).expect("a reading");
        assert!(field(&value, "root").is_none(), "{argument} resolved to a slot");
        assert_eq!(text(&value, "root_argument"), argument);
        assert!(field(&value, "running").is_none());
    }
}

/// **The label mapping is the one `rivet-image` writes**, checked against
/// `Slot::system_partlabel` rather than against a literal in this test — so a
/// change to either side has to change both.
#[test]
fn each_slots_partlabel_resolves_to_that_slot_and_not_the_other() {
    for slot in [Slot::A, Slot::B] {
        let argument = format!("PARTLABEL={}", slot.system_partlabel());
        let mut fake = Fake(Ok(board_says(Some(slot), Some(&argument), Some(false))));
        let value = perform(&["slot"], &mut fake).expect("a reading");
        assert_eq!(text(&value, "root"), slot.as_str());
        assert_eq!(text(&value, "inactive"), slot.other().as_str());
        assert!(boolean(&value, "agree"));
    }
}

/// **An unread trial flag is absent, never `false`.** `false` asserts that
/// doing nothing is safe, and `ADR-018` §5.1 rolls an uncommitted trial back at
/// the next restart — so that is the one claim a build which could not read the
/// flag must not make.
#[test]
fn an_unread_trial_flag_is_absent_rather_than_reported_as_a_committed_boot() {
    let mut fake = Fake(Ok(board_says(
        Some(Slot::A),
        Some("PARTLABEL=system-a"),
        None,
    )));
    let value = perform(&["slot"], &mut fake).expect("a reading");
    assert!(
        field(&value, "in_trial_boot").is_none(),
        "an unread flag was reported as a committed boot"
    );
    assert!(text(&value, "trial_unmeasured").contains("tryboot"));
    let means = text(&value, "means");
    assert!(means.contains("WAS NOT MEASURED"), "{means}");
    assert!(means.contains("rolls back"), "{means}");
}

/// And a trial that IS pending says what an ordinary restart will do, because
/// doing nothing is what rolls it back.
#[test]
fn a_pending_trial_says_that_an_ordinary_restart_goes_back() {
    // **An UNCOMMITTED trial**: entered by tryboot, and the default is still
    // the other slot. `board_says` makes the default the booted slot, so this
    // one is built by hand — that difference is the whole subject.
    let mut read = board_says(Some(Slot::B), Some("PARTLABEL=system-b"), Some(true));
    read.default_slot = Some(Slot::A);
    let mut fake = Fake(Ok(read));
    let value = perform(&["slot"], &mut fake).expect("a reading");
    assert!(boolean(&value, "in_trial_boot"));
    assert!(!boolean(&value, "committed"));
    let means = text(&value, "means");
    assert!(means.contains("UNCOMMITTED ONE-SHOT TRIAL"), "{means}");
    assert!(means.contains("ADR-018 §5.1"), "{means}");
}

/// **A trial that PID 1 already confirmed must not warn about rolling back** —
/// the defect this whole field exists for, added 2026-09-05.
///
/// The tryboot property records how the firmware STARTED this boot and never
/// changes; committing rewrites `autoboot.txt` while the machine runs. So a
/// healthy confirmed trial reports `in_trial_boot: true` for the rest of its
/// life, and the old prose read that as *"an ordinary restart goes back to the
/// other slot"* — the opposite of the truth.
///
/// **It misled three readers**: two agents on separate days raising the same
/// false alarm, and this session during a live recovery, where the answer
/// decided whether the maintainer's display came back. The board's own journal
/// said `boot confirmed: partition 2 is now the default` at the same moment
/// `get slot` was warning that a restart would abandon it.
#[test]
fn an_entered_trial_that_was_confirmed_does_not_warn_about_rolling_back() {
    // Entered by tryboot AND the default — which is exactly what `commit`
    // leaves behind, and what the board looked like on 2026-09-05.
    let mut fake = Fake(Ok(board_says(
        Some(Slot::B),
        Some("PARTLABEL=system-b"),
        Some(true),
    )));
    let value = perform(&["slot"], &mut fake).expect("a reading");

    assert!(boolean(&value, "in_trial_boot"), "how it was entered is unchanged");
    assert!(boolean(&value, "committed"), "and it has been committed");

    let means = text(&value, "means");
    assert!(means.contains("CONFIRMED"), "{means}");
    // **The two phrasings that were wrong, named exactly.** A blunter check —
    // banning the substring "rolls back" — fails against the correct sentence
    // *"nothing rolls back"*, which is how this assertion was written first and
    // why it is written this way now.
    assert!(
        !means.contains("UNCOMMITTED"),
        "a committed boot must not be called uncommitted: {means}"
    );
    assert!(
        !means.contains("goes back to the default slot"),
        "a confirmed boot must not be described as returning elsewhere: {means}"
    );
}

/// A machine that cannot answer says so rather than reporting slot `a`, which
/// is what a defaulting implementation would produce and would be right half
/// the time.
#[test]
fn a_port_that_refuses_never_defaults_to_a_slot() {
    let mut fake = Fake(Err(MachineError::Refused("no /proc/cmdline".to_string())));
    let error = perform(&["slot"], &mut fake).expect_err("must not name a slot");
    assert_eq!(error.code(), ErrorCode::MachineRefused);
}

/// A second word is refused — and the refusal points at the verb that DOES take
/// a slot, because `rivet get slot b` is the natural mistake and the thing it
/// resembles changes the machine.
#[test]
fn a_second_word_is_refused_and_names_the_verb_that_takes_one() {
    let mut fake = Fake(Ok(board_says(Some(Slot::A), Some("PARTLABEL=system-a"), Some(false))));
    let error = perform(&["slot", "b"], &mut fake).expect_err("refused");
    assert_eq!(error.code(), ErrorCode::Arguments);
    assert!(error.message().contains("trial-boot"), "{}", error.message());
}
