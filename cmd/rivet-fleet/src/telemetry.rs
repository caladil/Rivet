//! **What a running machine is doing right now** — the facts behind the monitor
//! pane, and the poller that fetches them without wrecking the machine's own
//! record of itself.
//!
//! The maintainer asked for it on 2026-08-29. Rivet is managed over MCP and TLS
//! alone, so the human needs a terminal interface that lets them manage it, and
//! the screen he asked for first is a per-device `top`-like view of the
//! machine's current status — cpu, memory, disk, thermals and running
//! processes, with the kernel noise filtered out by default (.rulings/ R-328).
//!
//! `ADR-035` is why this is not a nicety. The pivot leaves an API control plane
//! and nothing else, and at phase 5 there is no shell to run `top` in — so this
//! screen is not a convenience over a shell, it is what a human has **instead**
//! of one. `docs/design-candidates/Admin-Surface-Survey.md` is the survey behind
//! the facts chosen.
//!
//! # The constraint that shapes everything here: a poll costs a record
//!
//! `rivet-mcpd` writes at least one line **per connection** — read in
//! `cmd/rivet-mcpd/src/main.rs`, which logs `{peer} closed after N answer(s)` at
//! close and inherits stderr on purpose because *"its own stderr is the
//! supervisor's capture (ADR-022 §4)"*. So on the control plane, one connection
//! is one journal line — **and a connection is one poll CYCLE rather than one
//! subject**, which is the distinction `ADR-034` §11.5.2 rules on and the reason
//! this file is shaped the way it is.
//!
//! `ADR-022` §8 caps that journal at **8 MiB total** — `LOG_FILE_MAX_BYTES`
//! (1 MiB) times `LOG_FILES_KEPT` (8) in `rivet_core::state`, checked there
//! rather than taken from the ADR's prose. **One poll per second is ~86,400
//! records a day**, so a monitor left open overnight evicts everything else the
//! machine had to say. The screen would destroy the evidence somebody opened it
//! to find.
//!
//! **The tiering below was designed for the expensive channel before this pane
//! was on it, and did not have to change when it arrived.** `ADR-034` §11.5.1
//! refused every shell-out and §11.5.2 moved this pane onto the control plane,
//! so the channel a poll pays for is the one the tiering already assumed.
//!
//! **A cycle also spawns a `rivet mcp` process on the board.** `PLAN.md` records
//! that `rivet-mcpd` spawns one per CONNECTION and not per request, found by
//! reading the listener on 2026-08-29 — so the fast tier's three subjects cost
//! one process a tick and not three, and the fast default is five seconds rather
//! than two because a process on a 926 MiB board is what a tick now costs.
//!
//! So the reads are **tiered**, and the tiers are chosen from how fast each fact
//! can actually change rather than from how fast a screen could redraw:
//!
//! | Tier | Subjects | Every | Why |
//! | :-- | :-- | :-- | :-- |
//! | fast | `memory`, `process`, `cpu` | 5 s (adjustable) | the `top` feel at what a poll costs — `htop` is 1.5 s and `top` defaults to 3 s, and neither spawns a process on the machine it watches |
//! | medium | `thermal`, `status` | 5 s | thermal mass is slow, and the supervisor's own belief changes only when something starts or stops |
//! | slow | `storage` | 30 s | `statvfs` over every mount; it changes on the timescale of an install |
//! | once | `slot`, `boot`, `device` | never again | they do not change while somebody watches — and re-asking would spend a journal record to learn nothing |
//!
//! ## ~~There is no `once` tier in this build~~ — there is, since 2026-08-29
//!
//! **Struck rather than edited** (`AGENTS.md` §2, §7), because the sentence was
//! a true statement about a build that no longer exists and the reason it went
//! stale is the interesting part. It read: *"the facts that would populate one —
//! `slot`, `boot`, `device` — are not subjects `rivet get` answers"*, and
//! the test `the_subjects_the_monitor_does_not_poll` was written to **fail** the
//! day one of them landed. All four landed the same day.
//!
//! What the tier is for: `slot` answers which A/B partition this machine booted
//! and `device` what block devices exist. **Neither can change without a
//! reboot**, and a reboot ends the session that is watching. `boot` is the
//! exception that proves the shape — `uptime_seconds` *does* change, by exactly
//! one second per second, which is a thing a screen can compute rather than ask
//! for. So the reading is taken once and the age beside it is what makes the
//! uptime current.
//!
//! # CPU is a rate, and this is the layer that owns the second reading
//!
//! `rivet get cpu` answers **counters** and refuses to answer a percentage,
//! because a percentage is a difference between two readings over the time
//! between them — see `rivet_core::operation::cpu`. That is the correct place to
//! draw the line, and it makes *this* the layer that holds the other reading:
//! the poller is the only thing here that asks twice.
//!
//! So [`Poller`] keeps the previous `cpu` and `process` answers and computes
//! [`CpuRates`] and [`ProcessRates`] when the next one lands. **Until the second
//! reading arrives there is no rate and none is offered** — `None`, which the
//! pane renders as a sentence rather than as `0%`. Zero percent is a measurement
//! of an idle machine and it is the single most misleading thing this screen
//! could put on itself.
//!
//! ## Five properties, which matter more than the numbers
//!
//! 1. **Cycles never overlap.** [`Poller::tick`] sends nothing while one is in
//!    flight — it **skips**, and counts the skip so the screen can say so. A
//!    queue would turn a slow link into a backlog that hammers the board.
//! 2. **Polling stops when nobody is looking.** The caller passes `active`, and
//!    the pane is active only when it is the one on screen. Biggest saving
//!    available and it costs nothing.
//! 3. **One worker, one transport — and one connection per CYCLE.** The
//!    [`Source`] is built once, on the worker thread, and lives as long as the
//!    screen does; a tick hands it every subject that is due, and [`Tls`] puts
//!    them on one connection — the shape a kept connection drops into, and it
//!    is true of the build now (`ADR-034` §11.5.2).
//! 4. **Failure widens the interval.** A board that has stopped answering is
//!    retried at 10 s, then 20, then 40, up to [`MAX_INTERVAL`]. Retrying a dead
//!    machine at the fast interval for ever is how a monitoring tool becomes the
//!    outage.
//! 5. **The screen says which of those it is doing.** [`Liveness`] — derived
//!    from this poller and never from a clock, because an indicator that keeps
//!    animating while the transport is dead asserts *alive* about something that
//!    is not, and is worse than no indicator at all.
//!
//! # Absence is not zero, anywhere in this file
//!
//! `AGENTS.md` §5, and it is the reason every optional number here is an
//! `Option` rather than a defaulted field:
//!
//! * a kernel with no `CONFIG_PSI` has not measured *no* memory pressure, it has
//!   measured **nothing** — `rivet_core::operation::memory` says so in its own
//!   words and this preserves the distinction;
//! * a kernel thread has no resident size, and reporting `0` would claim it
//!   occupies none;
//! * a subject this machine's build does not answer is [`Outcome::Unsupported`],
//!   which is neither a value nor a failure to retry.
//!
//! And a reading that could not be refreshed **keeps its value and loses its
//! currency**: [`Slot::age`] is what the screen renders beside every number, so
//! a stale figure is never shown as a live one. That is the one genuinely
//! dangerous failure of a monitoring interface and it is designed against rather
//! than hoped away.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::json::Json;

/// One machine fact this screen reads.
///
/// **The wire word is the contract** — `rivet_core::operation::<module>::SUBJECT`
/// — and it is spelled here once so no call site holds a literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Subject {
    Cpu,
    Memory,
    Process,
    Thermal,
    Status,
    Storage,
    Slot,
    Boot,
    Device,
}

impl Subject {
    /// Every subject the monitor polls, in the order it lays them out.
    pub const ALL: &'static [Subject] = &[
        Subject::Cpu,
        Subject::Memory,
        Subject::Process,
        Subject::Thermal,
        Subject::Status,
        Subject::Storage,
        Subject::Slot,
        Subject::Boot,
        Subject::Device,
    ];

    /// The word `rivet get` takes.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Process => "process",
            Self::Thermal => "thermal",
            Self::Status => "status",
            Self::Storage => "storage",
            Self::Slot => "slot",
            Self::Boot => "boot",
            Self::Device => "device",
        }
    }

    #[must_use]
    pub const fn tier(self) -> Tier {
        match self {
            Self::Cpu | Self::Memory | Self::Process => Tier::Fast,
            Self::Thermal | Self::Status => Tier::Medium,
            Self::Storage => Tier::Slow,
            Self::Slot | Self::Boot | Self::Device => Tier::Once,
        }
    }
}

/// How often a tier is worth re-asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    Fast,
    Medium,
    Slow,
    /// **Asked until it answers, and then never again.**
    ///
    /// Which slot booted and what block devices exist cannot change without a
    /// reboot, and a reboot ends the session watching. Re-asking would spend a
    /// journal record — `ADR-022` §8's 8 MiB, this module's opening constraint —
    /// to learn what the screen already knows.
    ///
    /// **A failure is still retried**, with the back-off every other tier gets:
    /// *the machine did not answer* is not the same as *the machine answered*,
    /// and giving up on the first would leave the pane permanently blank about a
    /// board that came back. [`SLOW`] is the base it widens from.
    Once,
}

