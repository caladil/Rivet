//! **Finding the supervisor and asking it to re-read.** `ADR-032` §3.2, the
//! adapter's half of `rivet_core::operation::reload`.
//!
//! `crate::shutdown` is the sibling and the shape is deliberately its: a seam of
//! two or three platform calls, an interlock that **decides** above them, and
//! every branch reachable from a test on a development host.
//!
//! # Why this signals the supervisor and not pid 1
//!
//! `ADR-017` §3.1.10 chose `SIGUSR1` for a trial boot and gave the reason in
//! passing:
//!
//! > `SIGHUP` conventionally means *reload configuration*, and a system that
//! > overloaded it would eventually meet something that sends `SIGHUP` meaning
//! > the conventional thing — at which point the board reboots into an untested
//! > slot.
//!
//! So the word is reserved for what it conventionally means, and the process
//! that re-reads the document is the supervisor. Routing it through pid 1 would
//! mean **a fourth row in §3.1.10's table** — a section that calls each row *"a
//! new contract and it is written down as one"* — for no gain: pid 1 would
//! forward it unchanged, and the thing it protects (a signal pid 1 has not
//! blocked panics the kernel) is not at stake when the target is an ordinary
//! process.
//!
//! # The interlock, and why it is two conditions rather than one
//!
//! **Both `pid 1` is `rivet-initd` AND the target is its child named
//! `rivet-sysd`.** Either alone is weaker than it looks:
//!
//! * A process named `rivet-sysd` could be anything on a development host — a
//!   `cargo test` binary, an editor's file watcher holding the name in `argv`.
//!   `/proc/<pid>/comm` is the kernel's name and harder to spoof than `argv`,
//!   but it is still not evidence about **this machine**.
//! * pid 1 being `rivet-initd` says the machine is a Rivet and says nothing
//!   about which of its children is the supervisor.
//!
//! Together they say *this is a Rivet board and that is the process pid 1
//! started to supervise it*, which is exactly the claim being relied on.
//!
//! **And more than one match is a refusal**, never a choice. `AGENTS.md` §5 —
//! refuse rather than guess — and the guess here signals a process this build
//! cannot identify.
//!
//! # `hangup` takes a pid and `crate::syscall::kill_init` does not
//!
//! Stated because it is a real weakening of that module's rule and it is
//! deliberate. `kill_init`'s pid is the literal `1` precisely so that no caller
//! can choose one; here the pid is the whole question, so the guard moves to
//! **which pid may be produced**: [`find`] is the only thing that returns one,
//! it returns it only when both conditions above hold, and the signal is not a
//! parameter — there is exactly one this may send.

use rivet_core::operation::MachineError;

use crate::shutdown::{INIT_COMM, PROC_1_COMM};

/// `/proc/<pid>/comm` for the supervisor. `ADR-017` §2.1's name for it, and the
/// name `cmd/rivet-sysd` is installed under.
///
/// **Thirteen characters**, which matters for the same reason `INIT_COMM`'s
/// length does: `/proc/<pid>/comm` is `TASK_COMM_LEN` and truncates at 15, so a
/// longer name would be reported cut and this comparison would never match.
pub const SUPERVISOR_COMM: &str = "rivet-sysd";

const _: () = assert!(
    SUPERVISOR_COMM.len() <= 15,
    "the supervisor's name must fit in /proc/<pid>/comm, or no board is ever recognised"
);

/// **The adapter's own I/O seam** — reading `/proc`, listing it, and one signal.
///
/// Three methods, and the third is the one that differs from
/// [`crate::shutdown::InitProcess`]: see the module documentation for why a pid
/// is a parameter here and what replaces the guard that gives up.
pub trait Supervisor {
    /// Read a `/proc` file whole. Bytes, for [`crate::shutdown::InitProcess`]'s
    /// reason — some of these files are NUL-separated and are not strings.
    fn read_proc(&mut self, path: &str) -> Result<Vec<u8>, i32>;

    /// Every process id `/proc` currently lists.
    ///
    /// **A snapshot, and it may be stale by the time it is used.** That is not a
    /// defect to engineer around: a pid that exits between this and the signal
    /// produces `ESRCH`, which [`request`] reports as what the system said. The
    /// hazard worth naming is pid *reuse*, and the window here is one read of
    /// `/proc/<pid>/comm` — narrow enough that the alternative (a pidfd) would
    /// be a platform dependency bought for a race nothing has observed.
    fn pids(&mut self) -> Result<Vec<i32>, i32>;

    /// `kill(pid, SIGHUP)`.
    ///
    /// **The signal is not a parameter.** There is exactly one this may send,
    /// which is the half of `kill_init`'s guard that survives here.
    fn hangup(&mut self, pid: i32) -> Result<(), i32>;
}

