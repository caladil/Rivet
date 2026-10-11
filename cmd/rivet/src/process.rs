//! Reading this machine's process table — `ADR-035` phase 4.
//!
//! ~~**`/proc/<pid>/status`, one file per process, and nothing else.**~~ **Two
//! files per process since 2026-08-29**, and the sentence is struck rather than
//! edited because the *one file* was an argued property and losing it cost
//! something (`AGENTS.md` §2, §7). `crate::journal`'s note carries why either
//! belongs here rather than behind `BootMedium`, and it is true of any Linux
//! this runs on rather than of Rivet in particular.
//!
//! # `/proc` changes while you read it, and that is not an error
//!
//! Between listing the directory and opening a `status` file, a short-lived
//! process can exit. Every reader of `/proc` has to decide what that means, and
//! the wrong answers are to fail the whole read or to silently shrink the list.
//! This counts them — see [`ProcessRead::vanished`].
//!
//! # The second file, and what it cost
//!
//! `status` carries everything about a process **except its CPU time**, which
//! lives in `stat` — fields 14 and 15, in `USER_HZ` ticks. Without them there is
//! no per-process CPU figure at all, however many times this table is read:
//! `state`, `threads` and `resident_bytes` are all *states*, and two readings of
//! a state are two states. A counter is what a rate is computed from, and `stat`
//! is where the only counter is.
//!
//! **The property that was given up** is the one the header used to claim: with
//! one file, a process that exited mid-read was either wholly read or wholly
//! missed, never half of one process and half of the state it was in a moment
//! later. With two, the second read can fail on its own — so the CPU times are
//! [`Option`] and go absent together, and the rest of the row is unaffected.
//! That is strictly better than the alternatives of dropping the process
//! (a shrinking count nobody can reason about) or reporting `0` (a real reading
//! of a process that has done nothing, and therefore a lie that cannot be told
//! apart from a measurement).
//!
//! **The two files are not read atomically and nothing here pretends they
//! are.** A process's `status` and its `stat` are microseconds apart; the CPU
//! counters are cumulative and monotonic, so the worst that skew produces is a
//! figure a few microseconds stale against a resident size — which is far
//! inside the interval any caller would compute a rate over.
//!
//! # `stat`'s field numbering, and the parenthesis that breaks it
//!
//! `/proc/<pid>/stat` is space-separated **except** that field 2 is the
//! executable name wrapped in parentheses, and a program is free to have a space
//! or a `)` in its name. Splitting the whole line on whitespace therefore
//! miscounts every field after it — which is the classic `/proc` parsing defect
//! and produces plausible numbers rather than an error. So the parser here finds
//! the **last** `)` in the line and counts from there, which is what the kernel's
//! own documentation says to do.

use rivet_core::operation::machine::{MachineError, ProcessInfo, ProcessRead};

const PROC: &str = "/proc";

/// Every process, in ascending pid order.
///
/// # Errors
///
/// If `/proc` itself cannot be listed. A process that vanishes mid-read is
/// counted, not an error.
pub fn read() -> Result<ProcessRead, MachineError> {
    let entries = std::fs::read_dir(PROC)
        .map_err(|e| MachineError::Refused(format!("{PROC} could not be listed: {e}")))?;

    // **Read once for the whole table rather than once per process**, and its
    // failure is not fatal: a machine that cannot say how long a tick is still
    // has a process table, and the CPU times simply go absent — which is
    // `ProcessInfo::user_millis`'s documented case and not a special one.
    let hz = ticks_per_second();

    let mut processes = Vec::new();
    let mut vanished = 0u32;
    for entry in entries {
        // **A directory entry that errors is COUNTED, not dropped.** `.flatten()`
        // stood here and was silently discarding them, which contradicted this
        // module's own claim that a shrinking count is one nobody can reason
        // about — found by running it and finding numbers that did not add up.
        let Ok(entry) = entry else {
            vanished = vanished.saturating_add(1);
            continue;
        };
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|n| n.parse::<u32>().ok()) else {
            continue; // `/proc` is full of things that are not processes.
        };
        match std::fs::read_to_string(format!("{PROC}/{pid}/status")) {
            Ok(text) => {
                if let Some(mut info) = parse(pid, &text) {
                    // **The second file, and its failure is this row's CPU time
                    // going absent rather than this row going missing.** A
                    // process that exited between the two reads is still a
                    // process that was running when the first one succeeded.
                    if let Some(hz) = hz {
                        let stat = std::fs::read_to_string(format!("{PROC}/{pid}/stat")).ok();
                        if let Some((user, system)) =
                            stat.as_deref().and_then(cpu_ticks)
                        {
                            info.user_millis = Some(millis(user, hz));
                            info.system_millis = Some(millis(system, hz));
                        }
                    }
                    // **The third and fourth files, and the same rule again**:
                    // a row whose `wchan` could not be read is a row with no
                    // `wchan`, never a row that is missing. Both are tiny reads
                    // of already-open directories, and neither can fail in a
                    // way that costs the table a process.
                    info.wchan = waiting_in(pid);
                    info.syscall = stopped_in(pid);
                    processes.push(info);
                } else {
                    vanished = vanished.saturating_add(1);
                }
            }
            // **Gone between the listing and the open.** Ordinary.
            Err(_) => vanished = vanished.saturating_add(1),
        }
    }
    processes.sort_by_key(|p| p.pid);
    Ok(ProcessRead { processes, vanished })
}