/// The intervals a person can cycle the fast tier through, with `d`.
///
/// **`top`'s key**, and the same idea: the fast tier is the only one that is
/// adjustable, because it is the only one whose interval is a matter of taste.
/// Five seconds for thermals is a fact about thermal mass and thirty for a
/// filesystem is a fact about how often one changes, and neither becomes truer
/// because a person pressed a key.
pub const FAST_INTERVALS: &[Duration] = &[
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(3),
    Duration::from_secs(5),
    Duration::from_secs(10),
];

/// Where the fast tier starts. **Five seconds**, since 2026-08-29.
///
/// ~~*Two seconds*: `htop` refreshes at 1.5 s and `top` defaults to 3 s, so this
/// is the responsive end of normal without overlapping requests on a household
/// network, where `PLAN.md` §51 measured a round trip at 213 ms.~~
///
/// **Struck rather than edited** (`AGENTS.md` §2, §7). The argument was about a
/// round trip and it was right about one; what changed is what a poll *costs*.
/// `ADR-034` §11.5.2 moved this pane onto the control plane, where a cycle
/// spawns a `rivet mcp` process on a 926 MiB board — so the interval is now a
/// question about that board's load rather than about how live a screen feels,
/// and the maintainer offered five.
///
/// [`FAST_INTERVALS`] keeps its 1/2/3/5/10 range: the interval is still a matter
/// of taste, and `d` is still how a person says so.
pub const FAST_DEFAULT: Duration = Duration::from_secs(5);

/// The medium tier. Thermal mass means a board's temperature cannot change fast,
/// and `Admin-Surface-Survey.md` §7.6 measured the read as the most expensive
/// one available — several sysfs files plus an ioctl on `/dev/vcio`.
pub const MEDIUM: Duration = Duration::from_secs(5);

/// The slow tier. `statvfs` across every mount, answering a question that
/// changes when something is installed.
pub const SLOW: Duration = Duration::from_secs(30);

/// The widest a failing subject is backed off to.
///
/// **A minute.** Past that a person is no longer watching a monitor, they are
/// waiting for a machine to come back, and the `r` key is how they ask again.
pub const MAX_INTERVAL: Duration = Duration::from_secs(60);

/// The error code a machine returns for a subject its build has never heard of.
///
/// **The code, not the sentence.** `ADR-006` §4.1's whole argument is that an
/// agent branches on the code rather than on English, and the same applies here:
/// matching the prose would break the moment somebody improved the wording.
pub const UNKNOWN_SUBJECT: &str = "E_UNKNOWN_SUBJECT";

// ---------------------------------------------------------------------------
// The port
// ---------------------------------------------------------------------------

/// **Where a reading comes from** — the seam between this screen and a machine.
///
/// `AGENTS.md` §3's second kind of seam: an adapter's own, so that the screen can
/// be driven without a board.
///
/// # Two methods, and the second one is the interesting one
///
/// [`Source::get`] asks for one subject and is what a fake in a test implements.
/// [`Source::get_each`] asks for **every subject due on one tick** and exists
/// because `ADR-034` §11.5.2 ruled the cost model: `rivet-mcpd` spawns a
/// `rivet mcp` process **per CONNECTION, not per request**, so three subjects on
/// three connections cost a 926 MiB board three processes and the same three on
/// one connection cost it one.
///
/// The default implementation loops, which is correct and is what a fake wants.
/// [`Tls`] overrides it, which is the whole optimisation.
pub trait Source {
    /// Ask for one subject and give back what the machine printed.
    ///
    /// # Errors
    ///
    /// Whatever the transport or the machine said, verbatim. **Not
    /// interpreted** — [`Outcome::classify`] is the one place a refusal is read,
    /// so a transport cannot decide on its own that a machine is missing a
    /// subject.
    fn get(&mut self, subject: Subject) -> Result<String, String>;

    /// Ask for several subjects, over as few connections as this transport can.
    ///
    /// **The returned vector is exactly as long as `subjects` and in the same
    /// order.** An implementation that cannot batch loops, which is what this
    /// default does and why it is a default rather than a required method.
    fn get_each(&mut self, subjects: &[Subject]) -> Vec<Result<String, String>> {
        subjects.iter().map(|subject| self.get(*subject)).collect()
    }
}

/// **Reading over `ADR-035`'s control plane** — mutually authenticated TLS on
/// port 7777, and no other route.
///
/// # One implementation of [`Source`], and that is deliberate
///
/// `ADR-034` §11.5.1 refused every shell-out — *"there is nothing left at the
/// far end"* — and §11.5.2 made this pane the first thing that ruling breaks.
/// **This repository does not keep dead transports**, and one kept beside a live
/// one is a thing an agent reaches for because it is there.
///
/// The trait survived the change it was written for: *"when the fleet tool grows
/// an identity, one new implementation of this trait moves the whole pane onto
/// the control plane and nothing above it changes."* Nothing above it changed.
///
/// # What this costs, said out loud
///
/// One TCP connection, one TLS handshake and one `rivet mcp` process **per poll
/// CYCLE** — not per subject. The module header's property 3 is now true of the
/// build and not only of the shape.
///
/// The connection is still not kept between cycles: the listener closes after
/// its response, so keeping one is a change at that end and `ADR-034` §11.5.2
/// puts it out of scope. The batching is most of the benefit anyway.
pub struct Tls {
    /// The transport, built once and living on the worker thread.
    pub transport: rivet_remote::tls::Tls,
}

impl Source for Tls {
    fn get(&mut self, subject: Subject) -> Result<String, String> {
        self.transport.get(subject.word())
    }

    fn get_each(&mut self, subjects: &[Subject]) -> Vec<Result<String, String>> {
        // **The whole batch on one connection.** There is no `--output json`
        // here and no format to ask for: a tool call answers the operation's own
        // JSON, which is `ADR-006` §3.1's co-equality doing what it was for.
        let words: Vec<&str> = subjects.iter().map(|s| s.word()).collect();
        self.transport.get_each(&words)
    }
}

// ---------------------------------------------------------------------------
// The readings
// ---------------------------------------------------------------------------

/// Memory pressure, in **hundredths of a percent**.
///
/// The unit is `rivet_core::operation::memory`'s and the reason is its: the
/// structured surface has no floating-point value, and whole percent would
/// collapse the difference between a machine stalling 0.4% of the time and one
/// that is not stalling at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pressure {
    pub some_avg10: i64,
    pub some_avg60: i64,
    pub full_avg10: i64,
    pub full_avg60: i64,
}

/// What `get memory` answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Memory {
    pub total_bytes: u64,
    /// **The one to read.** Free memory plus what the kernel would reclaim.
    pub available_bytes: u64,
    /// **Not headroom**, and the classic misdiagnosis of Linux memory.
    pub free_bytes: u64,
    pub cached_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_free_bytes: u64,
    /// **Absent on a kernel without `CONFIG_PSI`**, which has measured nothing
    /// rather than zero.
    pub pressure: Option<Pressure>,
}

/// One process, as the kernel reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Process {
    pub pid: i64,
    pub parent_pid: i64,
    pub name: String,
    /// `R`, `S`, `D`, `Z`… **`D` and `Z` are the ones to look at**: an
    /// uninterruptible sleep on a Pi is very often I/O against a failing card,
    /// and a zombie means something is not reaping its children.
    pub state: String,
    pub threads: i64,
    /// A kernel thread. Filtered out of the list by default, which is what the
    /// maintainer asked for; the key that brings them back is in the footer.
    pub kernel: bool,
    /// **Absent for a kernel thread**, which has no user address space at all.
    pub resident_bytes: Option<u64>,
    /// CPU milliseconds this process has consumed since it started — user plus
    /// system, summed here because the screen shows one figure.
    ///
    /// **A counter, not a share.** It is what [`ProcessRates`] is computed from,
    /// and on its own it says how much work a process has done in its life
    /// rather than what it is doing now. **Absent where the machine could not
    /// read it**, never zero: zero milliseconds is a real reading of a process
    /// that has done nothing.
    pub cpu_millis: Option<u64>,
}

impl Process {
    /// Whether this process is in a state worth reading twice.
    #[must_use]
    pub fn is_trouble(&self) -> bool {
        self.state.starts_with('D') || self.state.starts_with('Z')
    }
}

/// What `get process` answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Processes {
    pub rows: Vec<Process>,
    /// Listed and then gone before they could be read. **Not an error**: `/proc`
    /// changes while it is read. A large number on an idle machine is itself a
    /// finding, which is why it is on screen.
    pub vanished: i64,
    pub userland: i64,
}

/// One mounted filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filesystem {
    pub source: String,
    pub mount_point: String,
    pub fstype: String,
    pub read_only: bool,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    /// **Absent on a filesystem that has no such number**, rather than 0.
    pub capacity_percent: Option<i64>,
    /// Which of Rivet's own locations this filesystem holds.
    pub holds: String,
}