/// **Find the supervisor. Signals nothing.**
///
/// Both conditions, in the order that fails cheapest: pid 1 first, because one
/// read settles every development host, and only then the scan.
pub fn find(sup: &mut dyn Supervisor) -> Result<i32, MachineError> {
    // Fail closed, exactly as `crate::shutdown::identify` does: an unreadable
    // `/proc` is not evidence, and `AGENTS.md` §7 forbids concluding anything
    // from one failed probe.
    let init = sup.read_proc(PROC_1_COMM).map_err(|errno| {
        MachineError::Unsafe(format!(
            "refusing to ask this machine to reload: {PROC_1_COMM} could not be read ({}). A \
             reload is delivered to this machine's own supervisor (ADR-032 §3.2), and this build \
             will not go looking for one on a machine whose pid 1 it cannot identify. NOTHING \
             WAS SENT.",
            rivet_adapter_image::Errno(errno)
        ))
    })?;
    let init = comm(&init);
    if init != INIT_COMM {
        return Err(MachineError::Unsafe(format!(
            "refusing to ask this machine to reload: pid 1 is `{init}`, not `{INIT_COMM}`. This \
             is not a Rivet machine, so nothing on it is the supervisor a reload is for. NOTHING \
             WAS SENT. {ask} The command is `apply reload confirm`.",
            ask = rivet_core::control::ASK_A_BOARD,
        )));
    }

    let pids = sup.pids().map_err(|errno| {
        MachineError::Refused(format!(
            "pid 1 is `{INIT_COMM}`, so this is a Rivet machine, but /proc could not be listed \
             ({}) and the supervisor cannot be found. NOTHING WAS SENT.",
            rivet_adapter_image::Errno(errno)
        ))
    })?;

    let scanned = pids.len();
    let mut found: Vec<i32> = Vec::new();
    // **What was seen, for the refusal to quote.** `AGENTS.md` §5 — report what
    // the system said — and it earned its place immediately: the first run of
    // this against a real machine refused, and *"there is no rivet-sysd"* said
    // nothing about which of the two conditions failed or what was there
    // instead. A refusal that cannot be acted on costs a whole run to diagnose.
    let mut rivets: Vec<String> = Vec::new();
    for pid in pids {
        // A process that vanished between the listing and this read is simply
        // not a match — `Err` here is ordinary rather than a fault, and it is
        // the common case on a busy machine.
        let Ok(raw) = sup.read_proc(&format!("/proc/{pid}/comm")) else {
            continue;
        };
        let name = comm(&raw);
        let parent = parent_of(sup, pid);
        if name.contains("rivet") {
            rivets.push(format!(
                "{pid} `{name}` (parent {})",
                parent.map_or_else(|| "unreadable".to_string(), |p| p.to_string())
            ));
        }
        if name == SUPERVISOR_COMM && parent == Some(1) {
            found.push(pid);
        }
    }

    match found.as_slice() {
        [pid] => Ok(*pid),
        [] => Err(MachineError::Refused(format!(
            "pid 1 is `{INIT_COMM}`, so this is a Rivet machine, and none of the {scanned} \
             processes in /proc is a `{SUPERVISOR_COMM}` running as a child of pid 1. There is \
             nothing to ask to reload. NOTHING WAS SENT — `rivet get log` is where a supervisor \
             that failed to start said so. What IS running with `rivet` in its name: {seen}.",
            seen = if rivets.is_empty() {
                "nothing".to_string()
            } else {
                rivets.join("; ")
            },
        ))),
        many => Err(MachineError::Unsafe(format!(
            "refusing to ask this machine to reload: there are {} processes named \
             `{SUPERVISOR_COMM}` running as children of pid 1 ({}), and this build will not \
             choose between them (AGENTS.md §5 — refuse rather than guess). A Rivet machine has \
             exactly one supervisor (ADR-017 §3.1), so this machine is in a state nothing here \
             describes. NOTHING WAS SENT.",
            many.len(),
            many.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        ))),
    }
}

/// **Ask the supervisor to re-read the document.** The interlock runs first.
///
/// `Ok(())` means the signal was delivered, which is all `kill(2)` can report —
/// and `ADR-032` §3.2's own limit is sharper: the signal cannot carry a reply at
/// all, so even a delivered one says nothing about what the reload did.
pub fn request(sup: &mut dyn Supervisor) -> Result<(), MachineError> {
    let pid = find(sup)?;
    sup.hangup(pid).map_err(|errno| {
        MachineError::Refused(format!(
            "the supervisor is pid {pid} and the interlock permitted this, but kill({pid}, \
             SIGHUP) failed: {}. The document has NOT been re-read.",
            rivet_adapter_image::Errno(errno)
        ))
    })
}

