//! What `get thermal` answers, and the one claim it must never make.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::machine::{
    CpuFrequency, MachineError, ThermalRead, ThermalZone, Throttle, UnreadableZone,
};

struct Fake(Result<ThermalRead, MachineError>);

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
    fn read_thermal(&mut self) -> Result<ThermalRead, MachineError> {
        match &self.0 {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(MachineError::Refused(e.why().to_string())),
        }
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

/// A board with a zone and a clock and no route to the firmware — **the shape
/// the first commit of this subject actually produces on hardware.**
fn portable_half() -> ThermalRead {
    ThermalRead {
        zones: vec![ThermalZone {
            name: "cpu_thermal".to_string(),
            millidegrees: 53_556,
        }],
        zones_unreadable: Vec::new(),
        zone_source: "/sys/class/thermal".to_string(),
        frequency: Some(CpuFrequency {
            current_hz: 1_500_000_000,
            maximum_hz: 1_500_000_000,
            minimum_hz: Some(600_000_000),
            ceiling_hz: Some(1_500_000_000),
            governor: Some("ondemand".to_string()),
            source: "/sys/devices/system/cpu/cpu0/cpufreq".to_string(),
        }),
        frequency_unmeasured: None,
        throttle: None,
        throttle_unmeasured: Some("no mailbox read in this build".to_string()),
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

fn boolean(value: &Value, key: &str) -> bool {
    match field(value, key) {
        Some(Value::Bool(b)) => *b,
        other => panic!("{key} is not a boolean: {other:?}"),
    }
}

fn text(value: &Value, key: &str) -> String {
    match field(value, key) {
        Some(Value::Str(s)) => s.clone(),
        other => panic!("{key} is not a string: {other:?}"),
    }
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    match field(value, key) {
        Some(Value::List(items)) => items,
        other => panic!("{key} is not a list: {other:?}"),
    }
}

/// **The single most important property of this subject.** A machine whose
/// firmware flags could not be read reports NO flags — not eight falses, which
/// would read as *this board has never been under-volted since it booted* and
/// would be a claim nobody measured.
#[test]
fn unread_throttle_flags_are_absent_and_are_never_reported_as_clear() {
    let mut fake = Fake(Ok(portable_half()));
    let value = perform(&["thermal"], &mut fake).expect("a reading");
    assert!(
        field(&value, "throttle").is_none(),
        "flags were invented for a machine that could not be asked"
    );
    let why = text(&value, "throttle_unmeasured");
    assert!(!why.is_empty(), "absent flags must carry a reason");
}

/// And the prose says it too, because a caller reading only `means` must not
/// come away believing the board is known to be healthy.
#[test]
fn the_prose_refuses_the_inference_that_absent_flags_are_clear_flags() {
    let mut fake = Fake(Ok(portable_half()));
    let value = perform(&["thermal"], &mut fake).expect("a reading");
    let means = text(&value, "means");
    assert!(means.contains("WERE NOT READ"), "{means}");
    assert!(means.contains("under-volted"), "{means}");
}

/// When they *are* read, every one of the eight is carried through — and the
/// `_since_boot` half is the reason the subject exists.
#[test]
fn measured_throttle_flags_carry_both_halves_and_the_raw_word() {
    let mut reading = portable_half();
    reading.throttle_unmeasured = None;
    reading.throttle = Some(Throttle {
        under_voltage_now: false,
        frequency_capped_now: false,
        throttled_now: false,
        soft_temperature_limit_now: false,
        under_voltage_since_boot: true,
        frequency_capped_since_boot: true,
        throttled_since_boot: true,
        soft_temperature_limit_since_boot: false,
        raw: 0x0007_0000,
        source: "/dev/vcio".to_string(),
    });
    let mut fake = Fake(Ok(reading));
    let value = perform(&["thermal"], &mut fake).expect("a reading");
    let t = field(&value, "throttle").expect("flags were measured");

    assert!(!boolean(t, "under_voltage_now"), "nothing is wrong right now");
    assert!(
        boolean(t, "under_voltage_since_boot"),
        "and the board was under-volted earlier, which is the fact nothing else recovers"
    );
    assert_eq!(number(t, "raw"), 0x0007_0000, "the firmware's own word is kept whole");
    assert_eq!(text(t, "source"), "/dev/vcio");
    assert!(
        field(&value, "throttle_unmeasured").is_none(),
        "a measured set must not also carry an excuse for being absent"
    );
}

/// **A current clock below the maximum is not reported as a throttle**, because
/// a governor scales an idle machine down and a field named for the inference
/// would be wrong far more often than right.
#[test]
fn an_idle_machine_below_its_maximum_is_not_called_throttled() {
    let mut reading = portable_half();
    if let Some(f) = reading.frequency.as_mut() {
        f.current_hz = 600_000_000;
    }
    let mut fake = Fake(Ok(reading));
    let value = perform(&["thermal"], &mut fake).expect("a reading");
    let clock = field(&value, "frequency").expect("a clock");
    assert!(!boolean(clock, "at_maximum"));
    assert!(
        !boolean(clock, "ceiling_below_maximum"),
        "the ceiling did not move; only the current clock did"
    );
    let means = text(&value, "means");
    assert!(means.contains("NOT BY ITSELF A THROTTLE"), "{means}");
}

/// A ceiling below the hardware maximum is the comparison that *would* show
/// one, and it is made here so no caller has to know to make it.
#[test]
fn a_lowered_ceiling_is_reported_as_a_lowered_ceiling() {
    let mut reading = portable_half();
    if let Some(f) = reading.frequency.as_mut() {
        f.ceiling_hz = Some(1_000_000_000);
        f.current_hz = 1_000_000_000;
    }
    let mut fake = Fake(Ok(reading));
    let value = perform(&["thermal"], &mut fake).expect("a reading");
    let clock = field(&value, "frequency").expect("a clock");
    assert!(boolean(clock, "ceiling_below_maximum"));
    assert_eq!(number(clock, "ceiling_hz"), 1_000_000_000);
    assert_eq!(number(clock, "maximum_hz"), 1_500_000_000);
}

/// A machine with no `cpufreq` reports no clock **and says why** — never a zero
/// clock, which would be a reading nobody took.
#[test]
fn a_machine_with_no_cpufreq_says_so_rather_than_reporting_zero_hertz() {
    let mut reading = portable_half();
    reading.frequency = None;
    reading.frequency_unmeasured = Some("no cpufreq on this machine".to_string());
    let mut fake = Fake(Ok(reading));
    let value = perform(&["thermal"], &mut fake).expect("a reading");
    assert!(field(&value, "frequency").is_none(), "a zero clock was invented");
    assert!(text(&value, "frequency_unmeasured").contains("cpufreq"));
}

/// **A zone that could not be read is reported, never dropped** — `get
/// storage`'s rule: an answer that omits it says the machine has no such zone.
#[test]
fn a_zone_that_could_not_be_read_is_reported_rather_than_omitted() {
    let mut reading = portable_half();
    reading.zones_unreadable = vec![UnreadableZone {
        zone: "thermal_zone1".to_string(),
        why: "EACCES".to_string(),
    }];
    let mut fake = Fake(Ok(reading));
    let value = perform(&["thermal"], &mut fake).expect("a reading");
    let missed = list(&value, "zones_unreadable");
    assert_eq!(missed.len(), 1);
    assert_eq!(text(&missed[0], "zone"), "thermal_zone1");
    assert_eq!(text(&missed[0], "why"), "EACCES", "the system's own words");
    assert_eq!(list(&value, "zones").len(), 1, "the readable zone survives");
}

/// Millidegrees travel unconverted: this surface has no floating-point value,
/// and rounding to whole degrees here would throw away what the kernel stated.
#[test]
fn a_zone_carries_thousandths_of_a_degree_and_its_own_name() {
    let mut fake = Fake(Ok(portable_half()));
    let value = perform(&["thermal"], &mut fake).expect("a reading");
    let zones = list(&value, "zones");
    assert_eq!(text(&zones[0], "name"), "cpu_thermal");
    assert_eq!(number(&zones[0], "millidegrees"), 53_556);
}

/// A machine that cannot answer says so rather than reporting a cool board with
/// no throttling — which is what an empty reading would look like.
#[test]
fn a_port_that_refuses_is_not_a_cool_and_unthrottled_machine() {
    let mut fake = Fake(Err(MachineError::Refused("no /sys".to_string())));
    let error = perform(&["thermal"], &mut fake).expect_err("must not report a healthy board");
    assert_eq!(error.code(), ErrorCode::MachineRefused);
}

/// A second word is refused rather than ignored — `schema`'s precedent.
#[test]
fn a_second_word_is_refused() {
    let mut fake = Fake(Ok(portable_half()));
    let error = perform(&["thermal", "zone0"], &mut fake).expect_err("refused");
    assert_eq!(error.code(), ErrorCode::Arguments);
}