/// What `get storage` answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Storage {
    pub filesystems: Vec<Filesystem>,
    /// Mount-table entries whose usage could not be measured. **Never dropped
    /// silently**: an answer that omitted one would say *there is no such
    /// filesystem*.
    pub unreadable: i64,
    pub unreadable_detail: Vec<String>,
}

/// One service, as the **supervisor believes** it to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    pub name: String,
    pub enabled: bool,
    pub state: String,
    /// Absent when the supervisor has no process for it.
    pub pid: Option<i64>,
    pub restarts: i64,
    pub ready: bool,
}

/// What `get status` answered — the supervisor's own memory, over `ADR-032`
/// §3.2's query socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Supervisor {
    pub phase: String,
    pub services: Vec<Service>,
}

/// One processor's counters — **milliseconds since boot, and not a
/// percentage.**
///
/// `rivet_core::operation::machine::CpuTimes`' unit and its argument. The two
/// sums are the machine's own: `total_millis` and `busy_millis` are computed by
/// the subject rather than here, so that this screen and any other caller
/// compute a utilisation from the same denominator and cannot disagree about
/// whether `iowait` counts as work. It does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuTimes {
    pub name: String,
    pub total_millis: u64,
    /// Neither idle nor waiting on I/O.
    pub busy_millis: u64,
    /// Waiting on I/O. **Beside a `D`-state process, this is a failing card** —
    /// `ADR-018` §7 — and it is kept apart from `busy` so the screen cannot
    /// report a blocked machine as a loaded one.
    pub iowait_millis: u64,
}

/// The kernel's load average, in hundredths, and **not a CPU utilisation.**
///
/// On Linux it counts tasks in uninterruptible sleep as well as runnable ones,
/// which every other Unix does not — so a load of 4 with idle processors is
/// something stuck waiting rather than something working.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Load {
    pub avg1: i64,
    pub avg5: i64,
    pub avg15: i64,
    pub runnable: i64,
    pub existing: i64,
}

/// What `get cpu` answered. **Counters, and the load average.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cpu {
    pub processors: i64,
    /// The kernel's own aggregate line, not a sum of [`Self::cpus`].
    pub total: CpuTimes,
    pub cpus: Vec<CpuTimes>,
    /// **Absent where the machine states none**, rather than three zeroes — a
    /// load average of zero is a real reading of an idle machine.
    pub load: Option<Load>,
}

/// **A utilisation, which needed two readings to exist.**
///
/// Computed by [`Poller`] when a second `cpu` answer arrives, and `None` until
/// then. Percentages of a single processor, so `total` is a percentage of the
/// whole machine and each entry of `cpus` is a percentage of that core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuRates {
    /// Busy time as a percentage of elapsed processor time, machine-wide.
    pub total: u64,
    /// Time spent waiting on I/O, as a percentage. **Reported separately from
    /// busy**, because a machine at 90% iowait is not a busy machine.
    pub iowait: u64,
    /// One entry per processor, in the order the machine listed them.
    pub cpus: Vec<(String, u64)>,
    /// How far apart the two readings were. **On screen**, because a rate over
    /// an unstated interval is not a measurement.
    pub over: Duration,
}

/// **Per-process utilisation**, which also needed two readings.
///
/// A percentage **of one processor**, which is `top`'s own convention and is
/// stated because it is surprising: a four-threaded process pinning a four-core
/// board reads 400%, not 100%. The alternative — dividing by the core count —
/// makes a single-threaded runaway on a quad-core machine read 25%, which is the
/// number nobody is looking for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessRates {
    /// pid, and hundredths of a percent of one processor.
    ///
    /// **Hundredths** for `Pressure`'s reason: a process using 0.4% of a core
    /// steadily is a different machine from one using none, and whole percent
    /// collapses them.
    pub by_pid: Vec<(i64, u64)>,
    pub over: Duration,
}

impl ProcessRates {
    /// What this process was measured at, or `None` if it was not.
    ///
    /// **`None` for a process that was not in the earlier reading** — one that
    /// started since — because a rate needs two samples of the *same* process
    /// and a pid that has just appeared has one. Rendering `0%` for it would say
    /// it is doing nothing, which is very often the opposite of true for a
    /// process that just started.
    #[must_use]
    pub fn of(&self, pid: i64) -> Option<u64> {
        self.by_pid
            .iter()
            .find(|(p, _)| *p == pid)
            .map(|(_, percent)| *percent)
    }
}

/// One thermal zone, as its driver names itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    /// `cpu_thermal` on a Pi 4, `coretemp` on x86.
    pub name: String,
    /// **Thousandths of a degree Celsius**, which is the unit Linux states.
    pub millidegrees: i64,
}

/// This CPU's clock against its ceiling — **the throttle observed rather than
/// inferred.**
///
/// `docs/design-candidates/Admin-Surface-Survey.md` §7.1: temperature is the
/// weakest of the four thermal facts and this is the second strongest. But the
/// comparison that matters is `ceiling` against `maximum` and **not** `current`
/// against `maximum`: a governor scales an idle machine down, so a Pi at 600 MHz
/// against a 1500 MHz ceiling is behaving correctly and a screen that called
/// that a throttle would cry wolf all day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clock {
    pub current_hz: u64,
    pub maximum_hz: u64,
    pub minimum_hz: Option<u64>,
    /// What policy currently allows. **Below `maximum_hz` is the throttle.**
    pub ceiling_hz: Option<u64>,
    /// The subject's own comparison, made once there rather than by every
    /// caller. Absent when there is no ceiling to compare.
    pub ceiling_below_maximum: Option<bool>,
    pub governor: Option<String>,
    pub at_maximum: bool,
}

/// The firmware's own account of whether it has held this machine back.
///
/// **The `_since_boot` half is the point.** A board that is fine now and was
/// under-volted at 03:00 is indistinguishable from one that never was, and
/// nothing else on the machine recovers that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Throttle {
    pub under_voltage_now: bool,
    pub frequency_capped_now: bool,
    pub throttled_now: bool,
    pub soft_temperature_limit_now: bool,
    pub under_voltage_since_boot: bool,
    pub frequency_capped_since_boot: bool,
    pub throttled_since_boot: bool,
    pub soft_temperature_limit_since_boot: bool,
}

/// What `get thermal` answered.
///
/// **Every half is optional and an absent half carries its reason**, which is
/// the subject's own rule and the one that matters most here:
/// `throttled_since_boot: false` is a claim about the last several hours of a
/// board's life, and a build that could not reach the firmware has established
/// nothing of the kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thermal {
    pub zones: Vec<Zone>,
    pub zones_unreadable: Vec<String>,
    pub clock: Option<Clock>,
    /// Present exactly when [`Self::clock`] is absent.
    pub clock_unmeasured: Option<String>,
    pub throttle: Option<Throttle>,
    /// Present exactly when [`Self::throttle`] is absent. **The most important
    /// string in this struct**: absent flags are not clear flags.
    pub throttle_unmeasured: Option<String>,
}

impl Thermal {
    /// The hottest zone, which is the one worth putting on a line.
    #[must_use]
    pub fn hottest(&self) -> Option<&Zone> {
        self.zones.iter().max_by_key(|z| z.millidegrees)
    }

    /// Whether any `_since_boot` flag is set. **The question this subject exists
    /// to answer**, and it is deliberately not askable when the flags were not
    /// read — the type makes that an `Option` all the way to the screen.
    #[must_use]
    pub fn ever_held_back(&self) -> Option<bool> {
        self.throttle.as_ref().map(|t| {
            t.under_voltage_since_boot
                || t.frequency_capped_since_boot
                || t.throttled_since_boot
                || t.soft_temperature_limit_since_boot
        })
    }
}

/// What `get slot` answered — **which A/B partition is running, and whether the
/// two authorities agree.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slots {
    /// `a` or `b`. **Absent when the firmware and the kernel disagree**, which
    /// is the subject's whole design: naming one would name the partition an
    /// update overwrites.
    pub running: Option<String>,
    /// The one an update writes into. Present exactly when `running` is.
    pub inactive: Option<String>,
    pub booted: Option<String>,
    pub root: Option<String>,
    /// Present only when both authorities were measured. **`false` is serious.**
    pub agree: Option<bool>,
    /// **Absent, never `false`, for a flag nobody read** — `false` asserts that
    /// doing nothing is safe, and a trial that is not committed rolls back.
    pub in_trial_boot: Option<bool>,
    pub trial_unmeasured: Option<String>,
}

/// What `get boot` answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Boot {
    pub boot_id: Option<String>,
    /// **Off the monotonic counter**, which is the trustworthy number on
    /// hardware with no real-time clock (`ADR-021`). Nothing here says *when*
    /// this machine booted, and a screen that computed now-minus-uptime would be
    /// exactly as wrong as the clock is.
    pub uptime_seconds: u64,
    /// How many boots the journal still holds.
    pub boots: Option<usize>,
    /// Whether this boot appears among them. **`false` is worth looking at**: it
    /// means nothing has been written to the journal during this boot.
    pub recorded: Option<bool>,
}

