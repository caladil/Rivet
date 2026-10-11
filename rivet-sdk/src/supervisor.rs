//! **Is the supervisor alive, and where is it stuck** — read from `/proc`, in
//! this process, with nothing in between. `ADR-056` §3, `ADR-057` §5,
//! `.rulings/` R-199 D5.
//!
//! # Why this is not [`crate::facts`]
//!
//! **An instrument may not depend on the thing it measures.** The facts call is
//! answered by the supervisor, so the supervisor's own liveness cannot come
//! from it — a wedged supervisor would simply stop answering, and a screen
//! would have no way to say whether that meant *stopped* or *busy* or
//! *nothing has changed*.
//!
//! This module asks the kernel instead. It opens files, formats what it read,
//! and makes no connection, holds no identity and has nothing that can time
//! out.
//!
//! **The pairing is what makes a screen worth having**: when the supervisor
//! wedges, [`crate::facts::ask`] returns
//! [`Silent`](crate::facts::NotAnswered::Silent) and this keeps working, so a
//! dashboard reports *the supervisor stopped* rather than merely going stale.
//!
//! # It needs a grant, and the grant is a name in the platform
//!
//! `rivet_core::confinement::SUPERVISOR_WINDOWS` is the list of programs whose
//! ruleset carries the two leaves this reads: the supervisor's own `/proc`
//! directory, read-only and **not** listable, and `/proc/uptime`. There is no
//! manifest key for it and there is deliberately not going to be one — a
//! capability that can be asked for is one that will be. **An application that
//! is not on that list gets `Refused` on every row here**, which is an answer
//! and is reported as one.
//!
//! # What it can say, and what it cannot
//!
//! **It can say whether the supervisor has stopped**, and nothing else on the
//! machine can. [`Sample::voluntary`] and [`Sample::cpu_ticks`] are cumulative
//! counters; read against [`Sample::uptime_ms`] they answer *has this supervisor
//! done anything in the last five hours*, which is the fact the 2026-09-18
//! incident turned on.
//!
//! **It cannot reliably say where.** `wchan` is world-readable by mode and its
//! *contents* are gated on `ptrace_may_access`, so a different-uid reader is
//! handed the single character `0` — which is also what a running task reads.
//! `syscall` is mode `0400` root and is refused outright. Both are reported
//! honestly: see [`Reading::Indeterminate`], and `ADR-057` §5 for what would
//! change it.
//!
//! # One sample of a counter says nothing
//!
//! `ADR-056` §3. The question is *is it turning*, and a turning loop is a
//! **difference**. Take two samples and use [`Sample::moved_since`]; a screen
//! that drew one snapshot would leave a person holding a number in their head
//! and refreshing, which is the reading nobody takes correctly at three in the
//! morning.

/// Where the process table is. An argument everywhere below so that the whole
/// module is testable with no `/proc` at all — `AGENTS.md` §8.3.
pub const PROC_DIR: &str = "/proc";

/// Seconds since boot, and the interval every cumulative counter is worthless
/// without: 900 ms of CPU over five minutes and 900 ms over five hours are
/// different machines. A second copy of
/// `rivet_core::confinement::UPTIME_PATH`, held honest by
/// `tests/agreement.rs`.
pub const UPTIME_PATH: &str = "/proc/uptime";

/// **What the kernel's clock tick is worth**, in milliseconds.
///
/// `utime` and `stime` are counted in `USER_HZ`, which is fixed at 100 in the
/// kernel's userspace ABI on every architecture Rivet targets. **Stated rather
/// than asked**: `sysconf(_SC_CLK_TCK)` is a C call, this module is one of the
/// ones `crate`'s own rule keeps free of `unsafe`, and a wrong constant here
/// would scale a number rather than break it — so [`Sample::cpu_ticks`] carries
/// the raw count as well, and that one cannot be wrong.
pub const MS_PER_TICK: u64 = 10;

/// **One reading, and what its absence means.**
///
/// Four variants because the failures are not interchangeable and a probe that
/// cannot tell its own answer from an error is not a probe (`AGENTS.md` §7).
/// *Refused* is a policy; *absent* is a machine; *indeterminate* is a kernel
/// that answered and did not say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading<T> {
    /// The kernel said so.
    Answered(T),
    /// **The file was read and what came back cannot be told from something
    /// else.** The clause says which two.
    ///
    /// `wchan` is the case this exists for: its contents are gated on
    /// `ptrace_may_access`, and a reader the kernel refuses is handed `0` —
    /// exactly what a task that is *running* rather than parked reads. Both
    /// answers are one character and neither is a mistake.
    Indeterminate(&'static str),
    /// The kernel refused it — Landlock, the file's mode, or `ptrace_may_access`
    /// — with its own words.
    Refused(String),
    /// It is not there. A pid that has gone, or a machine with no `/proc`.
    Absent(String),
}

