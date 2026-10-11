//! Catch `SIGTERM`, take eight seconds to go, and say so every second.
//!
//! `.rulings/` R-141, register `R-34`. `Cargo.toml` carries the argument for
//! why this program exists; what follows is how it works.
//!
//! The shape is deliberately dull, because the interesting part is what the
//! MACHINE does around it:
//!
//! ```text
//!   rivet-initd ──SIGTERM──► rivet-sysd ──SIGTERM──► slow-stop-probe
//!                                                          │ t+0     "SIGTERM"
//!                                                          │ t+1..7  "still here"
//!                                                          │ t+8     exit 0
//!                                 ◄── reaped, clean ────────┘
//!        ◄── supervisor exits of its own accord, NOT killed ┘
//! ```
//!
//! **What a passing run looks like**, read afterwards with
//! `rivet get log previous all`:
//!
//! * eight `slow-stop-probe:` records, one a second, the last one saying it is
//!   exiting 0;
//! * NO `did not exit within its grace period; sending SIGKILL` from
//!   `rivet-sysd` naming this service;
//! * `rivet-initd` recording that the supervisor exited, rather than that it
//!   killed it.
//!
//! **What a failing run looks like**, and it is worth writing down because it
//! is what the tree did before `rivet_core::shutdown` existed: the records stop
//! around t+5, because PID 1's five-second grace ran out and the supervisor —
//! the thing ingesting those records — was killed underneath them.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// How long this program takes to stop.
///
/// **Eight seconds is chosen to sit between the two numbers**, and that is the
/// whole of why it is this value:
///
/// * under `rivet_core::shutdown::SERVICE_STOP_GRACE_MS` (10 s), so a correct
///   supervisor waits rather than escalating — a probe that overran its own
///   budget would prove only that `SIGKILL` works;
/// * over the FIVE seconds `cmd/rivet-initd` used to give the whole supervisor,
///   so the defect this is testing for would still fire.
///
/// It is not read from `rivet-core`: this program is an application built for a
/// board and the constant it must straddle is the one in the image that is
/// RUNNING, not the one in the tree it was compiled from. Writing the number
/// here, with the reason, is the honest form of that.
const DRAIN_MS: u64 = 8_000;

/// The file the second record goes to, inside `RIVET_APP_DATA`.
const RECORD_FILE: &str = "shutdown-log.txt";

// The drain has to straddle the two graces or this program measures nothing.
// Checked by the COMPILER rather than by a test, for the reason
// `rivet_core::shutdown` gives for the same choice: a runtime assertion over
// constants is the same check reached an hour later, and clippy is right to
// call it `assertions_on_constants`.
//
// The numbers on the right are the ones in the IMAGE, not in this tree — see
// [`DRAIN_MS`]. They are written out here so that changing the drain without
// re-reading them does not build.

/// Under the five seconds `cmd/rivet-initd` used to allow the whole supervisor,
/// this would stop in time even with the defect present, and a clean shutdown
/// would prove nothing.
const _LONGER_THAN_THE_OLD_SUPERVISOR_GRACE: () = assert!(DRAIN_MS > 5_000);

/// Over `rivet_core::shutdown::SERVICE_STOP_GRACE_MS`, a `SIGKILL` is the
/// CORRECT behaviour, and the run would be measuring the escalation rather than
/// the wait.
const _SHORTER_THAN_THE_SERVICE_GRACE: () = assert!(DRAIN_MS < 10_000);

/// Set by the signal handler and read by the main loop. Nothing else.
static TERM: AtomicBool = AtomicBool::new(false);

/// **The only thing that runs in signal context.**
///
/// A relaxed-or-stronger store to a `static` atomic is one of the few things
/// POSIX allows here. No allocation, no formatting, no I/O — a `println!` in a
/// signal handler can deadlock on stdout's own lock, and would do it precisely
/// when this program is being asked to prove it can still write.
extern "C" fn on_term(_signum: libc::c_int) {
    TERM.store(true, Ordering::SeqCst);
}