/// One block device, whether or not anything is mounted on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub name: String,
    /// The kernel's own word — `disk` or `partition`.
    pub kind: Option<String>,
    /// The GPT label. **What `root=PARTLABEL=` names**, which makes it the most
    /// useful single string on a partition.
    pub partition_label: Option<String>,
    pub size_bytes: Option<u64>,
    /// Bytes actually written since boot. `ADR-018` §7 treats SD wear as a
    /// design constraint and this is the first thing that measures any of it.
    pub written_bytes: Option<u64>,
    /// **Verbatim where the device states it and absent where it does not.** A
    /// consumer SD card generally states none, and a percentage computed for one
    /// would be invented.
    pub life_time: Option<String>,
    pub pre_eol_info: Option<String>,
}

/// What `get device` answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Devices {
    pub devices: Vec<Device>,
    pub disks: i64,
    pub unreadable: usize,
}

/// One reading, whatever it was of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    Cpu(Cpu),
    Memory(Memory),
    Processes(Processes),
    Storage(Storage),
    Supervisor(Supervisor),
    Thermal(Thermal),
    Slots(Slots),
    Boot(Boot),
    Devices(Devices),
    // ~~`Verbatim(Vec<(String, String)>)` — a subject whose shape this build has
    // never seen, rendered field by field as the machine named them.~~
    //
    // **Removed 2026-08-29, and recorded here rather than silently.** It existed
    // for exactly one subject — `thermal`, which the pane was written against
    // before it was a subject — and it did that job: shipping a pane that polls
    // something it cannot parse, and shows whatever comes back rather than
    // inventing field names, is what made the pane possible a day early.
    //
    // It is gone because `thermal` landed and nothing constructs it any more.
    // Keeping an unreachable variant so that a *future* subject might want it is
    // the speculation this codebase does not carry: the argument for it is
    // written down, here and in `PLAN.md`, and a variant is four lines to
    // reinstate. What must NOT come back is a renderer written against invented
    // field names — `AGENTS.md` §5's guess that succeeds.
}

/// Read one subject's answer.
///
/// # Errors
///
/// A sentence quoting what the machine sent, when it is not the shape this
/// screen reads. **A missing required field is a refusal, not a zero**: a build
/// whose `memory` has no `cached_bytes` has changed shape, and rendering `0`
/// would state a fact nobody measured.
pub fn read(subject: Subject, text: &str) -> Result<Reading, String> {
    let value = crate::json::parse(text.trim())
        .map_err(|e| format!("`get {}` did not answer JSON: {e}", subject.word()))?;
    match subject {
        Subject::Cpu => read_cpu(&value).map(Reading::Cpu),
        Subject::Memory => read_memory(&value).map(Reading::Memory),
        Subject::Process => read_processes(&value).map(Reading::Processes),
        Subject::Storage => read_storage(&value).map(Reading::Storage),
        Subject::Status => read_supervisor(&value).map(Reading::Supervisor),
        Subject::Thermal => Ok(Reading::Thermal(read_thermal(&value))),
        Subject::Slot => Ok(Reading::Slots(read_slots(&value))),
        Subject::Boot => read_boot(&value).map(Reading::Boot),
        Subject::Device => read_devices(&value).map(Reading::Devices),
    }
}

/// The one refusal for a field the shape requires and the answer does not carry.
fn missing(subject: Subject, field: &str) -> String {
    format!(
        "`get {}` carries no `{field}`, so this machine's answer is not the shape this screen \
         reads. Nothing is shown for it rather than a zero, which would be a number nobody \
         measured (AGENTS.md §5). Check that the board's build and this tool's are the same age.",
        subject.word()
    )
}

/// One `cpus[]` or `total` entry.
///
/// **`total_millis` and `busy_millis` are read rather than summed here**, which
/// is the whole reason `rivet_core::operation::cpu` publishes them: two callers
/// summing for themselves is two answers to *what counts as busy*, and the one
/// that gets `iowait` on the wrong side reports a machine blocked on a dying
/// card as a fully loaded one.
fn read_times(value: &Json, where_at: &str) -> Result<CpuTimes, String> {
    let need = |field: &str| {
        value
            .u64_at(field)
            .ok_or_else(|| missing(Subject::Cpu, &format!("{where_at}.{field}")))
    };
    Ok(CpuTimes {
        name: value
            .str_at("name")
            .ok_or_else(|| missing(Subject::Cpu, &format!("{where_at}.name")))?
            .to_string(),
        total_millis: need("total_millis")?,
        busy_millis: need("busy_millis")?,
        iowait_millis: need("iowait_millis")?,
    })
}

fn read_cpu(value: &Json) -> Result<Cpu, String> {
    let total = read_times(
        value
            .get("total")
            .ok_or_else(|| missing(Subject::Cpu, "total"))?,
        "total",
    )?;
    let mut cpus = Vec::new();
    for row in value
        .array_at("cpus")
        .ok_or_else(|| missing(Subject::Cpu, "cpus"))?
    {
        cpus.push(read_times(row, "cpus[]")?);
    }
    Ok(Cpu {
        processors: value
            .i64_at("processors")
            .ok_or_else(|| missing(Subject::Cpu, "processors"))?,
        total,
        cpus,
        // **Present or absent, never three zeroes** — the subject's own rule,
        // and the figure a reader is likeliest to act on without checking.
        load: value.get("load").and_then(|l| {
            Some(Load {
                avg1: l.i64_at("avg1")?,
                avg5: l.i64_at("avg5")?,
                avg15: l.i64_at("avg15")?,
                runnable: l.i64_at("runnable").unwrap_or(0),
                existing: l.i64_at("existing").unwrap_or(0),
            })
        }),
    })
}

/// **Nothing in `thermal` is required**, which is why this cannot fail.
///
/// The subject's own shape: a board with a thermal zone and no `cpufreq`, or
/// with both and no route to the firmware, is an ordinary board. Every half is
/// optional and an absent half is reported as absent — so a refusal here would
/// be this screen inventing a requirement the machine does not have, and the
/// interesting reading is precisely the one where most of it is missing.
fn read_thermal(value: &Json) -> Thermal {
    let zones = value
        .array_at("zones")
        .unwrap_or_default()
        .iter()
        .filter_map(|z| {
            Some(Zone {
                name: z.str_at("name")?.to_string(),
                millidegrees: z.i64_at("millidegrees")?,
            })
        })
        .collect();
    let zones_unreadable = value
        .array_at("zones_unreadable")
        .unwrap_or_default()
        .iter()
        .map(|u| {
            format!(
                "{}: {}",
                u.str_at("zone").unwrap_or("?"),
                u.str_at("why").unwrap_or("no reason given")
            )
        })
        .collect();
    let clock = value.get("frequency").and_then(|f| {
        Some(Clock {
            current_hz: f.u64_at("current_hz")?,
            maximum_hz: f.u64_at("maximum_hz")?,
            minimum_hz: f.u64_at("minimum_hz"),
            ceiling_hz: f.u64_at("ceiling_hz"),
            ceiling_below_maximum: f.bool_at("ceiling_below_maximum"),
            governor: f.str_at("governor").map(str::to_string),
            at_maximum: f.bool_at("at_maximum").unwrap_or(false),
        })
    });
    // **All eight or none.** A partial set would be worse than none: the
    // question is *has this board ever been held back*, and answering it from
    // half the flags is answering it from a guess.
    let throttle = value.get("throttle").and_then(|t| {
        Some(Throttle {
            under_voltage_now: t.bool_at("under_voltage_now")?,
            frequency_capped_now: t.bool_at("frequency_capped_now")?,
            throttled_now: t.bool_at("throttled_now")?,
            soft_temperature_limit_now: t.bool_at("soft_temperature_limit_now")?,
            under_voltage_since_boot: t.bool_at("under_voltage_since_boot")?,
            frequency_capped_since_boot: t.bool_at("frequency_capped_since_boot")?,
            throttled_since_boot: t.bool_at("throttled_since_boot")?,
            soft_temperature_limit_since_boot: t.bool_at("soft_temperature_limit_since_boot")?,
        })
    });
    Thermal {
        zones,
        zones_unreadable,
        clock_unmeasured: if clock.is_none() {
            Some(
                value
                    .str_at("frequency_unmeasured")
                    .unwrap_or("no cpufreq policy was read, and the machine gave no reason")
                    .to_string(),
            )
        } else {
            None
        },
        clock,
        throttle_unmeasured: if throttle.is_none() {
            Some(
                value
                    .str_at("throttle_unmeasured")
                    .unwrap_or(
                        "the firmware's throttle flags were not read, and the machine gave no \
                         reason",
                    )
                    .to_string(),
            )
        } else {
            None
        },
        throttle,
    }
}