/// A `/proc/<pid>/comm` reading: lossy UTF-8, trailing newline gone.
///
/// Lossy for `crate::shutdown::text`'s reason — what this produces is reported,
/// and a name with a replacement character in it is not `rivet-sysd` and must
/// not be.
fn comm(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .trim_end_matches('\n')
        .to_string()
}

/// The parent pid out of `/proc/<pid>/stat`, or `None`.
///
/// **Parsed from the last `)` rather than by splitting on spaces**, and that is
/// not fussiness: field 2 is the executable name **in parentheses and not
/// escaped**, so a process called `rivet sysd)` would shift every field after it.
/// The kernel guarantees the last `)` closes that field, which is why every
/// careful reader of this file works backwards from it.
fn parent_of(sup: &mut dyn Supervisor, pid: i32) -> Option<i32> {
    let raw = sup.read_proc(&format!("/proc/{pid}/stat")).ok()?;
    let text = String::from_utf8_lossy(&raw);
    let after = text.rsplit_once(')')?.1;
    // What follows is " S ppid …": state, then the parent.
    after.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;
    use std::collections::BTreeMap;

    /// A `/proc` in a map, and a record of what was signalled.
    #[derive(Default)]
    struct FakeProc {
        files: BTreeMap<String, Vec<u8>>,
        pids: Option<Vec<i32>>,
        signalled: Vec<i32>,
        hangup_errno: Option<i32>,
    }

    impl FakeProc {
        /// A board: pid 1 is `rivet-initd` and pid 42 is its supervisor.
        fn board() -> Self {
            let mut fake = Self {
                pids: Some(vec![1, 42, 77]),
                ..Self::default()
            };
            fake.process(1, INIT_COMM, 0);
            fake.process(42, SUPERVISOR_COMM, 1);
            fake.process(77, "rivet-mdnsd", 42);
            fake
        }

        fn process(&mut self, pid: i32, comm: &str, ppid: i32) {
            self.files
                .insert(format!("/proc/{pid}/comm"), format!("{comm}\n").into_bytes());
            self.files.insert(
                format!("/proc/{pid}/stat"),
                format!("{pid} ({comm}) S {ppid} 1 1 0 -1 4194560 1 0").into_bytes(),
            );
        }
    }

    impl Supervisor for FakeProc {
        fn read_proc(&mut self, path: &str) -> Result<Vec<u8>, i32> {
            self.files.get(path).cloned().ok_or(libc_enoent())
        }
        fn pids(&mut self) -> Result<Vec<i32>, i32> {
            self.pids.clone().ok_or(libc_enoent())
        }
        fn hangup(&mut self, pid: i32) -> Result<(), i32> {
            self.signalled.push(pid);
            self.hangup_errno.map_or(Ok(()), Err)
        }
    }

    const fn libc_enoent() -> i32 {
        2
    }

    #[test]
    fn a_board_finds_its_supervisor_and_signals_exactly_it() {
        let mut fake = FakeProc::board();
        assert_eq!(find(&mut fake), Ok(42));
        request(&mut fake).expect("a healthy board");
        assert_eq!(fake.signalled, vec![42]);
    }

    /// **A development host is refused on the first read**, and the refusal
    /// names what it found — `AGENTS.md` §5.
    #[test]
    fn a_machine_whose_pid_1_is_not_rivets_is_refused_and_nothing_is_sent() {
        let mut fake = FakeProc::board();
        fake.process(1, "systemd", 0);
        let refusal = find(&mut fake).expect_err("a host is not a board");
        assert!(matches!(refusal, MachineError::Unsafe(_)));
        assert!(refusal.why().contains("systemd"), "{}", refusal.why());
        assert!(refusal.why().contains("NOTHING WAS SENT"), "{}", refusal.why());
        assert!(fake.signalled.is_empty());
    }

    /// **Both conditions, and a process named `rivet-sysd` that is not pid 1's
    /// child is not the supervisor.** This is the case a development host
    /// actually produces: a test binary or a watcher holding the name.
    #[test]
    fn a_process_with_the_right_name_and_the_wrong_parent_is_not_the_supervisor() {
        let mut fake = FakeProc::board();
        // Re-parent the supervisor onto another service. Same name, not pid 1's child.
        fake.process(42, SUPERVISOR_COMM, 77);
        let refusal = find(&mut fake).expect_err("that is not the supervisor");
        assert!(matches!(refusal, MachineError::Refused(_)));
        // And the refusal NAMES what it saw instead — the parent that is not
        // pid 1 is the whole finding, and a refusal that withheld it would
        // leave an operator with nothing to act on.
        assert!(
            refusal.why().contains("42 `rivet-sysd` (parent 77)"),
            "{}",
            refusal.why()
        );
        assert!(fake.signalled.is_empty());
    }

    /// **Two matches is a refusal, never a choice.** A Rivet machine has one
    /// supervisor; a machine with two is in a state nothing here describes.
    #[test]
    fn more_than_one_supervisor_is_refused_rather_than_chosen_between() {
        let mut fake = FakeProc::board();
        fake.pids = Some(vec![1, 42, 43]);
        fake.process(43, SUPERVISOR_COMM, 1);
        let refusal = find(&mut fake).expect_err("it chose one");
        assert!(matches!(refusal, MachineError::Unsafe(_)));
        assert!(refusal.why().contains("42, 43"), "{}", refusal.why());
        assert!(fake.signalled.is_empty());
    }

    /// A Rivet machine whose supervisor is not running says so, and points at
    /// where it would have said why.
    #[test]
    fn a_rivet_machine_with_no_supervisor_says_so_and_names_where_to_look() {
        let mut fake = FakeProc::board();
        fake.pids = Some(vec![1, 77]);
        let refusal = find(&mut fake).expect_err("there is no supervisor");
        assert!(refusal.why().contains("rivet get log"), "{}", refusal.why());
    }

    /// **The parent is parsed from the last `)`.** Field 2 of `/proc/<pid>/stat`
    /// is an unescaped name in parentheses, so a process whose name contains one
    /// shifts every field after it for a naive splitter — and the failure would
    /// be that a real supervisor stops being recognised.
    #[test]
    fn a_process_name_containing_a_paren_does_not_shift_the_parent_field() {
        let mut fake = FakeProc::board();
        fake.files.insert(
            "/proc/42/stat".to_string(),
            b"42 (rivet-sysd) (odd) S 1 1 1 0 -1 4194560".to_vec(),
        );
        assert_eq!(parent_of(&mut fake, 42), Some(1));
    }

    /// A refused `kill(2)` reports what the system said and does not claim the
    /// document was re-read.
    #[test]
    fn a_refused_signal_says_what_the_kernel_said() {
        let mut fake = FakeProc::board();
        fake.hangup_errno = Some(1); // EPERM
        let refusal = request(&mut fake).expect_err("the kernel refused");
        assert!(matches!(refusal, MachineError::Refused(_)));
        assert!(refusal.why().contains("has NOT been re-read"), "{}", refusal.why());
    }
}

