//! Tests for [`super`] — the monitor pane, as text.
//!
//! **These assert on rendered lines rather than on a terminal.** Every one of
//! them is about what a person can read, which is the property this pane has
//! that a passing type-check does not: `AGENTS.md` §5's *never false by
//! omission* is a claim about words on a screen.
//!
//! The rendering loop itself — raw mode, the alternate screen, the draw — is
//! exercised separately by driving the built binary in a pty; see the report for
//! that session. Nothing here proves the pane against a real board, because no
//! board was available: the readings below are hand-composed.

// A test that cannot fail loudly is not a test.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::time::{Duration, Instant};

use super::*;
use crate::telemetry::{
    Boot, Clock, Cpu, CpuTimes, Device, Devices, Filesystem, Load, Memory, Outcome, Pressure,
    Process, Processes, Reading, Service, Slots, Storage, Supervisor, Thermal, Throttle, Zone,
    seeded,
};

/// A four-core board with **one core doing most of the work**, in counters.
///
/// The arithmetic is arranged so the percentages read by inspection: taking
/// `cpu(0, 0)` and then `cpu(2_000_000, 400_000)` advances the aggregate by
/// **4,000,000 ms of processor time** (four cores × one million each), of which
/// 2,000,000 was busy and 400,000 was waiting — 50% and 10%. `cpu0` takes 40% of
/// the busy figure against a million of its own, so 80%, and the other three
/// take a thirtieth each.
fn cpu(busy: u64, waited: u64) -> Cpu {
    // Per core, `busy / 2` of processor time elapses between the two readings —
    // a million at the figures above.
    let one = |name: &str, share: u64| CpuTimes {
        name: name.to_string(),
        total_millis: 1_000_000 + busy / 2,
        busy_millis: share,
        iowait_millis: waited / 4,
    };
    Cpu {
        processors: 4,
        total: CpuTimes {
            name: "cpu".to_string(),
            // Four cores' worth: `busy * 2` is 4,000,000 at the figures above.
            total_millis: 4_000_000 + busy * 2,
            busy_millis: busy,
            iowait_millis: waited,
        },
        cpus: vec![
            one("cpu0", busy * 4 / 10),
            one("cpu1", busy / 30),
            one("cpu2", busy / 30),
            one("cpu3", busy / 30),
        ],
        load: Some(Load {
            avg1: 412,
            avg5: 130,
            avg15: 40,
            runnable: 2,
            existing: 97,
        }),
    }
}

/// A board whose firmware answered and whose ceiling has been lowered — **the
/// throttle observed rather than inferred.**
fn thermal() -> Thermal {
    Thermal {
        zones: vec![Zone {
            name: "cpu_thermal".to_string(),
            millidegrees: 52_341,
        }],
        zones_unreadable: Vec::new(),
        clock: Some(Clock {
            current_hz: 600_000_000,
            maximum_hz: 1_500_000_000,
            minimum_hz: Some(600_000_000),
            ceiling_hz: Some(1_000_000_000),
            ceiling_below_maximum: Some(true),
            governor: Some("ondemand".to_string()),
            at_maximum: false,
        }),
        clock_unmeasured: None,
        throttle: Some(Throttle {
            under_voltage_now: false,
            frequency_capped_now: true,
            throttled_now: true,
            soft_temperature_limit_now: false,
            under_voltage_since_boot: true,
            frequency_capped_since_boot: true,
            throttled_since_boot: true,
            soft_temperature_limit_since_boot: false,
        }),
        throttle_unmeasured: None,
    }
}

fn slots() -> Slots {
    Slots {
        running: Some("b".to_string()),
        inactive: Some("a".to_string()),
        booted: Some("b".to_string()),
        root: Some("b".to_string()),
        agree: Some(true),
        in_trial_boot: Some(false),
        trial_unmeasured: None,
    }
}

fn boot() -> Boot {
    Boot {
        boot_id: Some("c0ffee00-dead-beef".to_string()),
        uptime_seconds: 97_231,
        boots: Some(3),
        recorded: Some(true),
    }
}

fn devices() -> Devices {
    Devices {
        devices: vec![Device {
            name: "mmcblk0".to_string(),
            kind: Some("disk".to_string()),
            partition_label: None,
            size_bytes: Some(31_914_983_424),
            written_bytes: Some(536_870_912),
            life_time: None,
            pre_eol_info: None,
        }],
        disks: 1,
        unreadable: 0,
    }
}

fn memory() -> Memory {
    Memory {
        total_bytes: 971_505_664,
        available_bytes: 612_345_678,
        free_bytes: 41_216_000,
        cached_bytes: 503_316_480,
        swap_total_bytes: 0,
        swap_free_bytes: 0,
        pressure: Some(Pressure {
            some_avg10: 42,
            some_avg60: 10,
            full_avg10: 0,
            full_avg60: 0,
        }),
    }
}

