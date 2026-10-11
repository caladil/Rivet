//! **Asking pid 1 to stop this machine — and refusing to ask anything else.**
//!
//! `ADR-017` §3.1.10 gave pid 1 three signals and `cmd/rivet-initd`'s `pid1`
//! module carries the table that is the contract. `rivet_core::operation::stop`
//! is the operation an operator reaches; this module is the adapter under it, and
//! it is where the one decision that is not the domain's lives.
//!
//! | Domain asks for | This sends to pid 1 | pid 1 asks the kernel for |
//! | :-- | :-- | :-- |
//! | [`Stop::PowerOff`] | `SIGTERM` | `RB_POWER_OFF` |
//! | [`Stop::Restart`] | `SIGINT` | `RB_AUTOBOOT` |
//! | [`Stop::TrialRestart`] | `SIGUSR1` | `LINUX_REBOOT_CMD_RESTART2` with `"0 tryboot"` |
//!
//! # The interlock, which is the reason this module is not four lines
//!
//! **`kill(1, SIGTERM)` on a development host signals the host's own init.** This
//! binary is built for the board and also compiled, tested and — on a Linux
//! workstation or under WSL — *runnable* on the machine somebody is working on.
//! An agent or a maintainer running `rivet apply power-off confirm` in a
//! checkout must not be able to shut that machine down, and `AGENTS.md` §9 is
//! explicit that the guard has to name the target rather than observe the
//! surroundings:
//!
//! > **Name the target explicitly; do not rely on an ambient check.** Naming the
//! > device/serial/instance is the guard.
//!
//! The target here *is* pid 1 — there is only one and its number is fixed — so
//! naming it is not available and the reading has to be. What is done instead is
//! the strongest available form of the same idea: **pid 1 must identify itself as
//! Rivet's before anything is sent.** [`PROC_1_COMM`] is read at the point of
//! use and must be exactly [`INIT_COMM`].
//!
//! **It fails closed.** An unreadable `/proc/1/comm` is a refusal, not a
//! shrug — an error and a true negative are indistinguishable
//! (`AGENTS.md` §7), and the direction that is safe when they are confused is
//! *refuse*.
//!
//! **It runs on the plan path as well as the confirmed one.** A machine this
//! build may not stop refuses before it has described a shutdown, so an agent
//! never sees a plan for something that was never going to happen.
//!
//! # Why `comm` is the gate and `cmdline` is only corroboration
//!
//! Both are read and both reach the operator, because a refusal that says *what
//! it found* is the difference between "this is a workstation" and "the board is
//! broken". Only `comm` decides:
//!
//! * `/proc/1/comm` is the kernel's own name for the process — 15 characters plus
//!   a newline, set from the executable at `exec` and not chosen by anything in
//!   userspace after it. `rivet-initd` is twelve characters, and
//!   `the_name_this_interlock_matches_survives_the_kernels_truncation` pins that.
//! * `/proc/1/cmdline` is argv, which is a *claim* rather than a reading. It is
//!   also the one that is empty for a kernel thread. Making it part of the gate
//!   would add a way for the interlock to refuse a real board without adding a
//!   way for it to catch a development host, because every development host fails
//!   the `comm` check already — `systemd`, `init`, `launchd`, or WSL's `/init`.
//!
//! So it is reported and not relied on, which is `AGENTS.md` §9's distinction
//! between a guard and its corroboration applied honestly rather than as a slogan.
//!
//! # What this module is, in `AGENTS.md` §3.1's terms
//!
//! [`InitProcess`] is **an adapter's own I/O seam**, not a domain port: it names
//! `/proc`, a signal and a bare errno, none of which `rivet-core` may say. The
//! domain port is `rivet_core::operation::machine::Machine`, and `crate::boot`
//! is where the two meet — one composition root, three seams, and the platform
//! behind all of them.
//!
//! # What is unproven
//!
//! **No Rivet board has ever been stopped by anything.** Every test below drives
//! a fake or reads this host's own `/proc`, and the only thing the second kind can
//! prove is the half that must refuse. That a `SIGUSR1` delivered to a real
//! `rivet-initd` produces a `tryboot` boot is `ADR-026` §4's untouchable column
//! and is one experiment away — see `rivet_core::operation::trial`.

