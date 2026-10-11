//! `rivet get cpu` — **what this machine's processors have spent their time on,
//! and why that is not a percentage.**
//!
//! `ADR-035` phase 4. It is the last of the facts the maintainer named on
//! 2026-08-29 and it is the one that was named **first**: he asked for a
//! per-device screen like `top` that shows the machine's current status —
//! processor, memory, disk, thermals and running processes (`.rulings/` R-328).
//!
//! Memory, disk, thermals and processes all landed the same day and CPU did not,
//! because nothing on this machine answered any part of it. `cmd/rivet-fleet`'s
//! monitor pane shipped with a section whose whole content was a sentence saying
//! so — which was the right thing to ship and is not a thing to keep.
//!
//! # A single reading cannot report utilisation, and this subject will not
//!
//! **This is the entire shape of the answer.** `/proc/stat` holds counters:
//! milliseconds of CPU time spent in each state *since boot*, monotonically
//! increasing. `top` shows a percentage because `top` reads the file twice and
//! subtracts. A subject that answers one request has one reading, and:
//!
//! | What a caller has | What it can say |
//! | :-- | :-- |
//! | one reading | how the machine has spent its **whole life** — and nothing about now |
//! | two readings, and the interval | the utilisation over that interval |
//!
//! So the counters travel as counters and **no field here is a percentage.**
//! Inventing one would need a second reading this build does not have or an
//! interval it did not measure, and a plausible number nobody took is
//! `AGENTS.md` §5's guess that succeeds. `get memory` refuses the same way for
//! PSI and [`super::thermal`] for the firmware's throttle flags; this is the
//! third instance of one rule.
//!
//! **What a caller must not do with a first reading is render `0%`.** Zero
//! percent is a measurement of an idle machine, and it is the single most
//! misleading thing a monitoring surface can put on a screen.
//!
//! # The load average is the one number a single read *does* answer
//!
//! Because the kernel keeps it across time rather than the caller doing so. It
//! is here for exactly that reason, and it carries a caveat that has to travel
//! with it: **on Linux the load average is not a CPU utilisation.** It counts
//! tasks in uninterruptible sleep as well as runnable ones — so on a Pi with a
//! failing SD card the load climbs to 4 while the processors sit idle.
//! [`super::process`]'s `D` state is the same fact one process at a time, and
//! the pair of them is what identifies it. Every other Unix counts this
//! differently, which is why the caveat is in the answer rather than in a manual
//! nobody is holding at 03:00.
//!
//! # Per-CPU, not only the total
//!
//! The failure worth finding on a four-core board is **one core pinned while
//! three idle** — a spin loop, a driver stuck on an interrupt. An aggregate
//! divides that by four and it stops looking like anything. The kernel's own
//! aggregate line is reported too, and it is the kernel's rather than a sum
//! computed here: two answers to *how busy is this machine* is `ADR-011` §3, and
//! the one that can be wrong is the one somebody computed.