/// The process table, with `used` milliseconds of CPU on `rivet-mcpd` so that
/// two of these make a rate.
///
/// **`kthreadd` carries none deliberately** — the counters come from a different
/// file in `/proc` than the rest of a row and can go missing on their own, and a
/// dash in that column must read as *not measured* rather than as *idle*.
fn processes_at(used: u64) -> Processes {
    Processes {
        rows: vec![
            Process {
                pid: 1,
                parent_pid: 0,
                name: "rivet-initd".to_string(),
                state: "S".to_string(),
                threads: 1,
                kernel: false,
                resident_bytes: Some(2_162_688),
                cpu_millis: Some(240),
            },
            Process {
                pid: 2,
                parent_pid: 0,
                name: "kthreadd".to_string(),
                state: "S".to_string(),
                threads: 1,
                kernel: true,
                resident_bytes: None,
                cpu_millis: None,
            },
            Process {
                pid: 214,
                parent_pid: 1,
                name: "rivet-mcpd".to_string(),
                state: "S".to_string(),
                threads: 2,
                kernel: false,
                resident_bytes: Some(4_194_304),
                cpu_millis: Some(used),
            },
            Process {
                pid: 402,
                parent_pid: 1,
                name: "stuck".to_string(),
                state: "D".to_string(),
                threads: 1,
                kernel: false,
                resident_bytes: Some(1_048_576),
                cpu_millis: Some(25),
            },
        ],
        vanished: 0,
        userland: 3,
    }
}

fn processes() -> Processes {
    processes_at(4_900)
}

fn supervisor() -> Supervisor {
    Supervisor {
        phase: "running".to_string(),
        services: vec![
            Service {
                name: "rivet-mcpd".to_string(),
                enabled: true,
                state: "running".to_string(),
                pid: Some(214),
                restarts: 0,
                ready: true,
            },
            // **The disagreement worth finding**: the supervisor believes this
            // is running and the kernel has no such process.
            Service {
                name: "rivet-mdnsd".to_string(),
                enabled: true,
                state: "running".to_string(),
                pid: Some(331),
                restarts: 2,
                ready: true,
            },
        ],
    }
}

fn storage() -> Storage {
    Storage {
        filesystems: vec![
            Filesystem {
                source: "/dev/mmcblk0p3".to_string(),
                mount_point: "/state".to_string(),
                fstype: "ext4".to_string(),
                read_only: false,
                total_bytes: 4_000_000_000,
                used_bytes: 1_200_000_000,
                available_bytes: 2_600_000_000,
                capacity_percent: Some(31),
                holds: "state".to_string(),
            },
            // A pseudo-filesystem: nothing is filtered out, and a bar over a
            // zero total would be a bar that means nothing.
            Filesystem {
                source: "proc".to_string(),
                mount_point: "/proc".to_string(),
                fstype: "proc".to_string(),
                read_only: false,
                total_bytes: 0,
                used_bytes: 0,
                available_bytes: 0,
                capacity_percent: None,
                holds: String::new(),
            },
        ],
        unreadable: 0,
        unreadable_detail: Vec::new(),
    }
}

/// A machine that has answered everything, `answered` ago.
///
/// **Two readings of each counter subject**, taken a second apart, so that the
/// pane under test has the rates a real one would — and so that the arithmetic
/// they are drawn from is the real arithmetic rather than a number handed to the
/// renderer. `Poller::observe` is what makes that possible; see its note.
fn healthy(answered: Duration) -> (crate::telemetry::Poller, Instant) {
    let now = Instant::now();
    let at = now - answered;
    let mut poller = seeded(vec![
        (
            crate::telemetry::Subject::Memory,
            Outcome::Answered,
            Some((Reading::Memory(memory()), at)),
        ),
        (
            crate::telemetry::Subject::Status,
            Outcome::Answered,
            Some((Reading::Supervisor(supervisor()), at)),
        ),
        (
            crate::telemetry::Subject::Storage,
            Outcome::Answered,
            Some((Reading::Storage(storage()), at)),
        ),
        (
            crate::telemetry::Subject::Slot,
            Outcome::Answered,
            Some((Reading::Slots(slots()), at)),
        ),
        (
            crate::telemetry::Subject::Boot,
            Outcome::Answered,
            Some((Reading::Boot(boot()), at)),
        ),
        (
            crate::telemetry::Subject::Device,
            Outcome::Answered,
            Some((Reading::Devices(devices()), at)),
        ),
        (
            crate::telemetry::Subject::Thermal,
            Outcome::Answered,
            Some((Reading::Thermal(thermal()), at)),
        ),
    ]);
    // 4,000,000 ms of processor time between the two readings, 2,000,000 of it
    // busy: 50% across four cores. And 250 ms of CPU on pid 214 in one second:
    // 25.00% of a processor.
    let before = at - Duration::from_secs(1);
    poller.observe(crate::telemetry::Subject::Cpu, Reading::Cpu(cpu(0, 0)), before);
    poller.observe(
        crate::telemetry::Subject::Cpu,
        Reading::Cpu(cpu(2_000_000, 400_000)),
        at,
    );
    poller.observe(
        crate::telemetry::Subject::Process,
        Reading::Processes(processes_at(4_650)),
        before,
    );
    poller.observe(
        crate::telemetry::Subject::Process,
        Reading::Processes(processes()),
        at,
    );
    (poller, now)
}

/// The same machine before a **second** reading of anything has landed — which
/// is the state a pane is in for its first two seconds, and the state in which a
/// percentage must not appear.
fn first_reading_only() -> (crate::telemetry::Poller, Instant) {
    let now = Instant::now();
    let mut poller = seeded(vec![(
        crate::telemetry::Subject::Memory,
        Outcome::Answered,
        Some((Reading::Memory(memory()), now)),
    )]);
    poller.observe(
        crate::telemetry::Subject::Cpu,
        Reading::Cpu(cpu(2_000_000, 400_000)),
        now,
    );
    poller.observe(
        crate::telemetry::Subject::Process,
        Reading::Processes(processes()),
        now,
    );
    (poller, now)
}

