//! Reading this machine's processor counters — `ADR-035` phase 4.
//!
//! **`/proc/stat` and `/proc/loadavg`, and nothing else.** `crate::journal`'s
//! note carries why that belongs here rather than behind `BootMedium`: it needs
//! no `mount(2)`, no device tree and no pid 1, so the numbers are true about
//! whatever Linux is running, Rivet or not.
//!
//! # The unit conversion this module exists to do once
//!
//! The kernel states CPU time in **`USER_HZ` ticks**, which is a property of a C
//! header rather than of anything a reader should have to carry. `crate::memory`
//! makes the same conversion from kibibytes to bytes and states the same reason:
//! it happens here, in the adapter, so nothing downstream remembers a unit.
//!
//! **The tick rate is measured rather than assumed.** It is 100 on every target
//! Rivet builds for and it is still read out of `sysconf(_SC_CLK_TCK)`, because
//! a constant `100` in a parser would be right on the board and silently wrong
//! anywhere else — every figure off by a factor with nothing on screen to say
//! so. `crate::syscall::clock_ticks_per_second` holds the call and the argument.
//!
//! # What this module will not do, and it is the whole point of the subject
//!
//! **Compute a percentage.** `rivet_core::operation::cpu` carries the argument:
//! utilisation is a *rate*, a rate is the difference between two readings over
//! the time between them, and one read of `/proc/stat` is one reading. A
//! function here that returned a percentage would have to invent the other
//! sample or the interval.
//!
//! # `/proc/stat`'s columns, and the two that are deliberately dropped
//!
//! Each `cpu*` line is `user nice system idle iowait irq softirq steal guest
//! guest_nice`. The first eight are read. **`guest` and `guest_nice` are not**,
//! and that is not an oversight: the kernel already counts guest time inside
//! `user` and `nice`, so adding them would double-count it — a machine running a
//! VM would report more than 100% of its own processors. Rivet is not a
//! hypervisor and the columns are zero on it; they are dropped for the machine
//! where they are not.

use rivet_core::operation::machine::{CpuRead, CpuTimes, LoadAverage, MachineError};

const STAT: &str = "/proc/stat";
const LOADAVG: &str = "/proc/loadavg";

/// This machine's processor counters, in milliseconds.
///
/// # Errors
///
/// If `/proc/stat` cannot be read, does not carry an aggregate `cpu` line, or
/// this platform cannot say how long one of the kernel's ticks is. **A missing
/// `/proc/loadavg` is not an error** — see [`CpuRead::load`].
pub fn read() -> Result<CpuRead, MachineError> {
    let text = std::fs::read_to_string(STAT)
        .map_err(|e| MachineError::Refused(format!("{STAT} could not be read: {e}")))?;

    // **Asked before anything is parsed.** Without the rate, every figure below
    // would be a number of ticks wearing the word `millis` — which is the
    // plausible-and-wrong answer this whole subject is built against, so it is a
    // refusal rather than a fallback to 100.
    let hz = ticks_per_second()?;

    let mut cpus = Vec::new();
    let mut total = None;
    for line in text.lines() {
        let Some(name) = line.split_whitespace().next() else {
            continue;
        };
        if !name.starts_with("cpu") {
            // `/proc/stat` carries `intr`, `ctxt`, `btime`, `processes` and more
            // below the processor lines. None of them is this subject's.
            continue;
        }
        let Some(times) = parse(line, hz) else {
            continue;
        };
        if name == "cpu" {
            total = Some(times);
        } else {
            cpus.push(times);
        }
    }

    // **The aggregate is the one that must be there.** Every kernel writes it
    // first, so a `/proc/stat` without it is not a `/proc/stat` — and summing
    // the per-CPU lines to stand in for it would be the second answer to *how
    // busy is this machine* that `ADR-011` §3 refuses.
    let total = total.ok_or_else(|| {
        MachineError::Refused(format!(
            "{STAT} carries no aggregate `cpu` line, so it is not the file this expected. \
             Nothing is computed from the per-processor lines instead: a sum made here would be \
             a second answer to how busy this machine is."
        ))
    })?;

    // Ascending, so two readings of an unchanged machine produce the same bytes.
    // `/proc/stat` already writes them in order; sorting says so rather than
    // depending on it.
    cpus.sort_by(|a, b| a.name.cmp(&b.name));
    let processors = u32::try_from(cpus.len()).unwrap_or(u32::MAX);

    let (load, load_unmeasured) = match std::fs::read_to_string(LOADAVG) {
        Ok(raw) => match load(&raw) {
            Some(load) => (Some(load), None),
            None => (
                None,
                Some(format!(
                    "{LOADAVG} holds {raw:?}, which is not the five figures this expected"
                )),
            ),
        },
        Err(e) => (None, Some(format!("{LOADAVG} could not be read: {e}"))),
    };

    Ok(CpuRead {
        cpus,
        total,
        processors,
        load,
        load_unmeasured,
        source: STAT.to_string(),
    })
}