/// **Nothing in `slot` is required either**, and for a sharper reason than
/// `thermal`'s: the subject withholds `running` on purpose when its two
/// authorities disagree, so an answer with no running slot is the *most*
/// important one it can give. Refusing to read it would turn the finding into a
/// parse error.
fn read_slots(value: &Json) -> Slots {
    Slots {
        running: value.str_at("running").map(str::to_string),
        inactive: value.str_at("inactive").map(str::to_string),
        booted: value.str_at("booted").map(str::to_string),
        root: value.str_at("root").map(str::to_string),
        agree: value.bool_at("agree"),
        in_trial_boot: value.bool_at("in_trial_boot"),
        trial_unmeasured: value.str_at("trial_unmeasured").map(str::to_string),
    }
}

fn read_boot(value: &Json) -> Result<Boot, String> {
    Ok(Boot {
        boot_id: value.str_at("boot_id").map(str::to_string),
        // **The one field that must be there.** It is what the subject
        // guarantees, and a zero would say this machine booted a moment ago.
        uptime_seconds: value
            .u64_at("uptime_seconds")
            .ok_or_else(|| missing(Subject::Boot, "uptime_seconds"))?,
        // **Absent is not an empty journal.** Reporting `Some(0)` would say this
        // machine has never recorded a boot, which is a different statement.
        boots: value.array_at("boots").map(<[Json]>::len),
        recorded: value.bool_at("recorded"),
    })
}

fn read_devices(value: &Json) -> Result<Devices, String> {
    let rows = value
        .array_at("devices")
        .ok_or_else(|| missing(Subject::Device, "devices"))?;
    let devices = rows
        .iter()
        .filter_map(|d| {
            Some(Device {
                name: d.str_at("name")?.to_string(),
                kind: d.str_at("kind").map(str::to_string),
                partition_label: d.str_at("partition_label").map(str::to_string),
                size_bytes: d.u64_at("size_bytes"),
                written_bytes: d.get("io").and_then(|io| io.u64_at("written_bytes")),
                // Verbatim or absent — the subject refuses to turn a vendor code
                // into a percentage and so does this.
                life_time: d.str_at("life_time").map(str::to_string),
                pre_eol_info: d.str_at("pre_eol_info").map(str::to_string),
            })
        })
        .collect();
    Ok(Devices {
        devices,
        disks: value.i64_at("disks").unwrap_or(0),
        unreadable: value
            .array_at("devices_unreadable")
            .map_or(0, <[Json]>::len),
    })
}

fn read_memory(value: &Json) -> Result<Memory, String> {
    let need = |field: &str| {
        value
            .u64_at(field)
            .ok_or_else(|| missing(Subject::Memory, field))
    };
    Ok(Memory {
        total_bytes: need("total_bytes")?,
        available_bytes: need("available_bytes")?,
        free_bytes: need("free_bytes")?,
        cached_bytes: need("cached_bytes")?,
        swap_total_bytes: need("swap_total_bytes")?,
        swap_free_bytes: need("swap_free_bytes")?,
        // **Present or absent, never zeroed** — the subject's own rule.
        pressure: value.get("pressure").and_then(|p| {
            Some(Pressure {
                some_avg10: p.i64_at("some_avg10")?,
                some_avg60: p.i64_at("some_avg60")?,
                full_avg10: p.i64_at("full_avg10")?,
                full_avg60: p.i64_at("full_avg60")?,
            })
        }),
    })
}

fn read_processes(value: &Json) -> Result<Processes, String> {
    let rows = value
        .array_at("processes")
        .ok_or_else(|| missing(Subject::Process, "processes"))?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(Process {
            pid: row
                .i64_at("pid")
                .ok_or_else(|| missing(Subject::Process, "processes[].pid"))?,
            parent_pid: row
                .i64_at("parent_pid")
                .ok_or_else(|| missing(Subject::Process, "processes[].parent_pid"))?,
            name: row
                .str_at("name")
                .ok_or_else(|| missing(Subject::Process, "processes[].name"))?
                .to_string(),
            state: row
                .str_at("state")
                .ok_or_else(|| missing(Subject::Process, "processes[].state"))?
                .to_string(),
            threads: row
                .i64_at("threads")
                .ok_or_else(|| missing(Subject::Process, "processes[].threads"))?,
            kernel: row
                .bool_at("kernel")
                .ok_or_else(|| missing(Subject::Process, "processes[].kernel"))?,
            // Optional by design, and the absence is the fact.
            resident_bytes: row.u64_at("resident_bytes"),
            // **Both or neither.** The subject emits the pair from one file and
            // omits the pair when it could not be read, so a row carrying one
            // and not the other is a shape this screen has not seen — and
            // summing a half of it would report a process as doing half the work
            // it is doing.
            cpu_millis: match (row.u64_at("user_millis"), row.u64_at("system_millis")) {
                (Some(user), Some(system)) => Some(user.saturating_add(system)),
                _ => None,
            },
        });
    }
    Ok(Processes {
        rows: out,
        vanished: value
            .i64_at("vanished")
            .ok_or_else(|| missing(Subject::Process, "vanished"))?,
        userland: value
            .i64_at("userland")
            .ok_or_else(|| missing(Subject::Process, "userland"))?,
    })
}

fn read_storage(value: &Json) -> Result<Storage, String> {
    let rows = value
        .array_at("filesystems")
        .ok_or_else(|| missing(Subject::Storage, "filesystems"))?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let need = |field: &str| {
            row.u64_at(field)
                .ok_or_else(|| missing(Subject::Storage, &format!("filesystems[].{field}")))
        };
        out.push(Filesystem {
            source: row.str_at("source").unwrap_or("—").to_string(),
            mount_point: row
                .str_at("mount-point")
                .ok_or_else(|| missing(Subject::Storage, "filesystems[].mount-point"))?
                .to_string(),
            fstype: row.str_at("fstype").unwrap_or("—").to_string(),
            read_only: row.bool_at("read-only").unwrap_or(false),
            total_bytes: need("total-bytes")?,
            used_bytes: need("used-bytes")?,
            available_bytes: need("available-bytes")?,
            capacity_percent: row.i64_at("capacity-percent"),
            holds: row.str_at("holds").unwrap_or_default().to_string(),
        });
    }
    Ok(Storage {
        filesystems: out,
        unreadable: value.i64_at("unreadable").unwrap_or(0),
        unreadable_detail: value
            .array_at("unreadable-detail")
            .unwrap_or_default()
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
    })
}

fn read_supervisor(value: &Json) -> Result<Supervisor, String> {
    let rows = value
        .array_at("services")
        .ok_or_else(|| missing(Subject::Status, "services"))?;
    let mut services = Vec::with_capacity(rows.len());
    for row in rows {
        services.push(Service {
            name: row
                .str_at("name")
                .ok_or_else(|| missing(Subject::Status, "services[].name"))?
                .to_string(),
            enabled: row.bool_at("enabled").unwrap_or(false),
            state: row
                .str_at("state")
                .ok_or_else(|| missing(Subject::Status, "services[].state"))?
                .to_string(),
            // Absent when the supervisor has no process for it — which is
            // exactly the case the cross-check is looking for.
            pid: row.i64_at("pid"),
            restarts: row.i64_at("restarts").unwrap_or(0),
            ready: row.bool_at("ready").unwrap_or(false),
        });
    }
    Ok(Supervisor {
        phase: value
            .str_at("phase")
            .ok_or_else(|| missing(Subject::Status, "phase"))?
            .to_string(),
        services,
    })
}

// ---------------------------------------------------------------------------
// Where a subject stands
// ---------------------------------------------------------------------------

/// What the last attempt at a subject did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Nobody has asked yet. **Distinct from a failure**, for
    /// `crate::tui::reach::Reach::Unknown`'s reason: a screen that says
    /// *unreadable* before it has looked is stating something it does not know.
    Never,
    /// The machine answered and the answer read.
    Answered,
    /// It did not answer, or the answer did not read. The sentence is the
    /// machine's or the transport's, verbatim.
    Failed(String),
    /// **The machine's build has no such subject.** Not a failure to retry: it
    /// will be missing at the next tick too, and at every one after it, until
    /// somebody deploys a build that has it.
    Unsupported(String),
}

impl Outcome {
    /// Read a transport's refusal.
    ///
    /// **The one place a refusal is interpreted.** It matches
    /// [`UNKNOWN_SUBJECT`] — the wire code — rather than the sentence around it,
    /// because `ADR-006` §4.1's whole argument is that a caller branches on the
    /// code and not on English.
    #[must_use]
    pub fn classify(why: String) -> Self {
        if why.contains(UNKNOWN_SUBJECT) {
            Self::Unsupported(why)
        } else {
            Self::Failed(why)
        }
    }
}

/// Where one subject stands: what it last said, when, and what happened since.
#[derive(Debug, Clone)]
pub struct Slot {
    pub subject: Subject,
    /// The last reading that arrived, and when it did. **Kept across a
    /// failure**, so the screen can show the old numbers *labelled as old*
    /// rather than showing nothing — but never without [`Slot::age`] beside
    /// them.
    pub reading: Option<(Reading, Instant)>,
    pub outcome: Outcome,
    /// When a request was last *sent*, which is what the schedule runs off.
    pub attempted: Option<Instant>,
    /// Consecutive failures, for the back-off.
    pub failures: u32,
}