/// The pane, as one string.
fn rendered(view: &Monitor, poller: Option<&crate::telemetry::Poller>, now: Instant) -> String {
    draw(
        view,
        poller,
        None,
        now,
        Glyphs::ascii(),
        Palette::from_env(),
        100,
        60,
    )
    .iter()
    .map(|line| {
        line.spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
    })
    .collect::<Vec<_>>()
    .join("\n")
}

// ---------------------------------------------------------------------------
// What is on the screen
// ---------------------------------------------------------------------------

#[test]
fn a_healthy_machine_shows_every_section_it_can_answer() {
    let (poller, now) = healthy(Duration::from_millis(400));
    let text = rendered(&Monitor::new(), Some(&poller), now);
    for wanted in [
        "monitor", "CPU", "Memory", "Thermal", "Storage", "Services", "Processes",
    ] {
        assert!(text.contains(wanted), "{wanted} is missing:\n{text}");
    }
    // The numbers a person came for.
    assert!(text.contains("available"), "{text}");
    assert!(text.contains("/state"), "{text}");
    assert!(text.contains("rivet-mcpd"), "{text}");
}

/// **The rule this pane exists to keep.** Every section that has numbers under
/// it says how old they are, in the same line as its name.
#[test]
fn every_section_that_shows_a_number_says_how_old_it_is() {
    let (poller, now) = healthy(Duration::from_millis(1_400));
    let text = rendered(&Monitor::new(), Some(&poller), now);
    let with_ages = text
        .lines()
        .filter(|line| line.contains("updated") && line.contains("ago"))
        .count();
    assert!(
        with_ages >= 4,
        "only {with_ages} section(s) said how old their figures are:\n{text}"
    );
    // The fast tier's own default, read from the constant rather than typed:
    // it moved from 2 s to 5 s on 2026-08-29 (`ADR-034` §11.5.2) and a literal
    // here would have been the only thing that had to be remembered.
    assert!(
        text.contains(&format!(
            "every {}s",
            crate::telemetry::FAST_DEFAULT.as_secs()
        )),
        "the interval is not shown:\n{text}"
    );
}

/// **The dangerous case.** A refresh that failed must not leave old numbers
/// looking current — the heading says so, in the error style, with the age of
/// what is still on screen.
#[test]
fn a_failed_refresh_says_so_and_dates_the_figures_that_are_still_there() {
    let now = Instant::now();
    let at = now - Duration::from_secs(47);
    let poller = seeded(vec![(
        crate::telemetry::Subject::Memory,
        Outcome::Failed(
            "`rivet \"get\" \"memory\"` failed on root@10.0.0.2 (exit status: 255): \
             Connection timed out"
                .to_string(),
        ),
        Some((Reading::Memory(memory()), at)),
    )]);
    let text = rendered(&Monitor::new(), Some(&poller), now);

    assert!(text.contains("LAST REFRESH FAILED"), "{text}");
    assert!(text.contains("47s old"), "the age of the figures is missing:\n{text}");
    assert!(text.contains("Connection timed out"), "the reason is missing:\n{text}");
    // The figures are still there — hiding them would lose the last thing known
    // about a machine that has just stopped answering.
    assert!(text.contains("available"), "{text}");
}

/// A reading that is simply old, with no failure, is still called out. Two and a
/// half intervals; see `crate::telemetry::Slot::is_stale`.
#[test]
fn a_reading_that_is_merely_old_is_marked_stale() {
    let (poller, now) = healthy(Duration::from_secs(30));
    let text = rendered(&Monitor::new(), Some(&poller), now);
    assert!(text.contains("STALE"), "{text}");
}

/// **Not asked yet is not unreadable**, which is `crate::tui::reach`'s rule one
/// layer in: a screen that says *unreadable* before it has looked is stating
/// something it does not know.
#[test]
fn a_pane_that_has_asked_nothing_claims_nothing() {
    let poller = seeded(Vec::new());
    let text = rendered(&Monitor::new(), Some(&poller), Instant::now());
    assert!(text.contains("not asked yet"), "{text}");
    assert!(!text.contains("FAILED"), "{text}");
    // And with no reader at all, it says that instead of showing a machine.
    let none = rendered(&Monitor::new(), None, Instant::now());
    assert!(none.contains("Opening a reader"), "{none}");
}

// ---------------------------------------------------------------------------
// What is NOT on the screen, and is said rather than approximated
// ---------------------------------------------------------------------------

/// The pane, cut to one section.
fn section(text: &str, from: &str, to: &str) -> String {
    text.lines()
        .skip_while(|l| !l.contains(from))
        .take_while(|l| !l.contains(to))
        .collect::<Vec<_>>()
        .join(" ")
}

