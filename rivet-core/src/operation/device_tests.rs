//! What `get device` answers, and the number it must refuse to invent.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::machine::{BlockDevice, BlockIo, DeviceRead, MachineError, Unmeasured};

struct Fake(Result<DeviceRead, MachineError>);

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
    fn read_devices(&mut self) -> Result<DeviceRead, MachineError> {
        match &self.0 {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(MachineError::Refused(e.why().to_string())),
        }
    }
    fn read_network(&mut self) -> Result<crate::operation::machine::NetworkRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
    }
    fn read_slots(&mut self) -> Result<crate::operation::machine::SlotRead, MachineError> {
        Err(MachineError::Refused("not this test's port".to_string()))
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

fn disk() -> BlockDevice {
    BlockDevice {
        name: "mmcblk0".to_string(),
        kind: Some("disk".to_string()),
        node: Some("mmcblk0".to_string()),
        size_bytes: Some(62_333_952 * 512),
        read_only: Some(false),
        removable: Some(false),
        model: Some("SC32G".to_string()),
        io: Some(BlockIo {
            read_bytes: 1_234_567 * 512,
            written_bytes: 7_654_321 * 512,
            reads_completed: 12_345,
            writes_completed: 9_012,
        }),
        ..BlockDevice::default()
    }
}

/// The inactive slot's `/system` — a real partition with **no filesystem view**,
/// which is the case `get storage` cannot see and this subject exists for.
fn inactive_system() -> BlockDevice {
    BlockDevice {
        name: "mmcblk0p4".to_string(),
        kind: Some("partition".to_string()),
        node: Some("mmcblk0p4".to_string()),
        partition_label: Some(crate::state::Slot::B.system_partlabel().to_string()),
        partition_number: Some(4),
        parent: Some("mmcblk0".to_string()),
        size_bytes: Some(1_310_720 * 512),
        read_only: Some(false),
        ..BlockDevice::default()
    }
}

fn card() -> DeviceRead {
    DeviceRead {
        devices: vec![disk(), inactive_system()],
        devices_unreadable: Vec::new(),
        source: "/sys/class/block".to_string(),
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

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    match field(value, key) {
        Some(Value::List(items)) => items,
        other => panic!("{key} is not a list: {other:?}"),
    }
}

/// The whole shape, including the partition label that identifies a slot.
#[test]
fn the_devices_and_their_partition_labels_are_answered() {
    let mut fake = Fake(Ok(card()));
    let value = perform(&["device"], &mut fake).expect("a reading");
    let devices = list(&value, "devices");
    assert_eq!(devices.len(), 2);
    assert_eq!(text(&devices[0], "name"), "mmcblk0");
    assert_eq!(text(&devices[0], "model"), "SC32G");
    assert_eq!(number(&devices[0], "size_bytes"), 62_333_952 * 512);
    assert_eq!(
        text(&devices[1], "partition_label"),
        crate::state::Slot::B.system_partlabel(),
        "the label is what root=PARTLABEL= names and is checked against Slot, not a literal"
    );
    assert_eq!(number(&devices[1], "partition_number"), 4);
    assert_eq!(text(&devices[1], "parent"), "mmcblk0");
}

/// **`disks` counts whole disks and not partitions**, so the count does not
/// depend on every caller knowing which `kind` word to filter on.
#[test]
fn disks_counts_whole_disks_and_not_partitions() {
    let mut fake = Fake(Ok(card()));
    let value = perform(&["device"], &mut fake).expect("a reading");
    assert_eq!(number(&value, "disks"), 1);
    assert_eq!(list(&value, "devices").len(), 2, "both are still reported");
}

/// **THE NUMBER THIS MUST REFUSE TO INVENT.** A consumer SD card states no
/// endurance, and the answer reports none and says so — rather than producing a
/// figure the device does not know (survey §7.3).
#[test]
fn a_card_that_states_no_endurance_gets_none_reported_and_the_prose_says_why() {
    let mut fake = Fake(Ok(card()));
    let value = perform(&["device"], &mut fake).expect("a reading");
    for device in list(&value, "devices") {
        assert!(field(device, "life_time").is_none(), "an endurance was invented");
        assert!(field(device, "pre_eol_info").is_none());
    }
    let means = text(&value, "means");
    assert!(means.contains("NO DEVICE HERE STATES A REMAINING ENDURANCE"), "{means}");
    assert!(means.contains("invented"), "{means}");
    assert!(means.contains("written_bytes"), "{means}");
}

/// And where a device DOES state them, they come through **verbatim** — vendor
/// codes, not a percentage this build worked out.
#[test]
fn an_emmc_that_states_its_life_time_has_it_carried_through_unconverted() {
    let mut reading = card();
    reading.devices[0].life_time = Some("0x02 0x01".to_string());
    reading.devices[0].pre_eol_info = Some("0x01".to_string());
    let mut fake = Fake(Ok(reading));
    let value = perform(&["device"], &mut fake).expect("a reading");
    let emmc = &list(&value, "devices")[0];
    assert_eq!(text(emmc, "life_time"), "0x02 0x01", "a vendor code was converted");
    assert_eq!(text(emmc, "pre_eol_info"), "0x01");
    assert!(text(&value, "means").contains("VERBATIM"));
}

/// **Bytes written is real and is reported**, which is the half survey §7.3
/// says closes the loop on `ADR-018` §7's decisions about SD wear.
#[test]
fn the_write_volume_is_reported_in_bytes() {
    let mut fake = Fake(Ok(card()));
    let value = perform(&["device"], &mut fake).expect("a reading");
    let io = field(&list(&value, "devices")[0], "io").expect("counters");
    assert_eq!(number(io, "written_bytes"), 7_654_321 * 512);
    assert_eq!(number(io, "writes_completed"), 9_012);
}

/// **A device with no model, no label and no counters omits them all** rather
/// than reporting empty strings and zeroes — a `loop` device is a real device
/// with none of those things.
#[test]
fn a_device_with_nothing_to_say_omits_fields_rather_than_defaulting_them() {
    let mut fake = Fake(Ok(DeviceRead {
        devices: vec![BlockDevice {
            name: "loop0".to_string(),
            ..BlockDevice::default()
        }],
        devices_unreadable: Vec::new(),
        source: "/sys/class/block".to_string(),
    }));
    let value = perform(&["device"], &mut fake).expect("a reading");
    let loop0 = &list(&value, "devices")[0];
    assert_eq!(text(loop0, "name"), "loop0");
    for absent in ["kind", "model", "size_bytes", "io", "read_only", "partition_label"] {
        assert!(field(loop0, absent).is_none(), "{absent} was defaulted");
    }
}

/// **A device that could not be read is reported, never omitted** — on an A/B
/// machine the device missing from an answer is the one somebody is looking for.
#[test]
fn a_device_that_could_not_be_read_is_reported_rather_than_omitted() {
    let mut reading = card();
    reading.devices_unreadable.push(Unmeasured {
        what: "mmcblk0p3".to_string(),
        why: "uevent could not be read".to_string(),
    });
    let mut fake = Fake(Ok(reading));
    let value = perform(&["device"], &mut fake).expect("a reading");
    let missed = list(&value, "devices_unreadable");
    assert_eq!(text(&missed[0], "what"), "mmcblk0p3");
    assert!(text(&value, "means").contains("COULD NOT BE READ"));
}

/// A machine that cannot answer says so rather than reporting no devices, which
/// is not a state a machine that booted off one can be in.
#[test]
fn a_port_that_refuses_is_not_a_machine_with_no_block_devices() {
    let mut fake = Fake(Err(MachineError::Refused("no /sys/class/block".to_string())));
    let error = perform(&["device"], &mut fake).expect_err("must not report an empty table");
    assert_eq!(error.code(), ErrorCode::MachineRefused);
}

/// A second word is refused — and the refusal names `storage`, because *how
/// full is it* is the neighbouring question about the same hardware.
#[test]
fn a_second_word_is_refused_and_names_the_subject_for_the_other_question() {
    let mut fake = Fake(Ok(card()));
    let error = perform(&["device", "mmcblk0"], &mut fake).expect_err("refused");
    assert_eq!(error.code(), ErrorCode::Arguments);
    assert!(error.message().contains("storage"), "{}", error.message());
}
