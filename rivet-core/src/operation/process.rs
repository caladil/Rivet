//! `rivet get process` — **what is actually executing**, from the kernel.
//!
//! `ADR-035` phase 4, and the second of the five the maintainer named.
//!
//! # It is not `get status`, and the difference is the point
//!
//! | | |
//! | :-- | :-- |
//! | `get status` | what the **supervisor believes** it is running, from its own memory (`ADR-032` §3.2's query socket) |
//! | `get process` | what the **kernel says** is executing, from `/proc` |
//!
//! `ADR-011` §3 is about two things that both look authoritative, and usually
//! the answer is to delete one. **Here both are correct and both are needed**,
//! because the interesting cases are exactly where they disagree:
//!
//! * a service the supervisor calls `running` with no process behind it;
//! * a **userland process that is not one of the supervisor's services at all**,
//!   which on a machine with no shell should be impossible;
//! * a zombie, which means something is not reaping its children.
//!
//! None of those is visible from either side alone. This is the second half.
//!
//! # What to look at first
//!
//! `state` — `D` and `Z`. Uninterruptible sleep is a process stuck in the
//! kernel, and on a Pi that is very often I/O against a failing SD card
//! (`ADR-018` §7's whole concern); it cannot be killed and it will not appear
//! as a service failure. A zombie means its parent is not reaping.
//!
//! `kernel` — because the **userland** list on a Rivet should be short and
//! wholly accounted for by the document. That is a property nobody can check
//! without separating kernel threads out, and it is the cheapest anomaly check
//! this surface offers.

use super::machine::Observed;
use super::{ErrorCode, Field, OperationError};
use crate::value::Value;

/// The word. Part of the interface contract; never renamed.
pub const SUBJECT: &str = "process";

/// What this subject's result carries. `ADR-006` §7.
pub const OUTPUTS: &[Field] = &[
    Field {
        name: "subject",
        description: "What was read. Always `process` in this build.",
    },
    Field {
        name: "processes",
        description: "Every process, IN ASCENDING PID ORDER so that two readings of an \
                      unchanged machine produce the same bytes. Each carries `pid`, \
                      `parent_pid`, `name`, `state`, `threads`, `kernel`, and \
                      `resident_bytes` for everything that is not a kernel thread. THIS IS \
                      THE KERNEL'S ANSWER, not the supervisor's: `get status` says what the \
                      supervisor believes it is running, and the cases worth finding are \
                      where the two disagree. Since 2026-08-29 each row also carries \
                      `user_millis` and `system_millis` — CPU TIME CONSUMED SINCE THE PROCESS \
                      STARTED, in milliseconds, and CUMULATIVE. They are counters and not \
                      percentages: a per-process utilisation is the difference between two \
                      readings of them divided by the time between, and one reading gives \
                      none. Without them there was no per-process CPU figure at all, however \
                      often this subject was called, because everything else in a row is a \
                      STATE and two readings of a state are two states. Absent — never `0` — \
                      where they could not be read, which they can be on their own: they come \
                      from a different file in /proc than the rest of the row, and 0 ms of CPU \
                      is a real reading of a process that has done nothing. SINCE 2026-09-16 \
                      each row also carries `wchan` and `syscall` — THE KERNEL FUNCTION THE \
                      PROCESS IS PARKED IN and THE CALL IT IS STOPPED IN. These answer the \
                      question the counters cannot: frozen counters say a process is doing \
                      nothing, and these say WHAT IT IS WAITING FOR. A supervisor blocked \
                      writing to a socket nobody reads, and one blocked reading a file, are \
                      both `state` `S` with both counters frozen, and they are ONE WORD APART \
                      here. `syscall` is the sharper: the first number is the syscall the \
                      program made, which on aarch64 is 73 for ppoll and 64 for write. \
                      `running` in `syscall` means the task is on a CPU rather than parked, \
                      and IS an answer. Absent for a kernel thread and wherever the kernel \
                      declined, never a placeholder.",
    },
    Field {
        name: "vanished",
        description: "How many processes were listed and then gone before they could be read. \
                      NOT AN ERROR: /proc changes while it is being read and a short-lived \
                      process exiting mid-read is the system working. Reported rather than \
                      dropped, because a count that silently shrinks is one nobody can reason \
                      about. A large number here on an idle machine would itself be a finding.",
    },
    Field {
        name: "userland",
        description: "How many of them are NOT kernel threads. On a Rivet this should be a \
                      short list wholly accounted for by the document's services plus the \
                      process answering this question — there is no shell to start anything \
                      else. It is computed here so that the check does not depend on every \
                      caller knowing to make it.",
    },
];