impl Slot {
    fn new(subject: Subject) -> Self {
        Self {
            subject,
            reading: None,
            outcome: Outcome::Never,
            attempted: None,
            failures: 0,
        }
    }

    /// How long ago the reading on screen arrived.
    ///
    /// **The number the screen must always render beside a value.** A stale
    /// figure shown as a live one is the one genuinely dangerous failure of a
    /// monitoring interface.
    #[must_use]
    pub fn age(&self, now: Instant) -> Option<Duration> {
        self.reading
            .as_ref()
            .map(|(_, at)| now.saturating_duration_since(*at))
    }

    /// Whether what is on screen is old enough that a person should be told.
    ///
    /// **Two and a half intervals.** One would flag every reading in the moment
    /// before its refresh lands; three would let a whole missed cycle pass
    /// unremarked.
    #[must_use]
    pub fn is_stale(&self, now: Instant, interval: Duration) -> bool {
        match self.age(now) {
            None => false,
            Some(age) => age.as_millis() * 2 > interval.as_millis() * 5,
        }
    }

    /// The reading, if there is one.
    #[must_use]
    pub fn value(&self) -> Option<&Reading> {
        self.reading.as_ref().map(|(reading, _)| reading)
    }
}

// ---------------------------------------------------------------------------
// The poller
// ---------------------------------------------------------------------------

/// **One poll cycle, on its way to the worker.**
///
/// A vector rather than a subject, since 2026-08-29: `ADR-034` §11.5.2 rules
/// **one connection per cycle carrying every subject due on that tick**, so the
/// unit of work handed across this channel has to be the cycle.
struct Job(Vec<Subject>);

/// One answer, on its way back.
struct Answer {
    subject: Subject,
    result: Result<String, String>,
}

/// **The tiered poller.** One worker thread, one request in flight at a time.
///
/// Created when a person opens the monitor and dropped when they leave the
/// device — which is what makes the worker's [`Source`] the natural home for a
/// kept connection.
pub struct Poller {
    jobs: Sender<Job>,
    answers: Receiver<Answer>,
    /// **How many answers this cycle is still waiting for.** The whole of
    /// property 1: nothing is sent while this is above zero, across every tier,
    /// so the tiers share one slot rather than each having their own.
    ///
    /// A count rather than a flag since 2026-08-29, because a cycle now carries
    /// every due subject on one connection and arrives as one answer per
    /// subject. A flag cleared by the first of them would let the next tick
    /// send while the rest of the batch was still on the wire.
    in_flight: usize,
    /// When this cycle was sent. **The only clock the in-flight indicator may
    /// read** — see [`Liveness`], and the reason it is a field rather than a
    /// wall clock the renderer keeps for itself.
    sent_at: Option<Instant>,
    slots: Vec<Slot>,
    fast: Duration,
    /// Ticks where something was due and could not be sent. On screen, because a
    /// monitor that is quietly falling behind should say so.
    pub skipped: u64,
    /// Whether the person has stopped it by hand, separately from the screen not
    /// being on top.
    pub paused: bool,
    /// **The other reading.** `get cpu` answers counters and refuses to answer a
    /// percentage; this is the layer that asks twice, so this is where the
    /// earlier sample lives.
    previous_cpu: Option<(Cpu, Instant)>,
    /// pid to cumulative CPU milliseconds, from the previous `process` answer.
    previous_processes: Option<(Vec<(i64, u64)>, Instant)>,
    /// **`None` until a second reading has landed**, and the pane renders that
    /// as a sentence rather than as `0%`.
    cpu_rates: Option<CpuRates>,
    process_rates: Option<ProcessRates>,
}

impl Poller {
    /// Start a worker that reads through `source`.
    ///
    /// **The source is moved onto the worker thread and lives there**, which is
    /// the point: it is constructed once, and a transport that keeps a socket
    /// keeps it for the life of the screen.
    #[must_use]
    pub fn spawn(mut source: Box<dyn Source + Send>) -> Self {
        let (jobs, to_worker) = channel::<Job>();
        let (to_ui, answers) = channel::<Answer>();
        // Detached, and its answer may never arrive — `crate::tui::reach::Probes`'
        // reason. The worker ends when `jobs` is dropped, which happens when the
        // screen closes.
        let _ = std::thread::Builder::new()
            .name("monitor".to_string())
            .spawn(move || {
                for Job(subjects) in to_worker {
                    // **One call for the whole cycle**, so a transport that can
                    // put them on one connection does. The answers come back in
                    // the order asked and are unpacked in that order.
                    let results = source.get_each(&subjects);
                    for (subject, result) in subjects.into_iter().zip(results) {
                        if to_ui.send(Answer { subject, result }).is_err() {
                            return;
                        }
                    }
                }
            });
        Self {
            jobs,
            answers,
            in_flight: 0,
            sent_at: None,
            slots: Subject::ALL.iter().copied().map(Slot::new).collect(),
            fast: FAST_DEFAULT,
            skipped: 0,
            paused: false,
            previous_cpu: None,
            previous_processes: None,
            cpu_rates: None,
            process_rates: None,
        }
    }

    /// **The machine's utilisation, if two readings have landed.**
    ///
    /// `None` before the second one, and the pane must render that as *not yet
    /// known* rather than as `0%` — `AGENTS.md` §5, and the same rule
    /// `get memory` follows for PSI.
    #[must_use]
    pub const fn cpu_rates(&self) -> Option<&CpuRates> {
        self.cpu_rates.as_ref()
    }

    /// Per-process utilisation, on the same terms.
    #[must_use]
    pub const fn process_rates(&self) -> Option<&ProcessRates> {
        self.process_rates.as_ref()
    }

    /// Where a subject stands.
    ///
    /// **`Option`, and a search rather than an index.** Every subject has a slot
    /// by construction — [`Subject::ALL`] builds them — so this cannot be `None`
    /// today. It is still written this way because the alternative is an
    /// `unwrap` or an index, and a panic here takes the operator's terminal down
    /// with it (`crate::tui`'s standing rule).
    #[must_use]
    pub fn slot(&self, subject: Subject) -> Option<&Slot> {
        self.slots.iter().find(|s| s.subject == subject)
    }

    fn slot_mut(&mut self, subject: Subject) -> Option<&mut Slot> {
        self.slots.iter_mut().find(|s| s.subject == subject)
    }

    /// The fast tier's current interval, which is what the screen displays.
    #[must_use]
    pub const fn fast_interval(&self) -> Duration {
        self.fast
    }

    /// Move the fast tier to the next interval in [`FAST_INTERVALS`], wrapping.
    pub fn cycle_fast_interval(&mut self) {
        let at = FAST_INTERVALS.iter().position(|d| *d == self.fast);
        let next = at.map_or(0, |at| (at + 1) % FAST_INTERVALS.len());
        self.fast = FAST_INTERVALS.get(next).copied().unwrap_or(FAST_DEFAULT);
    }

    /// How long this subject waits before being asked again, back-off included.
    #[must_use]
    pub fn interval(&self, subject: Subject) -> Duration {
        let base = match subject.tier() {
            Tier::Fast => self.fast,
            Tier::Medium => MEDIUM,
            // A `Once` subject that answered is never due again — [`most_overdue`]
            // skips it — so this base is only ever the floor a FAILED one is
            // retried from, which is the same question `Slow` asks.
            Tier::Slow | Tier::Once => SLOW,
        };
        let failures = self.slot(subject).map_or(0, |s| s.failures);
        if failures == 0 {
            return base;
        }
        // Doubling, capped. A board that has stopped answering must not be
        // retried at 2 s for ever — a monitoring tool that does that becomes
        // part of the outage.
        let factor = 1u32 << failures.min(5);
        base.saturating_mul(factor).min(MAX_INTERVAL)
    }

    /// **Ask again for everything, now** — the `r` key.
    ///
    /// Including the subjects marked [`Outcome::Unsupported`]. That is
    /// deliberate: a board that has been updated since the screen opened now has
    /// the subject, and there is no other way for a person to find that out.
    pub fn refresh_all(&mut self) {
        for slot in &mut self.slots {
            slot.attempted = None;
            slot.failures = 0;
            if matches!(slot.outcome, Outcome::Unsupported(_)) {
                slot.outcome = Outcome::Never;
            }
        }
    }

    /// Take whatever has come back. **Never blocks.**
    pub fn collect(&mut self) {
        let answers: Vec<Answer> = self.answers.try_iter().collect();
        for answer in answers {
            self.in_flight = self.in_flight.saturating_sub(1);
            if self.in_flight == 0 {
                self.sent_at = None;
            }
            let outcome = match answer.result {
                Err(why) => Outcome::classify(why),
                Ok(text) => match read(answer.subject, &text) {
                    Ok(reading) => {
                        let now = Instant::now();
                        // **The rate is computed against what was there before
                        // the new reading replaces it**, which is the only
                        // moment both samples exist.
                        self.take_rates(&reading, now);
                        if let Some(slot) = self.slot_mut(answer.subject) {
                            slot.reading = Some((reading, now));
                        }
                        Outcome::Answered
                    }
                    // A reply that did not parse is a failure of this subject
                    // and not of the machine — and it is shown, because a screen
                    // that silently kept the old numbers would be exactly the
                    // stale-reading defect this whole module is built against.
                    Err(why) => Outcome::Failed(why),
                },
            };
            if let Some(slot) = self.slot_mut(answer.subject) {
                match &outcome {
                    Outcome::Answered => slot.failures = 0,
                    Outcome::Failed(_) => slot.failures = slot.failures.saturating_add(1),
                    // Not a failure to back off from: it will be missing at
                    // every tick until a different build is deployed.
                    Outcome::Unsupported(_) | Outcome::Never => {}
                }
                slot.outcome = outcome;
            }
        }
    }

