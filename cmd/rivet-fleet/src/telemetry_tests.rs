//! Tests for [`super`] — the readings and the tiered poller.
//!
//! **Every fixture below is hand-composed and labelled as such**, which
//! `AGENTS.md` §4 asks for: *"a vector captured from real hardware is worth more
//! than a hand-composed one, and the difference must be visible in the file."*
//! No board was available to this work, so the field names are taken from the
//! subjects' own `OUTPUTS` declarations in `rivet_core::operation::{memory,
//! process, storage, status}` and the *values* are invented. That means these
//! prove the reader agrees with the declared shape; they do **not** prove it
//! agrees with the bytes a Raspberry Pi sends.
//!
//! [`the_subjects_this_screen_polls_are_ones_the_core_answers`] is the guard
//! against the half of that which can be checked mechanically.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::sync::mpsc::{Receiver, Sender, channel};

use super::*;

/// Hand-composed, from `rivet_core::operation::memory::OUTPUTS`.
const MEMORY: &str = r#"{"subject":"memory","total_bytes":971505664,
  "available_bytes":612345678,"free_bytes":41216000,"cached_bytes":503316480,
  "swap_total_bytes":0,"swap_free_bytes":0,
  "pressure":{"some_avg10":42,"some_avg60":10,"full_avg10":0,"full_avg60":0}}"#;

/// The same machine on a kernel built without `CONFIG_PSI`.
const MEMORY_NO_PSI: &str = r#"{"subject":"memory","total_bytes":971505664,
  "available_bytes":612345678,"free_bytes":41216000,"cached_bytes":503316480,
  "swap_total_bytes":0,"swap_free_bytes":0}"#;

/// Hand-composed, from `rivet_core::operation::process::OUTPUTS`.
///
/// **`kthreadd` deliberately carries no `user_millis`/`system_millis`.** They
/// come from a different file in `/proc` than the rest of a row and can go
/// missing on their own, which is the case a `0` would erase — and which
/// [`a_process_with_no_cpu_counter_is_absent_from_the_rates_rather_than_zeroed`]
/// needs a row for.
const PROCESSES: &str = r#"{"subject":"process","processes":[
  {"pid":1,"parent_pid":0,"name":"rivet-initd","state":"S","threads":1,
   "kernel":false,"resident_bytes":2162688,"user_millis":130,"system_millis":110},
  {"pid":2,"parent_pid":0,"name":"kthreadd","state":"S","threads":1,"kernel":true},
  {"pid":214,"parent_pid":1,"name":"rivet-mcpd","state":"S","threads":2,
   "kernel":false,"resident_bytes":4194304,"user_millis":4000,"system_millis":900},
  {"pid":402,"parent_pid":1,"name":"stuck","state":"D","threads":1,
   "kernel":false,"resident_bytes":1048576,"user_millis":20,"system_millis":5}],
  "vanished":1,"userland":3}"#;

/// Hand-composed, from `rivet_core::operation::storage::RECORD_KEYS`.
const STORAGE: &str = r#"{"subject":"storage","filesystems":[
  {"source":"/dev/mmcblk0p3","mount-point":"/state","fstype":"ext4",
   "read-only":false,"total-bytes":4000000000,"used-bytes":1200000000,
   "available-bytes":2600000000,"capacity-percent":31,"holds":"state"},
  {"source":"proc","mount-point":"/proc","fstype":"proc","read-only":false,
   "total-bytes":0,"used-bytes":0,"available-bytes":0,"holds":""}],
  "count":2,"unreadable":0,"mount-table":"/proc/self/mountinfo"}"#;

/// Hand-composed, from `rivet_core::operation::status::OUTPUTS`.
const STATUS: &str = r#"{"subject":"status","phase":"running","services":[
  {"name":"rivet-mcpd","enabled":true,"state":"running","pid":214,
   "restarts":0,"ready":true},
  {"name":"rivet-mdnsd","enabled":true,"state":"running","pid":331,
   "restarts":2,"ready":true},
  {"name":"rivet-smbd","enabled":false,"state":"idle","restarts":0,"ready":false}]}"#;

/// Hand-composed, from `rivet_core::operation::cpu::OUTPUTS`. A four-core board
/// with **one core doing the work**, which is the shape the per-CPU figures
/// exist to make visible.
const CPU: &str = r#"{"subject":"cpu","processors":4,
  "total":{"name":"cpu","user_millis":400000,"nice_millis":0,"system_millis":100000,
    "idle_millis":1500000,"iowait_millis":10000,"irq_millis":0,"softirq_millis":0,
    "steal_millis":0,"total_millis":2010000,"busy_millis":500000},
  "cpus":[
   {"name":"cpu0","user_millis":380000,"nice_millis":0,"system_millis":90000,
    "idle_millis":30000,"iowait_millis":2500,"irq_millis":0,"softirq_millis":0,
    "steal_millis":0,"total_millis":502500,"busy_millis":470000},
   {"name":"cpu1","user_millis":7000,"nice_millis":0,"system_millis":3500,
    "idle_millis":490000,"iowait_millis":2500,"irq_millis":0,"softirq_millis":0,
    "steal_millis":0,"total_millis":503000,"busy_millis":10500},
   {"name":"cpu2","user_millis":7000,"nice_millis":0,"system_millis":3500,
    "idle_millis":490000,"iowait_millis":2500,"irq_millis":0,"softirq_millis":0,
    "steal_millis":0,"total_millis":503000,"busy_millis":10500},
   {"name":"cpu3","user_millis":6000,"nice_millis":0,"system_millis":3000,
    "idle_millis":490000,"iowait_millis":2500,"irq_millis":0,"softirq_millis":0,
    "steal_millis":0,"total_millis":501500,"busy_millis":9000}],
  "load":{"avg1":412,"avg5":130,"avg15":40,"runnable":2,"existing":97},
  "read_from":"/proc/stat","means":"NO UTILISATION IS REPORTED HERE …"}"#;

/// The same machine on a build that could not read `/proc/loadavg`.
const CPU_NO_LOAD: &str = r#"{"subject":"cpu","processors":1,
  "total":{"name":"cpu","user_millis":1,"nice_millis":0,"system_millis":0,
    "idle_millis":9,"iowait_millis":0,"irq_millis":0,"softirq_millis":0,
    "steal_millis":0,"total_millis":10,"busy_millis":1},
  "cpus":[{"name":"cpu0","user_millis":1,"nice_millis":0,"system_millis":0,
    "idle_millis":9,"iowait_millis":0,"irq_millis":0,"softirq_millis":0,
    "steal_millis":0,"total_millis":10,"busy_millis":1}],
  "load_unmeasured":"/proc/loadavg could not be read: No such file or directory",
  "read_from":"/proc/stat","means":"…"}"#;

/// Hand-composed, from `rivet_core::operation::thermal::OUTPUTS`. **A board
/// whose firmware answered**: the flags are there and the since-boot half says
/// it was under-volted at some point.
const THERMAL: &str = r#"{"subject":"thermal",
  "zones":[{"name":"cpu_thermal","millidegrees":52341}],"zones_unreadable":[],
  "frequency":{"current_hz":600000000,"maximum_hz":1500000000,
    "minimum_hz":600000000,"ceiling_hz":1000000000,"ceiling_below_maximum":true,
    "governor":"ondemand","at_maximum":false,"source":"policy0"},
  "throttle":{"under_voltage_now":false,"frequency_capped_now":true,
    "throttled_now":true,"soft_temperature_limit_now":false,
    "under_voltage_since_boot":true,"frequency_capped_since_boot":true,
    "throttled_since_boot":true,"soft_temperature_limit_since_boot":false,
    "raw":327686,"source":"/dev/vcio"},
  "read_from":"/sys/class/thermal","means":"…"}"#;