/// Install [`on_term`] for `SIGTERM`.
///
/// # Errors
///
/// The `sigaction` call's own error, which on a healthy kernel cannot happen
/// for `SIGTERM` — it is reported rather than unwrapped because a probe that
/// panicked here would look exactly like a probe that was killed.
fn catch_sigterm() -> std::io::Result<()> {
    // SAFETY: `act` is fully initialised before use — zeroed, then every field
    // this call reads is set. `on_term` has the C ABI and does nothing that is
    // unsafe in signal context. The third argument is null because the previous
    // disposition is not wanted.
    unsafe {
        let mut act: libc::sigaction = std::mem::zeroed();
        act.sa_sigaction = on_term as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::sigemptyset(&raw mut act.sa_mask);
        // `SA_RESTART` so the one-second sleep in the main loop is resumed
        // rather than answering `EINTR`. Nothing here would mishandle `EINTR`,
        // but a probe should not need the reader to check that.
        act.sa_flags = libc::SA_RESTART;
        if libc::sigaction(libc::SIGTERM, &raw const act, std::ptr::null_mut()) != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// The kernel's monotonic clock, in seconds, as a string.
///
/// This is what makes the two records comparable with the journal: `ADR-022`
/// stamps every record with `mono`, and a Pi has no RTC so wall time is the
/// epoch until something corrects it. An unreadable `/proc/uptime` answers
/// `"unknown"` rather than a guess — `AGENTS.md` §5.
fn mono() -> String {
    match std::fs::read_to_string("/proc/uptime") {
        Ok(text) => text
            .split_whitespace()
            .next()
            .map_or_else(|| "unparsable".to_string(), ToString::to_string),
        Err(_) => "unknown".to_string(),
    }
}

/// Writes every line twice — stdout for the journal, a file for the case where
/// the journal is the thing that stopped working.
struct Recorder {
    file: Option<std::fs::File>,
    /// Why there is no file, when there is none. Reported once, at startup, so
    /// a run with half a record says so instead of looking complete.
    why_no_file: Option<String>,
}

impl Recorder {
    /// Open the file half of the record under `dir`. `None` means this program
    /// was not handed a data directory, which is the case when it is run by
    /// hand.
    ///
    /// # It APPENDS, and the first version truncated
    ///
    /// **Found on the board, 2026-09-04, on the first run that mattered.** The
    /// file was opened with `File::create`, which truncates. The probe wrote
    /// eight drain lines during the shutdown, `fsync`ed each one — and then the
    /// machine came back up, started it again, and the first thing it did was
    /// erase the record it had just been rebooted to produce. The journal had
    /// kept the whole account, so nothing was lost; the SECOND record, the one
    /// that exists precisely because the journal might not survive, was the
    /// half that did not.
    ///
    /// That is `AGENTS.md` §7's shape in an unusual direction: two independent
    /// readings are only two if the second one still exists when you go to read
    /// it. Appending costs an ever-growing file — bounded in practice, since
    /// this writes eleven lines per boot — and keeps every shutdown it has ever
    /// seen, which is what makes the file comparable across boots at all.
    fn to(dir: Option<&Path>) -> Self {
        let Some(dir) = dir else {
            return Self {
                file: None,
                why_no_file: Some(
                    "no RIVET_APP_DATA, so the only record is stdout. That is normal when this \
                     is run by hand and a FINDING when a supervisor started it, because the \
                     manifest asks for state = \"private\""
                        .to_string(),
                ),
            };
        };
        let path = dir.join(RECORD_FILE);
        match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => Self { file: Some(file), why_no_file: None },
            Err(why) => Self {
                file: None,
                why_no_file: Some(format!("{} could not be opened: {why}", path.display())),
            },
        }
    }

    /// One line, to both halves, durably.
    ///
    /// `sync_all` per line is the point rather than an excess: the machine is
    /// about to be rebooted, and a record still in the page cache when the
    /// kernel is asked to restart proves nothing.
    fn say(&mut self, line: &str) {
        println!("slow-stop-probe: {line}");
        let _ = std::io::stdout().flush();
        if let Some(file) = self.file.as_mut() {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
            let _ = file.sync_all();
        }
    }
}

/// `RIVET_APP_DATA`, read as the SDK would read it.
///
/// Read from the environment directly rather than through `rivetos-sdk`: this
/// probe is measuring the SUPERVISOR, and one fewer thing between it and the
/// answer is worth the four lines. The variable name is `ADR-015` §3.2A's and
/// is the same string the SDK exposes as `rivet_sdk::app::APP_DATA_ENV`.
fn app_data() -> Option<PathBuf> {
    match std::env::var("RIVET_APP_DATA") {
        Ok(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ => None,
    }
}

fn main() {
    let data = app_data();
    let mut rec = Recorder::to(data.as_deref());

    rec.say(&format!(
        "started, pid {}, mono {}, drain {DRAIN_MS} ms on SIGTERM",
        std::process::id(),
        mono(),
    ));
    if let Some(why) = rec.why_no_file.clone() {
        rec.say(&format!("no file record: {why}"));
    } else if let Some(dir) = data.as_deref() {
        rec.say(&format!("file record at {}", dir.join(RECORD_FILE).display()));
    }

    if let Err(why) = catch_sigterm() {
        rec.say(&format!(
            "SIGTERM COULD NOT BE CAUGHT: {why}. This process will die on the default \
             disposition and this run proves nothing about the shutdown path"
        ));
    } else {
        rec.say("SIGTERM is caught; waiting for one");
    }

    // Idle. Nothing is journalled while waiting: this program can sit for days
    // between the deploy and the reboot that measures it, and a heartbeat would
    // push the interesting records out of a rolling 1 MB journal (`ADR-022` §8).
    while !TERM.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    let began = std::time::Instant::now();
    rec.say(&format!("SIGTERM at mono {}; draining for {DRAIN_MS} ms", mono()));

    let mut announced = 0_u64;
    loop {
        let elapsed = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
        if elapsed >= DRAIN_MS {
            break;
        }
        if elapsed / 1_000 > announced {
            announced = elapsed / 1_000;
            rec.say(&format!("still draining, {elapsed} ms in, mono {}", mono()));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let took = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
    rec.say(&format!("drain complete after {took} ms, mono {}; exiting 0", mono()));
    std::process::exit(0);
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]
// A test that cannot fail loudly is not a test — `AGENTS.md` §10.5.
mod tests {
    use super::*;

    /// **The load-bearing test.** Everything else in this program is a `sleep`
    /// and a `println!`; the one part that can be silently wrong is the
    /// `sigaction` call, and being wrong there does not produce a bad answer —
    /// it produces a process that dies instantly on `SIGTERM` and a shutdown
    /// that looks fast because nothing was slow.
    ///
    /// Break it by changing the signal number in [`catch_sigterm`] to
    /// `SIGUSR1`: the handler is then not installed for `SIGTERM`, the default
    /// disposition applies, and `raise` **terminates the test binary** — the
    /// harness reports the whole run killed by signal 15. Verified 2026-09-04.
    #[test]
    fn sigterm_is_caught_rather_than_killing_this_process() {
        catch_sigterm().expect("SIGTERM must be catchable on any Linux this runs on");
        assert!(!TERM.load(Ordering::SeqCst), "nothing has raised one yet");

        // SAFETY: `raise` on the calling thread, with the handler above already
        // installed. The whole test is that this returns at all.
        let sent = unsafe { libc::raise(libc::SIGTERM) };
        assert_eq!(sent, 0, "raise(SIGTERM) failed");

        assert!(
            TERM.load(Ordering::SeqCst),
            "the handler did not run, so this program would be killed on SIGTERM instead of \
             draining, and a shutdown measured with it would show nothing"
        );
        TERM.store(false, Ordering::SeqCst);
    }

    /// The file half of the record is the half that does not depend on the
    /// supervisor still draining a pipe, so it is worth proving it is written
    /// rather than assumed.
    #[test]
    fn a_line_reaches_the_file_and_not_only_stdout() {
        let dir = std::env::temp_dir().join(format!("slow-stop-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a temp directory");

        let mut rec = Recorder::to(Some(dir.as_path()));
        assert!(rec.why_no_file.is_none(), "{:?}", rec.why_no_file);
        rec.say("draining");
        drop(rec);

        let written = std::fs::read_to_string(dir.join(RECORD_FILE)).expect("the record file");
        assert_eq!(written, "draining\n", "the file carries the line, unprefixed");

        // **The regression.** The first version of this used `File::create`, so
        // the next boot erased the shutdown it had just recorded — see
        // [`Recorder::to`]. A second run must ADD to the file, not replace it.
        let mut again = Recorder::to(Some(dir.as_path()));
        again.say("a later boot");
        drop(again);

        let both = std::fs::read_to_string(dir.join(RECORD_FILE)).expect("the record file");
        assert_eq!(
            both, "draining\na later boot\n",
            "the previous boot's record must survive this one starting"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// No data directory is a REPORTED state, not a silent one. A run whose
    /// only record is stdout has to say so, or a half-record reads as complete.
    #[test]
    fn a_missing_data_directory_is_named_rather_than_swallowed() {
        let rec = Recorder::to(None);
        assert!(rec.file.is_none());
        let why = rec.why_no_file.expect("a reason");
        assert!(why.contains("RIVET_APP_DATA"), "{why}");
    }

    /// `/proc/uptime` is read, not guessed. On a host that has it the answer
    /// parses as a number; on one that does not, the word says which.
    #[test]
    fn mono_reports_the_clock_or_says_it_could_not() {
        let m = mono();
        assert!(
            m.parse::<f64>().is_ok() || m == "unknown" || m == "unparsable",
            "mono() answered {m:?}, which is neither a reading nor a named absence"
        );
    }
}