    /// **Turn two readings of a counter into the rate one reading cannot give.**
    ///
    /// Called with each new `cpu` or `process` answer, before it displaces the
    /// one before it. Everything here is guarded so that a rate is produced only
    /// when it is real:
    ///
    /// * **no earlier sample, no rate** — the first reading of a counter says
    ///   how the machine has spent its life and nothing about now;
    /// * **a counter that went backwards, no rate** — the machine rebooted
    ///   between the two, and subtracting would produce an enormous negative
    ///   that saturates into a plausible-looking number;
    /// * **no elapsed processor time, no rate** — two readings inside one tick
    ///   have nothing between them to divide by.
    ///
    /// In every one of those cases the rate stays `None` and the pane says it
    /// does not know. `0%` is a measurement of an idle machine.
    fn take_rates(&mut self, reading: &Reading, now: Instant) {
        match reading {
            Reading::Cpu(cpu) => {
                self.cpu_rates = self
                    .previous_cpu
                    .as_ref()
                    .and_then(|(before, at)| rates_between(before, cpu, now.saturating_duration_since(*at)));
                self.previous_cpu = Some((cpu.clone(), now));
            }
            Reading::Processes(processes) => {
                let sample: Vec<(i64, u64)> = processes
                    .rows
                    .iter()
                    .filter_map(|p| p.cpu_millis.map(|ms| (p.pid, ms)))
                    .collect();
                self.process_rates = self.previous_processes.as_ref().and_then(|(before, at)| {
                    let over = now.saturating_duration_since(*at);
                    process_rates_between(before, &sample, over)
                });
                self.previous_processes = Some((sample, now));
            }
            _ => {}
        }
    }

    /// **Send one cycle**, carrying every subject that is due.
    ///
    /// `active` is whether this screen is the one on top. **Polling stops when
    /// nobody is looking**, which is the module header's property 2 and the
    /// largest saving available.
    ///
    /// Returns the subjects that were sent, for the tests that hold the
    /// no-overlap rule and the one-connection-per-cycle rule.
    ///
    /// **A cycle rather than a subject, since 2026-08-29.** `ADR-034` §11.5.2:
    /// the far end spawns a process per connection, so asking three subjects one
    /// at a time costs three processes to learn what one connection would have
    /// answered.
    pub fn tick(&mut self, now: Instant, active: bool) -> Vec<Subject> {
        if !active || self.paused {
            return Vec::new();
        }
        let due = self.due_now(now);
        if due.is_empty() {
            return Vec::new();
        }
        if self.busy() {
            // **Skipped, never queued.** A slow link must not become a backlog
            // that hammers the board when it recovers.
            self.skipped = self.skipped.saturating_add(1);
            return Vec::new();
        }
        if self.jobs.send(Job(due.clone())).is_err() {
            // The worker is gone. Recorded on every subject of the cycle rather
            // than swallowed.
            for subject in &due {
                if let Some(slot) = self.slot_mut(*subject) {
                    slot.outcome = Outcome::Failed(
                        "the reader thread has ended, so this screen cannot ask again. Leave \
                         the device and open it once more."
                            .to_string(),
                    );
                }
            }
            return Vec::new();
        }
        self.in_flight = due.len();
        self.sent_at = Some(now);
        for subject in &due {
            if let Some(slot) = self.slot_mut(*subject) {
                slot.attempted = Some(now);
            }
        }
        due
    }

    /// Whether a cycle is in flight.
    ///
    /// Read by the tests that hold the one-at-a-time rule; [`Poller::tick`]
    /// enforces it itself, so nothing in the interface has to ask first — the
    /// same shape and the same reason as `crate::tui::log::Fetches::busy`.
    #[must_use]
    pub const fn busy(&self) -> bool {
        self.in_flight > 0
    }

    /// **What the pane must say about whether it is alive.**
    ///
    /// See [`Liveness`] for the whole of why this is derived here and not in the
    /// renderer: it is a statement about the poller, and a renderer holding a
    /// clock could make it while the poller was doing nothing at all.
    #[must_use]
    pub fn liveness(&self, now: Instant) -> Liveness {
        if let Some(sent) = self.sent_at
            && self.busy()
        {
            return Liveness::InFlight {
                since: now.saturating_duration_since(sent),
            };
        }
        if self.paused {
            return Liveness::Stopped(PAUSED);
        }

        // **Backing off is *everything* that has been asked, failing** — not one
        // subject of several. A single subject a build cannot answer already
        // says so on its own section heading, and colouring the whole pane for
        // it would cry wolf; a board that has stopped answering fails the whole
        // cycle at once, because the whole cycle is one connection.
        let asked: Vec<&Slot> = self
            .slots
            .iter()
            .filter(|slot| {
                slot.attempted.is_some() && !matches!(slot.outcome, Outcome::Unsupported(_))
            })
            .collect();
        if !asked.is_empty()
            && asked
                .iter()
                .all(|slot| matches!(slot.outcome, Outcome::Failed(_)))
        {
            let at = asked
                .iter()
                .map(|slot| self.interval(slot.subject))
                .max()
                .unwrap_or(MAX_INTERVAL);
            return Liveness::BackingOff { at };
        }

        // Otherwise: counting down to whichever subject comes due first.
        let mut soonest: Option<(Duration, Duration)> = None;
        for slot in &self.slots {
            if matches!(slot.outcome, Outcome::Unsupported(_)) || self.settled(slot) {
                continue;
            }
            let interval = self.interval(slot.subject);
            let elapsed = slot
                .attempted
                .map_or(interval, |at| now.saturating_duration_since(at).min(interval));
            let left = interval.saturating_sub(elapsed);
            if soonest.is_none_or(|(best, _)| left < best) {
                soonest = Some((left, interval));
            }
        }
        match soonest {
            Some((left, interval)) => Liveness::Waiting {
                elapsed: interval.saturating_sub(left),
                interval,
            },
            // Every subject is answered and none is due again: the `once` tier
            // alone, which happens only if the fast tier has been made
            // unsupported. Nothing is coming, and saying so beats a clock face
            // that will never fill.
            None => Liveness::Stopped(NOTHING_LEFT),
        }
    }

    /// Whether this slot is one of the `once` tier's, already answered.
    fn settled(&self, slot: &Slot) -> bool {
        slot.subject.tier() == Tier::Once
            && slot.outcome == Outcome::Answered
            && slot.attempted.is_some()
    }

    /// The subjects that are due, **most overdue first**.
    ///
    /// **Every one of them, since 2026-08-29** — they go on one connection, so
    /// there is no longer a reason to pick one. The ordering is kept because it
    /// is what the batch's own order becomes, and because it is still the
    /// answer to *which of these mattered most* if a cycle is cut short.
    ///
    /// **Furthest, not first.** Asking in list order would starve the slow tier
    /// on a link where the fast one is always due.
    fn due_now(&self, now: Instant) -> Vec<Subject> {
        let mut due: Vec<(Duration, Subject)> = Vec::new();
        for slot in &self.slots {
            if matches!(slot.outcome, Outcome::Unsupported(_)) {
                continue;
            }
            // **Asked once, and answered.** Which slot booted and what block
            // devices exist cannot change without a reboot, and a reboot ends
            // the session watching — so re-asking would spend a journal record
            // to learn what is already on screen.
            //
            // **`attempted` is what re-arms it**, not the outcome: `refresh_all`
            // clears that field for every subject, so `r` re-asks these along
            // with everything else and needs no special case of its own. A
            // person who has just armed a trial boot presses it and gets a new
            // answer.
            if self.settled(slot) {
                continue;
            }
            let interval = self.interval(slot.subject);
            let overdue = match slot.attempted {
                // Never asked: maximally overdue, so a freshly opened screen
                // fills in on its first cycle.
                None => Duration::MAX,
                Some(at) => {
                    let since = now.saturating_duration_since(at);
                    if since < interval {
                        continue;
                    }
                    since - interval
                }
            };
            due.push((overdue, slot.subject));
        }
        // Descending by how overdue, and by subject where two are equal — so a
        // freshly opened screen, where every subject is `Duration::MAX` overdue,
        // asks in a stable order rather than in whatever order a sort left them.
        due.sort_by(|(a, left), (b, right)| b.cmp(a).then(left.cmp(right)));
        due.into_iter().map(|(_, subject)| subject).collect()
    }
}