// ---------------------------------------------------------------------------
// The implementors, on the same terms `crate::shutdown`'s are
// ---------------------------------------------------------------------------

/// `/proc` and `kill(2)`, on the machine this is running on.
///
/// Stateless for [`crate::shutdown::ProcInit`]'s reason: every fact is read at
/// the point of use, and caching which pid is the supervisor would cache the one
/// verdict this module exists to take.
#[cfg(target_os = "linux")]
impl Supervisor for crate::shutdown::ProcInit {
    fn read_proc(&mut self, path: &str) -> Result<Vec<u8>, i32> {
        std::fs::read(path).map_err(|e| crate::syscall::errno_of(&e))
    }

    fn pids(&mut self) -> Result<Vec<i32>, i32> {
        let entries = std::fs::read_dir("/proc").map_err(|e| crate::syscall::errno_of(&e))?;
        Ok(entries
            .flatten()
            .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
            .collect())
    }

    fn hangup(&mut self, pid: i32) -> Result<(), i32> {
        crate::syscall::hangup(pid)
    }
}

/// A supervisor this build cannot reach, which **refuses** rather than
/// pretending — `AGENTS.md` §3, and the same posture
/// [`crate::shutdown::NoInit`] takes.
#[cfg(any(test, not(target_os = "linux")))]
impl Supervisor for crate::shutdown::NoInit {
    fn read_proc(&mut self, _path: &str) -> Result<Vec<u8>, i32> {
        Err(libc_enosys())
    }

    fn pids(&mut self) -> Result<Vec<i32>, i32> {
        Err(libc_enosys())
    }

    fn hangup(&mut self, _pid: i32) -> Result<(), i32> {
        Err(libc_enosys())
    }
}

/// `ENOSYS`, written out because this crate compiles where `libc` does not.
///
/// **The right errno rather than a convenient one**: `find` turns a failed read
/// of `/proc/1/comm` into *"this build will not go looking on a machine whose
/// pid 1 it cannot identify"*, and `ENOSYS` is the truth on a host that has no
/// `/proc` at all.
#[cfg(any(test, not(target_os = "linux")))]
const fn libc_enosys() -> i32 {
    38
}