/// **The maintainer's headline ask, on the screen.** CPU is a rate, the machine
/// answers counters, and the pane holds the second reading — so with two
/// readings there is a figure, and it is per-core as well as aggregate.
#[test]
fn cpu_is_shown_as_a_utilisation_once_two_readings_exist() {
    let (poller, now) = healthy(Duration::from_millis(200));
    let text = rendered(&Monitor::new(), Some(&poller), now);
    let cpu = section(&text, "CPU", "Memory");

    assert!(cpu.contains("50%"), "the busy figure is missing: {cpu}");
    assert!(cpu.contains("busy"), "{cpu}");
    // **The interval is beside the figure.** A rate over an unstated interval is
    // not a measurement.
    assert!(cpu.contains("over 1"), "the interval is missing: {cpu}");
    // Per core, because one pinned while three idle is what an average hides.
    for core in ["cpu0", "cpu1", "cpu2", "cpu3"] {
        assert!(cpu.contains(core), "{core} is missing: {cpu}");
    }
    // **iowait apart from busy.** A board at 90% iowait is waiting on a failing
    // card, not working.
    assert!(cpu.contains("iowait"), "{cpu}");
    assert!(cpu.contains("NOT work"), "iowait is not distinguished: {cpu}");
    // The load average, labelled as what it is not.
    assert!(cpu.contains("4.12"), "the load average is missing: {cpu}");
    assert!(cpu.contains("NOT NECESSARILY BUSY"), "{cpu}");
    assert!(cpu.contains("uninterruptible sleep"), "{cpu}");
}

/// **THE assertion about CPU on this pane.** A rate needs two readings, and
/// before the second one lands the honest answer is a sentence — never `0%`,
/// which is a measurement of an idle machine and the one number a monitor must
/// never guess.
#[test]
fn a_first_cpu_reading_says_it_has_no_percentage_rather_than_drawing_zero() {
    let (poller, now) = first_reading_only();
    let text = rendered(&Monitor::new(), Some(&poller), now);
    let cpu = section(&text, "CPU", "Memory");

    assert!(cpu.contains("no utilisation yet"), "{cpu}");
    assert!(cpu.contains("RATE"), "it does not say why: {cpu}");
    assert!(!cpu.contains("0%"), "it drew a zero for a rate nobody has: {cpu}");
    assert!(
        !cpu.contains("busy"),
        "a busy figure appeared from one reading: {cpu}"
    );

    // And the process table's CPU column is dashes with a sentence above it
    // saying they are not measured — not idle.
    assert!(text.contains("CPU% needs two readings"), "{text}");
    let row = text
        .lines()
        .find(|l| l.contains("rivet-mcpd") && l.trim_start().starts_with("214"))
        .expect("the process row");
    assert!(row.contains('—'), "{row}");
    assert!(!row.contains("0.00"), "a rate was drawn as zero: {row}");
}

/// Per-process CPU, once two readings exist — a share of **one** processor,
/// which is `top`'s convention and is stated on screen because it surprises
/// people.
#[test]
fn a_process_row_shows_its_share_of_one_processor_and_says_so() {
    let (poller, now) = healthy(Duration::from_millis(200));
    let text = rendered(&Monitor::new(), Some(&poller), now);
    let row = text
        .lines()
        .find(|l| l.contains("rivet-mcpd") && l.trim_start().starts_with("214"))
        .expect("the process row");
    assert!(row.contains("25.00"), "250 ms in 1 s is 25.00%: {row}");
    assert!(
        text.contains("share of ONE processor"),
        "the convention is not stated:\n{text}"
    );

    // **A process whose counter the machine could not read shows a dash**, and
    // the pane says a dash is not measured rather than idle.
    let kernel = rendered(
        &Monitor {
            kernel_threads: true,
            ..Monitor::new()
        },
        Some(&poller),
        now,
    );
    let thread = kernel
        .lines()
        .find(|l| l.contains("kthreadd"))
        .expect("the kernel thread");
    assert!(thread.contains('—'), "{thread}");
    assert!(!thread.contains("0.00"), "an unread counter was drawn as zero: {thread}");
}

/// **Thermals lead with the throttle, not the temperature** — the survey's §7.1
/// order — and the throttle they draw is the ceiling against the maximum rather
/// than the current clock against it.
#[test]
fn thermal_leads_with_the_throttle_and_shows_the_clock_against_its_ceiling() {
    let (poller, now) = healthy(Duration::from_millis(200));
    let text = rendered(&Monitor::new(), Some(&poller), now);
    let thermal = section(&text, "Thermal", "Storage");

    // The sticky flags, first, because they are the only fact of the four that
    // cannot be recovered any other way.
    assert!(thermal.contains("SINCE BOOT"), "{thermal}");
    assert!(thermal.contains("UNDER-VOLTED"), "{thermal}");
    // The clock, current against maximum.
    assert!(thermal.contains("600 MHz"), "{thermal}");
    assert!(thermal.contains("1500 MHz"), "{thermal}");
    // **The comparison that IS the throttle.**
    assert!(thermal.contains("CEILING LOWERED"), "{thermal}");
    assert!(thermal.contains("1000 MHz"), "{thermal}");
    // And the temperature last, labelled as the spot reading it is.
    assert!(thermal.contains("52.3°C"), "{thermal}");
    assert!(thermal.contains("SPOT"), "{thermal}");

    // The order on screen is the survey's: flags, then clock, then temperature.
    let flags = text.find("SINCE BOOT").expect("the flags");
    let clock = text.find("600 MHz").expect("the clock");
    let temperature = text.find("52.3°C").expect("the temperature");
    assert!(
        flags < clock && clock < temperature,
        "the weakest fact is not last:\n{text}"
    );
}