/// **Whether this pane is alive, and how it says so.**
///
/// `ADR-034` §11.5.2, and the ruling is one sentence: *"The indicator is derived
/// from the poller's state, never from a wall clock."*
///
/// # Why that sentence is the whole feature
///
/// A five-second refresh is only honest if the screen says so while it waits;
/// otherwise a still pane and a dead one look identical, which is this pane's
/// founding failure — *a stale number rendered as a live one* — in its other
/// direction. But an indicator that animates off a clock is **worse than none**:
/// it keeps cycling while the transport is dead, and an animation that says
/// *alive* about something that is not is a claim the screen has no basis for.
///
/// So every variant here is a fact about [`Poller`]. There is no `now` in the
/// type: the two variants that carry a duration carry one measured *from
/// something the poller did* — when the cycle was sent, or how long a slot's
/// interval has widened to — and [`Poller::liveness`] is the only thing that
/// constructs them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    /// **Nothing is going to happen until something changes.** Said in words
    /// rather than animated, which the ruling requires of *no transport* and
    /// which is equally true of a poller a person has stopped by hand.
    Stopped(&'static str),
    /// **Counting down to the next due poll**, drawn as a clock face filling.
    Waiting {
        /// How far into the interval this is.
        elapsed: Duration,
        /// The interval it is counting through, which is what makes the
        /// fraction meaningful and is on screen beside it.
        interval: Duration,
    },
    /// **A request is on the wire**, drawn as an indeterminate spinner.
    ///
    /// Never a countdown: the remaining time is unknown, and the honest
    /// statement is *waiting on the board*. `since` is measured from the moment
    /// the cycle was sent — so the spinner turns only while a request is
    /// genuinely out, which is the difference between an indicator and an
    /// animation.
    InFlight { since: Duration },
    /// **Everything asked is failing**, and the retry interval has widened.
    BackingOff {
        /// What the interval is now, which is the number a person wants.
        at: Duration,
    },
}

/// [`Liveness::Stopped`]'s words when a person pressed the key themselves.
pub const PAUSED: &str = "paused — nothing is being asked of this machine";

/// [`Liveness::Stopped`]'s words when there is nothing left that will come due.
pub const NOTHING_LEFT: &str =
    "nothing left to ask — every subject this build answers has been read";

/// [`Liveness::Stopped`]'s words when this run holds no control-plane identity.
///
/// **Composed here rather than in the renderer** so that the pane and
/// `crate::operator`'s refusals cannot drift into two accounts of one state.
pub const NO_TRANSPORT: &str = "no reader — this pane has opened no connection to the machine";

/// **The whole of the CPU arithmetic**, in one place so it can be tested
/// without a poller, a thread or a clock.
///
/// The denominator is `total_millis` and not the wall clock: elapsed *processor*
/// time is what busy time is a share of, and on a machine whose reading was late
/// the two differ. Using the wall clock would report a board that answered
/// slowly as less busy than it was.
///
/// `None` when the counters did not move forward — see [`Poller::take_rates`].
fn rates_between(before: &Cpu, after: &Cpu, over: Duration) -> Option<CpuRates> {
    let (total, iowait) = share(&before.total, &after.total)?;
    // Per-CPU, matched **by name** rather than by position: a machine that
    // hot-plugged a core between two readings would otherwise have every
    // subsequent core compared against its neighbour, which produces numbers
    // rather than an error.
    let cpus = after
        .cpus
        .iter()
        .filter_map(|now| {
            let then = before.cpus.iter().find(|c| c.name == now.name)?;
            let (busy, _) = share(then, now)?;
            Some((now.name.clone(), busy))
        })
        .collect();
    Some(CpuRates {
        total,
        iowait,
        cpus,
        over,
    })
}

/// Busy and iowait as whole percentages of the processor time that elapsed.
///
/// `None` when nothing elapsed, or when a counter went backwards — a machine
/// that rebooted between two readings, which `checked_sub` catches and a
/// saturating subtraction would turn into a confident `0%`.
fn share(before: &CpuTimes, after: &CpuTimes) -> Option<(u64, u64)> {
    let elapsed = after.total_millis.checked_sub(before.total_millis)?;
    if elapsed == 0 {
        return None;
    }
    let busy = after.busy_millis.checked_sub(before.busy_millis)?;
    let waited = after.iowait_millis.checked_sub(before.iowait_millis)?;
    Some((
        (busy.saturating_mul(100) / elapsed).min(100),
        (waited.saturating_mul(100) / elapsed).min(100),
    ))
}

/// Per-process CPU, in **hundredths of a percent of one processor**.
///
/// `top`'s convention, stated because it surprises people: a four-threaded
/// process pinning a four-core board reads 400%, not 100%. Dividing by the core
/// count instead would make a single-threaded runaway on a quad-core machine
/// read 25%, which is not the number anybody is hunting for.
///
/// **A pid absent from the earlier sample gets no entry at all.** It is a process
/// that started since, and a rate needs two samples of the same one; `0%` for it
/// would say it is doing nothing, which is very often the opposite of true for
/// something that has just started.
fn process_rates_between(
    before: &[(i64, u64)],
    after: &[(i64, u64)],
    over: Duration,
) -> Option<ProcessRates> {
    let elapsed = u64::try_from(over.as_millis()).unwrap_or(u64::MAX);
    if elapsed == 0 {
        return None;
    }
    let by_pid = after
        .iter()
        .filter_map(|(pid, now)| {
            let (_, then) = before.iter().find(|(p, _)| p == pid)?;
            // A counter that went backwards is a pid reused by a different
            // process — which happens, and produces a wildly wrong figure rather
            // than an error, so it is dropped rather than saturated.
            let used = now.checked_sub(*then)?;
            Some((*pid, used.saturating_mul(10_000) / elapsed))
        })
        .collect::<Vec<_>>();
    Some(ProcessRates { by_pid, over })
}

/// One subject's state, for [`seeded`]. A named type because the tuple is deep
/// enough that `clippy::type_complexity` is right about it.
#[cfg(test)]
pub type Seed = (Subject, Outcome, Option<(Reading, Instant)>);

/// A poller with readings already in it, and no worker behind it.
///
/// **For the tests of the screen that draws them** — `crate::tui::top` — which
/// are about layout and honesty rather than about scheduling, and must not
/// depend on a thread answering in time. It is `cfg(test)` so that nothing in
/// the running program can construct a reader that never reads.
#[cfg(test)]
#[must_use]
pub fn seeded(states: Vec<Seed>) -> Poller {
    let (jobs, _dead) = channel::<Job>();
    let (_gone, answers) = channel::<Answer>();
    let mut poller = Poller {
        jobs,
        answers,
        in_flight: 0,
        sent_at: None,
        slots: Subject::ALL.iter().copied().map(Slot::new).collect(),
        fast: FAST_DEFAULT,
        skipped: 0,
        paused: false,
        previous_cpu: None,
        previous_processes: None,
        cpu_rates: None,
        process_rates: None,
    };
    for (subject, outcome, reading) in states {
        if let Some(slot) = poller.slot_mut(subject) {
            slot.outcome = outcome;
            slot.reading = reading;
        }
    }
    poller
}

#[cfg(test)]
impl Poller {
    /// **Feed a reading in as though the worker had brought it back**, so that a
    /// test of the screen exercises the real rate arithmetic rather than a
    /// fabricated rate handed to it.
    ///
    /// It exists because [`seeded`] can put a *reading* in a slot and cannot put
    /// a *rate* anywhere — a rate is not a reading, it is what two of them make,
    /// and a setter that installed one directly would let a pane test pass over
    /// arithmetic nothing ran. Calling this twice is how a test gets one.
    pub fn observe(&mut self, subject: Subject, reading: Reading, at: Instant) {
        self.take_rates(&reading, at);
        if let Some(slot) = self.slot_mut(subject) {
            slot.reading = Some((reading, at));
            slot.outcome = Outcome::Answered;
            slot.attempted = Some(at);
            slot.failures = 0;
        }
    }

    /// **Put a cycle on the wire without a worker to answer it.**
    ///
    /// The in-flight state is the one [`seeded`] cannot reach — a seeded poller
    /// has no worker, so `tick` would record the send failing rather than
    /// waiting. This sets exactly what a successful send sets and nothing else,
    /// so a test of [`Liveness`] exercises the real derivation.
    pub fn pretend_sent(&mut self, subjects: &[Subject], at: Instant) {
        self.in_flight = subjects.len();
        self.sent_at = Some(at);
        for subject in subjects {
            if let Some(slot) = self.slot_mut(*subject) {
                slot.attempted = Some(at);
            }
        }
    }

    /// **Record a failed attempt**, as [`Poller::collect`] would.
    ///
    /// `failures` is what widens the interval, so a test of the backing-off
    /// state has to set it the way an answer would rather than assert on a
    /// number it chose.
    pub fn pretend_failed(&mut self, subject: Subject, at: Instant, times: u32, why: &str) {
        if let Some(slot) = self.slot_mut(subject) {
            slot.attempted = Some(at);
            slot.failures = times;
            slot.outcome = Outcome::Failed(why.to_string());
        }
    }
}

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;