/// One `/proc/<pid>/status` into a [`ProcessInfo`].
///
/// `None` when the text carries no `Name`, which is what a file being replaced
/// mid-read looks like.
fn parse(pid: u32, text: &str) -> Option<ProcessInfo> {
    let field = |key: &str| -> Option<&str> {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(':'))
            .map(str::trim)
    };
    let name = field("Name")?.to_string();
    // `State:` is `S (sleeping)` — the letter is the fact, the word is a gloss.
    let state = field("State")
        .and_then(|s| s.split_whitespace().next())
        .unwrap_or("?")
        .to_string();
    // **`VmRSS` absent means kernel thread**, which is read from the absence
    // rather than from the brackets around a name: a userland program is free
    // to call itself `[whatever]` and this must not believe it.
    let resident_bytes = field("VmRSS")
        .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
        .map(|kib| kib.saturating_mul(1024));
    Some(ProcessInfo {
        pid,
        parent_pid: field("PPid").and_then(|v| v.parse().ok()).unwrap_or(0),
        name,
        state,
        threads: field("Threads").and_then(|v| v.parse().ok()).unwrap_or(1),
        kernel: resident_bytes.is_none(),
        resident_bytes,
        // **Filled in by the caller, from the OTHER file.** `status` does not
        // carry CPU time at all; `read` reads `/proc/<pid>/stat` for it and
        // leaves these `None` when it could not. See this module's header for
        // why that is the shape rather than a failure.
        user_millis: None,
        system_millis: None,
        // **Filled in by the caller too**, from two more files, for the reason
        // directly above.
        wchan: None,
        syscall: None,
    })
}

/// `/proc/<pid>/wchan` — the kernel function this process is parked in.
///
/// **The kernel writes the single character `0` for a task it will not answer
/// about**, and that is reported as `None`: a symbol named zero handed to a
/// reader is worse than an absence, because it looks like an answer. An empty
/// file means the same and is treated the same.
fn waiting_in(pid: u32) -> Option<String> {
    read_wchan(&std::fs::read_to_string(format!("{PROC}/{pid}/wchan")).ok()?)
}

/// [`waiting_in`]'s judgement, separated from its file read so it can be tested
/// on a machine that is not the one being asked about.
fn read_wchan(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text == "0" {
        return None;
    }
    Some(text.to_string())
}

/// `/proc/<pid>/syscall` — the call this process is stopped in.
///
/// **`running` is kept, and it is an answer**: the kernel writes it for a task
/// that is on a CPU rather than parked, and *this process is not waiting for
/// anything* is exactly what a reader chasing a wedged program needs to know.
///
/// A kernel thread made no syscall and the file is empty; that is `None`.
fn stopped_in(pid: u32) -> Option<String> {
    read_syscall(&std::fs::read_to_string(format!("{PROC}/{pid}/syscall")).ok()?)
}

/// [`stopped_in`]'s judgement, separated for [`read_wchan`]'s reason.
///
/// **An all-zero line is not a syscall and must not be reported as one.** A
/// kernel thread makes none, and this kernel says so by writing
/// `0 0x0 0x0 0x0 0x0 0x0 0x0 0x0 0x0` rather than by writing nothing —
/// measured on `rivet-01`, where 138 of 142 rows were that line. Passing it on
/// would be worse than noise: **syscall 0 on `aarch64` is `io_setup`**, so a
/// reader scanning this field would find a hundred processes apparently stopped
/// in one, and it would cost seven kilobytes of every stored capture to say it.
fn read_syscall(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || is_all_zero(text) {
        return None;
    }
    Some(text.to_string())
}