impl<T> Reading<T> {
    /// The value, if there is one.
    pub const fn value(&self) -> Option<&T> {
        match self {
            Self::Answered(v) => Some(v),
            _ => None,
        }
    }

    /// Whether this row answered.
    pub const fn answered(&self) -> bool {
        matches!(self, Self::Answered(_))
    }

    /// **One short clause for a screen**, whichever of the four this is.
    pub fn say(&self) -> String
    where
        T: std::fmt::Display,
    {
        match self {
            Self::Answered(v) => v.to_string(),
            Self::Indeterminate(why) => format!("indeterminate — {why}"),
            Self::Refused(why) => format!("refused — {why}"),
            Self::Absent(why) => format!("absent — {why}"),
        }
    }
}

/// **Which process this is, and whether it is still the one that started us** —
/// `ADR-056` §3.
///
/// The supervisor spawns every application, so an application's parent **is**
/// the supervisor. There is no scan, no name lookup and no list — and a scan
/// would be wrong on its own terms besides, because two processes can carry one
/// name and the one this application's fate is tied to is its parent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Identified {
    /// The parent is the same process it was at startup. Its pid.
    Supervisor(u32),
    /// **The parent is now pid 1.** The supervisor that started this
    /// application is gone and this process was reparented to `rivet-initd`.
    /// That is an answer, not a failure to find one.
    Reparented,
    /// **The parent changed and is not pid 1.** The supervisor was replaced
    /// while this application kept running, which means a restart nobody asked
    /// about. Worth naming rather than smoothing over.
    Replaced {
        /// Its pid at startup.
        was: u32,
        /// Its pid now.
        now: u32,
    },
    /// **The parent was already pid 1 at startup**, so this process was not
    /// started by a supervisor and there is none to identify. It says exactly
    /// that rather than guessing.
    NeverSupervised,
}

/// **The supervisor, as `/proc` describes it at one instant.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    /// Which process was read, and whether it is the one that started this
    /// application. See [`Identified`].
    pub identified: Identified,
    /// The program's name, from `comm` — what confirms the identification.
    /// On a Rivet it reads `rivet-sysd`.
    pub comm: Reading<String>,
    /// **The state letter.** `S` is sleeping, `R` running, `D` uninterruptible,
    /// `Z` a zombie.
    ///
    /// `S` is what a healthy supervisor parked in `ppoll` reads **and** what a
    /// wedged one blocked in `connect` reads, so on its own it is a negative:
    /// it rules out the card, which is what cost one incident its diagnosis.
    pub state: Reading<char>,
    /// **How many threads it has.** The supervisor's own safety argument for
    /// calling `setenv` between `fork` and `exec` depends on this being `1`;
    /// anything else is that argument's condition being false, and it costs one
    /// line to see.
    pub threads: Reading<u32>,
    /// **How many times the loop has parked itself.** A loop that is turning
    /// moves this constantly, and frozen counters read against [`Self::uptime_ms`]
    /// are the whole detection signal.
    pub voluntary: Reading<u64>,
    /// Preemption, which separates *starved* from *parked*.
    pub nonvoluntary: Reading<u64>,
    /// **Cumulative CPU in clock ticks**, user plus system. Raw, because
    /// [`MS_PER_TICK`] is stated rather than asked — see that constant.
    pub cpu_ticks: Reading<u64>,
    /// **The kernel function it is parked in**, when the kernel will say.
    ///
    /// [`Reading::Indeterminate`] for any reader the kernel will not let
    /// `ptrace`, which is every application reading the root-owned supervisor.
    /// `ADR-057` §5.
    pub wchan: Reading<String>,
    /// **The syscall it is stopped in.** `/proc/<pid>/syscall` is mode `0400`
    /// and root-owned, so this is [`Reading::Refused`] for an application, by
    /// the file's own mode and before `ptrace_may_access` is reached.
    pub syscall: Reading<String>,
    /// **Milliseconds since the machine booted** — the interval the cumulative
    /// counters above are read against, and the number that makes *the
    /// supervisor has done nothing for five hours* sayable at all.
    ///
    /// **Milliseconds and not the file's own seconds-with-two-decimals.** This
    /// number exists to be compared with another reading of itself, and a
    /// float that is only ever subtracted from a float is a float somebody
    /// eventually tests for equality.
    pub uptime_ms: Reading<u64>,
}