/// Perform it.
///
/// # Errors
///
/// If a second word was given, or `/proc` could not be listed.
pub(super) fn perform(
    arguments: &[&str],
    observed: &mut dyn Observed,
) -> Result<Value, OperationError> {
    if let Some(extra) = arguments.get(1) {
        return Err(OperationError::about(
            ErrorCode::Arguments,
            SUBJECT,
            format!(
                "`{SUBJECT}` takes no second word and {extra:?} is one. The whole table is the \
                 answer; select from it with a JSON tool rather than with an argument this \
                 build does not have. There is no per-pid form, and if one is wanted it is a \
                 new input rather than a second word."
            ),
        ));
    }

    let read = observed.read_processes().map_err(|e| {
        OperationError::about(ErrorCode::MachineRefused, SUBJECT, e.why().to_string())
    })?;

    let userland = read.processes.iter().filter(|p| !p.kernel).count();
    let rows: Vec<Value> = read
        .processes
        .iter()
        .map(|p| {
            let mut fields = vec![
                ("pid", Value::Integer(i64::from(p.pid))),
                ("parent_pid", Value::Integer(i64::from(p.parent_pid))),
                ("name", Value::str(p.name.clone())),
                ("state", Value::str(p.state.clone())),
                ("threads", Value::Integer(i64::from(p.threads))),
                ("kernel", Value::Bool(p.kernel)),
            ];
            // **Present or absent, never zero.** A kernel thread has no user
            // address space; reporting `0` would say it occupies none, which is
            // a different claim and a false one.
            if let Some(bytes) = p.resident_bytes {
                fields.push((
                    "resident_bytes",
                    Value::Integer(i64::try_from(bytes).unwrap_or(i64::MAX)),
                ));
            }
            // **Present or absent, never zero**, and for a sharper reason than
            // the resident size above: `0` is a *real* reading here — a process
            // that has been scheduled and has done nothing has consumed zero
            // milliseconds — so a zero standing in for *not read* would be
            // indistinguishable from a measurement. `crate::operation::cpu`
            // carries the rest of the argument: these are counters, and the
            // percentage a caller wants is the difference between two readings.
            let millis = |n: u64| Value::Integer(i64::try_from(n).unwrap_or(i64::MAX));
            if let Some(user) = p.user_millis {
                fields.push(("user_millis", millis(user)));
            }
            if let Some(system) = p.system_millis {
                fields.push(("system_millis", millis(system)));
            }
            // **Where it is parked and what it is stopped in** — present or
            // absent, never a placeholder, for the same rule as everything
            // above. These are what separate a process that is *doing nothing*
            // from one that is *waiting for something*, and the counters alone
            // cannot: on 2026-09-16 a supervisor stopped with both counters
            // frozen and a `state` of `S`, and the two hypotheses that survived
            // the audit are one `wchan` apart.
            if let Some(wchan) = &p.wchan {
                fields.push(("wchan", Value::str(wchan.clone())));
            }
            if let Some(syscall) = &p.syscall {
                fields.push(("syscall", Value::str(syscall.clone())));
            }
            Value::object(fields)
        })
        .collect();

    Ok(Value::object(vec![
        ("subject", Value::str(SUBJECT)),
        ("processes", Value::List(rows)),
        ("vanished", Value::Integer(i64::from(read.vanished))),
        ("userland", Value::Integer(i64::try_from(userland).unwrap_or(i64::MAX))),
    ]))
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