/// Whether a `syscall` line is the kernel's way of saying *none*.
///
/// The call number is `0` and every argument is `0x0`. Written as a check on
/// the whole line rather than on the first field alone, because a real
/// `io_setup` would carry arguments and must still be reported.
fn is_all_zero(line: &str) -> bool {
    let mut fields = line.split_whitespace();
    let Some(call) = fields.next() else {
        return false;
    };
    // `all` over no arguments is `true`, which is deliberate: a line reading
    // just `0` names no call and carries nothing, so it is an absence too.
    call == "0" && fields.all(|f| f == "0x0")
}

/// Fields 14 and 15 of `/proc/<pid>/stat` — `utime` and `stime`, in ticks.
///
/// **Counted from the last `)` rather than from the start of the line**, and
/// this is the whole difficulty of the file. Field 2 is the executable name in
/// parentheses and a program may have a space or a `)` in its name, so a plain
/// `split_whitespace` over the line miscounts every field after it. The failure
/// is the bad kind: it does not error, it returns some other process's numbers.
/// The kernel's own `proc(5)` says to scan for the last `)`, and that is what
/// this does.
///
/// `None` when the line is not that shape — which is what a process replaced
/// mid-read looks like, and which the caller turns into an absent CPU time
/// rather than a zero.
fn cpu_ticks(text: &str) -> Option<(u64, u64)> {
    // Everything after the closing parenthesis begins at field 3 (`state`), so
    // `utime` is the 12th word of the remainder and `stime` the 13th.
    let after = text.rsplit_once(')')?.1;
    let mut fields = after.split_whitespace();
    let user = fields.nth(11)?.parse().ok()?;
    let system = fields.next()?.parse().ok()?;
    Some((user, system))
}

/// Ticks to milliseconds — `crate::cpu::millis`' arithmetic, and the same
/// reason for multiplying first.
const fn millis(ticks: u64, hz: u64) -> u64 {
    if hz == 0 {
        return 0;
    }
    ticks.saturating_mul(1_000) / hz
}

/// How many kernel ticks make a second, or `None` where this build cannot ask.
///
/// **`None` rather than a refusal**, which is the difference between this and
/// `crate::cpu`: there, the whole subject is CPU time and a guessed rate would
/// make every figure wrong, so it refuses. Here the CPU times are two fields of
/// a table that answers plenty without them, so their absence is reported as
/// absence and the table still comes back.
#[cfg(target_os = "linux")]
fn ticks_per_second() -> Option<u64> {
    crate::syscall::clock_ticks_per_second().ok()
}