impl Sample {
    /// **Did it move?** `Some(true)` when any counter advanced since an earlier
    /// sample, `Some(false)` when none did, and `None` when neither sample
    /// could read one.
    ///
    /// `ADR-056` §3: the whole question is a difference. A supervisor whose
    /// CPU and voluntary context switches have not moved while the machine's
    /// uptime climbed is a supervisor that has stopped, and that single
    /// comparison is the fact nothing else on the machine can state.
    #[must_use]
    pub fn moved_since(&self, earlier: &Self) -> Option<bool> {
        let mut compared = false;
        let mut moved = false;
        if let (Reading::Answered(now), Reading::Answered(was)) = (&self.voluntary, &earlier.voluntary)
        {
            compared = true;
            moved |= now > was;
        }
        if let (Reading::Answered(now), Reading::Answered(was)) =
            (&self.nonvoluntary, &earlier.nonvoluntary)
        {
            compared = true;
            moved |= now > was;
        }
        if let (Reading::Answered(now), Reading::Answered(was)) = (&self.cpu_ticks, &earlier.cpu_ticks)
        {
            compared = true;
            moved |= now > was;
        }
        compared.then_some(moved)
    }

    /// **Whether this reading amounts to anything at all.**
    ///
    /// `false` for an application that was not granted the window, which is
    /// every application not in `SUPERVISOR_WINDOWS`. A screen that drew
    /// *stopped* on the strength of a refused read would be reporting its own
    /// confinement as the machine's failure.
    #[must_use]
    pub fn anything_was_read(&self) -> bool {
        self.state.answered() || self.voluntary.answered() || self.cpu_ticks.answered()
    }
}

/// **A watcher, holding the pid the supervisor had at startup** — `ADR-056`
/// §3.3.
///
/// Begin it once, when the application starts, and read it as often as a screen
/// refreshes. The comparison between the two pids is itself diagnostic, which
/// is why the startup value is held rather than re-derived.
#[derive(Debug, Clone, Copy)]
pub struct Watch {
    /// The parent pid at the moment [`Watch::begin`] was called.
    at_start: u32,
}

impl Watch {
    /// **Record who started this application.** Call it at startup, before
    /// anything can have reparented.
    #[must_use]
    pub fn begin() -> Self {
        Self {
            at_start: parent_pid(),
        }
    }

    /// The pid this application's parent had at startup.
    #[must_use]
    pub const fn started_by(self) -> u32 {
        self.at_start
    }

    /// **Read the supervisor now.** Never fails: every field says why it is
    /// absent.
    #[must_use]
    pub fn read(self) -> Sample {
        self.read_in(PROC_DIR, UPTIME_PATH, parent_pid())
    }