/// **The degradation this build must keep.** `Admin-Surface-Survey.md` §7.6
/// measured that the throttle flags need `/dev/vcio`; a board where that is
/// absent answers with `throttle_unmeasured` and NO flags, which must render as
/// *not measured* rather than as *never throttled*.
const THERMAL_NO_FLAGS: &str = r#"{"subject":"thermal",
  "zones":[{"name":"cpu_thermal","millidegrees":48312}],
  "zones_unreadable":[{"zone":"thermal_zone1","why":"Permission denied"}],
  "frequency_unmeasured":"no cpufreq policy under /sys/devices/system/cpu",
  "throttle_unmeasured":"/dev/vcio could not be opened: No such device",
  "read_from":"/sys/class/thermal","means":"…"}"#;

/// Hand-composed, from `rivet_core::operation::slot::OUTPUTS`.
const SLOT: &str = r#"{"subject":"slot","running":"b","inactive":"a","booted":"b",
  "booted_from":"/proc/device-tree/chosen/bootloader/partition","root":"b",
  "root_argument":"PARTLABEL=system-b","agree":true,"in_trial_boot":false,
  "read_from":"/proc/cmdline","means":"…"}"#;

/// **The finding the subject exists for**: the firmware and the kernel name
/// different slots, so no running slot is reported at all.
const SLOT_DISAGREE: &str = r#"{"subject":"slot","booted":"a",
  "booted_from":"/proc/device-tree/chosen/bootloader/partition","root":"b",
  "root_argument":"PARTLABEL=system-b","agree":false,
  "trial_unmeasured":"no /proc/device-tree on this machine",
  "read_from":"/proc/cmdline","means":"…"}"#;

/// Hand-composed, from `rivet_core::operation::boot::OUTPUTS`.
const BOOT: &str = r#"{"subject":"boot","boot_id":"c0ffee00-dead-beef-0000-000000000001",
  "uptime_seconds":97231,"idle_seconds":370000,"read_from":"/proc/uptime",
  "means":"…","boots":["a","b","c0ffee00-dead-beef-0000-000000000001"],"recorded":true}"#;

/// Hand-composed, from `rivet_core::operation::device::OUTPUTS`.
const DEVICE: &str = r#"{"subject":"device","devices":[
  {"name":"mmcblk0","kind":"disk","node":"/dev/mmcblk0","size_bytes":31914983424,
   "read_only":false,"removable":false,"model":"SC32G",
   "io":{"read_bytes":1073741824,"written_bytes":536870912,
     "reads_completed":12000,"writes_completed":4000}},
  {"name":"mmcblk0p3","kind":"partition","node":"/dev/mmcblk0p3",
   "partition_label":"system-a","partition_number":3,"parent":"mmcblk0",
   "size_bytes":4000000000,"read_only":false}],
  "devices_unreadable":[],"disks":1,"read_from":"/sys/class/block","means":"…"}"#;

// ---------------------------------------------------------------------------
// A machine that answers from a script
// ---------------------------------------------------------------------------

/// A [`Source`] that answers what it was told to, and records what was asked.
///
/// **It answers on demand rather than instantly**: the poller runs its source on
/// a worker thread, so a test that wants to observe a request *in flight* has to
/// be able to hold one there. `release` is that.
struct Scripted {
    answers: Vec<(Subject, Result<String, String>)>,
    asked: Sender<Subject>,
    /// A gate the test opens once per request, when it wants the answer to come
    /// back at a moment it chose.
    gate: Option<Receiver<()>>,
}

impl Source for Scripted {
    fn get(&mut self, subject: Subject) -> Result<String, String> {
        let _ = self.asked.send(subject);
        if let Some(gate) = &self.gate {
            // A closed gate ends the wait, so a dropped test does not hang.
            let _ = gate.recv();
        }
        self.answers
            .iter()
            .find(|(s, _)| *s == subject)
            .map_or_else(
                || Err(format!("this fake has no answer for {}", subject.word())),
                |(_, answer)| answer.clone(),
            )
    }
}

/// A [`Source`] that records **each batch it was handed**, whole.
///
/// The scripted source above answers one subject at a time and cannot see the
/// shape of a cycle; this one records exactly what
/// [`Source::get_each`] received, which is the only place the batching is
/// observable without a board.
struct Batched {
    answers: Vec<(Subject, Result<String, String>)>,
    batches: Sender<Vec<Subject>>,
}

impl Source for Batched {
    fn get(&mut self, subject: Subject) -> Result<String, String> {
        self.answers
            .iter()
            .find(|(s, _)| *s == subject)
            .map_or_else(
                || Err(format!("this fake has no answer for {}", subject.word())),
                |(_, answer)| answer.clone(),
            )
    }

    fn get_each(&mut self, subjects: &[Subject]) -> Vec<Result<String, String>> {
        let _ = self.batches.send(subjects.to_vec());
        subjects.iter().map(|s| self.get(*s)).collect()
    }
}

/// What a machine that answers everything says for one subject.
///
/// **One place**, so that a test about scheduling does not silently become a
/// test about which fixtures somebody remembered to add — a `match` here would
/// stop compiling when a subject is added, which is the point.
fn answer(subject: Subject) -> String {
    match subject {
        Subject::Cpu => CPU,
        Subject::Memory => MEMORY,
        Subject::Process => PROCESSES,
        Subject::Storage => STORAGE,
        Subject::Status => STATUS,
        Subject::Thermal => THERMAL,
        Subject::Slot => SLOT,
        Subject::Boot => BOOT,
        Subject::Device => DEVICE,
    }
    .to_string()
}

/// A poller over a scripted machine, plus the channel of what it asked for.
fn scripted(
    answers: Vec<(Subject, Result<String, String>)>,
) -> (Poller, Receiver<Subject>, Sender<()>) {
    let (asked, heard) = channel();
    let (open, gate) = channel();
    let source = Scripted {
        answers,
        asked,
        gate: Some(gate),
    };
    (Poller::spawn(Box::new(source)), heard, open)
}