/// How many kernel ticks make a second, on the platform this is built for.
///
/// **On Linux it is measured**; anywhere else there is no `/proc` to read in the
/// first place, so this refuses in the same words the caller would have used and
/// the development host gets an honest answer rather than a Linux one.
#[cfg(target_os = "linux")]
fn ticks_per_second() -> Result<u64, MachineError> {
    crate::syscall::clock_ticks_per_second().map_err(|errno| {
        MachineError::Refused(format!(
            "this machine could not say how long one of its kernel's clock ticks is \
             (sysconf(_SC_CLK_TCK) failed, errno {errno}), and every CPU figure in {STAT} is \
             stated in those ticks. Nothing is reported rather than times converted against a \
             rate this build guessed at."
        ))
    })
}

/// **There is no `/proc` here, so there is nothing to convert.** The refusal
/// names the file rather than the platform, because that is the true statement:
/// a Windows development host has no `/proc/stat`, and `crate::syscall` is
/// `#[cfg(target_os = "linux")]` so there is no `sysconf` to call either.
#[cfg(not(target_os = "linux"))]
fn ticks_per_second() -> Result<u64, MachineError> {
    Err(MachineError::Refused(format!(
        "{STAT} does not exist on this platform, so no processor counters were read"
    )))
}

/// One `cpu0 12345 0 6789 …` line, with every figure in **milliseconds**.
///
/// `None` when the line carries fewer than the four fields every kernel since
/// 2.6 writes. The later columns are optional and default to zero — `steal` is
/// absent on a kernel built without paravirtualisation, and zero is the right
/// answer there rather than a reason to discard the line: bare metal really has
/// stolen no time.
fn parse(line: &str, hz: u64) -> Option<CpuTimes> {
    let mut fields = line.split_whitespace();
    let name = fields.next()?.to_string();
    let figures: Vec<u64> = fields.map(|f| f.parse().unwrap_or(0)).collect();
    if figures.len() < 4 {
        return None;
    }
    let at = |i: usize| millis(figures.get(i).copied().unwrap_or(0), hz);
    Some(CpuTimes {
        name,
        user_millis: at(0),
        nice_millis: at(1),
        system_millis: at(2),
        idle_millis: at(3),
        iowait_millis: at(4),
        irq_millis: at(5),
        softirq_millis: at(6),
        steal_millis: at(7),
        // Columns 8 and 9 — `guest` and `guest_nice` — are deliberately not
        // read. See this module's header: the kernel already counts them inside
        // `user` and `nice`.
    })
}

/// Ticks to milliseconds.
///
/// **Multiplied before dividing**, so a machine at 100 Hz does not lose the
/// bottom two digits of every figure to integer division. The multiplication is
/// saturating: at 1000 Hz a counter would have to reach 5.8 million years of CPU
/// time to overflow, and saturating there is still better than wrapping into a
/// small number that looks like a reading.
const fn millis(ticks: u64, hz: u64) -> u64 {
    if hz == 0 {
        // Unreachable: `clock_ticks_per_second` refuses a zero rather than
        // returning one. Guarded anyway because this is `pid 1`'s dependency
        // tree and a division by zero here is a panic in a login shell.
        return 0;
    }
    ticks.saturating_mul(1_000) / hz
}