/// **The degradation that must survive.** `Admin-Surface-Survey.md` §7.6
/// measured that the throttle flags need `/dev/vcio`; a board where that is
/// absent must render **not measured**, never *never throttled* — which is the
/// one claim this pane could make that would be both wrong and reassuring.
#[test]
fn a_board_whose_throttle_flags_were_not_read_says_so_rather_than_saying_never() {
    let now = Instant::now();
    let mut degraded = thermal();
    degraded.throttle = None;
    degraded.throttle_unmeasured =
        Some("/dev/vcio could not be opened: No such device".to_string());
    degraded.clock = None;
    degraded.clock_unmeasured = Some("no cpufreq policy under /sys".to_string());
    let poller = seeded(vec![(
        crate::telemetry::Subject::Thermal,
        Outcome::Answered,
        Some((Reading::Thermal(degraded), now)),
    )]);
    let text = rendered(&Monitor::new(), Some(&poller), now);

    assert!(text.contains("NOT MEASURED"), "{text}");
    assert!(text.contains("/dev/vcio"), "the reason is missing:\n{text}");
    assert!(
        text.contains("03:00"),
        "it does not say what the absence costs:\n{text}"
    );
    // The words that would be false.
    for lie in ["never throttled", "no throttling", "never under-volted"] {
        assert!(
            !text.contains(lie),
            "the pane claimed {lie:?} from flags nobody read:\n{text}"
        );
    }
    // And the half that WAS read still shows.
    assert!(text.contains("52.3°C"), "{text}");
}