/// Wait for the worker to answer, so the assertion is about a settled poller
/// rather than about a race. **Bounded**, so a broken poller fails the test
/// rather than hanging the suite.
///
/// **The gate is opened once per round**, because a cycle now carries every due
/// subject and [`Scripted`] waits on the gate for each of them. A single send
/// would hold a batch of nine at its second subject for ever.
fn settle(poller: &mut Poller, open: &Sender<()>) {
    for _ in 0..400 {
        let _ = open.send(());
        poller.collect();
        if !poller.busy() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the poller never came back from a request");
}

// ---------------------------------------------------------------------------
// The readings
// ---------------------------------------------------------------------------

#[test]
fn a_memory_answer_reads_every_field_the_subject_declares() {
    let Reading::Memory(memory) = read(Subject::Memory, MEMORY).expect("a reading") else {
        panic!("wrong reading");
    };
    assert_eq!(memory.total_bytes, 971_505_664);
    assert_eq!(memory.available_bytes, 612_345_678);
    assert_eq!(memory.free_bytes, 41_216_000);
    assert_eq!(memory.cached_bytes, 503_316_480);
    assert_eq!(memory.swap_total_bytes, 0);
    assert_eq!(
        memory.pressure,
        Some(Pressure {
            some_avg10: 42,
            some_avg60: 10,
            full_avg10: 0,
            full_avg60: 0,
        })
    );
}

/// **The single most important assertion in this file.** A kernel with no
/// `CONFIG_PSI` has not measured *no* stalling — it has measured nothing, and a
/// screen that rendered zeroes would be asserting the machine is fine.
#[test]
fn a_kernel_without_psi_reports_no_pressure_rather_than_zero_pressure() {
    let Reading::Memory(memory) = read(Subject::Memory, MEMORY_NO_PSI).expect("a reading") else {
        panic!("wrong reading");
    };
    assert_eq!(memory.pressure, None);
    // And the rest of the reading is unaffected: absence of one fact is not
    // absence of the answer.
    assert_eq!(memory.total_bytes, 971_505_664);
}

/// **A required field that is missing is a refusal, not a zero.** A build whose
/// `memory` has no `cached_bytes` has changed shape, and rendering `0` would
/// state a number nobody measured.
#[test]
fn a_missing_required_field_refuses_and_names_itself() {
    let refusal = read(
        Subject::Memory,
        r#"{"subject":"memory","total_bytes":1,"available_bytes":1,"free_bytes":1}"#,
    )
    .expect_err("a short answer must refuse");
    assert!(refusal.contains("cached_bytes"), "{refusal}");
    assert!(refusal.contains("AGENTS.md"), "{refusal}");
}

#[test]
fn a_process_answer_reads_the_table_and_keeps_the_optional_field_optional() {
    let Reading::Processes(processes) = read(Subject::Process, PROCESSES).expect("a reading")
    else {
        panic!("wrong reading");
    };
    assert_eq!(processes.rows.len(), 4);
    assert_eq!(processes.vanished, 1);
    assert_eq!(processes.userland, 3);
    assert_eq!(processes.rows[0].name, "rivet-initd");
    assert_eq!(processes.rows[0].resident_bytes, Some(2_162_688));
    // **A kernel thread has no user address space**, and `Some(0)` would claim
    // it occupies none — a different statement, and a false one.
    assert!(processes.rows[1].kernel);
    assert_eq!(processes.rows[1].resident_bytes, None);

    // **The two CPU counters are summed into one figure and are BOTH-or-NEITHER.**
    // A row carrying one and not the other is a shape this screen has not seen,
    // and summing half of it would report a process as doing half the work it is
    // doing.
    assert_eq!(processes.rows[2].cpu_millis, Some(4_900));
    assert_eq!(
        processes.rows[1].cpu_millis, None,
        "a counter the machine did not read was invented"
    );
}

/// A row with `user_millis` and no `system_millis` — a shape no build emits, and
/// one this reader must not half-read. **Neither**, because half a sum reported
/// as a whole one is a number nobody measured.
#[test]
fn a_row_carrying_half_the_cpu_counters_carries_none_of_them() {
    let half = r#"{"subject":"process","processes":[
      {"pid":1,"parent_pid":0,"name":"x","state":"S","threads":1,"kernel":false,
       "resident_bytes":1,"user_millis":4000}],"vanished":0,"userland":1}"#;
    let Reading::Processes(processes) = read(Subject::Process, half).expect("a reading") else {
        panic!("wrong reading");
    };
    assert_eq!(processes.rows[0].cpu_millis, None);
}

/// `D` and `Z` are the states worth reading twice — uninterruptible sleep on a
/// Pi is very often I/O against a failing card.
#[test]
fn uninterruptible_sleep_and_zombies_are_flagged_and_nothing_else_is() {
    let Reading::Processes(processes) = read(Subject::Process, PROCESSES).expect("a reading")
    else {
        panic!("wrong reading");
    };
    let trouble: Vec<&str> = processes
        .rows
        .iter()
        .filter(|p| p.is_trouble())
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(trouble, vec!["stuck"]);
}

#[test]
fn a_storage_answer_reads_and_keeps_capacity_optional() {
    let Reading::Storage(storage) = read(Subject::Storage, STORAGE).expect("a reading") else {
        panic!("wrong reading");
    };
    assert_eq!(storage.filesystems.len(), 2);
    assert_eq!(storage.filesystems[0].mount_point, "/state");
    assert_eq!(storage.filesystems[0].capacity_percent, Some(31));
    // `/proc` carries no capacity at all — the subject's own documentation says
    // it is absent rather than 0, and that survives to here.
    assert_eq!(storage.filesystems[1].capacity_percent, None);
}

#[test]
fn a_status_answer_reads_the_supervisors_own_belief() {
    let Reading::Supervisor(supervisor) = read(Subject::Status, STATUS).expect("a reading")
    else {
        panic!("wrong reading");
    };
    assert_eq!(supervisor.phase, "running");
    assert_eq!(supervisor.services.len(), 3);
    assert_eq!(supervisor.services[0].pid, Some(214));
    // A service the supervisor is not running has no pid, and that absence is
    // exactly what the cross-check on screen reads.
    assert_eq!(supervisor.services[2].pid, None);
}

/// **The counters, and nothing that reads as a percentage.** `get cpu` answers
/// milliseconds since boot; a utilisation is what two of these readings make,
/// and it is computed one layer up.
#[test]
fn a_cpu_answer_reads_the_counters_and_carries_no_percentage() {
    let Reading::Cpu(cpu) = read(Subject::Cpu, CPU).expect("a reading") else {
        panic!("wrong reading");
    };
    assert_eq!(cpu.processors, 4);
    assert_eq!(cpu.cpus.len(), 4);
    assert_eq!(cpu.total.total_millis, 2_010_000);
    assert_eq!(cpu.total.busy_millis, 500_000);
    // `busy` excludes `iowait` as well as `idle`, which the subject computed and
    // this reader takes rather than recomputing — so the two are separate
    // figures that do not add up to the whole.
    assert_eq!(cpu.total.iowait_millis, 10_000);
    assert!(
        cpu.total.busy_millis + cpu.total.iowait_millis < cpu.total.total_millis,
        "busy and waiting must be distinct shares of a larger whole"
    );
    assert_eq!(cpu.load.map(|l| l.avg1), Some(412));
}

/// **A machine that states no load average reports none, not three zeroes.** A
/// load of zero is a real reading of an idle machine and it is the figure a
/// person is likeliest to act on without checking.
#[test]
fn a_machine_with_no_load_average_reports_none_rather_than_zero() {
    let Reading::Cpu(cpu) = read(Subject::Cpu, CPU_NO_LOAD).expect("a reading") else {
        panic!("wrong reading");
    };
    assert_eq!(cpu.load, None);
    // And the counters are unaffected: one fact missing is not the answer
    // missing.
    assert_eq!(cpu.total.total_millis, 10);
}

/// **The whole of the thermal subject, and the throttle is what it leads with.**
#[test]
fn a_thermal_answer_reads_the_clock_the_flags_and_the_temperature() {
    let Reading::Thermal(thermal) = read(Subject::Thermal, THERMAL).expect("a reading") else {
        panic!("wrong reading");
    };
    let clock = thermal.clock.as_ref().expect("a cpufreq policy");
    assert_eq!(clock.current_hz, 600_000_000);
    assert_eq!(clock.maximum_hz, 1_500_000_000);
    // **The comparison that is the throttle**, and it is the ceiling's rather
    // than the current clock's: a governor scales an idle machine down.
    assert_eq!(clock.ceiling_hz, Some(1_000_000_000));
    assert_eq!(clock.ceiling_below_maximum, Some(true));
    assert!(!clock.at_maximum);

    assert_eq!(thermal.ever_held_back(), Some(true));
    assert_eq!(thermal.hottest().map(|z| z.millidegrees), Some(52_341));
}