use super::machine::Observed;
use super::{ErrorCode, Field, OperationError};
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "cpu";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `cpu` in this build.",
    },
    Field {
        name: "processors",
        description: "How many processors the machine stated, as a count rather than as the \
                      length of a list a caller has to measure.",
    },
    Field {
        name: "total",
        description: "The KERNEL'S OWN aggregate across every processor — not a sum computed \
                      here, because two answers to `how busy is this machine` is one too many \
                      (ADR-011 §3). It carries `name` and the same eight counters each entry of \
                      `cpus` does, plus `total_millis` and `busy_millis`, which are the two sums \
                      a utilisation is computed from and are made once here rather than by every \
                      caller.",
    },
    Field {
        name: "cpus",
        description: "Every processor's counters, in ascending order so that two readings of an \
                      unchanged machine produce the same bytes. PER-CPU BECAUSE THE FAILURE \
                      WORTH FINDING IS ONE CORE PINNED WHILE THREE IDLE, which an aggregate \
                      divides by four until it stops looking like anything. Each carries `name` \
                      as the kernel writes it, then `user_millis`, `nice_millis`, \
                      `system_millis`, `idle_millis`, `iowait_millis`, `irq_millis`, \
                      `softirq_millis` and `steal_millis`, plus `total_millis` and `busy_millis`. \
                      `busy_millis` EXCLUDES iowait as well as idle: a machine blocked on a dying \
                      SD card is not working, and counting it as busy is how that gets missed.",
    },
    Field {
        name: "load",
        description: "The kernel's load average — `avg1`, `avg5` and `avg15` in HUNDREDTHS \
                      (`0.52` arrives as `52`, this surface having no floating-point value), plus \
                      `runnable` and `existing`. IT IS NOT A CPU UTILISATION, and on Linux \
                      specifically: this number counts tasks in UNINTERRUPTIBLE SLEEP as well as \
                      runnable ones, which every other Unix does not. On a Pi with a failing SD \
                      card the load climbs to 4 while the processors sit idle — `get process`'s \
                      `D` state is that same fact one process at a time. It is here because it is \
                      the ONE figure a single reading can honestly report: the kernel maintains \
                      it across time, so reading it once is a measurement rather than a first \
                      sample.",
    },
    Field {
        name: "load_unmeasured",
        description: "Present exactly when `load` is absent, and says why. NEVER THREE ZEROES: a \
                      load average of zero is a real reading of an idle machine and it is the one \
                      a reader is likeliest to act on without checking.",
    },
    Field {
        name: "read_from",
        description: "Where the counters were read, so the answer names a place.",
    },
    Field {
        name: "means",
        description: "What this reading does and does not assert, in prose — and the sentence it \
                      always contains is that NO UTILISATION IS REPORTED HERE, because \
                      utilisation is a rate and a rate needs two readings. A caller wanting a \
                      percentage takes this subject twice and subtracts; a caller that has taken \
                      it once must say it does not know rather than render 0%.",
    },
];