/// There is no `/proc` on this platform, so nothing reaches the conversion.
#[cfg(not(target_os = "linux"))]
const fn ticks_per_second() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]
    use super::*;

    const USERLAND: &str = "Name:\trivet\nState:\tS (sleeping)\nPPid:\t1\nThreads:\t4\n\
                            VmRSS:\t   12345 kB\n";
    const KERNEL: &str = "Name:\tkworker/0:1\nState:\tI (idle)\nPPid:\t2\nThreads:\t1\n";

    #[test]
    fn a_userland_process_carries_its_resident_size_in_bytes() {
        let p = parse(42, USERLAND).expect("a process");
        assert_eq!(p.pid, 42);
        assert_eq!(p.name, "rivet");
        assert_eq!(p.state, "S", "the letter is the fact, not the gloss");
        assert_eq!(p.parent_pid, 1);
        assert_eq!(p.threads, 4);
        assert_eq!(p.resident_bytes, Some(12345 * 1024));
        assert!(!p.kernel);
    }

    /// **A kernel thread is recognised by having no `VmRSS`, not by brackets.**
    /// A userland program can call itself `[kworker/0:1]` and this must not
    /// believe it.
    #[test]
    fn a_kernel_thread_is_recognised_by_the_absence_of_a_resident_size() {
        let p = parse(7, KERNEL).expect("a process");
        assert!(p.kernel);
        assert_eq!(p.resident_bytes, None);

        let liar = "Name:\t[kworker/0:1]\nState:\tS\nPPid:\t1\nThreads:\t1\nVmRSS:\t 8 kB\n";
        let p = parse(8, liar).expect("a process");
        assert!(!p.kernel, "a name in brackets is not evidence of anything");
    }

    /// Status with no `Name` is a file that was replaced mid-read.
    #[test]
    fn a_status_with_no_name_is_not_a_process() {
        assert!(parse(1, "Threads:\t1\n").is_none());
    }

    /// A `/proc/<pid>/stat` in the kernel's own layout. **Hand-composed**
    /// (`AGENTS.md` §4) from `proc(5)`'s field list — the values are invented and
    /// the SHAPE is what is under test. See [`CAPTURED`] for the real one.
    const STAT: &str = "214 (rivet-mcpd) S 1 214 214 0 -1 4194560 361 0 0 0 \
                        123 45 0 0 20 0 2 0 8871 4194304 1024 …";

    /// **CAPTURED, not composed** — `/proc/1/stat` read off Linux
    /// 6.6.114.1-microsoft-standard-WSL2 on 2026-08-29, verbatim, with
    /// `getconf CLK_TCK` reporting 100 on the same machine. `AGENTS.md` §4 asks
    /// for the difference between a captured vector and an invented one to be
    /// visible in the file, so it is: this is what a kernel actually writes,
    /// including the 52 trailing fields nothing here reads.
    ///
    /// **It is not a Raspberry Pi.** A Pi's `/proc/1/stat` has the same layout —
    /// the format is the kernel's rather than the architecture's — but nothing
    /// in this file has been read off the board, and the difference is a fact
    /// about what has been proven rather than a caveat about the format.
    const CAPTURED: &str = "1 (systemd) S 0 1 1 0 -1 4194560 5026 28679 157 197 13 11 11 14 20 0 \
                            1 0 5154 27701248 3696 18446744073709551615 1 1 0 0 0 0 671173123 \
                            4096 1260 0 0 0 17 12 0 0 0 0 0 0 0 0 0 0 0 0 0";

    /// **Fields 14 and 15, counted from the last `)`.** Without them there is no
    /// per-process CPU figure at all, however often the table is read.
    #[test]
    fn a_stat_line_yields_the_two_cpu_counters() {
        assert_eq!(cpu_ticks(STAT), Some((123, 45)));
    }

    /// **The same parser against bytes a kernel actually wrote**, rather than
    /// against a line composed to satisfy it. The expected figures were read out
    /// of the captured line by `cut` on the same machine, independently of this
    /// code, which is what makes this a check rather than a restatement:
    /// `utime=13 stime=11`, and `getconf CLK_TCK` on that host says 100.
    #[test]
    fn the_captured_line_parses_to_the_figures_the_shell_read_from_it() {
        assert_eq!(cpu_ticks(CAPTURED), Some((13, 11)));
        assert_eq!(millis(13, 100), 130, "13 ticks at 100 Hz is 130 ms");
    }

    /// **The defect this parser exists to avoid**, and it is the classic one: a
    /// process whose name contains a space or a `)` shifts every field after it,
    /// and a `split_whitespace` over the whole line returns some OTHER field's
    /// number without erroring. Counting from the last `)` is what `proc(5)`
    /// says to do and it is what makes these two lines agree.
    #[test]
    fn a_process_name_with_a_space_or_a_bracket_does_not_shift_the_columns() {
        let awkward = "214 (weird ) name) S 1 214 214 0 -1 4194560 361 0 0 0 \
                       123 45 0 0 20 0 2 0 8871 4194304 1024 …";
        assert_eq!(
            cpu_ticks(awkward),
            Some((123, 45)),
            "a name containing a space and a bracket moved the columns"
        );
    }

    /// A line that is not a `stat` yields nothing, which the caller turns into
    /// an **absent** CPU time — never a zero, which is a real reading of a
    /// process that has done nothing.
    #[test]
    fn a_line_that_is_not_a_stat_yields_no_counters_rather_than_zeroes() {
        for not_a_stat in ["", "\n", "214 rivet-mcpd S 1", "(no fields after)"] {
            assert_eq!(cpu_ticks(not_a_stat), None, "{not_a_stat:?} parsed");
        }
    }

    /// **`0` is the kernel declining, and it must not reach a reader as a
    /// symbol named zero** — `/proc/<pid>/wchan` writes a bare `0` for a task
    /// it will not answer about, and a caller shown `"0"` beside
    /// `"do_sys_poll"` has no way to tell a refusal from a function.
    #[test]
    fn a_wchan_the_kernel_declined_to_answer_is_an_absence_and_not_a_symbol() {
        assert_eq!(read_wchan("0"), None);
        assert_eq!(read_wchan("0\n"), None);
        assert_eq!(read_wchan(""), None);
        assert_eq!(read_wchan("  \n"), None);
    }

    /// The healthy supervisor, read off `rivet-01` on 2026-09-16.
    #[test]
    fn a_wchan_that_names_a_function_is_kept_with_its_whitespace_trimmed() {
        assert_eq!(read_wchan("do_sys_poll\n"), Some("do_sys_poll".to_string()));
        assert_eq!(
            read_wchan("unix_stream_read_generic"),
            Some("unix_stream_read_generic".to_string())
        );
    }

    /// **`running` is an answer and is kept.** The kernel writes it for a task
    /// that is on a CPU rather than parked, and *this process is not waiting
    /// for anything* is exactly what a reader chasing a wedged program needs.
    #[test]
    fn a_syscall_of_running_is_reported_rather_than_dropped() {
        assert_eq!(read_syscall("running\n"), Some("running".to_string()));
    }

    /// A kernel thread made no syscall and the file is empty.
    #[test]
    fn a_kernel_thread_makes_no_syscall_and_that_is_an_absence() {
        assert_eq!(read_syscall(""), None);
        assert_eq!(read_syscall("
"), None);
        // **What this kernel actually writes**, measured on `rivet-01`: 138
        // of 142 rows. Not an empty file -- an all-zero line, which would
        // otherwise read as `io_setup`, syscall 0 on aarch64.
        assert_eq!(read_syscall("0 0x0 0x0 0x0 0x0 0x0 0x0 0x0 0x0
"), None);
    }

    /// **A real call with zero arguments is still a real call.** The check is
    /// on the whole line, not on the first field, so an `io_setup` that
    /// carried arguments is reported.
    #[test]
    fn a_syscall_numbered_zero_with_real_arguments_is_not_mistaken_for_none() {
        assert_eq!(
            read_syscall("0 0x7fad5b2f90 0x0 0x0 0x0 0x0 0x0 0x0 0x0"),
            Some("0 0x7fad5b2f90 0x0 0x0 0x0 0x0 0x0 0x0 0x0".to_string())
        );
        assert_eq!(read_syscall("0"), None, "a bare 0 names no call and carries nothing");
    }

    /// The whole line is kept, arguments and all. The first number is the call
    /// — `73` is `ppoll` on `aarch64`, which is where a healthy supervisor
    /// sits — and the arguments are what would name the descriptor a blocked
    /// `write` is blocked on.
    #[test]
    fn a_syscall_line_is_kept_whole_because_the_arguments_are_the_diagnosis() {
        let line = "73 0x7fad5b2f90 0x1d 0x7fc6355c90 0x0 0x8 0x0 0x7fc6355c80 0x36e55c";
        assert_eq!(read_syscall(&format!("{line}\n")), Some(line.to_string()));
    }

    /// Ticks to milliseconds, multiplying before dividing so the bottom of every
    /// figure survives — `crate::cpu`'s arithmetic and its test, one file over.
    #[test]
    fn the_counters_become_milliseconds_at_the_kernels_own_rate() {
        assert_eq!(millis(123, 100), 1_230);
        assert_eq!(millis(1, 100), 10, "one tick at 100 Hz is 10 ms, not 0");
        assert_eq!(millis(123, 1000), 123);
    }

    /// **A parsed process starts with no CPU time and gains it from the second
    /// file.** The `None` here is not an oversight: `status` genuinely does not
    /// carry it, and a `0` in its place would be indistinguishable from a
    /// measurement of a process that has done nothing.
    #[test]
    fn a_status_alone_carries_no_cpu_time_and_says_so_by_absence() {
        let p = parse(42, USERLAND).expect("a process");
        assert_eq!(p.user_millis, None);
        assert_eq!(p.system_millis, None);
    }

    /// The states worth looking for survive the round trip.
    #[test]
    fn the_states_that_matter_are_reported_as_their_letter() {
        for (raw, want) in [("D (disk sleep)", "D"), ("Z (zombie)", "Z"), ("R (running)", "R")] {
            let text = format!("Name:\tx\nState:\t{raw}\nPPid:\t1\nThreads:\t1\nVmRSS:\t1 kB\n");
            assert_eq!(parse(1, &text).expect("a process").state, want);
        }
    }
}