/// **THE assertion of this file, after the PSI one.** A board whose firmware
/// could not be reached has NOT established that it has never been under-volted
/// — it has established nothing — and eight falses would read as a clean bill of
/// health for the last several hours of its life.
///
/// `Admin-Surface-Survey.md` §7.6 measured that these flags need `/dev/vcio`, so
/// this is not a hypothetical degradation.
#[test]
fn a_board_whose_firmware_could_not_be_reached_reports_no_flags_rather_than_clear_ones() {
    let Reading::Thermal(thermal) =
        read(Subject::Thermal, THERMAL_NO_FLAGS).expect("a reading")
    else {
        panic!("wrong reading");
    };
    assert_eq!(thermal.throttle, None);
    assert_eq!(
        thermal.ever_held_back(),
        None,
        "an unread flag was answered as a measurement"
    );
    assert!(
        thermal
            .throttle_unmeasured
            .as_deref()
            .is_some_and(|why| why.contains("/dev/vcio")),
        "{:?}",
        thermal.throttle_unmeasured
    );
    // The clock is missing on its own and says so on its own — the halves fail
    // independently, which is the subject's own shape.
    assert_eq!(thermal.clock, None);
    assert!(thermal.clock_unmeasured.is_some());
    // And the half that WAS read still arrives.
    assert_eq!(thermal.hottest().map(|z| z.millidegrees), Some(48_312));
    assert_eq!(thermal.zones_unreadable.len(), 1);
}

/// A slot answer whose two authorities agree names the running slot and the one
/// an update writes into.
#[test]
fn a_slot_answer_names_the_running_slot_and_the_one_an_update_writes() {
    let Reading::Slots(slots) = read(Subject::Slot, SLOT).expect("a reading") else {
        panic!("wrong reading");
    };
    assert_eq!(slots.running.as_deref(), Some("b"));
    assert_eq!(slots.inactive.as_deref(), Some("a"));
    assert_eq!(slots.agree, Some(true));
    assert_eq!(slots.in_trial_boot, Some(false));
}

/// **And one whose authorities disagree names NO running slot**, which is the
/// finding rather than a parse failure: a machine that booted one slot's
/// firmware configuration into the other's filesystem is one where
/// `apply trial-boot` would arm the wrong partition.
#[test]
fn a_slot_answer_with_two_authorities_disagreeing_names_no_running_slot() {
    let Reading::Slots(slots) = read(Subject::Slot, SLOT_DISAGREE).expect("a reading") else {
        panic!("wrong reading");
    };
    assert_eq!(slots.running, None, "a slot was named from a disagreement");
    assert_eq!(slots.agree, Some(false));
    assert_eq!(slots.booted.as_deref(), Some("a"));
    assert_eq!(slots.root.as_deref(), Some("b"));
    // **Absent, never `false`** — `false` would assert that doing nothing is
    // safe, which an unread flag has not established.
    assert_eq!(slots.in_trial_boot, None);
    assert!(slots.trial_unmeasured.is_some());
}

#[test]
fn a_boot_answer_reads_the_uptime_and_whether_this_boot_is_in_the_journal() {
    let Reading::Boot(boot) = read(Subject::Boot, BOOT).expect("a reading") else {
        panic!("wrong reading");
    };
    assert_eq!(boot.uptime_seconds, 97_231);
    assert_eq!(boot.boots, Some(3));
    assert_eq!(boot.recorded, Some(true));
}

#[test]
fn a_device_answer_reads_the_partition_labels_and_the_bytes_written() {
    let Reading::Devices(devices) = read(Subject::Device, DEVICE).expect("a reading") else {
        panic!("wrong reading");
    };
    assert_eq!(devices.disks, 1);
    assert_eq!(devices.devices.len(), 2);
    assert_eq!(devices.devices[0].written_bytes, Some(536_870_912));
    assert_eq!(
        devices.devices[1].partition_label.as_deref(),
        Some("system-a")
    );
    // **Absent where the device states none**, which a consumer SD card does —
    // and a percentage computed for one would be invented.
    assert_eq!(devices.devices[0].life_time, None);
}

/// **Every subject this screen polls has a reader written against its real field
/// names**, and none is shown verbatim any more.
///
/// The test this replaces asserted the opposite: that `thermal`'s answer was
/// carried through untouched, because no such subject existed and a renderer
/// written against invented names would have been a guess that succeeds. That
/// escape hatch — `Reading::Verbatim` and `Json::scalars` — is gone with the
/// last thing that needed it, and this is what stands where it did: an assertion
/// that every subject parses into a *shape* rather than into a bag of strings.
#[test]
fn every_polled_subject_reads_into_a_shape_rather_than_into_strings() {
    for subject in Subject::ALL {
        let reading = read(*subject, &answer(*subject))
            .unwrap_or_else(|e| panic!("`get {}` did not read: {e}", subject.word()));
        // The variant a subject reads into is the subject's own, which is what
        // makes a renderer possible at all.
        let matched = matches!(
            (subject, &reading),
            (Subject::Cpu, Reading::Cpu(_))
                | (Subject::Memory, Reading::Memory(_))
                | (Subject::Process, Reading::Processes(_))
                | (Subject::Storage, Reading::Storage(_))
                | (Subject::Status, Reading::Supervisor(_))
                | (Subject::Thermal, Reading::Thermal(_))
                | (Subject::Slot, Reading::Slots(_))
                | (Subject::Boot, Reading::Boot(_))
                | (Subject::Device, Reading::Devices(_))
        );
        assert!(
            matched,
            "`get {}` read into {reading:?}, which is not its own shape",
            subject.word()
        );
    }
}

/// Text that is not JSON at all — a shell's complaint, a transport's banner — fails
/// loudly rather than becoming an empty table.
#[test]
fn an_answer_that_is_not_json_refuses_rather_than_reading_as_empty() {
    for text in ["", "bash: rivet: not found", "<html>"] {
        assert!(read(Subject::Memory, text).is_err(), "{text:?}");
        assert!(read(Subject::Process, text).is_err(), "{text:?}");
    }
}

// ---------------------------------------------------------------------------
// Telling a missing subject from a broken machine
// ---------------------------------------------------------------------------

/// **The code, not the sentence.** A build without `get thermal` is a permanent
/// answer; a board that timed out is a transient one, and treating them the same
/// means either retrying something that will never work or giving up on
/// something that would.
#[test]
fn a_subject_this_build_does_not_have_is_told_apart_from_a_machine_that_failed() {
    let missing = Outcome::classify(
        r#"`rivet "get" "thermal"` failed (exit status: 1): {"error":"E_UNKNOWN_SUBJECT","subject":"thermal"}"#
            .to_string(),
    );
    assert!(matches!(missing, Outcome::Unsupported(_)), "{missing:?}");

    for transient in [
        "cannot reach 10.0.0.2: connection refused",
        r#"failed (exit status: 255): Host key verification failed."#,
        r#"{"error":"E_MACHINE_REFUSED","subject":"memory"}"#,
    ] {
        let outcome = Outcome::classify(transient.to_string());
        assert!(matches!(outcome, Outcome::Failed(_)), "{transient}: {outcome:?}");
    }
}

// ---------------------------------------------------------------------------
// The poller
// ---------------------------------------------------------------------------

/// **Cycles never overlap.** A slow link must not become a backlog that hammers
/// the board when it recovers — so a tick while one is in flight is skipped and
/// counted, never queued.
#[test]
fn a_second_cycle_is_skipped_rather_than_queued_while_one_is_in_flight() {
    let (mut poller, _asked, open) = scripted(vec![(Subject::Memory, Ok(MEMORY.to_string()))]);
    let start = Instant::now();

    // A freshly opened pane has everything due, and it all goes at once.
    assert_eq!(
        poller.tick(start, true).len(),
        Subject::ALL.len(),
        "a cycle carries every subject that is due"
    );
    assert!(poller.busy());
    assert_eq!(poller.skipped, 0);

    // The fast tier comes due again while that cycle is still out. Nothing is
    // sent, and the screen is told it is falling behind.
    let mut later = start;
    for _ in 0..5 {
        later += FAST_DEFAULT + Duration::from_secs(1);
        assert!(poller.tick(later, true).is_empty());
    }
    assert_eq!(poller.skipped, 5);

    settle(&mut poller, &open);
    // Now the slot is free again and the next cycle goes.
    assert!(!poller.tick(later, true).is_empty());
}