use rivet_adapter_image::Errno;
use rivet_core::operation::{MachineError, Stop};

/// The kernel's name for the process this build is willing to signal.
///
/// **Thirteen characters**, which matters: `/proc/<pid>/comm` is `TASK_COMM_LEN`
/// (16) minus the NUL, so a name of more than fifteen would be truncated there
/// and the interlock would refuse the very board it is meant to permit. A test
/// pins it rather than a comment.
///
/// **It was `rivet-init` — twelve characters — until 2026-08-24**, when PID 1
/// was renamed to match the `-d` of `netd`/`timed`. The old name is now a
/// near-miss this interlock must refuse, and the test below carries it as one.
/// A card built before that rename runs a PID 1 whose `comm` is the old name,
/// so **this build refuses to signal it** — which is the interlock working, not
/// a defect. Rebuild the card.
pub const INIT_COMM: &str = "rivet-initd";

/// The longest name `/proc/<pid>/comm` can carry — `TASK_COMM_LEN - 1`.
pub const COMM_MAX: usize = 15;

/// **The gate on the gate, and it fails at compile time.**
///
/// If [`INIT_COMM`] ever grew past [`COMM_MAX`], the kernel would truncate what
/// `/proc/1/comm` reports and this interlock would refuse **every board** while
/// every test that supplies its own bytes carried on passing. `AGENTS.md` §4 asks
/// for exactly this shape — *"a cheap guard that fails at compile time"* — for a
/// constant whose real check can only run on hardware.
const _: () = assert!(
    INIT_COMM.len() <= COMM_MAX,
    "rivet-initd's name must fit in /proc/<pid>/comm, or no board is ever recognised"
);

/// The kernel's name for pid 1. **The gate.**
pub const PROC_1_COMM: &str = "/proc/1/comm";

/// Pid 1's argv, NUL-separated. **Corroboration, never the gate** — see the
/// module documentation.
pub const PROC_1_CMDLINE: &str = "/proc/1/cmdline";

/// How much of pid 1's command line is quoted back in a refusal.
///
/// A refusal goes to stderr in a `-c` session, so it is
/// bounded rather than however long somebody's boot line happens to be.
const CMDLINE_QUOTED: usize = 160;

/// The three signals `ADR-017` §3.1.10's table names, **and no others**.
///
/// A closed enum rather than an `i32` on purpose: [`crate::syscall::kill_init`]
/// takes one of these and there is therefore no way for anything in this crate to
/// ask the kernel to send pid 1 an arbitrary signal. The pid is a literal in that
/// function for the same reason.
///
/// **These are the kernel's names, not the domain's.** `SIGUSR1` means *shut down
/// and restart into the armed slot* on this system and means that nowhere else;
/// `ADR-017` §3.1.10 calls it *"a new contract"* and puts the sentence at
/// `cmd/rivet-initd`'s `Sig::User1`. Nothing in this image may send it for any
/// other reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitSignal {
    /// `SIGTERM`.
    Term,
    /// `SIGINT` — the signal the kernel itself sends pid 1 for Ctrl-Alt-Delete.
    Interrupt,
    /// `SIGUSR1` — `ADR-017` §3.1.10's third signal.
    User1,
}

impl InitSignal {
    /// The one place the domain's request becomes a signal.
    ///
    /// **Total and exhaustive**: a fourth [`Stop`] would not compile until it had
    /// a signal, and a signal pid 1 does not block would kill it —
    /// `Kernel panic - not syncing: Attempted to kill init!` — which is the
    /// hazard `ADR-017` §3.1.10 spends a section on.
    pub const fn of(stop: Stop) -> Self {
        match stop {
            Stop::PowerOff => Self::Term,
            Stop::Restart => Self::Interrupt,
            Stop::TrialRestart => Self::User1,
        }
    }

    /// The name an operator reads on the console when pid 1 announces it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Term => "SIGTERM",
            Self::Interrupt => "SIGINT",
            Self::User1 => "SIGUSR1",
        }
    }
}

