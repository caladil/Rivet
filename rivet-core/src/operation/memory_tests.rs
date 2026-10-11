//! What `get memory` answers, and the distinction it must not collapse.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::operation::machine::{
    KernelMemory, MachineError, MemoryPressure, MemoryRead, SlabCache, SlabCaches,
};

struct Fake(Result<MemoryRead, MachineError>);

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
    fn read_processes(
        &mut self,
    ) -> Result<crate::operation::machine::ProcessRead, MachineError> {
        Err(MachineError::Refused("not this fake's port".to_string()))
    }
    fn read_memory(&mut self) -> Result<MemoryRead, MachineError> {
        match &self.0 {
            Ok(v) => Ok(v.clone()),
            Err(e) => Err(MachineError::Refused(e.why().to_string())),
        }
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

fn some(pressure: Option<MemoryPressure>) -> MemoryRead {
    MemoryRead {
        total_bytes: 8 * 1024 * 1024 * 1024,
        available_bytes: 6 * 1024 * 1024 * 1024,
        free_bytes: 128 * 1024 * 1024,
        cached_bytes: 5 * 1024 * 1024 * 1024,
        swap_total_bytes: 0,
        swap_free_bytes: 0,
        pressure,
        kernel: KernelMemory::default(),
        slab_caches: None,
    }
}

fn slab(name: &str, objects: u64, object_size: u64, merged: Option<&[&str]>) -> SlabCache {
    SlabCache {
        name: name.to_string(),
        active_objects: objects / 2,
        objects,
        object_size,
        merged: merged.map(|m| m.iter().map(|s| (*s).to_string()).collect()),
    }
}

fn list<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    match field(value, key) {
        Some(Value::List(items)) => items,
        other => panic!("{key} is not a list: {other:?}"),
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    match field(value, key) {
        Some(Value::Str(s)) => s,
        other => panic!("{key} is not a string: {other:?}"),
    }
}

/// The nine kernel fields, each named once, for the tests that cover all of them.
const KERNEL_FIELDS: [&str; 9] = [
    "slab_bytes",
    "slab_reclaimable_bytes",
    "slab_unreclaimable_bytes",
    "shmem_bytes",
    "page_tables_bytes",
    "kernel_stack_bytes",
    "vmalloc_used_bytes",
    "cma_total_bytes",
    "cma_free_bytes",
];

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

/// **`free` and `available` are both reported and they are different.** A test
/// rather than a comment, because the whole reason this subject documents the
/// distinction is that collapsing it is the classic misreading.
#[test]
fn free_and_available_are_both_reported_and_are_not_the_same_number() {
    let mut fake = Fake(Ok(some(None)));
    let value = perform(&["memory"], &mut fake).expect("a reading");
    assert_eq!(number(&value, "free_bytes"), 128 * 1024 * 1024);
    assert_eq!(number(&value, "available_bytes"), 6 * 1024 * 1024 * 1024);
    assert!(
        number(&value, "available_bytes") > number(&value, "free_bytes"),
        "the fixture must exercise the difference these two exist to show"
    );
}

/// **Absent pressure is absent, not zero.** Zeroes would assert that nothing is
/// stalling, which a kernel without CONFIG_PSI has not measured.
#[test]
fn a_kernel_without_psi_reports_no_pressure_rather_than_no_stalling() {
    let mut fake = Fake(Ok(some(None)));
    let value = perform(&["memory"], &mut fake).expect("a reading");
    assert!(
        field(&value, "pressure").is_none(),
        "absence was reported as a measurement"
    );
}

/// And when it is there, it is carried through in hundredths.
#[test]
fn pressure_is_reported_in_hundredths_of_a_percent() {
    let mut fake = Fake(Ok(some(Some(MemoryPressure {
        some_avg10: 42,
        some_avg60: 1230,
        full_avg10: 0,
        full_avg60: 7,
    }))));
    let value = perform(&["memory"], &mut fake).expect("a reading");
    let p = field(&value, "pressure").expect("pressure was measured");
    assert_eq!(number(p, "some_avg10"), 42);
    assert_eq!(number(p, "some_avg60"), 1230);
    assert_eq!(number(p, "full_avg60"), 7);
}

/// A machine that cannot answer says so, rather than reporting zeroes — which
/// would be a claim that it has no memory.
#[test]
fn a_port_that_refuses_is_an_error_and_not_a_machine_with_no_memory() {
    let mut fake = Fake(Err(MachineError::Refused("no /proc".to_string())));
    let error = perform(&["memory"], &mut fake).expect_err("must not report zeroes");
    assert_eq!(error.code(), ErrorCode::MachineRefused);
}

/// A second word is refused rather than ignored — `schema`'s precedent.
#[test]
fn a_second_word_is_refused() {
    let mut fake = Fake(Ok(some(None)));
    let error = perform(&["memory", "swap"], &mut fake).expect_err("refused");
    assert_eq!(error.code(), ErrorCode::Arguments);
}

/// **The kernel's holdings are carried through, each under its own name.**
/// Distinct values per field, so a swapped pair fails rather than passing.
#[test]
fn every_kernel_holding_is_reported_under_its_own_name() {
    let mut read = some(None);
    read.kernel = KernelMemory {
        slab_bytes: Some(1),
        slab_reclaimable_bytes: Some(2),
        slab_unreclaimable_bytes: Some(3),
        shmem_bytes: Some(4),
        page_tables_bytes: Some(5),
        kernel_stack_bytes: Some(6),
        vmalloc_used_bytes: Some(7),
        cma_total_bytes: Some(8),
        cma_free_bytes: Some(9),
    };
    let value = perform(&["memory"], &mut Fake(Ok(read))).expect("a reading");
    for (at, key) in KERNEL_FIELDS.iter().enumerate() {
        assert_eq!(number(&value, key), i64::try_from(at + 1).unwrap(), "{key}");
    }
}

/// **A line the kernel did not write is absent, not zero** — a kernel without
/// CONFIG_CMA has not reserved zero bytes for devices; it has said nothing.
/// And one absent line does not take its neighbours with it.
#[test]
fn a_kernel_line_that_was_not_written_is_absent_and_its_neighbours_are_not() {
    let mut read = some(None);
    read.kernel.slab_bytes = Some(4096);
    let value = perform(&["memory"], &mut Fake(Ok(read))).expect("a reading");
    assert_eq!(number(&value, "slab_bytes"), 4096);
    for key in KERNEL_FIELDS.iter().skip(1) {
        assert!(field(&value, key).is_none(), "{key} was reported without a reading");
    }
}

/// **Additive only.** The fields a consumer already reads keep their names, and
/// the kernel's are new ones beside them — Rivet Monitor and `rivet-fleet`
/// read the old names and must keep finding them.
#[test]
fn the_fields_a_consumer_already_reads_are_all_still_there() {
    let mut read = some(None);
    read.kernel.cma_free_bytes = Some(1);
    let value = perform(&["memory"], &mut Fake(Ok(read))).expect("a reading");
    for key in [
        "subject",
        "total_bytes",
        "available_bytes",
        "free_bytes",
        "cached_bytes",
        "swap_total_bytes",
        "swap_free_bytes",
    ] {
        assert!(field(&value, key).is_some(), "{key} went missing");
    }
}

/// Every declared kernel field is one this subject can emit, so `explain`
/// names nothing the answer never carries.
#[test]
fn every_kernel_field_is_declared() {
    let declared: Vec<&str> = OUTPUTS.iter().map(|f| f.name).collect();
    for key in KERNEL_FIELDS {
        assert!(declared.contains(&key), "{key} is emitted but not declared");
    }
}

/// **Largest first, at most twenty, and the count is every cache the kernel
/// listed.** Bytes are `objects × object_size`, so a cache with many small
/// objects can outrank one with few large ones — the order is by what is held,
/// not by either factor.
#[test]
fn slab_caches_are_the_twenty_holding_most_largest_first_and_the_count_is_all() {
    let mut caches: Vec<SlabCache> = (1..=30).map(|i| slab(&format!("c{i:02}"), i, 100, None)).collect();
    caches.push(slab("many-small", 10_000, 8, None)); // 80,000 bytes, the most
    let mut read = some(None);
    read.slab_caches = Some(SlabCaches::Read(caches));
    let value = perform(&["memory"], &mut Fake(Ok(read))).expect("a reading");

    assert_eq!(number(&value, "slab_cache_count"), 31);
    let reported = list(&value, "slab_caches");
    assert_eq!(reported.len(), SLAB_CACHES_REPORTED);
    assert_eq!(text(&reported[0], "name"), "many-small");
    assert_eq!(number(&reported[0], "bytes"), 80_000);
    assert_eq!(text(&reported[1], "name"), "c30");
    assert_eq!(number(&reported[1], "bytes"), 3_000);
    let sizes: Vec<i64> = reported.iter().map(|c| number(c, "bytes")).collect();
    assert!(sizes.windows(2).all(|w| w[0] >= w[1]), "not largest first: {sizes:?}");
    assert_eq!(number(&reported[1], "objects"), 30);
    assert_eq!(number(&reported[1], "active_objects"), 15);
    assert_eq!(number(&reported[1], "object_size"), 100);
}

/// Equal holdings are ordered by name, so an unchanged machine answers the same
/// bytes twice.
#[test]
fn slab_caches_holding_the_same_are_ordered_by_name() {
    let mut read = some(None);
    read.slab_caches = Some(SlabCaches::Read(vec![
        slab("b", 1, 64, None),
        slab("a", 1, 64, None),
    ]));
    let value = perform(&["memory"], &mut Fake(Ok(read))).expect("a reading");
    let names: Vec<&str> = list(&value, "slab_caches").iter().map(|c| text(c, "name")).collect();
    assert_eq!(names, ["a", "b"]);
}

/// **A listing that did not parse is a refusal with its text**, and neither a
/// count nor a list is reported beside it — a partial list would be read as
/// the whole one.
#[test]
fn an_unparseable_slab_listing_is_a_refusal_and_not_a_shorter_list() {
    let mut read = some(None);
    read.slab_caches = Some(SlabCaches::Refused("line 7 does not parse: \"garbage\"".to_string()));
    let value = perform(&["memory"], &mut Fake(Ok(read))).expect("the rest still answers");
    assert!(text(&value, "slab_caches_refused").contains("garbage"));
    assert!(field(&value, "slab_caches").is_none());
    assert!(field(&value, "slab_cache_count").is_none());
    assert_eq!(number(&value, "available_bytes"), 6 * 1024 * 1024 * 1024);
}

/// **An unread listing is absent, not empty** — an empty list would say the
/// kernel holds no slab at all.
#[test]
fn an_unread_slab_listing_is_absent_and_not_an_empty_one() {
    let value = perform(&["memory"], &mut Fake(Ok(some(None)))).expect("a reading");
    for key in ["slab_cache_count", "slab_caches", "slab_caches_refused"] {
        assert!(field(&value, key).is_none(), "{key} was reported without a reading");
    }
}

/// `merged` is carried when the aliases were read — empty meaning *nothing was
/// merged here* — and absent when they were not.
#[test]
fn merged_names_are_carried_when_read_and_absent_when_not() {
    let mut read = some(None);
    read.slab_caches = Some(SlabCaches::Read(vec![
        slab("kmalloc-192", 100, 192, Some(&["drm_sched_fence", "cred"])),
        slab("task_struct", 10, 8448, Some(&[])),
        slab("dentry", 1, 192, None),
    ]));
    let value = perform(&["memory"], &mut Fake(Ok(read))).expect("a reading");
    let caches = list(&value, "slab_caches");
    let by = |name: &str| caches.iter().find(|c| text(c, "name") == name).expect(name);
    let merged: Vec<&Value> = list(by("kmalloc-192"), "merged").iter().collect();
    assert_eq!(merged, [&Value::str("drm_sched_fence"), &Value::str("cred")]);
    assert!(list(by("task_struct"), "merged").is_empty());
    assert!(field(by("dentry"), "merged").is_none());
}

/// Every slab field this subject can emit is declared.
#[test]
fn every_slab_field_is_declared() {
    let declared: Vec<&str> = OUTPUTS.iter().map(|f| f.name).collect();
    for key in ["slab_cache_count", "slab_caches", "slab_caches_refused"] {
        assert!(declared.contains(&key), "{key} is emitted but not declared");
    }
}