    /// **The read, with its paths and the parent pid as arguments**, so the
    /// whole module can be driven over a directory of fixture files on a
    /// machine with no `/proc` — which is every machine an author works on.
    #[must_use]
    pub fn read_in(self, proc_dir: &str, uptime_path: &str, parent_now: u32) -> Sample {
        let identified = if self.at_start == 1 {
            Identified::NeverSupervised
        } else if parent_now == 1 {
            Identified::Reparented
        } else if parent_now == self.at_start {
            Identified::Supervisor(parent_now)
        } else {
            Identified::Replaced {
                was: self.at_start,
                now: parent_now,
            }
        };

        // **The pid that is READ is the one that started this application**,
        // never the one it has now. After a reparent the current parent is pid
        // 1, and reading pid 1's counters would report `rivet-initd` turning
        // while calling it the supervisor. The old pid is gone, every row comes
        // back `Absent`, and *the supervisor that started this is gone* is the
        // honest reading.
        let dir = format!("{proc_dir}/{}", self.at_start);
        let status = read_text(&format!("{dir}/status"));
        let stat = read_text(&format!("{dir}/stat"));

        Sample {
            identified,
            comm: read_text(&format!("{dir}/comm")).map(|t| t.trim().to_string()),
            state: status
                .clone()
                .and_then_parse(|t| status_field(t, "State").and_then(|v| v.chars().next())),
            threads: status
                .clone()
                .and_then_parse(|t| status_field(t, "Threads").and_then(|v| v.parse().ok())),
            voluntary: status.clone().and_then_parse(|t| {
                status_field(t, "voluntary_ctxt_switches").and_then(|v| v.parse().ok())
            }),
            nonvoluntary: status.clone().and_then_parse(|t| {
                status_field(t, "nonvoluntary_ctxt_switches").and_then(|v| v.parse().ok())
            }),
            cpu_ticks: stat.and_then_parse(cpu_ticks),
            wchan: read_text(&format!("{dir}/wchan")).and_then_reading(|t| {
                let symbol = t.trim();
                // **`0` is the kernel refusing without saying so.**
                // `proc_pid_wchan` prints the symbol only when
                // `ptrace_may_access` allows it and prints `0` otherwise — and
                // `0` is also what a task that is not parked reads. One
                // character, two answers, and no way here to tell them apart.
                if symbol == "0" || symbol.is_empty() {
                    Reading::Indeterminate(
                        "the kernel prints `0` both for a task that is not parked and for a \
                         reader it will not let ptrace, and an application may not ptrace the \
                         root-owned supervisor (ADR-057 §5)",
                    )
                } else {
                    Reading::Answered(symbol.to_string())
                }
            }),
            syscall: read_text(&format!("{dir}/syscall")).map(|t| t.trim().to_string()),
            uptime_ms: read_text(uptime_path).and_then_parse(|t| {
                t.split_whitespace()
                    .next()
                    .and_then(|s| s.parse::<f64>().ok())
                    .map(|seconds| (seconds * 1000.0) as u64)
            }),
        }
    }
}

/// This process's parent, without a C call.
///
/// `std::os::unix::process::parent_id` is `getppid(2)` and is safe, which is
/// what keeps this module inside `crate`'s rule that every module but four is
/// free of `unsafe`. Reading `/proc/self/status` would have been the other way
/// and is the wrong one twice over: `/proc/self` is this process's own
/// directory and no policy grants it, so a confined application is refused its
/// own `PPid`.
#[cfg(unix)]
fn parent_pid() -> u32 {
    std::os::unix::process::parent_id()
}

/// Off Unix there is no parent to name. The module still compiles so an author
/// can run its parsers — see `crate`'s note on why that matters.
#[cfg(not(unix))]
const fn parent_pid() -> u32 {
    1
}

/// Read a file, keeping WHY it could not be read.
fn read_text(path: &str) -> Reading<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Reading::Answered(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Reading::Absent(e.to_string()),
        // **Every other error is a refusal, and `PermissionDenied` is the one
        // that matters.** Landlock and a file mode both arrive here, and an
        // application that is not in `SUPERVISOR_WINDOWS` gets this on every
        // row — which is a fact about this program's grant and never a fact
        // about the supervisor.
        Err(e) => Reading::Refused(e.to_string()),
    }
}

impl Reading<String> {
    /// Turn the text into something else, keeping the reason a failed read
    /// failed. A parse that finds nothing becomes [`Reading::Indeterminate`]
    /// rather than an absence: the file WAS read, and what is missing is this
    /// build's understanding of it.
    fn and_then_parse<T>(self, f: impl FnOnce(&str) -> Option<T>) -> Reading<T> {
        self.and_then_reading(|t| {
            f(t).map_or(
                Reading::Indeterminate("the file was read and this build did not find the field"),
                Reading::Answered,
            )
        })
    }

    fn and_then_reading<T>(self, f: impl FnOnce(&str) -> Reading<T>) -> Reading<T> {
        match self {
            Self::Answered(text) => f(&text),
            Self::Indeterminate(why) => Reading::Indeterminate(why),
            Self::Refused(why) => Reading::Refused(why),
            Self::Absent(why) => Reading::Absent(why),
        }
    }

    fn map<T>(self, f: impl FnOnce(&str) -> T) -> Reading<T> {
        self.and_then_reading(|t| Reading::Answered(f(t)))
    }
}

/// One `Name:\tvalue` field out of `/proc/<pid>/status`.
///
/// A line scan rather than a parser, for `rivet-sysd`'s `serial_in` reason:
/// this is five fields out of sixty and taking a reader for it would be more
/// surface than the thing it reads.
fn status_field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key == name).then(|| value.trim())
    })
}