impl core::fmt::Display for InitSignal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// **The adapter's own I/O seam** — reading `/proc` and signalling pid 1.
///
/// Two methods, deliberately: reading a path and sending one of three signals is
/// the whole of what stopping a machine needs from the platform. Every branch
/// below is therefore reachable from a test on a development host, including the
/// ones a board should never be asked to produce.
pub trait InitProcess {
    /// Read a `/proc` file whole. Bytes rather than text, because
    /// [`PROC_1_CMDLINE`] is NUL-separated and is not a string.
    fn read_proc(&mut self, path: &str) -> Result<Vec<u8>, i32>;

    /// `kill(1, signal)`.
    ///
    /// **The pid is not a parameter.** There is exactly one process this may
    /// reach, and an implementor that let a caller choose would be a way to
    /// signal something else with this interlock's blessing.
    fn signal_init(&mut self, signal: InitSignal) -> Result<(), i32>;
}

/// What pid 1 turned out to be, as it was read.
///
/// Carried out of the interlock so that a refusal can quote it — `AGENTS.md` §5:
/// *report what the system said, not what it was expected to say.*
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pid1 {
    /// `/proc/1/comm`, with its trailing newline removed.
    pub comm: String,
    /// `/proc/1/cmdline`, NULs rendered as spaces and bounded — or `None` when it
    /// could not be read, which is not by itself a refusal.
    pub cmdline: Option<String>,
}

impl Pid1 {
    /// Whether this is a machine Rivet may stop.
    pub fn is_rivet(&self) -> bool {
        self.comm == INIT_COMM
    }

    /// How to describe it in a refusal, both readings at once.
    fn described(&self) -> String {
        match &self.cmdline {
            Some(cmdline) => format!("`{}` (command line: `{cmdline}`)", self.comm),
            None => format!("`{}` (its command line could not be read)", self.comm),
        }
    }
}

/// **Read pid 1 and decide whether this machine may be stopped. Sends nothing.**
///
/// The refusal is [`MachineError::Unsafe`] in both directions, which
/// `rivet_core::operation::stop` maps to `E_UNSAFE`: the operator's next step
/// is the same either way — *stop, and run this on the board* — and `ADR-006`
/// §4.1's test for a code is whether it names a different next action.
pub fn identify(init: &mut dyn InitProcess) -> Result<Pid1, MachineError> {
    // Fail closed. An unreadable /proc is not evidence that this is a Rivet
    // machine, and the direction that is safe when a reading is missing is to
    // refuse — `AGENTS.md` §7: an error and a true negative are
    // indistinguishable, so never conclude from a single failed probe.
    let comm = init.read_proc(PROC_1_COMM).map_err(|errno| {
        MachineError::Unsafe(format!(
            "refusing to stop this machine: {PROC_1_COMM} could not be read ({}). This build \
             signals pid 1 to stop a machine (ADR-017 §3.1.5 item 6), and it will not do that \
             on a machine whose pid 1 it cannot identify — on a development host that is the \
             host's own init and it would stop the machine you are working on. NOTHING WAS \
             SENT.",
            Errno(errno)
        ))
    })?;

    let read = Pid1 {
        comm: text(&comm),
        cmdline: init.read_proc(PROC_1_CMDLINE).ok().map(|raw| bounded(&raw)),
    };

    if !read.is_rivet() {
        return Err(MachineError::Unsafe(format!(
            "refusing to stop this machine: pid 1 is {found}, not `{INIT_COMM}`. Rivet stops a \
             machine by signalling its own pid 1 (ADR-017 §3.1.5 item 6, §3.1.10), so signalling \
             this one would stop whatever host you are on rather than a board. NOTHING WAS SENT. \
             {ask} The command is `apply <subject> confirm`.",
            found = read.described(),
            ask = rivet_core::control::ASK_A_BOARD,
        )));
    }
    Ok(read)
}