/// **One cycle, one call to the transport** — `ADR-034` §11.5.2's whole
/// optimisation, at the seam where it is decidable.
///
/// `rivet-mcpd` spawns a `rivet mcp` process **per connection**, so a cycle that
/// reached the source once per subject would cost a 926 MiB board one process
/// per subject to learn what one connection answers. This is what says the
/// poller hands the batch over as a batch; whether the transport then puts it on
/// one socket is `rivet_remote::tls`'s to prove, and it does.
#[test]
fn every_subject_due_on_a_tick_reaches_the_source_in_one_call() {
    let (batches, seen) = channel();
    let source = Batched {
        answers: Subject::ALL.iter().map(|s| (*s, Ok(answer(*s)))).collect(),
        batches,
    };
    let mut poller = Poller::spawn(Box::new(source));

    let start = Instant::now();
    let sent = poller.tick(start, true);
    assert_eq!(sent.len(), Subject::ALL.len());
    for _ in 0..400 {
        poller.collect();
        if !poller.busy() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!poller.busy(), "the batch never came back");

    let calls: Vec<Vec<Subject>> = seen.try_iter().collect();
    assert_eq!(calls.len(), 1, "nine subjects took {} calls", calls.len());
    let mut asked = calls[0].clone();
    asked.sort_unstable();
    let mut wanted = Subject::ALL.to_vec();
    wanted.sort_unstable();
    assert_eq!(asked, wanted, "the cycle did not carry every due subject");
}

/// **Polling stops when nobody is looking**, which is the largest saving
/// available and costs nothing.
#[test]
fn nothing_is_sent_while_the_screen_is_not_the_one_on_top() {
    let (mut poller, _asked, _open) = scripted(vec![]);
    let now = Instant::now();
    for _ in 0..10 {
        assert!(poller.tick(now, false).is_empty());
    }
    // And it is not counted as falling behind, because it is not.
    assert_eq!(poller.skipped, 0);
    assert!(!poller.busy());
}

/// The same for a person who has stopped it by hand.
#[test]
fn nothing_is_sent_while_a_person_has_paused_it() {
    let (mut poller, _asked, _open) = scripted(vec![]);
    poller.paused = true;
    assert!(poller.tick(Instant::now(), true).is_empty());
    poller.paused = false;
    assert!(!poller.tick(Instant::now(), true).is_empty());
}

/// **A tier is a floor, not a queue position.** Every subject that is due goes
/// on one connection, and the order within that cycle is most-overdue-first —
/// so a cycle cut short by a transport that dies mid-batch has asked the
/// questions that mattered most.
#[test]
fn a_cycle_carries_what_is_due_most_overdue_first() {
    let (mut poller, asked, open) = scripted(
        Subject::ALL.iter().map(|s| (*s, Ok(answer(*s)))).collect(),
    );
    let start = Instant::now();
    // One cycle fills everything in.
    assert!(!poller.tick(start, true).is_empty());
    settle(&mut poller, &open);
    let seen: Vec<Subject> = asked.try_iter().collect();
    for subject in Subject::ALL {
        assert!(seen.contains(subject), "{subject:?} was never asked");
    }
    // Nothing is due again immediately.
    assert!(poller.tick(start, true).is_empty());

    // **The fast tier is moved to one second first**, because its default is
    // five and the medium tier's is also five — at the defaults the two come due
    // together, and a test of *which is more overdue* needs them apart.
    poller.cycle_fast_interval();
    poller.cycle_fast_interval();
    assert_eq!(poller.fast_interval(), Duration::from_secs(1));

    // Six seconds on: the fast tier is 5 s past its interval and the medium 1 s.
    let later = start + Duration::from_secs(6);
    let cycle = poller.tick(later, true);
    assert!(
        cycle.iter().all(|s| s.tier() != Tier::Once),
        "the once tier was asked again: {cycle:?}"
    );
    assert_eq!(
        cycle.first().map(|s| s.tier()),
        Some(Tier::Fast),
        "the most overdue tier is not first in the cycle: {cycle:?}"
    );
    assert!(
        cycle.iter().any(|s| s.tier() == Tier::Medium),
        "a due subject was left out of the cycle: {cycle:?}"
    );
}

/// **Failure widens the interval.** Retrying a dead board every two seconds is
/// how a monitoring tool becomes part of the outage.
#[test]
fn a_failing_subject_is_backed_off_and_the_back_off_is_capped() {
    // Every subject fails, so every subject backs off and the scheduler has no
    // reason to prefer one — which keeps this a test about the back-off rather
    // than about the order.
    let (mut poller, _asked, open) = scripted(
        Subject::ALL
            .iter()
            .map(|s| (*s, Err("cannot reach the machine".to_string())))
            .collect(),
    );
    let mut now = Instant::now();
    assert_eq!(poller.interval(Subject::Memory), FAST_DEFAULT);

    let mut last = FAST_DEFAULT;
    let mut widened_at_least_once = false;
    for round in 1..=40 {
        if !poller.tick(now, true).is_empty() {
            settle(&mut poller, &open);
        }
        let interval = poller.interval(Subject::Memory);
        assert!(interval >= last, "round {round}: {interval:?} < {last:?}");
        assert!(interval <= MAX_INTERVAL, "round {round}: {interval:?}");
        widened_at_least_once |= interval > last;
        last = interval;
        // Far enough that everything is due again whatever its back-off is.
        now += MAX_INTERVAL + Duration::from_secs(1);
    }
    assert!(widened_at_least_once, "the interval never widened at all");
    assert_eq!(last, MAX_INTERVAL, "the back-off never reached its cap");

    // The slot says what went wrong, rather than the screen showing nothing.
    let slot = poller.slot(Subject::Memory).expect("a slot");
    assert!(matches!(slot.outcome, Outcome::Failed(_)), "{:?}", slot.outcome);
    assert!(slot.reading.is_none(), "it never answered, so there is nothing to show");
}