/// `"0.52 0.58 0.59 1/234 5678"` as hundredths and two counts.
///
/// **Hundredths for [`rivet_core::operation::machine::MemoryPressure`]'s
/// reason**: the value model has no floating-point variant, and whole numbers
/// would collapse every load below one into zero — on a four-core board that is
/// the entire interesting range.
fn load(text: &str) -> Option<LoadAverage> {
    let mut fields = text.split_whitespace();
    let avg1 = hundredths(fields.next()?)?;
    let avg5 = hundredths(fields.next()?)?;
    let avg15 = hundredths(fields.next()?)?;
    // `1/234` — runnable over existing. A kernel that stopped writing it would
    // still have given three averages, which are the useful part, so this
    // degrades to zeroes rather than discarding them. They are counts of tasks
    // and not measurements of anything, which is why they may default where the
    // averages may not.
    let (runnable, existing) = fields
        .next()
        .and_then(|f| f.split_once('/'))
        .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
        .unwrap_or((0, 0));
    Some(LoadAverage {
        avg1,
        avg5,
        avg15,
        runnable,
        existing,
    })
}

/// `0.52` as `52`, and `1.5` as `150` rather than as `15`.
fn hundredths(figure: &str) -> Option<u32> {
    let (whole, frac) = figure.split_once('.').unwrap_or((figure, "0"));
    let whole: u32 = whole.parse().ok()?;
    let frac: u32 = format!("{frac:0<2}").get(..2)?.parse().ok()?;
    Some(whole.saturating_mul(100).saturating_add(frac))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// Four cores, **hand-composed** in `/proc/stat`'s own layout (`AGENTS.md`
    /// §4). The figures are invented; the shape is what is under test. See
    /// [`CAPTURED_STAT`] for bytes a kernel actually wrote.
    const STAT_TEXT: &str = "\
cpu  1000 20 300 90000 40 5 6 0 0 0
cpu0 400 10 100 20000 10 2 3 0 0 0
cpu1 200 5 80 23000 10 1 1 0 0 0
cpu2 200 3 60 23500 10 1 1 0 0 0
cpu3 200 2 60 23500 10 1 1 0 0 0
intr 12345 0 0
ctxt 987654
btime 1700000000
processes 4321
";

    /// **CAPTURED, not composed** — the first eight lines of `/proc/stat` read
    /// off Linux 6.6.114.1-microsoft-standard-WSL2 on 2026-08-29, verbatim, with
    /// `getconf CLK_TCK` reporting 100 on the same machine. `AGENTS.md` §4 wants
    /// the difference between a captured vector and an invented one visible in
    /// the file, so it is.
    ///
    /// **It is not a Raspberry Pi**, and nothing here has been read off the
    /// board. The `/proc/stat` format is the kernel's rather than the
    /// architecture's, so a Pi's lines have the same ten columns — but that is a
    /// statement about the format and not a measurement of the board.
    const CAPTURED_STAT: &str = "\
cpu  1290 0 699 147247 439 0 128 0 0 0
cpu0 17 0 67 5247 6 0 68 0 0 0
cpu1 33 0 24 5264 22 0 17 0 0 0
cpu2 23 0 33 5266 19 0 8 0 0 0
cpu3 27 0 17 5274 20 0 12 0 0 0
cpu4 128 0 63 5129 23 0 4 0 0 0
cpu5 50 0 16 5259 17 0 7 0 0 0
cpu6 186 0 25 5123 11 0 4 0 0 0
";

    /// **CAPTURED** — `/proc/loadavg` off the same host at the same moment.
    const CAPTURED_LOADAVG: &str = "0.22 0.07 0.02 1/484 269";

    /// Ticks in, milliseconds out — the conversion this module exists to do
    /// once, at the 100 Hz every Rivet target uses.
    #[test]
    fn a_stat_line_is_read_in_milliseconds_rather_than_ticks() {
        let times = parse("cpu0 400 10 100 20000 10 2 3 0 0 0", 100).expect("a processor");
        assert_eq!(times.name, "cpu0");
        assert_eq!(times.user_millis, 4_000, "400 ticks at 100 Hz is 4 s");
        assert_eq!(times.idle_millis, 200_000);
        assert_eq!(times.iowait_millis, 100);
        assert_eq!(times.steal_millis, 0);
    }

    /// **The rate is read, not assumed**, so a kernel at 1000 Hz produces
    /// different milliseconds from the same ticks — which is the whole reason
    /// `sysconf` is called rather than `100` written down.
    #[test]
    fn the_same_ticks_at_a_different_rate_are_different_milliseconds() {
        let slow = parse("cpu0 400 0 0 0", 100).expect("a processor");
        let fast = parse("cpu0 400 0 0 0", 1000).expect("a processor");
        assert_eq!(slow.user_millis, 4_000);
        assert_eq!(fast.user_millis, 400);
    }

    /// **The multiplication happens before the division**, so the fraction of a
    /// tick is not lost to integer arithmetic.
    #[test]
    fn a_conversion_does_not_lose_the_bottom_of_every_figure() {
        assert_eq!(millis(1, 100), 10, "one tick at 100 Hz is 10 ms, not 0");
        assert_eq!(millis(7, 1000), 7);
        assert_eq!(millis(0, 100), 0);
    }

    /// A kernel without the later columns still reads: `steal` absent on bare
    /// metal is a true zero rather than a reason to drop the processor.
    #[test]
    fn a_short_line_still_parses_and_the_missing_columns_are_truly_zero() {
        let times = parse("cpu1 100 0 50 900", 100).expect("a processor");
        assert_eq!(times.user_millis, 1_000);
        assert_eq!(times.idle_millis, 9_000);
        assert_eq!(times.steal_millis, 0);
        assert_eq!(times.softirq_millis, 0);
        // Three figures is not a processor line.
        assert!(parse("cpu1 100 0 50", 100).is_none());
    }

    /// **`busy` excludes `iowait` as well as `idle`.** A machine blocked on a
    /// dying SD card is not working, and counting it as busy is how that gets
    /// missed — `ADR-018` §7's concern, arriving as arithmetic.
    #[test]
    fn busy_time_counts_neither_idle_nor_waiting() {
        let times = parse("cpu0 400 10 100 20000 10 2 3 0", 100).expect("a processor");
        assert_eq!(
            times.busy_millis(),
            4_000 + 100 + 1_000 + 20 + 30,
            "user + nice + system + irq + softirq, and nothing else"
        );
        assert_eq!(
            times.total_millis(),
            times.busy_millis() + times.idle_millis + times.iowait_millis
        );
    }

    /// Hundredths, including the case whole numbers would erase entirely.
    #[test]
    fn a_load_average_becomes_hundredths_of_a_task() {
        let load = load("0.52 1.58 12.00 3/234 5678").expect("a load average");
        assert_eq!(load.avg1, 52, "0.52 is 52, and whole numbers would make it 0");
        assert_eq!(load.avg5, 158);
        assert_eq!(load.avg15, 1200);
        assert_eq!(load.runnable, 3);
        assert_eq!(load.existing, 234);
    }

    /// A single-digit fraction is tenths: `0.4` is 40, not 4 — `crate::memory`'s
    /// test, one file over, for the same arithmetic.
    #[test]
    fn a_one_digit_fraction_is_not_read_as_hundredths() {
        assert_eq!(load("0.4 0 0 1/1 2").expect("a load").avg1, 40);
        assert_eq!(load("3 0 0 1/1 2").expect("a load").avg1, 300);
    }

    /// A file that is not a load average yields nothing, so `read` reports it
    /// **unmeasured** rather than as an idle machine.
    #[test]
    fn a_file_that_is_not_a_load_average_is_not_read_as_zero() {
        for not_a_load in ["", "\n", "nonsense", "0.52 0.58"] {
            assert!(load(not_a_load).is_none(), "{not_a_load:?} parsed");
        }
    }

    /// The whole file: the aggregate is the kernel's own line and the four cores
    /// are separate, which is what makes one pinned core visible.
    #[test]
    fn every_processor_line_is_read_and_the_aggregate_is_the_kernels_own() {
        let mut cpus = Vec::new();
        let mut total = None;
        for line in STAT_TEXT.lines() {
            let Some(name) = line.split_whitespace().next() else {
                continue;
            };
            if !name.starts_with("cpu") {
                continue;
            }
            let times = parse(line, 100).expect("a processor line");
            if name == "cpu" {
                total = Some(times);
            } else {
                cpus.push(times);
            }
        }
        assert_eq!(cpus.len(), 4, "four cores, and the aggregate is not one of them");
        let total = total.expect("the aggregate line");
        assert_eq!(total.name, "cpu");
        assert_eq!(total.user_millis, 10_000);
        // The lines below the processors — `intr`, `ctxt`, `btime` — are not
        // processors and must not have been read as any.
        for cpu in &cpus {
            assert!(cpu.name.starts_with("cpu"), "{}", cpu.name);
        }
    }

    /// **The same two parsers against bytes a kernel actually wrote.** The
    /// expected figures were read off the captured text by eye and by `cat` on
    /// the host itself, which is what makes this a check rather than a
    /// restatement of the parser.
    #[test]
    fn the_captured_files_parse_to_what_the_kernel_wrote() {
        let mut cpus = Vec::new();
        let mut total = None;
        for line in CAPTURED_STAT.lines() {
            let times = parse(line, 100).expect("every captured line is a processor line");
            if times.name == "cpu" {
                total = Some(times);
            } else {
                cpus.push(times);
            }
        }
        assert_eq!(cpus.len(), 7, "seven per-CPU lines were captured");
        let total = total.expect("the aggregate");
        // `cpu  1290 0 699 147247 439 0 128` at 100 Hz.
        assert_eq!(total.user_millis, 12_900);
        assert_eq!(total.system_millis, 6_990);
        assert_eq!(total.idle_millis, 1_472_470);
        assert_eq!(total.iowait_millis, 4_390);
        assert_eq!(total.softirq_millis, 1_280);
        assert_eq!(
            total.busy_millis(),
            12_900 + 6_990 + 1_280,
            "user + system + softirq; the machine was almost entirely idle"
        );

        let load = load(CAPTURED_LOADAVG).expect("a load average");
        assert_eq!(load.avg1, 22, "0.22 — and whole numbers would make it 0");
        assert_eq!(load.avg5, 7);
        assert_eq!(load.avg15, 2);
        assert_eq!(load.runnable, 1);
        assert_eq!(load.existing, 484);
    }

    /// **The read either answers or refuses by name, and it never reports a
    /// machine that has run nothing.** This runs on whatever host the suite is
    /// on — Linux answers, Windows refuses — and asserts the property that holds
    /// on both.
    #[test]
    fn a_reading_on_this_host_is_either_an_answer_or_a_named_refusal() {
        match read() {
            Ok(reading) => {
                assert_eq!(reading.source, STAT);
                assert!(
                    reading.total.total_millis() > 0,
                    "a machine that has booted has spent some time somewhere"
                );
                assert_eq!(
                    reading.load.is_none(),
                    reading.load_unmeasured.is_some(),
                    "exactly one of the load average and its absence-reason is present"
                );
                assert_eq!(
                    usize::try_from(reading.processors).unwrap_or(0),
                    reading.cpus.len()
                );
            }
            Err(refusal) => assert!(
                refusal.why().contains(STAT) || refusal.why().contains("clock tick"),
                "{}",
                refusal.why()
            ),
        }
    }
}