/// **Ask pid 1 to stop this machine.** The interlock runs first, every time.
///
/// `Ok(())` means the signal was delivered, which is all `kill(2)` can report.
/// What happens next is pid 1's and it happens on the console — see
/// `rivet_core::operation::machine::Machine::request_stop`.
pub fn request(init: &mut dyn InitProcess, stop: Stop) -> Result<(), MachineError> {
    identify(init)?;
    let signal = InitSignal::of(stop);
    init.signal_init(signal).map_err(|errno| {
        // What the system said. A refused `kill(2)` to pid 1 is EPERM on almost
        // any machine where this is not root, and saying so beats inventing a
        // diagnosis (`AGENTS.md` §5).
        MachineError::Refused(format!(
            "pid 1 is `{INIT_COMM}` and the interlock permitted this, but kill(1, {signal}) \
             failed: {}. The machine has not been asked to stop.",
            Errno(errno)
        ))
    })
}

/// A `/proc` string field: UTF-8 where it is, with the trailing newline gone.
///
/// Lossy rather than refusing, because what this produces is *reported* and a
/// refusal that could not name what it found would be worse than one naming
/// mojibake. The comparison against [`INIT_COMM`] is unaffected: a name with a
/// replacement character in it is not `rivet-initd` and must not be.
fn text(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .trim_end_matches('\n')
        .to_string()
}

/// `/proc/1/cmdline`, made readable and bounded.
fn bounded(raw: &[u8]) -> String {
    let joined: String = String::from_utf8_lossy(raw)
        .split('\0')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if joined.chars().count() > CMDLINE_QUOTED {
        let head: String = joined.chars().take(CMDLINE_QUOTED).collect();
        return format!("{head}…");
    }
    joined
}

// ---------------------------------------------------------------------------
// The platform.
// ---------------------------------------------------------------------------

/// [`InitProcess`] over Linux's `/proc` and `crate::syscall`.
///
/// Stateless: every fact it deals in is read at the point of use, which is
/// `AGENTS.md` §5 — *the kernel is the authority for system state; never cache
/// its verdicts.* Caching pid 1's identity would be caching the one verdict this
/// module exists to take.
#[cfg(target_os = "linux")]
pub struct ProcInit;

#[cfg(target_os = "linux")]
impl InitProcess for ProcInit {
    fn read_proc(&mut self, path: &str) -> Result<Vec<u8>, i32> {
        std::fs::read(path).map_err(|e| crate::syscall::errno_of(&e))
    }

    fn signal_init(&mut self, signal: InitSignal) -> Result<(), i32> {
        crate::syscall::kill_init(signal)
    }
}

/// An init this build cannot reach, which **refuses** rather than pretending —
/// `AGENTS.md` §3.
///
/// The same posture `crate::boot::Detached` takes, and compiled on the same terms:
/// on a platform that is not Linux it is the implementation, and under
/// `cfg(test)` it is a contract with tests.
#[cfg(any(test, not(target_os = "linux")))]
pub struct NoInit {
    platform: &'static str,
}

#[cfg(any(test, not(target_os = "linux")))]
impl NoInit {
    pub const fn new(platform: &'static str) -> Self {
        Self { platform }
    }
}

#[cfg(any(test, not(target_os = "linux")))]
impl InitProcess for NoInit {
    fn read_proc(&mut self, _path: &str) -> Result<Vec<u8>, i32> {
        // ENOSYS: this platform has no /proc at all. It reaches the operator as
        // the fail-closed refusal above, naming the file it could not read.
        Err(38)
    }