/// Perform it.
///
/// # Errors
///
/// If a second word was given, or the kernel's own counters could not be read.
pub(super) fn perform(
    arguments: &[&str],
    observed: &mut dyn Observed,
) -> Result<Value, OperationError> {
    if let Some(extra) = arguments.get(1) {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{SUBJECT}` takes no second word and {extra:?} is one. The whole reading is the \
                 answer; select one processor from it with a JSON tool rather than with an \
                 argument this build does not have. And if {extra:?} was an interval, there is no \
                 such input: a utilisation is a rate, and this subject answers the counters a \
                 caller subtracts two readings of."
            ),
        ));
    }

    let read = observed.read_cpu().map_err(|e| {
        OperationError::about(ErrorCode::MachineRefused, SUBJECT, e.why().to_string())
    })?;

    let mut fields = vec![
        ("subject", Value::str(SUBJECT)),
        (
            "processors",
            Value::Integer(i64::from(read.processors)),
        ),
        ("total", times(&read.total)),
        (
            "cpus",
            Value::List(read.cpus.iter().map(times).collect()),
        ),
    ];

    // **Present or absent, never three zeroes.** A load average of zero is a
    // real reading of an idle machine — `super::memory`'s rule for PSI, and
    // this is the figure a reader is likeliest to act on without checking.
    if let Some(load) = read.load {
        fields.push((
            "load",
            Value::object(vec![
                ("avg1", Value::Integer(i64::from(load.avg1))),
                ("avg5", Value::Integer(i64::from(load.avg5))),
                ("avg15", Value::Integer(i64::from(load.avg15))),
                ("runnable", Value::Integer(i64::from(load.runnable))),
                ("existing", Value::Integer(i64::from(load.existing))),
            ]),
        ));
    } else if let Some(why) = &read.load_unmeasured {
        fields.push(("load_unmeasured", Value::str(why.clone())));
    }

    fields.push(("read_from", Value::str(read.source.clone())));
    fields.push(("means", Value::str(means(&read))));

    debug_assert!(
        super::get::in_declared_order(&fields, OUTPUTS.iter().map(|f| f.name)),
        "a get cpu result's keys must be a subsequence of OUTPUTS, in order"
    );
    Ok(Value::object(fields))
}

/// One processor's counters.
///
/// **`total_millis` and `busy_millis` are computed here and not by the caller**,
/// for [`super::process`]'s `userland` reason: a sum every caller makes for
/// itself is a sum they eventually make differently, and the difference between
/// including `iowait` in *busy* and excluding it is the difference between
/// reporting a dying SD card as a fully loaded machine and reporting it as an
/// idle one waiting.
fn times(t: &super::machine::CpuTimes) -> Value {
    let millis = |n: u64| Value::Integer(i64::try_from(n).unwrap_or(i64::MAX));
    Value::object(vec![
        ("name", Value::str(t.name.clone())),
        ("user_millis", millis(t.user_millis)),
        ("nice_millis", millis(t.nice_millis)),
        ("system_millis", millis(t.system_millis)),
        ("idle_millis", millis(t.idle_millis)),
        ("iowait_millis", millis(t.iowait_millis)),
        ("irq_millis", millis(t.irq_millis)),
        ("softirq_millis", millis(t.softirq_millis)),
        ("steal_millis", millis(t.steal_millis)),
        ("total_millis", millis(t.total_millis())),
        ("busy_millis", millis(t.busy_millis())),
    ])
}

/// **What this reading asserts, and the one thing it never does.**
///
/// Composed from what was measured, for [`super::thermal::means`]'s reason: the
/// sentence that matters changes with the answer, and a reader handed the wrong
/// caveat is worse off than one handed none. The first paragraph is the same
/// every time, because the thing it says is true of every possible reading of
/// this subject.
fn means(read: &super::machine::CpuRead) -> String {
    let mut said = String::from(
        "NO UTILISATION IS REPORTED HERE, and none can be: every figure in `total` and `cpus` is \
         a COUNTER of milliseconds since boot, and a percentage is a difference between two of \
         them divided by the time between. Take this subject twice and subtract. A caller that \
         has taken it once knows how this machine has spent its whole life and NOTHING about what \
         it is doing now — and must say so rather than rendering 0%, which is a measurement of an \
         idle machine. ",
    );

    // The one thing a single reading *can* say about how the machine has spent
    // itself, and it is a lifetime figure rather than a current one — which is
    // said in the same breath so that it cannot be read as a utilisation.
    if let Some(percent) = read
        .total
        .busy_millis()
        .saturating_mul(100)
        .checked_div(read.total.total_millis())
    {
        said.push_str(&format!(
            "Across this machine's whole uptime {percent}% of its processor time has been spent \
             doing work — a LIFETIME AVERAGE, not a current load, and on a machine that has been \
             up for a week it would barely move if every core were pinned right now. "
        ));
    }

    match read.load {
        Some(load) if load.avg1 > 0 => said.push_str(&format!(
            "The load average was read. IT IS NOT A UTILISATION: on Linux it counts tasks in \
             uninterruptible sleep as well as runnable ones, so a load of {}.{:02} on a machine \
             whose processors are idle means something is stuck waiting — on a Pi, very often the \
             SD card. Cross-check it against `get process` for a `D` state before reading it as \
             busy.",
            load.avg1 / 100,
            load.avg1 % 100
        )),
        Some(_) => said.push_str(
            "The load average was read and is below 0.01 over the last minute, which on this \
             hardware means nothing was runnable and nothing was stuck waiting.",
        ),
        None => said.push_str(
            "THE LOAD AVERAGE WAS NOT READ — see `load_unmeasured`. It is not reported as zero, \
             because zero is a real reading of an idle machine, and it is the one figure here a \
             single request could otherwise have answered honestly.",
        ),
    }

    said
}

#[cfg(test)]
#[path = "cpu_tests.rs"]
mod tests;