/// A subject the board's build does not have is asked **once** and then left
/// alone — and `r` is how a person asks again after deploying a build that has
/// it, because there is no other way to find out.
#[test]
fn an_unsupported_subject_is_asked_once_and_re_armed_only_by_a_manual_refresh() {
    let (mut poller, asked, open) = scripted(vec![(
        Subject::Thermal,
        Err(format!(r#"{{"error":"{UNKNOWN_SUBJECT}","subject":"thermal"}}"#)),
    )]);
    let mut now = Instant::now();

    // Drive until thermal has been asked and refused.
    for _ in 0..20 {
        if !poller.tick(now, true).is_empty() {
            settle(&mut poller, &open);
        }
        now += Duration::from_secs(1);
    }
    let asked_for_thermal = asked
        .try_iter()
        .filter(|s| *s == Subject::Thermal)
        .count();
    assert_eq!(asked_for_thermal, 1, "it was asked more than once");
    assert!(matches!(
        poller.slot(Subject::Thermal).expect("a slot").outcome,
        Outcome::Unsupported(_)
    ));

    poller.refresh_all();
    assert!(matches!(
        poller.slot(Subject::Thermal).expect("a slot").outcome,
        Outcome::Never
    ));
}

/// The fast tier is adjustable from the keyboard — `top`'s `d` — and the others
/// are not, because five seconds for thermals is a fact about thermal mass
/// rather than a matter of taste.
#[test]
fn the_fast_interval_cycles_and_only_the_fast_tier_moves() {
    let (mut poller, _asked, _open) = scripted(vec![]);
    let medium_before = poller.interval(Subject::Thermal);
    let slow_before = poller.interval(Subject::Storage);

    let mut seen = vec![poller.fast_interval()];
    for _ in 0..FAST_INTERVALS.len() {
        poller.cycle_fast_interval();
        seen.push(poller.fast_interval());
    }
    // It visited every option and came back to where it started.
    for wanted in FAST_INTERVALS {
        assert!(seen.contains(wanted), "{wanted:?} was skipped: {seen:?}");
    }
    assert_eq!(seen.first(), seen.last());

    assert_eq!(poller.interval(Subject::Thermal), medium_before);
    assert_eq!(poller.interval(Subject::Storage), slow_before);
}

/// **A reading is stale before it is wrong.** Two and a half intervals: one
/// would flag every reading in the moment before its refresh landed, three would
/// let a whole missed cycle pass unremarked.
#[test]
fn a_reading_is_called_stale_at_two_and_a_half_intervals() {
    let mut slot = Slot::new(Subject::Memory);
    let at = Instant::now();
    // Any reading will do — staleness is a fact about *when* a value arrived and
    // not about what it was.
    slot.reading = Some((read(Subject::Memory, MEMORY).expect("a reading"), at));
    let interval = Duration::from_secs(2);
    assert!(!slot.is_stale(at + Duration::from_secs(2), interval));
    assert!(!slot.is_stale(at + Duration::from_millis(4_900), interval));
    assert!(slot.is_stale(at + Duration::from_millis(5_100), interval));
    // A slot with nothing in it is not stale; it is empty, which the screen
    // says differently.
    let empty = Slot::new(Subject::Memory);
    assert!(!empty.is_stale(at + Duration::from_secs(600), interval));
}

/// **A poller with no reading says so rather than showing a zero.** The
/// difference between *not asked yet* and *asked and got nothing* is the
/// difference `crate::tui::reach` documents for reachability, one layer in.
#[test]
fn a_freshly_opened_poller_has_asked_nothing_and_claims_nothing() {
    let (poller, _asked, _open) = scripted(vec![]);
    for subject in Subject::ALL {
        let slot = poller.slot(*subject).expect("a slot");
        assert_eq!(slot.outcome, Outcome::Never);
        assert!(slot.value().is_none());
        assert_eq!(slot.age(Instant::now()), None);
    }
}

// ---------------------------------------------------------------------------
// The rate, which is what two readings make and one does not
// ---------------------------------------------------------------------------

/// **The single most important assertion about CPU on this screen.** One reading
/// of a counter is not a rate, and the honest answer is *I do not know* rather
/// than `0%` — which is a measurement of an idle machine and the one number a
/// monitor must never guess.
#[test]
fn one_cpu_reading_yields_no_utilisation_at_all() {
    let mut poller = seeded(Vec::new());
    let Reading::Cpu(cpu) = read(Subject::Cpu, CPU).expect("a reading") else {
        panic!("wrong reading");
    };
    poller.observe(Subject::Cpu, Reading::Cpu(cpu), Instant::now());
    assert!(
        poller.cpu_rates().is_none(),
        "a percentage was produced from one reading"
    );
}

/// And the second reading is what makes it exist. The fixture is a machine that
/// spent **half a second of one core's time working** over a wall second, on a
/// board with one processor — 50%.
#[test]
fn a_second_reading_is_what_makes_a_utilisation_exist() {
    let mut poller = seeded(Vec::new());
    let at = Instant::now();
    poller.observe(Subject::Cpu, Reading::Cpu(counters(0, 0)), at);
    // 1000 ms of processor time elapsed, 500 of it busy and 100 waiting.
    poller.observe(
        Subject::Cpu,
        Reading::Cpu(counters(500, 100)),
        at + Duration::from_secs(1),
    );

    let rates = poller.cpu_rates().expect("two readings make a rate");
    assert_eq!(rates.total, 50);
    // **Reported apart from busy**, because a board at 90% iowait is waiting on
    // a failing card rather than working.
    assert_eq!(rates.iowait, 10);
    assert_eq!(rates.cpus, vec![("cpu0".to_string(), 50)]);
    // The interval is carried, because a rate over an unstated interval is not a
    // measurement.
    assert_eq!(rates.over, Duration::from_secs(1));
}

/// **A machine that rebooted between two readings has no rate**, rather than a
/// confident one. Its counters restarted, so the subtraction is negative — which
/// a saturating one would turn into `0%`, the most reassuring possible answer at
/// the worst possible moment.
#[test]
fn counters_that_went_backwards_produce_no_rate_rather_than_zero() {
    let mut poller = seeded(Vec::new());
    let at = Instant::now();
    poller.observe(Subject::Cpu, Reading::Cpu(counters(500_000, 1_000)), at);
    // The board rebooted: the same counters, near zero again.
    poller.observe(
        Subject::Cpu,
        Reading::Cpu(counters(0, 0)),
        at + Duration::from_secs(2),
    );
    assert!(
        poller.cpu_rates().is_none(),
        "a rate was computed across a reboot"
    );
}

/// Per-process CPU, on the same terms — and it is a share of **one** processor,
/// which is `top`'s convention and is what makes a four-threaded process on four
/// cores read 400%.
#[test]
fn a_per_process_rate_is_a_share_of_one_processor_and_needs_two_readings() {
    let mut poller = seeded(Vec::new());
    let at = Instant::now();
    let first = read(Subject::Process, PROCESSES).expect("a reading");
    poller.observe(Subject::Process, first, at);
    assert!(
        poller.process_rates().is_none(),
        "a per-process percentage was produced from one reading"
    );

    // A second later, pid 214 has used another 250 ms of CPU and pid 1 none.
    let moved = PROCESSES.replace(r#""user_millis":4000"#, r#""user_millis":4250"#);
    let second = read(Subject::Process, &moved).expect("a reading");
    poller.observe(Subject::Process, second, at + Duration::from_secs(1));

    let rates = poller.process_rates().expect("two readings make a rate");
    assert_eq!(rates.of(214), Some(2_500), "250 ms in 1 s is 25.00%");
    assert_eq!(rates.of(1), Some(0), "it really did use no CPU");
    // **A pid that was not in the earlier reading gets NO entry**, rather than a
    // zero — a process that has just started is very often the busy one.
    assert_eq!(rates.of(9_999), None);
}

/// **A process whose CPU time the machine could not read gets no rate**, rather
/// than one computed from a zero it never measured.
#[test]
fn a_process_with_no_cpu_counter_is_absent_from_the_rates_rather_than_zeroed() {
    let Reading::Processes(processes) = read(Subject::Process, PROCESSES).expect("a reading")
    else {
        panic!("wrong reading");
    };
    let without = processes
        .rows
        .iter()
        .find(|p| p.cpu_millis.is_none())
        .expect("the fixture must carry a row whose CPU time was not read");
    let pid = without.pid;

    let mut poller = seeded(Vec::new());
    let at = Instant::now();
    poller.observe(
        Subject::Process,
        Reading::Processes(processes.clone()),
        at,
    );
    poller.observe(
        Subject::Process,
        Reading::Processes(processes),
        at + Duration::from_secs(1),
    );
    let rates = poller.process_rates().expect("a rate");
    assert_eq!(rates.of(pid), None, "a rate was invented for an unread counter");
}

/// One processor's counters, for the rate tests. **Hand-composed**, and the
/// arithmetic rather than the shape is what they are for.
fn counters(busy: u64, waited: u64) -> Cpu {
    let times = |name: &str| CpuTimes {
        name: name.to_string(),
        // The whole advances by twice the busy figure, so `counters(500, 100)`
        // against `counters(0, 0)` is 500 busy and 100 waiting out of 1000 —
        // 50% and 10%, readable by inspection.
        total_millis: busy * 2,
        busy_millis: busy,
        iowait_millis: waited,
    };
    Cpu {
        processors: 1,
        total: times("cpu"),
        cpus: vec![times("cpu0")],
        load: None,
    }
}

// ---------------------------------------------------------------------------
// The `once` tier
// ---------------------------------------------------------------------------

/// **Asked until it answers, and then never again.** Which slot booted cannot
/// change without a reboot, and a reboot ends the session watching — so
/// re-asking would spend a journal record (`ADR-022` §8's 8 MiB, this module's
/// governing constraint) to learn what is already on screen.
#[test]
fn a_once_tier_subject_is_asked_until_it_answers_and_then_left_alone() {
    let (mut poller, asked, open) = scripted(
        Subject::ALL.iter().map(|s| (*s, Ok(answer(*s)))).collect(),
    );
    let mut now = Instant::now();
    // Long enough that every other tier has come round many times.
    for _ in 0..200 {
        if !poller.tick(now, true).is_empty() {
            settle(&mut poller, &open);
        }
        now += Duration::from_secs(1);
    }
    let seen: Vec<Subject> = asked.try_iter().collect();
    for once in [Subject::Slot, Subject::Boot, Subject::Device] {
        assert_eq!(
            seen.iter().filter(|s| **s == once).count(),
            1,
            "`get {}` was asked more than once over 200 s",
            once.word()
        );
    }
    // And the fast tier really did keep going, so the test above is about the
    // tier rather than about a poller that stopped.
    assert!(
        seen.iter().filter(|s| **s == Subject::Memory).count() > 10,
        "the fast tier stopped, so nothing was proven about the once tier"
    );
}

/// **A `once` subject that FAILED is still retried**, with the back-off — *the
/// machine did not answer* is not *the machine answered*, and giving up on the
/// first would leave the pane permanently blank about a board that came back.
#[test]
fn a_once_tier_subject_that_failed_is_retried_rather_than_abandoned() {
    let (mut poller, asked, open) = scripted(vec![(
        Subject::Slot,
        Err("cannot reach the machine".to_string()),
    )]);
    let mut now = Instant::now();
    for _ in 0..30 {
        if !poller.tick(now, true).is_empty() {
            settle(&mut poller, &open);
        }
        now += MAX_INTERVAL + Duration::from_secs(1);
    }
    assert!(
        asked.try_iter().filter(|s| *s == Subject::Slot).count() > 1,
        "a `once` subject that never answered was asked only once"
    );
}

/// And `r` re-arms it, which is how a person asks again after doing something
/// that could have changed the answer.
#[test]
fn a_manual_refresh_re_arms_the_once_tier() {
    let (mut poller, asked, open) = scripted(
        Subject::ALL.iter().map(|s| (*s, Ok(answer(*s)))).collect(),
    );
    let mut now = Instant::now();
    for _ in 0..40 {
        if !poller.tick(now, true).is_empty() {
            settle(&mut poller, &open);
        }
        now += Duration::from_secs(1);
    }
    let before = asked.try_iter().filter(|s| *s == Subject::Slot).count();
    assert_eq!(before, 1);

    poller.refresh_all();
    for _ in 0..40 {
        if !poller.tick(now, true).is_empty() {
            settle(&mut poller, &open);
        }
        now += Duration::from_secs(1);
    }
    assert_eq!(
        asked.try_iter().filter(|s| *s == Subject::Slot).count(),
        1,
        "`r` did not re-ask the once tier"
    );
}

// ---------------------------------------------------------------------------
// The tripwire
// ---------------------------------------------------------------------------

/// **This screen polls only subjects the core actually answers**, checked
/// against `rivet_core` rather than against a comment.
///
/// It has no exceptions any more. It carried one for `thermal` — the subject the
/// pane polled *hoping* for an answer — from the day the pane was written until
/// the day the subject landed, which is the same day.
#[test]
fn the_subjects_this_screen_polls_are_ones_the_core_answers() {
    let get = rivet_core::operation::operation(rivet_core::operation::Verb::Get)
        .expect("the core answers `get`");
    let known: Vec<&str> = get.subjects().into_iter().map(|s| s.name).collect();
    for subject in Subject::ALL {
        assert!(
            known.contains(&subject.word()),
            "the monitor polls `get {}`, which this workspace's core does not answer. \
             Known: {known:?}",
            subject.word()
        );
    }
}

/// **The roadmap, as an assertion that fails when it is done** — and it has
/// fired once already, which is the point.
///
/// # What it said before, and what happened
///
/// It named `thermal`, `slot`, `boot`, `device` and `network` as subjects that
/// did not exist, and asserted their absence so that the day one landed a person
/// would be *told* rather than left to notice. **Four of the five landed on
/// 2026-08-29 and this test failed**, which is how the monitor got a `once` tier,
/// a thermal renderer and a CPU section. That is the mechanism working, so it is
/// kept rather than deleted.
///
/// # What it names now, and why each one
///
/// A subject this pane does not poll, that a monitor could use. Each is a
/// deliberate omission rather than an oversight:
///
/// * **`network`** — the pane shows what a machine is *doing to itself*.
///   Throughput is a rate like CPU and would need the same two-reading
///   machinery; whether it belongs on this screen or on `crate::tui::reach`,
///   which already owns reachability, is not this pane's call to make alone.
/// * **`install`** — the last install and how it ended is history, and the
///   Journal pane already answers it.
/// * **`trust`** — added 2026-08-29 with the subject. It is a machine fact and
///   `ADR-028` §5.5.6 says so in as many words, but **it does not move**:
///   `ADR-028` §5.5.4 makes the only way to change what a machine trusts an A/B
///   image update, so polling it on a monitor tick spends a round trip per
///   refresh on a value that cannot have changed since the last reboot. A
///   monitor is for what is changing. Where it *would* belong is beside a
///   deploy — *"will this board take what I signed"* — and that is
///   `cargo rivet run … -- get trust` today, on demand, which is the right
///   shape for a question asked once.
/// * **`phones`** — the phones paired with the fleet (`ADR-061`). It is a
///   record of the fleet server's, not a reading of the machine, and it changes
///   only when somebody pairs, renames or revokes a phone, each of which is an
///   act the operator is already watching. `cargo rivet fleet phones list`
///   answers it on demand; a refresh tick would spend a round trip on a list
///   that has not moved.
/// * **`fleet-root`** — the root certificate a fleet server mints once, at its
///   first boot (`ADR-063`). It is fixed for the life of that fleet, so polling
///   it learns nothing; it is fetched once, on demand, by
///   `cargo rivet fleet root <host>` when a workstation needs to trust it.
///
/// # It fired a second time, on 2026-08-30
///
/// `get catalogue` landed (`ADR-006` §8.13) and this test named it unaccounted
/// for. It is in `not_machine_facts` rather than `declined`, because it is not a
/// reading of a running machine at all: it is what this *build* can be asked,
/// and it cannot change while somebody watches — the `once` tier's own argument,
/// taken one step further to *not at all*.
///
/// **If a subject is added to `get` and is not here and not polled, this test
/// does not fire** — it cannot, because it names what it knows about. That is
/// [`the_subjects_this_screen_polls_are_ones_the_core_answers`]'s half of the
/// pair: that one checks every polled subject is real, and this one checks that
/// every real subject is either polled or deliberately not.
#[test]
fn the_subjects_the_monitor_does_not_poll() {
    let get = rivet_core::operation::operation(rivet_core::operation::Verb::Get)
        .expect("the core answers `get`");
    let known: Vec<&str> = get.subjects().into_iter().map(|s| s.name).collect();
    let polled: Vec<&str> = Subject::ALL.iter().map(|s| s.word()).collect();

    // Subjects that exist, are not polled, and are declared here as not being
    // monitor material — with the reason in this test's own documentation.
    let declined = ["network", "install", "trust", "phones", "fleet-root"];
    // Subjects that are not about the running machine at all: a document, a
    // schema, a caller-named path, a journal. Nothing here is a candidate and
    // grouping them keeps the list above about real decisions.
    // `catalogue` joined them on 2026-08-30 and this test fired to say so, which
    // is the mechanism working a second time: it is what this BUILD can be
    // asked, so it is constant for the length of a session and re-asking it
    // once a second would spend a journal record to learn nothing.
    let not_machine_facts = ["log", "schema", "desired-state", "file", "diff", "catalogue"];

    let unaccounted: Vec<&str> = known
        .iter()
        .copied()
        .filter(|s| {
            !polled.contains(s) && !declined.contains(s) && !not_machine_facts.contains(s)
        })
        .collect();
    assert!(
        unaccounted.is_empty(),
        "`get {}` exists and this pane neither polls it nor says why not. If it is monitor \
         material, wire it in — a variant and a tier in `crate::telemetry`, a renderer in \
         `crate::tui::top`. If it is not, add it to `declined` here WITH THE REASON in this \
         test's documentation, so the next person inherits a decision rather than a gap.",
        unaccounted.join("`, `get ")
    );

    // And the declining is honest: a name here that is not a subject at all is a
    // stale exemption, which is how a list like this rots into nothing.
    for word in declined.iter().chain(not_machine_facts.iter()) {
        assert!(
            known.contains(word),
            "`{word}` is excused from this pane and is not a subject of `get` at all — the \
             exemption is stale and hides whatever replaced it"
        );
    }
}

/// The transport is a seam, and this is the assertion that it stayed one: the
/// poller is built from a `Box<dyn Source>` and names no concrete transport.
///
/// It matters because a change of channel has to be one new implementation of
/// [`Source`] rather than a rewrite of the screen — which is what `ADR-035`
/// phase 5 asked of this pane, and what it got.
#[test]
fn the_poller_names_no_transport() {
    let source = include_str!("telemetry.rs");
    let body = source.split("#[cfg(test)]").next().expect("a body");
    let poller = body
        .split("pub struct Poller")
        .nth(1)
        .expect("the poller is in this file");
    assert!(
        !poller.contains("Tls") && !poller.contains("rivet_remote"),
        "the poller has grown a transport"
    );
}

// ---------------------------------------------------------------------------
// Liveness — ADR-034 §11.5.2
// ---------------------------------------------------------------------------

/// **A cycle on the wire is in flight, and stays in flight however long it
/// takes.**
#[test]
fn a_cycle_on_the_wire_is_in_flight_and_names_no_remaining_time() {
    let mut poller = seeded(vec![]);
    let sent = Instant::now();
    poller.pretend_sent(&[Subject::Cpu, Subject::Memory], sent);

    assert_eq!(
        poller.liveness(sent),
        Liveness::InFlight { since: Duration::ZERO }
    );
    assert_eq!(
        poller.liveness(sent + Duration::from_secs(3)),
        Liveness::InFlight { since: Duration::from_secs(3) },
        "a slow request stopped being in flight without an answer"
    );
}

/// **THE TEST THIS FEATURE IS FOR.**
///
/// `ADR-034` §11.5.2: *"The indicator is derived from the poller's state, never
/// from a wall clock. A countdown that keeps cycling while the transport is dead
/// is an animation that says alive about something that is not, and it would be
/// a worse defect than having no indicator at all."*
///
/// So: a poller whose every subject has failed is advanced by ten minutes of
/// wall-clock time, and it must **not** produce a countdown at any point along
/// the way. An implementation driven by elapsed time would fill and reset a
/// clock face roughly a hundred and twenty times over that span, and every one
/// of them would be a claim that a poll was coming.
#[test]
fn a_dead_transport_never_shows_a_countdown_however_long_the_clock_runs() {
    let mut poller = seeded(vec![]);
    let start = Instant::now();
    for subject in Subject::ALL {
        poller.pretend_failed(*subject, start, 3, "cannot reach the machine");
    }

    for second in 0..600 {
        let now = start + Duration::from_secs(second);
        match poller.liveness(now) {
            Liveness::BackingOff { at } => assert!(
                at >= FAST_DEFAULT && at <= MAX_INTERVAL,
                "at {second}s the back-off was {at:?}"
            ),
            other => panic!(
                "at {second}s a machine that has answered nothing showed {other:?}, which \
                 claims a poll is coming"
            ),
        }
    }
}

/// **And the back-off it names is the one the scheduler is actually using**, so
/// the number on screen is a fact rather than a decoration.
#[test]
fn the_back_off_shown_is_the_interval_the_poller_would_wait() {
    let mut poller = seeded(vec![]);
    let start = Instant::now();
    for subject in Subject::ALL {
        poller.pretend_failed(*subject, start, 2, "cannot reach the machine");
    }
    let widest = Subject::ALL
        .iter()
        .map(|s| poller.interval(*s))
        .max()
        .expect("subjects");
    assert_eq!(poller.liveness(start), Liveness::BackingOff { at: widest });
}

/// **One subject failing is not the pane failing.** A build that cannot answer
/// `thermal` already says so on that section's own heading; colouring the whole
/// pane for it would cry wolf until nobody read it.
#[test]
fn one_failing_subject_among_healthy_ones_is_not_the_whole_pane_backing_off() {
    let mut poller = seeded(vec![]);
    let start = Instant::now();
    poller.observe(Subject::Memory, Reading::Memory(read_memory_fixture()), start);
    poller.pretend_failed(Subject::Thermal, start, 1, "cannot reach the machine");

    assert!(
        matches!(poller.liveness(start), Liveness::Waiting { .. }),
        "{:?}",
        poller.liveness(start)
    );
}

/// **Waiting counts the interval it is actually waiting**, and reaches the end
/// of it exactly when the subject comes due.
#[test]
fn waiting_counts_through_the_interval_of_the_subject_that_is_due_next() {
    let mut poller = seeded(vec![]);
    let start = Instant::now();
    // Everything has answered, at the same moment, so the countdown is the
    // shortest interval among them — the fast tier's.
    for subject in Subject::ALL {
        poller.observe(*subject, Reading::Memory(read_memory_fixture()), start);
    }

    match poller.liveness(start) {
        Liveness::Waiting { elapsed, interval } => {
            assert_eq!(elapsed, Duration::ZERO);
            assert_eq!(interval, FAST_DEFAULT);
        }
        other => panic!("{other:?}"),
    }
    match poller.liveness(start + FAST_DEFAULT / 2) {
        Liveness::Waiting { elapsed, interval } => {
            assert_eq!(elapsed, FAST_DEFAULT / 2);
            assert_eq!(interval, FAST_DEFAULT);
        }
        other => panic!("{other:?}"),
    }
}

/// **A paused poller says so in words rather than animating.** A person who
/// pressed `p` has stopped it, and a clock face would be counting down to a poll
/// that is not coming.
#[test]
fn a_paused_poller_is_stopped_rather_than_waiting() {
    let mut poller = seeded(vec![]);
    poller.paused = true;
    assert_eq!(poller.liveness(Instant::now()), Liveness::Stopped(PAUSED));
}

/// A `memory` reading, for the tests above that need any reading at all.
fn read_memory_fixture() -> Memory {
    match read(Subject::Memory, MEMORY).expect("the fixture parses") {
        Reading::Memory(memory) => memory,
        other => panic!("{other:?}"),
    }
}