    fn signal_init(&mut self, _signal: InitSignal) -> Result<(), i32> {
        // Unreachable through `request`, which identifies first and cannot get
        // past a refused read. Written as a refusal anyway: a stub that returned
        // `Ok(())` would report a machine asked to stop on a platform with no
        // pid 1 (`AGENTS.md` §3).
        let _ = self.platform;
        Err(38)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    /// A pid 1 that answers as scripted and records every signal it was sent.
    ///
    /// **Nothing in this file signals a real process.** The one test that touches
    /// the real system reads `/proc` and stops there.
    struct FakeInit {
        comm: Result<Vec<u8>, i32>,
        cmdline: Result<Vec<u8>, i32>,
        signal_result: Result<(), i32>,
        sent: Vec<InitSignal>,
        read: Vec<String>,
    }

    impl FakeInit {
        /// A Rivet board.
        fn board() -> Self {
            Self {
                comm: Ok(b"rivet-initd\n".to_vec()),
                cmdline: Ok(b"/system/bin/rivet-initd\0".to_vec()),
                signal_result: Ok(()),
                sent: Vec::new(),
                read: Vec::new(),
            }
        }

        /// A development host. This is what an agent is actually running on.
        fn workstation(comm: &str, cmdline: &str) -> Self {
            Self {
                comm: Ok(format!("{comm}\n").into_bytes()),
                cmdline: Ok(cmdline.replace(' ', "\0").into_bytes()),
                signal_result: Ok(()),
                sent: Vec::new(),
                read: Vec::new(),
            }
        }
    }

    impl InitProcess for FakeInit {
        fn read_proc(&mut self, path: &str) -> Result<Vec<u8>, i32> {
            self.read.push(path.to_string());
            match path {
                PROC_1_COMM => self.comm.clone(),
                PROC_1_CMDLINE => self.cmdline.clone(),
                other => panic!("nothing may read {other}"),
            }
        }

        fn signal_init(&mut self, signal: InitSignal) -> Result<(), i32> {
            self.sent.push(signal);
            self.signal_result
        }
    }

    // -- the interlock ---------------------------------------------------------

    /// **The refusal this whole module exists for.** Every pid 1 a development
    /// machine actually has, refused by name, with nothing sent.
    #[test]
    fn a_pid_one_that_is_not_rivets_is_refused_and_nothing_is_signalled() {
        for (comm, cmdline) in [
            (
                "systemd",
                "/usr/lib/systemd/systemd --switched-root --system --deserialize 31",
            ),
            ("init", "/init"),
            ("bash", "/bin/bash"),
            ("launchd", "/sbin/launchd"),
            ("rivet", "/system/bin/rivet"),
            ("rivet-sysd", "/system/bin/rivet-sysd"),
            ("", ""),
        ] {
            let mut init = FakeInit::workstation(comm, cmdline);
            for stop in Stop::ALL {
                let refusal = request(&mut init, *stop)
                    .expect_err("a machine that is not a Rivet board must be refused");
                assert!(
                    matches!(refusal, MachineError::Unsafe(_)),
                    "{comm}: {refusal:?}"
                );
                assert!(refusal.why().contains(INIT_COMM), "{comm}: {refusal}");
                assert!(
                    refusal.why().contains("NOTHING WAS SENT"),
                    "{comm}: {refusal}"
                );
            }
            assert!(init.sent.is_empty(), "{comm} was signalled {:?}", init.sent);
        }
    }

    /// And the refusal **says what it found**, so an operator can tell a
    /// workstation from a broken board without reading the source.
    #[test]
    fn the_refusal_quotes_the_name_and_the_command_line_it_read() {
        let mut init = FakeInit::workstation("systemd", "/usr/lib/systemd/systemd --system");
        let refusal = identify(&mut init).expect_err("systemd is not rivet-initd");
        assert!(refusal.why().contains("systemd"), "{refusal}");
        assert!(
            refusal.why().contains("/usr/lib/systemd/systemd --system"),
            "{refusal}"
        );
        assert_eq!(init.read, vec![PROC_1_COMM, PROC_1_CMDLINE]);
    }

    /// **Fail closed.** A `/proc` that cannot be read is a refusal, not a
    /// shrug — and the refusal names the file, because "permission denied" with
    /// no path is the kind of message that gets diagnosed as a broken board.
    #[test]
    fn an_unreadable_proc_refuses_rather_than_assuming_either_answer() {
        for errno in [1, 2, 13, 0] {
            let mut init = FakeInit::board();
            init.comm = Err(errno);
            let refusal = request(&mut init, Stop::PowerOff).expect_err("fail closed");
            assert!(matches!(refusal, MachineError::Unsafe(_)), "{errno}");
            assert!(refusal.why().contains(PROC_1_COMM), "{errno}: {refusal}");
            assert!(
                refusal.why().contains("NOTHING WAS SENT"),
                "{errno}: {refusal}"
            );
            assert!(init.sent.is_empty(), "{errno} was signalled anyway");
        }
    }

    /// An unreadable *command line* is not a refusal: it is corroboration, and
    /// the module documentation says why making it a gate would refuse boards
    /// without catching hosts.
    #[test]
    fn an_unreadable_command_line_does_not_refuse_a_board_that_names_itself() {
        let mut init = FakeInit::board();
        init.cmdline = Err(2);
        let read = identify(&mut init).expect("comm is the gate");
        assert_eq!(read.comm, INIT_COMM);
        assert_eq!(read.cmdline, None);
        assert!(
            read.described().contains("could not be read"),
            "{}",
            read.described()
        );
    }

    /// **The name is matched exactly.** Every near-miss here is a real process
    /// name on some machine, and each of them signalling pid 1 would be this
    /// interlock failing open.
    ///
    /// **`rivet-init` is in the list on purpose**: it was [`INIT_COMM`] until
    /// the 2026-08-24 rename, so it is exactly the near-miss most likely to be
    /// met in the wild — a card built before that day. Refusing it is correct.
    #[test]
    fn only_the_exact_name_is_accepted_and_no_prefix_or_suffix_is() {
        for name in [
            "rivet-init",
            "rivet-initd2",
            "rivet-initdd",
            "Rivet-Initd",
            " rivet-initd",
            "rivet-initd ",
            "rivet_initd",
            "rivet",
        ] {
            let mut init = FakeInit::workstation(name, "/x");
            assert!(
                identify(&mut init).is_err(),
                "{name:?} was accepted as Rivet's pid 1"
            );
        }
    }

    /// **The kernel truncates `comm` at fifteen characters.** A name longer than
    /// that could never be matched, and the interlock would refuse every board
    /// while looking correct in every unit test that supplied its own bytes.
    #[test]
    fn the_name_this_interlock_matches_survives_the_kernels_truncation() {
        assert!(
            INIT_COMM.len() <= COMM_MAX,
            "{INIT_COMM} is {} bytes; /proc/<pid>/comm carries at most {COMM_MAX}",
            INIT_COMM.len()
        );
    }

    /// The trailing newline `/proc/<pid>/comm` carries is not part of the name.
    /// Forgetting it is how this interlock refuses every board.
    #[test]
    fn the_newline_proc_appends_is_not_part_of_the_name() {
        assert_eq!(text(b"rivet-initd\n"), INIT_COMM);
        assert_eq!(text(b"rivet-initd"), INIT_COMM);
        assert_eq!(text(b""), "");
    }

    // -- the accept path, which signals nothing real ---------------------------

    /// **The accept path.** A pid 1 that names itself is signalled — with the one
    /// signal `ADR-017` §3.1.10's table gives that request, and no other.
    #[test]
    fn a_rivet_pid_one_is_sent_exactly_the_signal_its_request_maps_to() {
        for (stop, expected) in [
            (Stop::PowerOff, InitSignal::Term),
            (Stop::Restart, InitSignal::Interrupt),
            (Stop::TrialRestart, InitSignal::User1),
        ] {
            let mut init = FakeInit::board();
            assert_eq!(request(&mut init, stop), Ok(()), "{stop}");
            assert_eq!(init.sent, vec![expected], "{stop}");
            // The identity is read every time, never once and remembered.
            assert_eq!(init.read, vec![PROC_1_COMM, PROC_1_CMDLINE]);
        }
    }

    /// **The three must not collapse.** A `trial-restart` that sent `SIGINT`
    /// would reboot the board onto the default slot, trial nothing, and look
    /// exactly like a success.
    #[test]
    fn the_three_requests_map_onto_three_distinct_signals() {
        let signals: Vec<InitSignal> = Stop::ALL.iter().map(|s| InitSignal::of(*s)).collect();
        assert_eq!(
            signals,
            vec![InitSignal::Term, InitSignal::Interrupt, InitSignal::User1]
        );
        for (i, a) in signals.iter().enumerate() {
            for b in signals.iter().skip(i + 1) {
                assert_ne!(a, b, "two requests send the same signal");
            }
            assert!(!a.as_str().is_empty());
            assert_eq!(a.to_string(), a.as_str());
        }
    }

    /// A `kill(2)` that fails is the *machine* refusing, not the request being
    /// wrong — a different code and a different next step for the operator.
    #[test]
    fn a_refused_kill_is_reported_as_the_machine_with_what_the_system_said() {
        let mut init = FakeInit::board();
        init.signal_result = Err(1); // EPERM
        let refusal = request(&mut init, Stop::TrialRestart).expect_err("EPERM");
        assert!(matches!(refusal, MachineError::Refused(_)), "{refusal:?}");
        assert!(refusal.why().contains("SIGUSR1"), "{refusal}");
        assert!(
            refusal.why().contains("has not been asked to stop"),
            "{refusal}"
        );
    }

    /// `identify` alone must never signal — it is what the plan path calls, and a
    /// dry run that stopped the machine would be the worst defect in this crate.
    #[test]
    fn identifying_a_machine_never_signals_it_even_when_it_is_a_board() {
        let mut init = FakeInit::board();
        assert!(identify(&mut init).is_ok());
        assert!(init.sent.is_empty(), "the dry run signalled pid 1");
    }

    // -- the command line, as it is rendered -----------------------------------

    #[test]
    fn a_nul_separated_command_line_becomes_readable_and_drops_the_trailing_nuls() {
        assert_eq!(bounded(b"/init\0"), "/init");
        assert_eq!(
            bounded(b"/sbin/init\0splash\0quiet\0"),
            "/sbin/init splash quiet"
        );
        assert_eq!(bounded(b""), "");
    }

    /// Bounded, because this lands on stderr in a session where stdout is
    /// load-bearing and a boot line can be very long.
    #[test]
    fn a_very_long_command_line_is_truncated_rather_than_printed_whole() {
        let long: Vec<u8> = std::iter::repeat_n(b'x', 4096).collect();
        let rendered = bounded(&long);
        assert!(
            rendered.chars().count() <= CMDLINE_QUOTED + 1,
            "{}",
            rendered.len()
        );
        assert!(rendered.ends_with('…'));
    }

    // -- the real machine this is running on -----------------------------------

    /// **The interlock, against this host's own `/proc`.**
    ///
    /// Every other test in this file supplies its own bytes and therefore proves
    /// only that the comparison works. This one reads the actual kernel, which is
    /// the only way to know that the *path* is right, that `comm` carries what it
    /// is believed to carry, and that a build machine is refused.
    ///
    /// It **reads and stops**. `identify` sends nothing, and no test in this
    /// workspace calls `request` against a real `InitProcess`.
    ///
    /// It is written to be meaningful wherever it runs rather than to assume a
    /// workstation: if this host's pid 1 ever *is* `rivet-initd`, the assertion
    /// flips rather than the test becoming vacuous.
    #[cfg(target_os = "linux")]
    #[test]
    fn this_hosts_own_pid_one_is_read_and_the_verdict_matches_what_it_says() {
        let mut real = ProcInit;
        let comm = real.read_proc(PROC_1_COMM);
        let Ok(bytes) = comm else {
            // No readable /proc/1/comm — a container with a hidden proc, say.
            // The interlock must then refuse, which is the fail-closed half.
            assert!(
                identify(&mut ProcInit).is_err(),
                "an unreadable /proc/1/comm must refuse"
            );
            return;
        };
        let name = text(&bytes);
        assert!(!name.is_empty(), "/proc/1/comm was empty");
        let verdict = identify(&mut ProcInit);
        if name == INIT_COMM {
            assert!(
                verdict.is_ok(),
                "pid 1 is rivet-initd and was refused: {verdict:?}"
            );
        } else {
            let refusal = verdict.expect_err(&format!(
                "pid 1 on this host is {name:?} and the interlock permitted stopping it"
            ));
            assert!(matches!(refusal, MachineError::Unsafe(_)), "{refusal:?}");
            assert!(refusal.why().contains(&name), "{refusal}");
        }
    }

    /// The stand-in for a platform with no `/proc` refuses both calls and never
    /// says `Ok` — `AGENTS.md` §3.
    #[test]
    fn a_build_with_no_proc_refuses_to_read_and_refuses_to_signal() {
        let mut none = NoInit::new("windows");
        assert!(none.read_proc(PROC_1_COMM).is_err());
        assert!(none.signal_init(InitSignal::Term).is_err());
        let refusal = request(&mut none, Stop::PowerOff).expect_err("no /proc, no stopping");
        assert!(matches!(refusal, MachineError::Unsafe(_)), "{refusal:?}");
    }
}