/// `utime + stime` out of `/proc/<pid>/stat`, in clock ticks.
///
/// **Split at the LAST `)` and not on whitespace.** Field two is the program's
/// name in parentheses and a program may be called anything — `rivet sysd)` is
/// a legal name and would shift every field after it. Every parser that split
/// this file on spaces has had that bug.
fn cpu_ticks(text: &str) -> Option<u64> {
    let after = text.rsplit_once(')')?.1;
    let fields: Vec<&str> = after.split_whitespace().collect();
    // After the closing parenthesis the fields are `state`, `ppid`, `pgrp`,
    // `session`, `tty_nr`, `tpgid`, `flags`, `minflt`, `cminflt`, `majflt`,
    // `cmajflt`, `utime`, `stime` — so index 11 and 12 with the name and pid
    // already consumed.
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    Some(utime + stime)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
    use super::*;

    /// A real `/proc/<pid>/status`, cut to the lines this reads, from a board.
    const STATUS: &str = "Name:\trivet-sysd\n\
                          State:\tS (sleeping)\n\
                          Tgid:\t146\n\
                          Pid:\t146\n\
                          PPid:\t1\n\
                          Threads:\t1\n\
                          SigBlk:\t0000000000014003\n\
                          voluntary_ctxt_switches:\t41823\n\
                          nonvoluntary_ctxt_switches:\t97\n";

    /// A real `/proc/<pid>/stat`, with the fields this reads in place.
    const STAT: &str = "146 (rivet-sysd) S 1 146 146 0 -1 4194560 2951 0 0 0 148 322 0 0 20 0 1 0 \
                        1837 12648448 1044 18446744073709551615\n";

    fn fixtures(dir: &std::path::Path, pid: u32) -> (String, String) {
        let proc_dir = dir.join("proc");
        let pid_dir = proc_dir.join(pid.to_string());
        std::fs::create_dir_all(&pid_dir).expect("a fixture directory");
        std::fs::write(pid_dir.join("status"), STATUS).expect("status");
        std::fs::write(pid_dir.join("stat"), STAT).expect("stat");
        std::fs::write(pid_dir.join("comm"), "rivet-sysd\n").expect("comm");
        let uptime = dir.join("uptime");
        std::fs::write(&uptime, "123456.78 987654.32\n").expect("uptime");
        (
            proc_dir.to_string_lossy().into_owned(),
            uptime.to_string_lossy().into_owned(),
        )
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rivet-sup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        dir
    }

    #[test]
    fn a_healthy_supervisor_reads_field_for_field() {
        let dir = scratch("healthy");
        let (proc_dir, uptime) = fixtures(&dir, 146);
        let watch = Watch { at_start: 146 };
        let sample = watch.read_in(&proc_dir, &uptime, 146);

        assert_eq!(sample.identified, Identified::Supervisor(146));
        assert_eq!(sample.comm, Reading::Answered("rivet-sysd".to_string()));
        assert_eq!(sample.state, Reading::Answered('S'));
        assert_eq!(sample.threads, Reading::Answered(1));
        assert_eq!(sample.voluntary, Reading::Answered(41_823));
        assert_eq!(sample.nonvoluntary, Reading::Answered(97));
        // 148 + 322 ticks.
        assert_eq!(sample.cpu_ticks, Reading::Answered(470));
        assert_eq!(sample.uptime_ms, Reading::Answered(123_456_780));
        assert!(sample.anything_was_read());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The `stat` parser survives a program named with a space in it**, which
    /// every parser that split this file on whitespace has got wrong.
    #[test]
    fn the_cpu_fields_are_found_past_a_name_that_contains_spaces() {
        let awkward = STAT.replacen("(rivet-sysd)", "(rivet sysd) (x)", 1);
        assert_eq!(cpu_ticks(&awkward), Some(470));
        assert_eq!(cpu_ticks(STAT), Some(470));
        assert_eq!(cpu_ticks("nonsense with no parenthesis"), None);
    }

    /// **A refused read is a refusal and not an absence**, and the distinction
    /// is the one `AGENTS.md` §7 is about: an application that is not in
    /// `SUPERVISOR_WINDOWS` must not report its own confinement as the
    /// supervisor having gone.
    #[test]
    fn a_missing_window_reads_as_absent_rather_than_as_a_stopped_supervisor() {
        let dir = scratch("nowindow");
        let proc_dir = dir.join("proc").to_string_lossy().into_owned();
        let uptime = dir.join("uptime").to_string_lossy().into_owned();
        let sample = Watch { at_start: 146 }.read_in(&proc_dir, &uptime, 146);
        assert!(matches!(sample.state, Reading::Absent(_)));
        assert!(matches!(sample.cpu_ticks, Reading::Absent(_)));
        assert!(
            !sample.anything_was_read(),
            "nothing was read, so nothing may be concluded about the supervisor"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **`0` in `wchan` is not a symbol and is not an error**, and saying so is
    /// the whole of `ADR-057` §5. The kernel prints it both for a task that
    /// is running and for a reader it will not let ptrace.
    #[test]
    fn a_wchan_of_zero_is_reported_as_indeterminate() {
        let dir = scratch("wchan");
        let (proc_dir, uptime) = fixtures(&dir, 146);
        std::fs::write(dir.join("proc/146/wchan"), "0\n").expect("wchan");
        let refused = Watch { at_start: 146 }.read_in(&proc_dir, &uptime, 146);
        assert!(matches!(refused.wchan, Reading::Indeterminate(_)));
        assert!(refused.wchan.say().starts_with("indeterminate"));

        std::fs::write(dir.join("proc/146/wchan"), "unix_wait_for_peer\n").expect("wchan");
        let named = Watch { at_start: 146 }.read_in(&proc_dir, &uptime, 146);
        assert_eq!(
            named.wchan,
            Reading::Answered("unix_wait_for_peer".to_string())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The four identifications, and each is an answer** — `ADR-056` §3.
    #[test]
    fn who_the_parent_is_now_is_itself_the_diagnosis() {
        let dir = scratch("ident");
        let (proc_dir, uptime) = fixtures(&dir, 146);
        let read = |at_start: u32, now: u32| {
            Watch { at_start }.read_in(&proc_dir, &uptime, now).identified
        };
        assert_eq!(read(146, 146), Identified::Supervisor(146));
        assert_eq!(
            read(146, 1),
            Identified::Reparented,
            "a parent of 1 is the supervisor that started this application being gone"
        );
        assert_eq!(
            read(146, 200),
            Identified::Replaced { was: 146, now: 200 },
            "a supervisor replaced under a running application is a restart nobody asked about"
        );
        assert_eq!(
            read(1, 1),
            Identified::NeverSupervised,
            "a process pid 1 started is one with no supervisor to identify, and it says so"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **After a reparent the OLD pid is read, never the new one.** Reading the
    /// current parent would report pid 1's counters turning and call them the
    /// supervisor's, which is a working machine drawn over a dead one.
    #[test]
    fn a_reparented_application_does_not_read_pid_ones_counters_instead() {
        let dir = scratch("reparent");
        let (proc_dir, uptime) = fixtures(&dir, 1);
        let sample = Watch { at_start: 146 }.read_in(&proc_dir, &uptime, 1);
        assert_eq!(sample.identified, Identified::Reparented);
        assert!(
            matches!(sample.voluntary, Reading::Absent(_)),
            "the fixture here is pid 1's, and reading it would have answered"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **One sample says nothing; two say everything** — `ADR-056` §3.
    #[test]
    fn a_stopped_supervisor_is_a_difference_and_not_a_snapshot() {
        let dir = scratch("moved");
        let (proc_dir, uptime) = fixtures(&dir, 146);
        let watch = Watch { at_start: 146 };
        let first = watch.read_in(&proc_dir, &uptime, 146);

        // The same counters, a second later: a supervisor that has stopped.
        let wedged = watch.read_in(&proc_dir, &uptime, 146);
        assert_eq!(
            wedged.moved_since(&first),
            Some(false),
            "nothing advanced, which read against a climbing uptime is the whole diagnosis"
        );

        // And one that is turning.
        std::fs::write(
            dir.join("proc/146/status"),
            STATUS.replacen("41823", "41999", 1),
        )
        .expect("status");
        let turning = watch.read_in(&proc_dir, &uptime, 146);
        assert_eq!(turning.moved_since(&first), Some(true));

        // A pair with nothing read cannot conclude either way, and says so
        // rather than reporting a supervisor that never moved.
        let blind = Watch { at_start: 999 }.read_in(&proc_dir, &uptime, 999);
        assert_eq!(blind.moved_since(&blind), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The real machine, when there is one.** Ignored by default because it
    /// depends on the host having `/proc`, and run by name on Linux to check
    /// the parsers against the kernel rather than against a fixture.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "reads this host's own /proc; run it by name on Linux"]
    fn this_hosts_own_parent_reads() {
        let sample = Watch::begin().read();
        assert!(
            sample.anything_was_read(),
            "nothing was read off a real /proc: {sample:?}"
        );
        assert!(sample.state.answered());
    }
}