/// A subject this build has no `get` for still renders as **not measured**,
/// never as a zero — the path that carried `thermal` before it landed, and that
/// the next subject will need.
#[test]
fn a_subject_this_build_cannot_answer_renders_as_unmeasured() {
    let poller = seeded(vec![(
        crate::telemetry::Subject::Thermal,
        Outcome::Unsupported(r#"{"error":"E_UNKNOWN_SUBJECT"}"#.to_string()),
        None,
    )]);
    let text = rendered(&Monitor::new(), Some(&poller), Instant::now());
    assert!(text.contains("NOT MEASURED"), "{text}");
    assert!(text.contains("get thermal"), "{text}");
    assert!(!text.contains('°'), "{text}");
}

/// **The `once` tier on screen**: which slot booted, how long the machine has
/// been up, and what it has under its filesystems.
#[test]
fn the_machine_section_shows_the_slot_the_uptime_and_the_devices() {
    let (poller, now) = healthy(Duration::from_millis(200));
    let text = rendered(&Monitor::new(), Some(&poller), now);
    let machine = section(&text, "Machine", "PID");

    assert!(machine.contains("running b"), "{machine}");
    assert!(machine.contains("update writes a"), "{machine}");
    // **The uptime is made current by arithmetic rather than by polling** — the
    // reading's age is added to it, which is what lets a once-only reading tick.
    assert!(machine.contains("1d3h"), "{machine}");
    assert!(machine.contains("monotonic"), "{machine}");
    assert!(machine.contains("3 boot(s)"), "{machine}");
    assert!(machine.contains("1 disk(s)"), "{machine}");
    assert!(machine.contains("written since boot"), "{machine}");
}

/// **The finding `get slot` exists for**, on screen: the firmware and the kernel
/// named different slots, so no running slot is shown and nothing should be
/// armed.
#[test]
fn two_authorities_disagreeing_about_the_slot_is_shouted_rather_than_resolved() {
    let now = Instant::now();
    let poller = seeded(vec![(
        crate::telemetry::Subject::Slot,
        Outcome::Answered,
        Some((
            Reading::Slots(Slots {
                running: None,
                inactive: None,
                booted: Some("a".to_string()),
                root: Some("b".to_string()),
                agree: Some(false),
                in_trial_boot: None,
                trial_unmeasured: Some("no device tree".to_string()),
            }),
            now,
        )),
    )]);
    let text = rendered(&Monitor::new(), Some(&poller), now);
    assert!(text.contains("THE TWO AUTHORITIES DISAGREE"), "{text}");
    assert!(text.contains("NOTHING SHOULD BE ARMED"), "{text}");
    assert!(!text.contains("running a"), "a slot was named: {text}");
    assert!(!text.contains("running b"), "a slot was named: {text}");
    // **An unread trial flag is not `false`**, which would assert that doing
    // nothing is safe.
    assert!(text.contains("trial"), "{text}");
    assert!(text.contains("NOT MEASURED"), "{text}");
}

/// **No `CONFIG_PSI` is not zero pressure.** Four zeroes would assert that the
/// machine is not stalling, which this build would not have measured.
#[test]
fn a_kernel_with_no_pressure_information_says_so_rather_than_showing_zeroes() {
    let now = Instant::now();
    let mut without = memory();
    without.pressure = None;
    let poller = seeded(vec![(
        crate::telemetry::Subject::Memory,
        Outcome::Answered,
        Some((Reading::Memory(without), now)),
    )]);
    let text = rendered(&Monitor::new(), Some(&poller), now);
    assert!(text.contains("CONFIG_PSI"), "{text}");
    assert!(text.contains("NOT MEASURED"), "{text}");
    assert!(!text.contains("0.00%"), "it drew zeroes for a thing nobody measured:\n{text}");
}

/// A process with no resident size shows a dash. `0` would claim it occupies
/// none of the machine, which is a different statement and a false one.
#[test]
fn a_process_with_no_resident_size_shows_a_dash_and_never_a_zero() {
    let (poller, now) = healthy(Duration::from_millis(200));
    let view = Monitor {
        kernel_threads: true,
        ..Monitor::new()
    };
    let text = rendered(&view, Some(&poller), now);
    let row = text
        .lines()
        .find(|l| l.contains("kthreadd"))
        .expect("the kernel thread is shown");
    assert!(row.contains('—'), "{row}");
    assert!(!row.contains(" 0 B"), "{row}");
}

/// A filesystem with no total says so rather than reading as empty — a
/// proportion of nothing is not zero percent.
#[test]
fn a_filesystem_with_no_size_is_not_drawn_as_an_empty_one() {
    let (poller, now) = healthy(Duration::from_millis(200));
    let text = rendered(&Monitor::new(), Some(&poller), now);
    // The mount point, not the `/proc/stat` the CPU section names.
    let row = text
        .lines()
        .find(|l| l.trim_start().starts_with("/proc "))
        .expect("the row");
    assert!(row.contains("no size to report"), "{row}");
    assert!(!row.contains("0%"), "{row}");
}

// ---------------------------------------------------------------------------
// The filter, and the disagreement
// ---------------------------------------------------------------------------

/// **Kernel threads are hidden by default and the hiding is visible.** What was
/// asked for, and the count plus the key in the footer are what stop it being a
/// hidden filter.
#[test]
fn kernel_threads_are_hidden_by_default_and_the_screen_says_how_many() {
    let (poller, now) = healthy(Duration::from_millis(200));
    let hidden = rendered(&Monitor::new(), Some(&poller), now);
    assert!(!hidden.contains("kthreadd"), "a kernel thread is in the table:\n{hidden}");
    assert!(hidden.contains("kernel thread(s) hidden"), "{hidden}");

    let shown = rendered(
        &Monitor {
            kernel_threads: true,
            ..Monitor::new()
        },
        Some(&poller),
        now,
    );
    assert!(shown.contains("kthreadd"), "{shown}");
    assert!(shown.contains("kernel thread(s) shown"), "{shown}");
}

/// **The case the two subjects exist to find.** The supervisor believes
/// `rivet-mdnsd` is running on pid 331 and the kernel has no such process.
#[test]
fn a_service_the_supervisor_claims_with_no_process_behind_it_is_called_out() {
    let (poller, now) = healthy(Duration::from_millis(200));
    let text = rendered(&Monitor::new(), Some(&poller), now);
    let row = text
        .lines()
        .find(|l| l.contains("rivet-mdnsd"))
        .expect("the service is listed");
    assert!(row.contains("NO SUCH PROCESS"), "{row}");
    // And the one that does agree is not shouted about.
    let ok = text
        .lines()
        .find(|l| l.contains("rivet-mcpd") && l.contains("running"))
        .expect("the healthy service is listed");
    assert!(!ok.contains("NO SUCH"), "{ok}");
}

/// The cross-check is between readings taken at different moments, and it says
/// so. A comparison across time that presents itself as simultaneous is the same
/// class of lie as a stale number shown as live.
#[test]
fn the_cross_check_admits_the_two_halves_are_not_simultaneous() {
    let now = Instant::now();
    let poller = seeded(vec![
        (
            crate::telemetry::Subject::Process,
            Outcome::Answered,
            Some((Reading::Processes(processes()), now - Duration::from_secs(1))),
        ),
        (
            crate::telemetry::Subject::Status,
            Outcome::Answered,
            Some((
                Reading::Supervisor(supervisor()),
                now - Duration::from_secs(5),
            )),
        ),
    ]);
    let text = rendered(&Monitor::new(), Some(&poller), now);
    assert!(text.contains("apart"), "{text}");
}

/// `D` is not merely a letter to a person who has never read `ps`. The row says
/// what it means, and what it usually means on this hardware.
#[test]
fn a_process_stuck_in_the_kernel_is_explained_rather_than_lettered() {
    let (poller, now) = healthy(Duration::from_millis(200));
    let text = rendered(&Monitor::new(), Some(&poller), now);
    // The table row, not the sentence about unclaimed userland above it.
    let row = text
        .lines()
        .find(|l| l.contains("stuck") && l.contains(" D "))
        .expect("the row");
    assert!(row.contains("uninterruptible"), "{row}");
    assert!(row.contains("failing card"), "{row}");
}

/// Ordering changes what is at the top and loses nothing.
#[test]
fn the_process_order_can_be_changed_and_no_row_disappears() {
    let (poller, now) = healthy(Duration::from_millis(200));
    for order in [Order::Resident, Order::Pid, Order::Name] {
        let text = rendered(
            &Monitor {
                order,
                ..Monitor::new()
            },
            Some(&poller),
            now,
        );
        for name in ["rivet-initd", "rivet-mcpd", "stuck"] {
            assert!(text.contains(name), "{order:?} lost {name}:\n{text}");
        }
    }
    // Resident order puts the largest first.
    let by_size = rendered(&Monitor::new(), Some(&poller), now);
    let mcpd = by_size.find("rivet-mcpd").expect("mcpd");
    let initd = by_size.rfind("rivet-initd").expect("initd");
    assert!(mcpd < initd, "4 MiB should sort above 2 MiB:\n{by_size}");
}

/// **The defect a terminal found and a type-check could not.**
///
/// The first version gave the process table whatever height was left over. At
/// 44 rows the sections above it filled the screen and *no process was on it* —
/// the pane the maintainer asked for is the process list, and it was the only
/// thing missing. The whole body scrolls now, so everything is reachable at any
/// height and the pane says how far down it is.
#[test]
fn a_short_pane_scrolls_rather_than_losing_what_it_cannot_fit() {
    let (poller, now) = healthy(Duration::from_millis(200));
    let text = |scroll: usize| -> String {
        draw(
            &Monitor {
                scroll,
                ..Monitor::new()
            },
            Some(&poller),
            None,
            now,
            Glyphs::ascii(),
            Palette::from_env(),
            100,
            // Shorter than everything the pane would like to draw.
            18,
        )
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
    };

    let top = text(0);
    // The banner is pinned, because it is the line that says whether any of
    // this is live.
    assert!(top.contains("monitor"), "{top}");
    assert!(top.contains("Memory"), "{top}");
    assert!(top.contains("lines 1–"), "it did not say there was more:\n{top}");

    // Far enough down to reach the table, which is what a person came for.
    let bottom = text(200);
    assert!(bottom.contains("monitor"), "the banner was scrolled away:\n{bottom}");
    assert!(bottom.contains("rivet-mcpd"), "{bottom}");
    assert!(!bottom.contains("NOT MEASURED"), "the top is still on screen:\n{bottom}");
    // Scrolling past the end lands ON the end rather than off it.
    assert_eq!(bottom, text(SCROLL_LIMIT));
}

/// Nothing this pane draws runs past the frame — a line that overflows corrupts
/// the box around it and not just itself.
#[test]
fn no_line_is_wider_than_the_pane() {
    let (poller, now) = healthy(Duration::from_millis(200));
    for width in [40_u16, 60, 80, 100, 200] {
        let lines = draw(
            &Monitor {
                kernel_threads: true,
                ..Monitor::new()
            },
            Some(&poller),
            None,
            now,
            Glyphs::ascii(),
            Palette::from_env(),
            width,
            60,
        );
        for line in &lines {
            let text: String = line
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>();
            assert!(
                text.chars().count() <= usize::from(width),
                "at width {width}, {} chars: {text:?}",
                text.chars().count()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The small functions everything above is built from
// ---------------------------------------------------------------------------

/// **Binary units, named as binary units.** A board with 971,505,664 bytes has
/// 926 MiB, and calling that 926 MB is how two readings of one machine disagree.
#[test]
fn bytes_are_rendered_in_the_units_they_are_measured_in() {
    assert_eq!(human(0), "0 B");
    assert_eq!(human(1023), "1023 B");
    assert_eq!(human(1024), "1.0 KiB");
    assert_eq!(human(971_505_664), "926 MiB");
    assert_eq!(human(4_000_000_000), "3.7 GiB");
    // No value is ever rendered as an empty string, which would read as absent.
    for n in [1u64, 999, 1_048_576, u64::MAX] {
        assert!(!human(n).is_empty());
    }
}

/// The unit is hundredths of a percent and the two decimals are the
/// measurement — a whole percent would collapse *stalling 0.4% of the time* into
/// *not stalling*.
#[test]
fn pressure_keeps_the_precision_it_was_measured_at() {
    assert_eq!(hundredths(0), "0.00%");
    assert_eq!(hundredths(42), "0.42%");
    assert_eq!(hundredths(100), "1.00%");
    assert_eq!(hundredths(10_000), "100.00%");
}

/// **A proportion of nothing is not zero percent.**
#[test]
fn a_percentage_of_nothing_is_no_percentage_at_all() {
    assert_eq!(percent(0, 0), None);
    assert_eq!(percent(5, 0), None);
    assert_eq!(percent(1, 2), Some(50));
    // And it never exceeds 100, which a rounding artefact could otherwise do.
    assert_eq!(percent(300, 100), Some(100));
}

#[test]
fn an_age_is_said_as_briefly_as_it_can_be_without_becoming_vague() {
    assert_eq!(ago(Duration::from_millis(1_400)), "1.4s");
    assert_eq!(ago(Duration::from_secs(47)), "47s");
    assert_eq!(ago(Duration::from_secs(150)), "2m30s");
    assert_eq!(ago(Duration::from_secs(7_260)), "2h1m");
}

// ---------------------------------------------------------------------------
// The liveness indicator — ADR-034 §11.5.2
// ---------------------------------------------------------------------------

use crate::telemetry::{Liveness, NOTHING_LEFT, NO_TRANSPORT, PAUSED};

/// One indicator, as the text a person reads plus the glyph in front of it.
fn shown(state: Liveness, glyphs: Glyphs) -> String {
    indicator(state, glyphs, Palette::from_env())
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}

/// Both sets, so that no assertion below can be about only one of them.
fn both_sets() -> [Glyphs; 2] {
    [Glyphs::nerd(), Glyphs::ascii()]
}

/// **The four states are four different things on screen.**
///
/// `ADR-034` §11.5.2 requires them *visually distinguishable*, and this is that
/// requirement as an assertion rather than as an intention — in **both** glyph
/// modes, because an indicator that only works with a nerd font is a liveness
/// signal that vanishes exactly where a terminal is least capable.
#[test]
fn the_four_states_are_distinguishable_in_both_glyph_modes() {
    for glyphs in both_sets() {
        let four = [
            shown(Liveness::Stopped(NO_TRANSPORT), glyphs),
            shown(
                Liveness::Waiting {
                    elapsed: Duration::from_secs(1),
                    interval: Duration::from_secs(5),
                },
                glyphs,
            ),
            shown(Liveness::InFlight { since: Duration::from_millis(300) }, glyphs),
            shown(Liveness::BackingOff { at: Duration::from_secs(20) }, glyphs),
        ];
        for (at, one) in four.iter().enumerate() {
            for (also, other) in four.iter().enumerate() {
                assert!(
                    at == also || one != other,
                    "two states render identically: {one:?}"
                );
            }
            // And every one of them has a first character that is a glyph
            // followed by words — `crate::tui::theme`'s standing rule.
            assert!(
                one.chars().count() > 3,
                "a state is drawn as a glyph and nothing else: {one:?}"
            );
        }
    }
}

/// **Waiting is a clock face filling**, and it fills.
#[test]
fn the_countdown_fills_as_the_interval_runs_out() {
    for glyphs in both_sets() {
        let interval = Duration::from_secs(5);
        let faces: Vec<char> = [0_u64, 1, 2, 3, 5]
            .iter()
            .map(|at| {
                clock_face(Duration::from_secs(*at), interval, glyphs)
            })
            .collect();
        assert_eq!(faces.first(), glyphs.waiting.first(), "it did not start empty");
        assert_eq!(faces.last(), glyphs.waiting.last(), "it did not end full");
        // Monotonic through the set, so it reads as one thing filling.
        let mut last = 0;
        for face in &faces {
            let at = glyphs
                .waiting
                .iter()
                .position(|f| f == face)
                .expect("a face from this set");
            assert!(at >= last, "the clock went backwards: {faces:?}");
            last = at;
        }
    }
}

/// **In flight is never a countdown.** The remaining time is unknown, and the
/// honest statement is that this end is waiting on the board — so no number of
/// seconds appears at all.
#[test]
fn a_request_on_the_wire_says_it_is_waiting_and_names_no_remaining_time() {
    for glyphs in both_sets() {
        let text = shown(Liveness::InFlight { since: Duration::from_millis(700) }, glyphs);
        assert!(text.contains("waiting on the board"), "{text}");
        assert!(!text.contains("next read"), "it counted down: {text}");
        // No digits at all: a number here could only be a remaining time, and
        // there is none to state.
        assert!(
            !text.chars().any(|c| c.is_ascii_digit()),
            "a number appeared beside an unknown remaining time: {text}"
        );
    }
}

/// **The spinner turns with the request and not with the clock.**
///
/// Its frame is a function of how long *this cycle* has been out, so the same
/// `since` always draws the same frame — and a `since` that stops advancing,
/// because nothing is on the wire, cannot advance it.
#[test]
fn the_spinner_frame_is_a_function_of_how_long_the_request_has_been_out() {
    for glyphs in both_sets() {
        let at = |ms: u64| shown(Liveness::InFlight { since: Duration::from_millis(ms) }, glyphs);
        assert_eq!(at(0), at(0), "the same state drew two different frames");
        // Far enough apart to be different frames of a four-frame spinner.
        assert_ne!(at(0), at(130));
        // And it comes back round rather than running off the end.
        assert_eq!(at(0), at(130 * 4));
    }
}

/// **The back-off names the interval it is now at**, which is the number a
/// person waiting for a board to come back actually wants.
#[test]
fn backing_off_names_the_interval_rather_than_only_saying_it_failed() {
    for glyphs in both_sets() {
        let text = shown(Liveness::BackingOff { at: Duration::from_secs(40) }, glyphs);
        assert!(text.contains("40s"), "{text}");
        assert!(text.contains("NOT ANSWERING"), "{text}");
    }
}

/// **No transport is said in words and is not animated.**
#[test]
fn a_pane_with_no_reader_says_so_in_words() {
    for glyphs in both_sets() {
        for why in [NO_TRANSPORT, PAUSED, NOTHING_LEFT] {
            let text = shown(Liveness::Stopped(why), glyphs);
            assert!(text.contains(why), "{text}");
            // Nothing that reads as a countdown or a spinner. **The glyph
            // POSITION is what is checked**, not the whole string: an ASCII
            // clock face is an ordinary letter and appears in English words.
            assert!(!text.contains("next read in"), "{text}");
            let first = text.chars().next().expect("a glyph");
            assert_eq!(first, glyphs.stopped, "{text}");
            assert!(!glyphs.spinning.contains(&first), "{text}");
            assert!(!glyphs.waiting.contains(&first), "{text}");
        }
    }
}

/// **The pane says why it has no reader, in the words that name the fix.**
///
/// A blank monitor and a monitor that was never given an identity look the same
/// to a person, and one of them is a flag they can pass.
#[test]
fn a_refusal_to_open_a_reader_is_shown_rather_than_swallowed() {
    let refusal = crate::operator::Operator::default().refusal();
    let lines = draw(
        &Monitor::new(),
        None,
        Some(&refusal),
        Instant::now(),
        Glyphs::ascii(),
        Palette::from_env(),
        100,
        40,
    );
    let text: String = lines
        .iter()
        .flat_map(|l| l.spans.iter())
        .map(|s| s.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(text.contains(NO_TRANSPORT), "{text}");
    assert!(text.contains("--identity"), "{text}");
}
