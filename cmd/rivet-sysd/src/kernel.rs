//! The seam between the supervisor's decisions and the operating system.
//!
//! Everything below this line is a syscall; everything above it is a decision.
//! `cmd/rivet-initd` established the pattern and the reason it exists there —
//! its branches cannot be reached from a test that needs root, a kernel and an
//! emulator — and the same is true here for a different reason: the branches
//! that matter most are *time-dependent*. Backoff expiring, a readiness
//! deadline passing, a child that will not die within its grace period. A test
//! that had to wait for those in real time would be slow enough that nobody
//! would run it, and `AGENTS.md` §4 says a suite that cannot be run rots.
//!
//! With the clock behind the port, `crate::testkernel::FakeKernel` advances
//! it by assignment and a thirty-second backoff is tested in microseconds.
//!
//! **[`LibcKernel`] must stay thin, and should get thinner rather than
//! smarter.** Whatever logic lives in it is logic no test can drive.
//!
//! ## This trait is the second one of its shape in this workspace
//!
//! `cmd/rivet-initd` has a `Kernel` trait, an `Errno`, an `ExitStatus`, a
//! `Reaped` and a fake with scripted answers. Three of those types are
//! *identical* here, and this file is where that duplication became visible
//! rather than theoretical. `ADR-013` §7 ruled that the platform crate emerges
//! at Stage 3 rather than being built speculatively, and this is exactly the
//! evidence that ruling asked to accumulate first: **`Errno`, `ExitStatus` and
//! the scripted fake want to be shared; the ports themselves do not.** PID 1
//! needs `mount(2)` and a reaper for the whole system; the supervisor needs
//! `poll(2)`, process spawning and a notification socket, and neither wants the
//! other's methods. See the report accompanying this change.
//!
//! ## What is not behind this port, and why that is deliberate
//!
//! Socket activation (`ADR-017` §3.2, §3.4 — `LISTEN_FDS`) is **not built**.
//! It requires Rivet to know which sockets a service wants, and that is a
//! service-definition question the record does not answer yet (`ADR-015` §3.2
//! has `listen` for *applications*; nothing describes it for a system service).
//! Adding a port method for it now would be guessing at its shape.

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};

// `BOOT_DISK` is deliberately NOT imported here any more (PLAN.md §62.7):
// this file writes a partition table, and the disk it writes to is now
// discovered from what is mounted rather than compiled in.
use rivet_adapter_image::{Errno as GrowErrno, Grew, GrowMedium, GrowStep};
use rivet_core::mount::MountFlag;

use crate::journal::Record;

/// Where a service's cgroup lives. `ADR-017` §3.1.3 makes cgroup membership the
/// authority for what is running, so that the supervisor observes rather than
/// remembers across its own restarts.
pub const CGROUP_ROOT: &str = "/sys/fs/cgroup/rivet";

/// The file whose readability **is** a mounted cgroup2 hierarchy.
///
/// The kernel creates `cgroup.controllers` in every directory of a cgroup2
/// filesystem, root included, and creates it nowhere else. So one `read(2)`
/// separates *this service has no cgroup yet* from *there is no cgroup2 here at
/// all* — the two facts `ADR-017` §3.1.3.1 was written to stop being reported
/// as one. `cmd/rivet-initd`'s `report_cgroup_evidence` already reads this
/// exact path, and `ADR-024` §8.1 read it on a board.
///
/// **A cgroup v1 hierarchy reads as unreachable through this**, because v1 has
/// no such file. That is the true answer for a supervisor that speaks only v2:
/// `rivet_core::mount::stage0_plan` mounts `cgroup2` and nothing here knows
/// what to do with v1's controllers.
pub const CGROUP_CONTROLLERS: &str = "/sys/fs/cgroup/cgroup.controllers";

/// Where the `sd_notify` sockets live. `/run` is a tmpfs PID 1 mounts, and
/// `rivet-core`'s stage 0 plan deliberately leaves it executable because
/// services are started from it.
pub const NOTIFY_DIR: &str = "/run/rivet/notify";

/// The kernel's boot identifier — `ADR-022` §3.2's `boot` field.
pub const BOOT_ID_PATH: &str = "/proc/sys/kernel/random/boot_id";

/// Where the kernel publishes this process's own mount table.
///
/// `/proc/self/mounts` rather than `/proc/mounts`, for the reason
/// `cmd/rivet-initd` gives: it is this process's mount namespace, which is the
/// one whose state [`crate::etc`] converges.
pub const MOUNTS_PATH: &str = "/proc/self/mounts";

/// The same table with **device numbers**, which [`MOUNTS_PATH`] does not carry.
///
/// `ADR-022` §12 item 1 needs to know which A/B slot is running, and on a board
/// `/proc/self/mounts` reports the root as `/dev/root` — a name with no
/// partition number in it. `mountinfo`'s third field is `major:minor` and is the
/// only reading of the three that answers; `PLAN.md` §26.4.1 records that
/// `/proc/cmdline` reads back empty on this hardware, and
/// `rivet_core::state::Slot::from_mountinfo` carries why hashing the running
/// binary is not a substitute.
pub const MOUNTINFO_PATH: &str = "/proc/self/mountinfo";

/// A raw OS error number, kept as data so a test can produce any of them.
///
/// Identical in shape to `cmd/rivet-initd`'s. See the note at the top of this
/// module: that is an observation for `ADR-013` §7, not an accident to tidy up
/// by having one binary depend on the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Errno(pub i32);

impl Errno {
    pub const EPERM: Self = Self(libc::EPERM);
    pub const ENOENT: Self = Self(libc::ENOENT);
    pub const EINTR: Self = Self(libc::EINTR);
    pub const ESRCH: Self = Self(libc::ESRCH);
    pub const EACCES: Self = Self(libc::EACCES);
    pub const EIO: Self = Self(libc::EIO);
    pub const EINVAL: Self = Self(libc::EINVAL);
    pub const ENOEXEC: Self = Self(libc::ENOEXEC);
    // The five the `/etc` projection classifies. Each names a state a machine
    // can actually be in — see `crate::etc`'s `mount_detail` and `write_detail`.
    pub const EBUSY: Self = Self(libc::EBUSY);
    pub const EROFS: Self = Self(libc::EROFS);
    pub const ENODEV: Self = Self(libc::ENODEV);
    pub const ENOTDIR: Self = Self(libc::ENOTDIR);
    pub const ENOSPC: Self = Self(libc::ENOSPC);
    /// A file larger than the caller was willing to read. `ADR-015` §4.2's
    /// launch verification bounds both of its reads.
    pub const EFBIG: Self = Self(libc::EFBIG);
    // The two ways a `fork(2)` fails for a reason that will pass. Named
    // 2026-08-23 with `is_a_program_this_image_cannot_run`, because that
    // classification is only meaningful if its *other* side has names too.
    pub const EAGAIN: Self = Self(libc::EAGAIN);
    pub const ENOMEM: Self = Self(libc::ENOMEM);
    /// A symlink where [`Kernel::write_file_nofollow`] required a real file.
    /// The kernel's own answer to `O_NOFOLLOW`, passed through unchanged.
    pub const ELOOP: Self = Self(libc::ELOOP);
    /// A file with more than one name where that same method required one.
    /// **This one is ours**, not the kernel's: a hard link is a legal file and
    /// the open succeeds, so the refusal is a check and `EMLINK` is the closest
    /// true thing to call it.
    pub const EMLINK: Self = Self(libc::EMLINK);
    /// `unlink(2)` on a directory. A `current` that is a real directory rather
    /// than the installer's symlink answers this, and the caller removes it as
    /// a directory instead — `crate::applications::remove_versions`.
    pub const EISDIR: Self = Self(libc::EISDIR);
    /// A one-shot program [`Kernel::run_within`] stopped at its deadline.
    pub const ETIMEDOUT: Self = Self(libc::ETIMEDOUT);
    /// The two ways `rename(2)` says the name it was given is taken —
    /// `crate::reclaim::rename_aside` moves to the next one.
    pub const EEXIST: Self = Self(libc::EEXIST);
    pub const ENOTEMPTY: Self = Self(libc::ENOTEMPTY);

    /// Whether this errno, from a spawn, means **the image is wrong** rather
    /// than **the moment is wrong**.
    ///
    /// `ADR-017` §3.2's whole model is that a service which needs something not
    /// yet ready retries, and §3.5 throttles the retries so a restart loop
    /// cannot write an SD card to death. Both assume the failure is *temporal*.
    /// A program that is not on the filesystem is not temporal: `/system` is
    /// read-only `erofs` (`ADR-018` §3), so nothing can appear there while the
    /// machine is running, and the fifth attempt will fail exactly as the first
    /// did.
    ///
    /// The four below are the ones the kernel returns when the answer cannot
    /// change, and each is the kernel's own verdict rather than a guess
    /// (`AGENTS.md` §5 — report what the system said):
    ///
    /// | | The kernel is saying |
    /// | :-- | :-- |
    /// | `ENOENT` | there is nothing at that path |
    /// | `ENOTDIR` | a component of that path is not a directory |
    /// | `EACCES` | it is there and not executable, or a directory on the way is not searchable |
    /// | `ENOEXEC` | it is there and is not something this kernel can execute — a host binary, or the wrong architecture |
    ///
    /// **`EPERM` is deliberately not here.** It is what a seccomp filter, a
    /// hardened kernel or a container runtime returns, and any of those can be
    /// a policy that changes; treating it as permanent would give up on a
    /// service that a later moment would have started.
    ///
    /// `EAGAIN` and `ENOMEM` are the clearest members of the other set: a fork
    /// that failed because the machine was momentarily out of process slots or
    /// memory is exactly what a backoff exists for.
    pub const fn is_a_program_this_image_cannot_run(self) -> bool {
        matches!(
            self,
            Self::ENOENT | Self::ENOTDIR | Self::EACCES | Self::ENOEXEC
        )
    }

    /// The errno the last syscall set.
    pub fn last() -> Self {
        Self::from_io(&std::io::Error::last_os_error())
    }

    /// An `io::Error` may carry no OS number at all. Zero is not a valid errno,
    /// so it is unambiguous as "the platform did not say".
    pub fn from_io(e: &std::io::Error) -> Self {
        Self(e.raw_os_error().unwrap_or(0))
    }
}

impl core::fmt::Display for Errno {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0 == 0 {
            return write!(f, "error with no OS error number");
        }
        write!(
            f,
            "{} (errno {})",
            std::io::Error::from_raw_os_error(self.0),
            self.0
        )
    }
}

/// How a child ended, decoded away from the platform's status word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatus {
    Exited(i32),
    Signalled(i32),
    /// Neither exited nor signalled. The supervisor does not act on it and must
    /// not silently mislabel it either.
    Other,
}

impl ExitStatus {
    /// `ADR-017` §3.5's `on-failure` needs exactly this question answered.
    ///
    /// A service killed by a signal counts as a failure even though a shutdown
    /// is how that usually happens — the supervisor knows which of its children
    /// it signalled, and asks this only about the ones it did not.
    pub fn is_success(self) -> bool {
        matches!(self, Self::Exited(0))
    }
}

impl core::fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Exited(code) => write!(f, "exit {code}"),
            Self::Signalled(sig) => write!(f, "killed by signal {sig}"),
            Self::Other => write!(f, "stopped or continued"),
        }
    }
}

/// A signal the supervisor *sends*.
///
/// Deliberately two. A supervisor that can send any signal invites a
/// configuration field that says which one, and that is a service-definition
/// question nothing in the record has ruled on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sig {
    Term,
    Kill,
    /// **What PID 1 reads as *shut down and come back*** — `cmd/rivet-initd`'s
    /// `RebootHow::Restart`, and the signal the kernel itself sends for
    /// Ctrl-Alt-Delete.
    ///
    /// Added 2026-09-01 for `rivet_core::kiosk`'s `restart`: the supervisor
    /// does not reboot the machine, it asks the thing whose job that is. PID 1
    /// stops the supervisor, waits for it, syncs the filesystems and only then
    /// asks the kernel (`ADR-017` §3.1.10) — none of which the supervisor could
    /// do for itself, and all of which a `reboot(2)` from here would skip.
    Int,
}

impl Sig {
    const fn number(self) -> i32 {
        match self {
            Self::Term => libc::SIGTERM,
            Self::Kill => libc::SIGKILL,
            Self::Int => libc::SIGINT,
        }
    }
}

/// Something the supervisor must react to.
///
/// The wait returns a *list* rather than one event on purpose: one `poll(2)`
/// can report a signal and three readiness notifications at once, and
/// collapsing them would mean waking again immediately to find the rest. It is
/// also the shape `ADR-022` §4's output capture needs — a pipe becoming
/// readable is one more event kind, not a new waiting mechanism.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// `SIGTERM` or `SIGINT`. `ADR-017` §3.1.5 item 6 puts shutdown in PID 1;
    /// this is our half of it.
    ///
    /// **It carries WHICH since 2026-08-26**, and it did not before. `ADR-022`
    /// §12 item 7 requires the shutdown record to say *whether a reboot or a
    /// power-off*, and this variant had collapsed the two — so the supervisor
    /// could not have said which if it had tried. A record that cannot
    /// distinguish a clean restart from a power-off is one §12.3 names as
    /// unable to answer the question it exists for.
    Terminate { how: Terminating },
    /// **`SIGHUP` — re-read the desired-state document.** `ADR-032` §3.2, built
    /// 2026-08-26.
    ///
    /// The Unix answer to *tell* rather than *ask*, and the honest limit is that
    /// **it cannot carry a reply**: the operator who sends it learns what
    /// happened from the journal, after the fact. `ADR-032` §3.2 accepts that
    /// for a reload and for nothing else.
    ///
    /// Distinct from [`Self::Terminate`] rather than folded into it, and the
    /// two must never be confused for the reason `ADR-017` §3.1.10 gives about
    /// its own three signals: a supervisor that read `SIGHUP` as a shutdown
    /// would stop a machine somebody meant to reconfigure.
    Reload,
    /// **Somebody asked this supervisor for something.** `ADR-032` §3.2's query
    /// socket, built 2026-08-26; `ADR-017` §6.3.1's *control surface* since
    /// 2026-08-29.
    ///
    /// ~~It carries no request text, and that is a decision rather than an
    /// omission … The day a second operation is ruled across, this variant gains
    /// it.~~ **That day came, and the variant gained it** — struck in place
    /// rather than rewritten, per `AGENTS.md` §7, because the sentence predicted
    /// its own expiry and it is worth seeing that it fired.
    ///
    /// It carries a **parsed** request rather than the bytes:
    /// `LibcKernel::drain_query` refuses anything the domain cannot read at the
    /// door, so a handler is never given a line it has to parse and can never
    /// be given one that is not one of the three commands this build answers.
    /// A request on the KIOSK socket — `rivet_core::kiosk`, the narrow
    /// kiosk socket ruled 2026-09-01 (`ADR-036` §5.4).
    ///
    /// Separate from [`Event::Queried`] because the surfaces are separate: this
    /// one is three verbs wide and reachable by one unprivileged application,
    /// and that one is the operator's whole control plane. A single variant
    /// carrying both would put the narrowing in a `match` arm rather than in
    /// the socket, which is the thing the socket exists to be.
    /// A request on the kiosk socket.
    ///
    /// **Any display lease that came with it is NOT here.** `Event` is
    /// `Clone + PartialEq` and an `OwnedFd` is neither — but the deeper reason
    /// is the one that already governs `kiosk_answering`: a descriptor is not a
    /// value to be copied around, it is a resource with one owner. It stays on
    /// the [`Kernel`] and the handler takes it with
    /// [`Kernel::take_kiosk_lease`]. `PLAN.md` §106.2.
    KioskAsked {
        request: rivet_core::kiosk::Request,
    },
    /// A request on the kiosk socket that could not be read.
    ///
    /// Carried rather than dropped: the launcher is the only caller, so a
    /// malformed line means the two halves disagree about the protocol and
    /// that is worth a record.
    KioskUnreadable {
        why: rivet_core::kiosk::RequestError,
    },
    Queried {
        request: rivet_core::operation::service_restart::Request,
    },
    /// **An application asked this machine what it is** — `rivet_core::facts`,
    /// `ADR-057`.
    ///
    /// It carries nothing, and that is the shape rather than an omission:
    /// there is exactly one question on this socket, so a field would be a
    /// vocabulary pretending to have a choice in it. `LibcKernel::drain_facts`
    /// refuses anything that is not `get facts` at the door, with the domain's
    /// own sentence, so a handler is never given a line it has to read.
    ///
    /// **Separate from [`Self::Queried`] for `KioskAsked`'s reason**: that one
    /// is the operator's whole control surface behind a `0600` socket, and this
    /// is one read-only question reachable by every application on the machine.
    /// A single variant carrying both would put the narrowing in a `match` arm
    /// rather than in the socket.
    FactsAsked,
    /// `SIGCHLD` — some child changed state. Which one is a question for
    /// [`Kernel::reap_ready`], because the signal does not say and coalesces.
    ChildChanged,
    /// A service declared itself ready over `NOTIFY_SOCKET` (`ADR-017` §3.3,
    /// §3.4).
    Ready { service: String },
    /// A service supplied a human status string, per the same convention.
    Status { service: String, text: String },
    /// **A service answered the health question** — `ADR-045` §3, 2026-09-08.
    ///
    /// `line` is the value half of `HEALTH=…`, exactly as it came off the
    /// answering socket. **Unparsed**, for [`Event::KernelMessage`]'s reason:
    /// what a health line *means* is `rivet_core::health::parse`'s and is pure,
    /// and this port's job is only to know that an answer is one line on a
    /// stream socket. A word outside the vocabulary therefore reaches the
    /// supervisor as *it answered badly*, which is one of §3.1.1's three
    /// outcomes and not the same as silence.
    Answered { service: String, line: String },
    /// **A service did not answer inside `rivet_core::health::PATIENCE_MS`** —
    /// `ADR-045` §3, and it is the event the whole reversal exists for.
    ///
    /// **This is what a WEDGED service produces.** A crashed one is already
    /// visible — the supervisor holds the pid and counts the restarts — but a
    /// deadlocked one keeps its pid, keeps its listening socket, and simply
    /// never accepts. Under the telling design that was indistinguishable from
    /// a healthy application with nothing new to say.
    ///
    /// `why` is the errno's own words when the connection could not be made at
    /// all, and the timeout in words when it was made and nothing came back.
    /// The two are one event because the consequence is one: no successful
    /// answer, therefore `unknown` (§4.2).
    Unanswered { service: String, why: String },
    /// **A service asked to be polled now** — `HEALTH=` on `NOTIFY_SOCKET`,
    /// `ADR-045` §3.
    ///
    /// **It carries no state, deliberately.** A push that arrives is not a
    /// health report; it is a request for one, and the poll is the truth. If
    /// this event carried the pushed word, an application that reported once at
    /// startup and then went quiet would have declared itself permanently
    /// healthy — which is exactly the power §3.1.1 takes away.
    HealthHint { service: String },
    /// **One line a service wrote** — `ADR-022` §4's second capture path, built
    /// 2026-08-24.
    ///
    /// One event per line, because a record is per line (`ADR-022` §4: "the
    /// journal captures their standard output and error and wraps each line as
    /// a record with the line as `msg`"). A partial line is held by the adapter
    /// until its newline arrives or the pipe reaches EOF, so a caller never
    /// sees half of one.
    Output {
        service: String,
        stream: Stream,
        text: String,
        /// The line was longer than [`MAX_CAPTURED_LINE`] and was cut. Carried
        /// rather than silent: a truncated line that looked whole would be the
        /// journal quietly disagreeing with what the service said.
        truncated: bool,
    },
    /// **One record from `/dev/kmsg`**, whole and unparsed — `ADR-022` §4.1.
    ///
    /// Unparsed because the parse is `crate::kmsg`'s and is pure: this port's
    /// job is to know that a `read(2)` on that descriptor returns exactly one
    /// record, and nothing else about what a kernel message *is*.
    KernelMessage { record: String },
    /// **Something arrived on the journal ingest socket** — `ADR-022` §11,
    /// built 2026-08-24.
    ///
    /// `peer` is `SO_PEERCRED`'s answer, taken at `accept(2)`, which is why it
    /// travels with the payload rather than being looked up later: the process
    /// that connected may have exited by the time the supervisor reads this.
    ///
    /// Unparsed, for [`Event::KernelMessage`]'s reason — what a line *means* is
    /// `rivet_core::journal::ingest`'s and is pure. This port's job is to know
    /// that a record is one newline-terminated line on a stream socket.
    Ingest {
        peer: rivet_core::journal::Peer,
        what: Ingest,
    },
    /// **A helper forked by [`Kernel::reclaim_in_child`] has exited**, and
    /// this is how it went: the directory and prefix it was sent to reclaim,
    /// and `failure`, `None` when everything it was sent to delete is gone.
    Reclaimed {
        service: String,
        dir: String,
        prefix: String,
        failure: Option<String>,
    },
    /// The timeout expired with nothing else to report.
    Timeout,
}

/// What one ingest connection delivered.
///
/// **The two refusals are events rather than silence** (`ADR-022` §11.5): a
/// sender whose record was too large or who held a connection open is a sender
/// whose record did not arrive, and a journal that dropped the fact would be
/// answering *"there is nothing about that in the log"* falsely.
///
/// What is *not* here is the connection that delivered **no bytes at all**. That
/// is not a record that failed to arrive — it is a probe, and manufacturing a
/// record for each one would hand anyone who can `connect(2)` a free
/// amplification into `ADR-022` §8's capped storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ingest {
    /// One complete line, exactly as it was written.
    Line(String),
    /// A line reached `rivet_core::journal::INGEST_MAX_BYTES` without a
    /// newline. The connection is closed and the bytes are dropped — a prefix
    /// of a record is not a record, and `AGENTS.md` §5 refuses rather than
    /// guessing at the rest.
    TooLarge { held: usize },
    /// The connection was open past
    /// `rivet_core::journal::INGEST_DEADLINE_MS` with an unterminated line in
    /// hand. Complete lines that arrived before that were already delivered.
    Unfinished { held: usize },
}

/// Which of a service's two streams a line came from.
///
/// **Kept, and not turned into a severity.** `ADR-022` §4 says a mechanism's
/// line is wrapped as a record with the line as `msg` and says nothing about
/// severity — and there is nothing to derive one from: a mechanism can write
/// *everything* to stderr, including its banner, so mapping stderr to `warn`
/// would make an ordinary boot look like a fault. `AGENTS.md` §5: refuse rather
/// than guess. The stream is reported as a field and the reader decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

impl Stream {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

impl core::fmt::Display for Stream {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What a program run to completion did, and what it said while doing it.
///
/// The status alone was the return type until 2026-08-24. It is a struct rather
/// than a tuple because a caller that ignored the output would compile either
/// way, and the whole point of `ADR-022` §4 is that a program's words do not
/// vanish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completed {
    pub status: ExitStatus,
    /// Every line, in order, with the stream it came from. Blank lines are
    /// dropped, as they are for a spawned service.
    pub output: Vec<(Stream, String)>,
}

/// How long a captured line may be before it is cut — 8 KiB.
///
/// A cap rather than a `Vec` that grows, because the thing on the other end is
/// a C mechanism this project does not control and a service that writes
/// megabytes without a newline must not be able to exhaust a machine with no
/// swap. `ADR-017` §3.5's restart-loop hazard is the same hazard by a different
/// route, and `ADR-022` §8 caps the file for the same reason.
///
/// 8 KiB is several times the longest line anything in the image produces: the
/// supervisor's own longest record is under 1 KiB, and the longest line any
/// mechanism in the image has produced is a key fingerprint.
pub const MAX_CAPTURED_LINE: usize = 8 * 1024;

/// The most bytes one pass of [`Kernel::wait`] reads from any one source — a
/// service's pipe, an ingest connection, a notify socket, `/dev/kmsg` — 64 KiB.
///
/// **This is what keeps one writer from owning the loop.** A drain that reads
/// until `WouldBlock` never returns while its writer refills the source faster
/// than the reads empty it, and the supervisor is single threaded: no reaping,
/// no convergence, no signal read, nothing from any other source. With a budget
/// the pass ends, the rest of the loop runs, and the next pass takes the next
/// 64 KiB.
///
/// **64 KiB is one Linux pipe's default capacity**, so a pipe that filled while
/// the supervisor was busy still empties in a single pass and its blocked
/// writer is released at once, exactly as before the budget existed. Only a
/// writer that refills faster than the loop turns is held to it.
pub const DRAIN_BUDGET_BYTES: usize = 64 * 1024;

/// How long the supervisor will wait on one request from the kiosk or query
/// socket before giving up on it.
///
/// **A ceiling on a stall, not a working timeout.** Both sockets are read to
/// end-of-file by a single-threaded supervisor, and both callers are local
/// programs writing a few hundred bytes over a unix socket — which takes
/// microseconds. Half a second is a thousand times that, and it is the
/// difference between a caller that goes quiet costing one slow pass and
/// costing the machine (`PLAN.md` §123, Wave 2.2).
pub const KIOSK_PATIENCE: std::time::Duration = std::time::Duration::from_millis(500);

/// What a service's cgroup says about it (`ADR-017` §3.1.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// The cgroup exists and holds these processes.
    Members(Vec<i32>),
    /// The cgroup exists and is empty: the service is definitely not running.
    Empty,
    /// **The question could not be asked.** Not the same as "not running", and
    /// conflating the two is how a supervisor starts a second copy of a service
    /// that is already up. `AGENTS.md` §5 — refuse rather than guess.
    ///
    /// **This variant alone does not say how bad the news is**, and that was a
    /// real defect: `ENOENT` here is *also* what a service which has simply
    /// never run on this boot produces, so the warning fired on every first
    /// boot of every service and asserted a double-start hazard that did not
    /// apply. [`Kernel::cgroup_hierarchy`] is what tells the two apart —
    /// `ADR-017` §3.1.3.1.
    Unavailable(Errno),
}

/// Whether the cgroup2 hierarchy *itself* can be read (`ADR-017` §3.1.3.1).
///
/// A separate question from [`Observation`], asked of the whole machine rather
/// than of one service, and asked only when an observation came back
/// [`Observation::Unavailable`] — because that is the only moment its answer
/// changes what the supervisor should say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hierarchy {
    /// A cgroup2 filesystem answers at `rivet_core::mount::CGROUP2_MOUNT`.
    ///
    /// The authority `ADR-017` §3.1.3 names exists. A service whose own cgroup
    /// is absent under it has not run on this boot, and there is nothing to
    /// adopt (§3.1.1).
    Present,
    /// It does not, and this is what the kernel said when asked.
    ///
    /// **The dangerous one.** With no hierarchy there is no authority for what
    /// is running, so a restarted supervisor cannot tell a live service from a
    /// dead one and will start a second copy of it.
    Unreachable(Errno),
}

/// A `mount(2)` flag word, in the platform's own bits.
///
/// The fourth type this crate and `cmd/rivet-initd` both want (`ADR-013` §7,
/// and the note at the top of this module). It arrived on 2026-08-23 with the
/// `/etc` projection, and it carries **two bits PID 1's copy does not need** —
/// [`REMOUNT`](Self::REMOUNT) and, reachable from a [`MountFlag`], nothing at
/// all — because PID 1 never re-mounts anything read-only after writing to it.
/// That asymmetry is itself the evidence `ADR-013` §7 asked to accumulate: the
/// *translation* wants sharing, the flag *sets* do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MountFlags(pub u64);

impl MountFlags {
    /// No bits at all.
    pub const NONE: Self = Self(0);

    // `MS_*` is `c_ulong` — 64-bit on aarch64, 32-bit on the armv6 of
    // `AGENTS.md` §10.2's deferred compatibility target. The cast is redundant
    // on the one target Rivet builds for today and load-bearing on the one it
    // might build for tomorrow, so it stays. Same call as PID 1's.
    #[allow(clippy::unnecessary_cast)]
    pub const NOEXEC: Self = Self(libc::MS_NOEXEC as u64);
    #[allow(clippy::unnecessary_cast)]
    pub const NOSUID: Self = Self(libc::MS_NOSUID as u64);
    #[allow(clippy::unnecessary_cast)]
    pub const NODEV: Self = Self(libc::MS_NODEV as u64);
    /// What makes `ADR-007` §3.6.1 item 1 true rather than aspirational: the
    /// tmpfs is remounted with this after the projection has written it, so a
    /// later `vi /etc/shells` fails with `EROFS` instead of succeeding and
    /// being discarded at the next boot.
    #[allow(clippy::unnecessary_cast)]
    pub const RDONLY: Self = Self(libc::MS_RDONLY as u64);
    /// Change the flags of a filesystem that is already mounted, rather than
    /// stacking a second one on the same point.
    #[allow(clippy::unnecessary_cast)]
    pub const REMOUNT: Self = Self(libc::MS_REMOUNT as u64);

    /// Translate `rivet-core`'s intent-named flags into this platform's bits.
    ///
    /// `ADR-014` §3 puts this mapping in an adapter and nowhere else: core says
    /// what the kernel must do, the adapter knows what the kernel calls it. The
    /// match is exhaustive, so a new [`MountFlag`] breaks this build rather than
    /// being silently dropped.
    pub fn from_core(flags: &[MountFlag]) -> Self {
        let mut bits = 0;
        for flag in flags {
            bits |= match flag {
                MountFlag::NoExec => Self::NOEXEC.0,
                MountFlag::NoSuid => Self::NOSUID.0,
                MountFlag::NoDev => Self::NODEV.0,
                MountFlag::NoAtime => Self::NOATIME.0,
            };
        }
        Self(bits)
    }

    #[allow(clippy::unnecessary_cast)]
    const NOATIME: Self = Self(libc::MS_NOATIME as u64);

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// How many shared endpoints one service may be granted.
///
/// **A cap, not a budget.** Eight is far above anything this platform has —
/// there is exactly one shared endpoint in the tree — and it exists so the
/// `pre_exec` closure can carry the list in a fixed array rather than a heap
/// pointer. A service that somehow exceeds it is **refused**, because a group
/// silently dropped is a connection that fails with `EACCES` and no reason.
const MAX_EARNED_GROUPS: usize = 8;

/// What to run. Not a service definition — see [`crate::service::ServiceDef`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Program {
    pub path: String,
    pub args: Vec<String>,
    /// Added to the inherited environment.
    pub env: Vec<(String, String)>,
    /// **Who to run it as.** `None` means uid 0 — the supervisor's own
    /// identity, unchanged. Added 2026-08-24.
    ///
    /// `ADR-015` §5.1's first sandbox mechanism is *"UID/GID per
    /// application"*, and this is where it reaches the kernel:
    /// [`Kernel::spawn`] drops to it between `fork` and `exec`, so a service
    /// that has one has **never** run a single instruction as root.
    ///
    /// **`None` is the honest value for a system service and not an omission.**
    /// `rivet-netd` configures interfaces over netlink and `rivet-mcpd` execs
    /// the CLI that installs payloads and reboots the machine; each of those is
    /// uid 0's work, and giving one an identity here would produce a service
    /// that starts and cannot do its job. A service needing one piece of root
    /// and nothing else has an identity and keeps that capability
    /// ([`Program::capabilities`]). `crate::definitions` is where that is
    /// decided, per service.
    ///
    /// It is on the *program* rather than beside it because it is part of what
    /// an execution **is**: `execve` runs under whatever identity the calling
    /// process has by then, so "what to run" and "as whom" are one question
    /// with one answer, asked once. [`Kernel::run_to_completion`] takes the
    /// same type and deliberately ignores this field — see there.
    pub identity: Option<rivet_core::service::AppIdentity>,
    /// **What it may reach, once it is running.** `ADR-042`, 2026-09-04.
    ///
    /// `None` is *unconfined* and is the honest value for a system service:
    /// `rivet-netd` reads `/sys`, `rivet-sysd` writes `/state` and `/etc`, and
    /// a policy on one of those would be a list of the whole filesystem. It is
    /// `Some` for every application, because an application's needs are small
    /// and **declared** — `rivet_core::confinement::policy` computes it from
    /// the manifest and `crate::supervise` appends the device nodes the
    /// declared classes expanded to.
    ///
    /// It sits beside [`Program::identity`] for that field's own reason: the
    /// ruleset is applied between `fork` and `exec`, is inherited by the
    /// `execve` and can never be removed, so *what to run*, *as whom* and
    /// *inside what* are one question asked once.
    ///
    /// **`None` here is not the same as an empty policy.** An empty
    /// [`rivet_core::confinement::Policy`] would build a ruleset that handles
    /// every right and grants none, which is a process that cannot open its
    /// own program. If this is ever `Some`, it has rows.
    pub confinement: Option<rivet_core::confinement::Policy>,
    /// **The supplementary groups this service EARNED**, one per shared
    /// endpoint it was granted — `.rulings/` R-206.
    ///
    /// **Empty for almost everything, and that is the important case.** A
    /// service granted no shared endpoint gets `setgroups(0, NULL)`, exactly
    /// as every service always has. This is not a relaxation of that: the call
    /// REPLACES the list, so root's inherited groups are gone either way.
    ///
    /// It sits beside [`Program::identity`] because it is part of the same
    /// question — *as whom* — and because it must be applied in the same
    /// moment, while `CAP_SETGID` is still held.
    ///
    /// **Derived from the grants and never from a manifest.** A consumer
    /// declares an endpoint; `crate::endpoints` decides whether that becomes a
    /// grant; only then does a gid appear here. An application cannot put
    /// itself in a group by writing one down.
    pub groups: Vec<u32>,
    /// **The capabilities this program keeps** — empty for everything except
    /// an image service whose work needs a named piece of root, such as
    /// `rivet-timed` setting the wall clock or `rivet-netd` writing netlink.
    ///
    /// Empty is the ordinary case and changes nothing: `setuid` to a non-zero
    /// uid discards every capability, and [`Kernel::spawn`] empties the
    /// bounding set of a program with an identity.
    ///
    /// **With an [`Program::identity`]**, a non-empty list is kept across the
    /// uid change as the permitted, effective, inheritable and **ambient**
    /// sets, and the bounding set is reduced to it, so the program `execve`s
    /// holding exactly these and can never gain another. Ambient, because the
    /// program file carries no file capabilities and `PR_SET_NO_NEW_PRIVS`
    /// would refuse them if it did.
    ///
    /// **Without one**, the program stays uid 0 and the list is its whole
    /// bounding set, with `PR_SET_NO_NEW_PRIVS` set: it keeps root's
    /// ownership of root's files and loses every other capability, and so
    /// does every child it starts. A ruleset without an identity is still
    /// refused ([`Program::confinement`]), so this shape is never confined.
    pub capabilities: Vec<Capability>,
}

/// **One Linux capability a program keeps across its change of uid** —
/// [`Program::capabilities`].
///
/// The number is the kernel's own, from `linux/capability.h`. A newtype over
/// it rather than an enum of the few in use, so that the test which proves the
/// mechanism can keep whichever capability the machine running the suite
/// actually holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Capability(u8);

impl Capability {
    /// `CAP_SYS_TIME`: set the system clock with `clock_settime(2)`.
    pub const SYS_TIME: Self = Self(25);

    /// `CAP_SETPCAP`: drop a capability from this process's own bounding set,
    /// which `rivet-bluetoothd` does to itself once its controller is bound.
    pub const SETPCAP: Self = Self(8);

    /// `CAP_NET_BIND_SERVICE`: bind a port below 1024, such as a DHCP
    /// client's 68.
    pub const NET_BIND_SERVICE: Self = Self(10);

    /// `CAP_NET_ADMIN`: change interfaces, addresses and routes over netlink.
    pub const NET_ADMIN: Self = Self(12);

    /// `CAP_NET_RAW`: open a raw or packet socket, which a DHCP client sends
    /// its first broadcast on and a supplicant reads EAPOL frames from.
    pub const NET_RAW: Self = Self(13);

    /// A version 3 capability set is two 32-bit words, so 64 numbers.
    const LIMIT: u8 = 64;

    /// The capability with this number, or `None` beyond what a capability
    /// set can hold.
    #[must_use]
    pub const fn from_number(number: u8) -> Option<Self> {
        if number < Self::LIMIT {
            Some(Self(number))
        } else {
            None
        }
    }

    /// The kernel's number for it.
    #[must_use]
    pub const fn number(self) -> u8 {
        self.0
    }

    /// Its bit in a mask, the shape `/proc/<pid>/status` prints sets in.
    #[must_use]
    pub const fn bit(self) -> u64 {
        1u64 << self.0
    }
}

/// The capabilities as one mask — what `CapEff`, `CapPrm`, `CapInh`,
/// `CapAmb` and `CapBnd` read for a program spawned keeping exactly these.
#[must_use]
pub fn capability_mask(capabilities: &[Capability]) -> u64 {
    capabilities.iter().fold(0, |mask, c| mask | c.bit())
}

/// What `localtime_r(3)` made of the wall clock — [`Kernel::local_time`].
///
/// **Three fields, and each answers a different question an operator has.**
/// The offset says *whether the zone took effect at all*; the abbreviation says
/// *which rule is in force right now*, which is the half a fixed offset cannot
/// express — `CST` and `CDT` are the same zone an hour apart; and the rendered
/// clock is the thing a person compares against their wrist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalReading {
    /// Seconds **east** of UTC, `struct tm`'s `tm_gmtoff` verbatim. Negative in
    /// the Americas.
    pub offset_s: i32,
    /// The abbreviation the library chose — `CST`, `CDT`, `UTC`. Copied from
    /// `tm_zone`, never composed here: `AGENTS.md` §5, report what the system
    /// said.
    pub abbreviation: String,
    /// `YYYY-MM-DD HH:MM:SS`, local. Composed from `struct tm`'s own fields
    /// rather than by `strftime`, so nothing depends on a locale.
    pub local: String,
}

/// A blocking, allocating transaction handed to [`Kernel::cec_send`] and run
/// off this process's own thread — `ADR-039` §8. `display_hours::send` is
/// the only caller today; the type is a plain function pointer rather than
/// `display_hours`'s name so this port stays free of one feature's
/// vocabulary, and so a test can hand it a body of its own (see
/// `tests::cec_worker_round_trips_success_and_failure_without_a_zombie`).
pub type CecTransaction = fn(bool) -> Result<String, String>;

/// A CEC transaction running in its own process, or already answered.
///
/// Returned by [`Kernel::cec_send`] and resolved by [`Kernel::cec_poll`],
/// which never blocks.
///
/// [`Self::Child`] is what a real fork produces: forked while this process
/// had exactly one thread, so the child may allocate freely before it calls
/// `_exit(2)` — see [`Kernel::cec_send`]'s doc for the argument, which is
/// [`Kernel::spawn`]'s `pre_exec` invariant applied to a second fork.
///
/// [`Self::Resolved`] is the test-only `FakeKernel`'s answer. A test
/// binary runs many `#[test]` functions on many threads inside one process,
/// so forking from one would carry exactly the hazard [`Self::Child`] exists
/// to avoid; the fake calls the transaction in its own thread instead and
/// hands the answer back pre-computed.
#[derive(Debug)]
pub enum CecWorker {
    /// `pid` is this process's own child; `read` is the pipe it reports on.
    Child {
        pid: i32,
        read: std::os::fd::OwnedFd,
    },
    /// The answer, taken exactly once by [`Kernel::cec_poll`].
    Resolved(Option<Result<String, String>>),
}

/// **One leaf to issue from this machine's fleet root**, and every place it
/// goes — [`Kernel::issue_in_child`].
///
/// Paths only: the order holds no key and no certificate, so the process that
/// builds it never holds the root's key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueOrder {
    /// The root's certificate.
    pub root_certificate: String,
    /// The root's private key. Read by the helper and by nothing else.
    pub root_key: String,
    /// The DNS names the leaf carries; the first is its subject.
    pub names: Vec<String>,
    /// The wall clock, in seconds since the epoch, that dates the leaf.
    pub now: u64,
    /// Every copy of the new leaf, in the order they are written.
    pub targets: Vec<IssueTarget>,
}

/// One directory a new leaf is written into, under that directory's own names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueTarget {
    /// The directory. It must already exist.
    pub dir: String,
    /// The file the leaf is written to.
    pub certificate: String,
    /// The file the leaf's private key is written to.
    pub key: String,
    /// The file the root's certificate is written to, or `None` where the root
    /// already rests beside it.
    pub root: Option<String>,
    /// Who owns the three files. Each is mode `0600`.
    pub uid: u32,
    /// The files' group.
    pub gid: u32,
}

/// The longest answer an issuing helper reports, tag byte included: under
/// `PIPE_BUF`, so its one `write(2)` is atomic.
const MAX_ISSUE_MESSAGE: usize = 4096;

/// **The whole of an issuing helper's work** — [`Kernel::issue_in_child`].
/// Reads the root, issues through `rivet_fleet_root`, and writes every
/// target: the root's certificate, then the key, then the certificate, each to
/// a fresh file beside its name and renamed over it, so a reader sees the old
/// file or the new one and never part of either. The certificate goes last,
/// so a reader that finds a new certificate finds its key.
///
/// **Nothing here follows a symbolic link.** A target's directory may belong to
/// the application it serves, so the fresh file is created with `O_EXCL` and
/// `O_NOFOLLOW`, an entry already at its name is unlinked first, and the
/// rename replaces whatever sits at the final name rather than writing through
/// it.
fn issue_certificate(
    order: &IssueOrder,
    random: &mut dyn FnMut(usize) -> Result<Vec<u8>, String>,
) -> Result<String, String> {
    let read = |path: &str| {
        std::fs::read_to_string(path).map_err(|e| format!("{path} could not be read: {e}"))
    };
    let root_certificate = read(&order.root_certificate)?;
    let root_key = read(&order.root_key)?;
    let leaf = rivet_fleet_root::judge::issue_from_root(
        &root_certificate,
        &root_key,
        &order.names,
        random,
        order.now,
    )
    .map_err(|e| e.to_string())?;
    drop(root_key);

    for target in &order.targets {
        let mut files = Vec::with_capacity(3);
        if let Some(root) = &target.root {
            files.push((root.as_str(), root_certificate.as_bytes()));
        }
        files.push((target.key.as_str(), leaf.key.as_bytes()));
        files.push((target.certificate.as_str(), leaf.certificate.as_bytes()));
        for (name, bytes) in files {
            place(&target.dir, name, bytes, target.uid, target.gid)?;
        }
        if let Ok(dir) = std::fs::File::open(&target.dir) {
            let _ = dir.sync_all();
        }
    }
    Ok(format!(
        "SHA-256 {}",
        rivet_fleet_root::fingerprint(&leaf.certificate_der)
    ))
}

/// Write `bytes` to `dir/name`, owned by `uid:gid`, mode `0600`, by way of a
/// fresh file renamed over it.
fn place(dir: &str, name: &str, bytes: &[u8], uid: u32, gid: u32) -> Result<(), String> {
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

    let fresh = format!("{dir}/.{name}.issuing");
    let at = format!("{dir}/{name}");
    let failed = |what: &str, e: &std::io::Error| format!("{what} {fresh}: {e}");
    match std::fs::remove_file(&fresh) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(failed("could not clear", &e)),
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .mode(0o600)
        .open(&fresh)
        .map_err(|e| failed("could not create", &e))?;
    file.write_all(bytes).map_err(|e| failed("could not write", &e))?;
    // `fchown` and `fchmod` on the descriptor: the path is not consulted again.
    std::os::unix::fs::fchown(&file, Some(uid), Some(gid))
        .map_err(|e| failed("could not give", &e))?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|e| failed("could not set the mode of", &e))?;
    file.sync_all().map_err(|e| failed("could not flush", &e))?;
    drop(file);
    std::fs::rename(&fresh, &at).map_err(|e| format!("could not rename {fresh} to {at}: {e}"))
}

/// `len` bytes from `getrandom(2)` with no flags, which blocks until the
/// kernel's pool is ready rather than answering weak bytes.
fn getrandom(len: usize) -> Result<Vec<u8>, String> {
    let mut out = vec![0u8; len];
    let mut filled = 0;
    while filled < len {
        let rest = &mut out[filled..];
        // SAFETY: `rest` is a live, writable buffer of exactly `rest.len()`
        // bytes, and `getrandom` writes at most that many.
        let got = unsafe { libc::getrandom(rest.as_mut_ptr().cast(), rest.len(), 0) };
        if got < 0 {
            let errno = Errno::last();
            if errno == Errno::EINTR {
                continue;
            }
            return Err(format!("getrandom(2) answered {errno}"));
        }
        filled += usize::try_from(got).unwrap_or(0);
    }
    Ok(out)
}

/// The longest message a [`CecWorker::Child`] may report, tag byte included.
///
/// Comfortably above anything [`rivet_adapter_cec::Error`]'s `Display` forms,
/// and comfortably below `PIPE_BUF` (4096 on Linux) — the write in the child
/// is therefore one atomic `write(2)`, never a partial one a reader could
/// observe half of.
const MAX_CEC_MESSAGE: usize = 512;

/// The real half of [`Kernel::cec_poll`]: reap `pid` without blocking and,
/// once it has exited, read whatever it wrote before `_exit(2)`.
///
/// **Reaped before it is read, never the other way round.** `pid`'s last
/// action before `_exit` is the one write to `read`'s other end, so a
/// `waitpid` that reports it gone is also a promise that the write already
/// happened and is sitting in the pipe — there is nothing left to race.
/// Reading first would have to guess whether an empty result means "nothing
/// yet" or "wrote nothing at all".
fn poll_cec_child(pid: i32, read: &std::os::fd::OwnedFd) -> Option<Result<String, String>> {
    use std::os::fd::AsRawFd as _;

    let mut status: libc::c_int = 0;
    // SAFETY: `pid` is this process's own child, forked by `cec_send` and
    // reaped nowhere else — `reap_ready` only waits on the pids in its own
    // `children` map, which this one was never added to. `WNOHANG` never
    // blocks.
    let waited = unsafe { libc::waitpid(pid, &raw mut status, libc::WNOHANG) };
    if waited != pid {
        // `0`: still running. Negative: `ECHILD`, or a signal interrupted the
        // call. Neither is "done"; both are asked again on the next pass.
        return None;
    }

    let no_answer = || {
        Some(Err(
            "the process that was telling the television ended without an answer".to_string(),
        ))
    };
    let mut buf = [0u8; MAX_CEC_MESSAGE];
    // SAFETY: `read`'s descriptor is open and owned by this `CecWorker`, and
    // `buf` is a live, correctly sized buffer on this function's stack. The
    // child is confirmed gone above, so this cannot block: its data, if any,
    // is already sitting in the pipe.
    let n = unsafe {
        libc::read(read.as_raw_fd(), buf.as_mut_ptr().cast::<libc::c_void>(), buf.len())
    };
    let Ok(len) = usize::try_from(n) else {
        return no_answer();
    };
    if len == 0 {
        return no_answer();
    }
    let Some((&tag, text)) = buf[..len].split_first() else {
        return no_answer();
    };
    let text = String::from_utf8_lossy(text).into_owned();
    Some(if tag == 0 { Ok(text) } else { Err(text) })
}

/// **In a forked child that will not `exec`: close every descriptor it
/// inherited except the standard streams and `keep`.**
///
/// Forking without an `exec` copies the whole descriptor table, and
/// `O_CLOEXEC` is an `exec`-time flag that a process which never execs gets no
/// benefit from. That is every pre-bound service listener, the signalfd, the
/// journal's own files, `/dev/kmsg`, the query/kiosk/facts sockets, and a
/// kiosk application's DRM lease if one is held — none of them opened with
/// this child in mind, and every one kept alive by its copy for as long as it
/// runs. A helper that blocks would keep a listener bound and a lease alive,
/// indefinitely, in a process nothing is watching.
///
/// Two ranges, one on each side of `keep`, because `close_range(2)` closes one
/// contiguous span. `keep` is always `> 3` when given: descriptors 0-2 are the
/// standard streams and are always open in a running supervisor, so `pipe2`
/// could never have returned one of them.
///
/// **A refusal is not checked.** `ENOSYS` on a kernel older than 5.9, most
/// plausibly — and the helper's work still runs either way: a worse cleanup is
/// not a reason to skip it.
fn close_inherited(keep: Option<libc::c_int>) {
    let from = match keep.and_then(|fd| u32::try_from(fd).ok()) {
        Some(keep) => {
            if keep > 3 {
                // SAFETY: raw `close_range(2)` via `syscall(2)`, because no
                // wrapper is guaranteed on every target this crate builds for.
                // The syscall reads and frees kernel state this process owns;
                // it touches no memory of its own. It allocates nothing and
                // walks no directory, so it is exactly as fork-safe as `fork`.
                unsafe {
                    libc::syscall(libc::SYS_close_range, 3u32, keep - 1, 0u32);
                }
            }
            keep + 1
        }
        None => 3,
    };
    // SAFETY: as above.
    unsafe {
        libc::syscall(libc::SYS_close_range, from, u32::MAX, 0u32);
    }
}

/// **The whole of a reclaim helper's work** — [`Kernel::reclaim_in_child`].
/// Returns the exit status the helper reports it with: `0` when every entry of
/// `dir` beginning with `prefix` is gone, otherwise the first failure's
/// `errno`, or `255` for a failure that carried none.
///
/// **Nothing here follows a symbolic link.** The type is read with
/// `symlink_metadata`, a link is unlinked as a link, and `remove_dir_all`
/// removes the links inside a tree rather than what they point at. `ENOENT` is
/// success: an entry another helper already took is gone, which is the answer
/// being asked for.
fn reclaim(dir: &str, prefix: &str) -> libc::c_int {
    let failed = |e: &std::io::Error| e.raw_os_error().filter(|c| (1..=255).contains(c)).unwrap_or(255);
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return 0,
        Err(e) => return failed(&e),
    };
    let mut status = 0;
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with(prefix) {
            continue;
        }
        let path = entry.path();
        let removed = match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&path),
            Ok(_) => std::fs::remove_file(&path),
            Err(e) => Err(e),
        };
        if let Err(e) = removed
            && e.kind() != std::io::ErrorKind::NotFound
            && status == 0
        {
            status = failed(&e);
        }
    }
    status
}

/// How often [`Kernel::run_within`] looks again when nothing has arrived on
/// either stream — which is what notices a program that exited after both
/// streams were already shut, and what keeps the deadline within this of exact.
const ONE_SHOT_STEP_MS: u64 = 10;

/// The command [`Kernel::run_to_completion`] and [`Kernel::run_within`] both
/// start: `program`, with the signal mask cleared and standard input set.
fn one_shot(program: &Program) -> std::process::Command {
    use std::os::unix::process::CommandExt as _;

    let mut cmd = std::process::Command::new(&program.path);
    cmd.args(&program.args);
    for (key, value) in &program.env {
        cmd.env(key, value);
    }
    // The same signal-mask reset `spawn` performs, and for the same
    // measured reason: `std::process::Command` does not clear the child's
    // inherited mask, and this parent runs with every signal
    // `arm_signals` blocks blocked for `signalfd`. A generator that inherited
    // a blocked SIGTERM could not be interrupted at all. The reset is
    // `sigemptyset` + `SIG_SETMASK`, so it clears whatever that set is.
    //
    // SAFETY: the closure runs in the child between `fork` and `exec` and
    // must be async-signal-safe. `sigemptyset` and `pthread_sigmask` are,
    // and `set` lives on the child's stack for the whole call. It allocates
    // nothing and takes no lock.
    unsafe {
        cmd.pre_exec(|| {
            let mut set: libc::sigset_t = std::mem::zeroed();
            if libc::sigemptyset(&raw mut set) != 0
                || libc::pthread_sigmask(libc::SIG_SETMASK, &raw const set, std::ptr::null_mut())
                    != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    // **`stdin` is set EXPLICITLY, and that one line is load-bearing.**
    //
    // `Command::output()` defaults stdin to `Stdio::null()` when nothing
    // else was set — and `Stdio::null()` opens `/dev/null` **in the parent,
    // before the fork**, so on a machine that has no `/dev/null` the whole
    // spawn fails with `ENOENT` *reported against the program's path*,
    // which is present. Under `tools/qemu/run.sh` the root is an initramfs
    // and the kernel does not auto-mount devtmpfs onto one, so there is no
    // `/dev/null` there — and every program run through this call would fail
    // to start, naming its own path as the thing that was missing.
    //
    // `inherit` is what `spawn` gives a service.
    cmd.stdin(std::process::Stdio::inherit());
    cmd
}

/// A finished one-shot program's status, in this port's terms.
fn decode_status(status: std::process::ExitStatus) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt as _;

    if let Some(code) = status.code() {
        ExitStatus::Exited(code)
    } else if let Some(sig) = status.signal() {
        ExitStatus::Signalled(sig)
    } else {
        ExitStatus::Other
    }
}

/// A one-shot program's two streams, as the lines [`Completed`] carries.
fn captured_lines(stdout: &[u8], stderr: &[u8]) -> Vec<(Stream, String)> {
    let mut output = Vec::new();
    for (stream, bytes) in [(Stream::Stdout, stdout), (Stream::Stderr, stderr)] {
        // Lossy for the reason `Capture::emit` is: a generator's output is
        // bytes, and a record that could not be written because one emitted
        // a stray byte would be the journal failing at its job.
        for line in String::from_utf8_lossy(bytes).lines() {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }
            let mut text = line.to_string();
            text.truncate(MAX_CAPTURED_LINE);
            output.push((stream, text));
        }
    }
    output
}

/// Set `O_NONBLOCK` on a descriptor this process owns.
fn set_nonblocking(fd: libc::c_int) {
    // SAFETY: `fcntl` reads and sets the flags of a descriptor the caller owns
    // and touches no memory of this process.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags >= 0 {
            let _ = libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
}

/// Append what a non-blocking pipe holds now, up to [`DRAIN_BUDGET_BYTES`] —
/// so a program that writes without stopping cannot hold its caller past a
/// deadline. `false` once the pipe has ended.
fn read_available(mut pipe: &std::fs::File, said: &mut Vec<u8>) -> bool {
    let mut buf = [0u8; 4096];
    let how = drain_within_budget(&mut pipe, &mut buf, |bytes| {
        said.extend_from_slice(bytes);
        true
    });
    !matches!(how, Drained::Ended | Drained::Failed)
}

/// Every operating-system fact the supervisor depends on.
///
/// Object-safe on purpose: the logic takes `&mut dyn Kernel`, so there is one
/// compiled copy of the decisions and the fake drives exactly the same code the
/// board runs.
pub trait Kernel {
    /// Milliseconds from a monotonic clock. Never jumps, which is why every
    /// deadline in this crate is expressed in it and none in wall time.
    fn now_ms(&mut self) -> u64;

    /// Milliseconds since the Unix epoch, or `None` if there is no readable
    /// wall clock. A Pi has no RTC (`ADR-021`), so `None` and "wrong" are both
    /// normal here and neither is worth failing over.
    fn wall_ms(&mut self) -> Option<u64>;

    /// Set the wall clock. `ADR-021` §3's restore, and the only call in this
    /// crate that moves it.
    ///
    /// **Never called unconditionally.** `crate::clock` decides, and `ADR-021`
    /// §3 makes the decision a *floor* — so this is reached only when the stored
    /// value is later than the running clock, and it therefore only ever moves
    /// time **forwards**. That matters to the rest of this crate: `ADR-021` §5
    /// puts every duration on [`Kernel::now_ms`], and the reason it is a
    /// separate clock is that this one can jump.
    ///
    /// A refusal is survivable and must be said: on Linux this needs
    /// `CAP_SYS_TIME`, so `EPERM` is what an unprivileged process gets, and a
    /// machine whose clock could not be restored still boots — with every record
    /// it writes stamped 1970 (the 1970-clock finding, 2026-08-23).
    fn set_wall_ms(&mut self, unix_ms: u64) -> Result<(), Errno>;

    /// `sethostname(2)`. `rivet_core::hostname`, ruled 2026-08-26.
    ///
    /// **A refusal is survivable and must be said.** This needs `CAP_SYS_ADMIN`
    /// on Linux, so `EPERM` is what an unprivileged process gets — and a
    /// machine whose kernel would not take its name still boots, still takes an
    /// address, and still resolves that name through `/etc/hosts`. What it
    /// costs is that `uname -n` and `/etc/hosts` then disagree, which is the
    /// two-answers state the ruling exists to prevent, so `crate::naming`
    /// reports it at `Warn` rather than swallowing it.
    fn set_hostname(&mut self, name: &str) -> Result<(), Errno>;

    /// **What the C library makes of the wall clock locally** — `ADR-021` §9.
    ///
    /// `localtime_r(3)`, which is musl reading `/etc/localtime` and telling us
    /// what it found there. It is on this port because it is I/O: the answer
    /// comes out of a file, not out of a calculation.
    ///
    /// # Why the supervisor asks at all
    ///
    /// **Because writing the file is not the same as the file working**, and
    /// `ADR-034` §5.3 forbids reporting a setting as applied when it is not.
    /// Every step of projecting `/etc/localtime` can succeed — the document
    /// parses, the tzfile is found, the bytes are written and read back — and
    /// the machine can still be on UTC, because musl checks the magic itself
    /// and falls back **in silence**. This call is the supervisor asking the
    /// library, after the fact, what it actually believes. `AGENTS.md` §5: the
    /// C library is the authority for its own answer, and it is asked rather
    /// than assumed.
    ///
    /// # Asked once, and that is a property of musl rather than a choice
    ///
    /// musl's `__tz.c` caches on the value of `TZ` — which is unset here, so
    /// the cached key is the string `/etc/localtime` and a second call **does
    /// not re-read the file**. So the reading is *this process's* answer, taken
    /// once, immediately after the file was written. That is exactly the claim
    /// worth making and no larger one: `ADR-021` §9.5 rules the key
    /// `on-next-restart` for the same reason, and a program already running
    /// when the file changed keeps the zone it resolved.
    ///
    /// `None` is a machine with no readable wall clock, which a Pi with no RTC
    /// can legitimately be before `ADR-021` §3's restore.
    fn local_time(&mut self) -> Option<LocalReading>;

    /// The journal's sink. Cannot fail: the supervisor has nowhere to report a
    /// failed report to.
    fn emit(&mut self, record: &Record);

    fn read_file(&mut self, path: &str) -> Result<String, Errno>;

    // -- the /etc projection's four (2026-08-23, `ADR-017` §3.1.8) -----------
    //
    // They are on this port rather than on a second one for a plain reason:
    // `crate::etc` has to *journal* what it did, the journal is reached through
    // `Kernel::emit`, and two `&mut dyn` borrows of one adapter is not a thing
    // Rust permits. A separate port would mean a separate adapter object and a
    // projection that could not say anything about itself — which is the defect
    // `ADR-017` §8 item 2 rules against one layer up.

    /// Create a directory and every missing parent. Succeeds if it is already
    /// a directory; fails if the path exists as something else.
    fn mkdir_p(&mut self, path: &str) -> Result<(), Errno>;

    /// Create **one** directory with a stated mode, if it is not already there.
    ///
    /// Added 2026-08-23, and it is deliberately not [`Kernel::mkdir_p`] with an
    /// extra argument. Three differences, each load-bearing for its callers —
    /// `crate::applications`, for an application's data directory, and
    /// `crate::journal`, for `/state/log`:
    ///
    /// * **One directory, no parents.** A caller that knows the mode of the
    ///   directory it wants rarely knows the mode of its parents, so creating
    ///   them here would create them with a mode nobody stated.
    /// * **The mode is applied even under a umask.** `mkdir(2)` masks the mode
    ///   it is given; a `0700` that arrived as `0700 & ~umask` would be a
    ///   directory whose mode depended on what PID 1 happened to inherit — and
    ///   a mode is what decides whether the one account that should reach a
    ///   directory is the only one that does.
    /// * **The answer says whether it was created**, so the journal can report
    ///   a first boot differently from every boot after it. `Ok(false)` is "it
    ///   was already a directory", which is the normal case.
    ///
    /// A path that exists as something other than a directory is an error, and
    /// which error it is is the platform's answer rather than this port's to
    /// invent (`AGENTS.md` §5).
    fn mkdir_mode(&mut self, path: &str, mode: u32) -> Result<bool, Errno>;

    /// Run a program to completion and say how it ended.
    ///
    /// **Synchronous, and that is the whole point of it being a separate method
    /// from [`Kernel::spawn`].** `ADR-017` §3.2 gives the supervisor no way to
    /// order one service after another — `converge` starts everything in one
    /// pass and waits for none — so anything that must finish *before* the next
    /// step cannot be a service. Its caller is `crate::repos`, which runs
    /// `git init`; `crate::splash` runs under [`Kernel::run_within`] instead.
    ///
    /// The child is **not** placed in a cgroup and is **not** reaped by
    /// [`Kernel::reap_ready`]: it is waited for here, so it is gone before this
    /// returns and there is nothing for either to observe.
    ///
    /// **Standard output and error are captured** — since 2026-08-24, when
    /// `spawn` was, and for the same reason: a one-shot program's output on the
    /// console and not in the journal is a line a board writes once and nobody
    /// can read afterwards. `ADR-022` §4's capture, applied to a program that
    /// runs to completion rather than to a service.
    ///
    /// **Waits, so it reads to EOF and cannot deadlock.** Unlike `spawn`'s
    /// pipes there is no poll loop here; the process is short-lived by
    /// definition.
    ///
    /// **[`Program::identity`] is ignored here, and that is a decision.** Every
    /// caller is doing work *for* something rather than *as* it — creating a
    /// root-owned directory tree, drawing a screen — and a program that dropped
    /// to some other identity could not do it. No definition in this binary
    /// sets the field on a program handed to this call.
    fn run_to_completion(&mut self, program: &Program) -> Result<Completed, Errno>;

    /// [`Kernel::run_to_completion`], **under a deadline** — `ADR-017` §3.
    ///
    /// For a program whose caller is the supervisor's loop and which the loop
    /// cannot afford to wait on without bound: the splash, which writes a
    /// device node and can block there or keep watching for an address. The
    /// loop is single threaded, so an unbounded wait here is a supervisor that
    /// reaps nothing, converges nothing and reads no signal for as long as the
    /// program takes.
    ///
    /// A program still running `limit_ms` after it started is sent `SIGKILL`
    /// and the call returns `ETIMEDOUT`. A program that has exited by then is
    /// answered as `run_to_completion` would answer it, whatever still holds
    /// its streams. One the kernel has not finished by the time of the answer
    /// is reaped later, beside the services, and costs this call nothing.
    ///
    /// # Errors
    ///
    /// `ETIMEDOUT` at the deadline, or whatever starting the program said.
    fn run_within(&mut self, program: &Program, limit_ms: u64) -> Result<Completed, Errno>;

    /// `chown(2)`, so that an application can write the directory that is its
    /// own. Added 2026-08-24 with `ADR-015` §5.1's per-application identity.
    ///
    /// **Numbers, not names.** The identity comes from
    /// `rivet_core::service::app_identity`, which is a function of the
    /// application's name; resolving a name through `getpwnam` here would ask
    /// `/etc/passwd` — a file **this same boot generates from that same
    /// function** — and make a projection an input to the thing it projects.
    ///
    /// Follows symlinks is the wrong answer for a path under `/state`, which is
    /// writable: this is `lchown`-shaped, and the implementation says which
    /// call it makes.
    fn chown(&mut self, path: &str, uid: u32, gid: u32) -> Result<(), Errno>;

    /// `chmod(2)` on a path that is already there.
    ///
    /// Distinct from [`Kernel::mkdir_mode`], which sets a mode on a directory
    /// it has just created and reports `Ok(false)` — doing nothing — for one
    /// that was already there. That is right for a directory whose mode is
    /// decided once; it is wrong for one whose mode is a **convergence
    /// target**. `/state/git` on the board at `10.0.0.2` was created
    /// `root:root 0755` by the code that shipped before `PLAN.md` §53.2 was
    /// ruled, and a supervisor that can only set a mode at creation time can
    /// never bring it to `2775`.
    ///
    /// Added 2026-08-27 with that ruling.
    fn chmod(&mut self, path: &str, mode: u32) -> Result<(), Errno>;

    /// Bring an existing tree to `PLAN.md` §53.2's arrangement: group `gid`
    /// throughout, and every **directory** group-writable and setgid.
    ///
    /// **Why this is one operation rather than a walk the caller writes.** The
    /// walk has three properties that are easy to get wrong once and then
    /// invisible: it must not follow symlinks (`/state` is writable, so a
    /// symlink planted in a repository could otherwise hand a directory
    /// somewhere else to the account group — [`Kernel::chown`]'s `lchown`
    /// argument, one level up); it must set the mode *after* the owner,
    /// because `chown(2)` clears setgid on some filesystems; and it must fix
    /// **directories only**.
    ///
    /// **Directories only, and that is deliberate rather than partial.** What
    /// a push needs is the right to *create entries*, which is a directory
    /// permission. Loose objects are immutable once written and refs are
    /// replaced by `rename(2)` into a directory rather than rewritten in
    /// place, so no file in a bare repository needs to become group-writable
    /// for a push to succeed — and every file that does not become
    /// group-writable is one an account cannot corrupt. Files git creates
    /// *after* this runs are git's business, and `crate::git::SHARED_INIT`
    /// is what tells it.
    ///
    /// Idempotent: running it on a tree already in that shape changes nothing.
    fn share_tree(&mut self, path: &str, gid: u32) -> Result<(), Errno>;

    /// `mount(2)`, in full generality, because the projection needs three
    /// different calls: mount the tmpfs, remount it writable on a re-render,
    /// and remount it read-only when the writing is done.
    fn mount(
        &mut self,
        source: &str,
        target: &str,
        fstype: &str,
        flags: MountFlags,
        data: Option<&str>,
    ) -> Result<(), Errno>;

    /// Is there a file at this path?
    ///
    /// **Three answers, not two**, and the third is why this is not `-> bool`.
    /// `Ok(false)` means the platform said there is nothing there; `Err` means
    /// the question could not be asked. `crate::etc` needs the distinction: it
    /// is deciding whether to name a binary in `/etc/shells`, and "I could not
    /// tell" must not become "yes" (`AGENTS.md` §5).
    fn file_exists(&mut self, path: &str) -> Result<bool, Errno>;

    /// **Is `path` itself a symbolic link** — `lstat(2)`, never following the
    /// final component. `ADR-049`.
    ///
    /// Distinct from [`Kernel::file_exists`], which answers a different
    /// question and answers it by following one: `metadata` says whether
    /// something is reachable AT a path, and a symlink to a real target
    /// reads as `Ok(true)` from it exactly as an ordinary file would.
    /// `crate::applications::deliver_staged` needs the question this asks
    /// instead, before it ever calls [`Kernel::mkdir_mode`] or writes a file:
    /// a directory an application owns is one it can replace an entry of with
    /// a link, and the entry has to be caught before it is used, not after.
    ///
    /// **`Ok(false)` covers both "nothing there" and "there, and it is
    /// something else"**, because every caller here takes the same next step
    /// for both — proceed as if the link is not in the way. Only `Ok(true)`
    /// changes what happens.
    fn is_symlink(&mut self, path: &str) -> Result<bool, Errno>;

    /// Read a whole file as **bytes**, refusing anything longer than `max`.
    ///
    /// [`Kernel::read_file`] returns a `String` and is the right shape for the
    /// documents this supervisor reads. `ADR-015` §4.2's launch verification
    /// reads two things that are not text — the 4 KiB signed header, and the
    /// program itself, in order to hash it — so it needs this instead.
    ///
    /// **The bound is an argument rather than a constant** because the two
    /// callers want very different numbers and neither should inherit the
    /// other's. A header is exactly `HEADER_BYTES`; a program is megabytes and
    /// is bounded only so that a `/state` somebody has filled with a huge file
    /// cannot make the supervisor allocate it. Over the bound is `EFBIG` and
    /// never a truncated read: hashing three quarters of a binary and calling
    /// it verified is the one outcome this must not have.
    fn read_file_bytes(&mut self, path: &str, max: usize) -> Result<Vec<u8>, Errno>;

    /// Write a whole file and set its mode.
    ///
    /// The mode is not a convenience: `/etc/shadow` is `0600` and everything
    /// else is `0644` (`rivet_core::etc::SHADOW_MODE`), and a `shadow`
    /// written world-readable would be a real defect that no test of the
    /// *contents* could catch.
    fn write_file(&mut self, path: &str, contents: &[u8], mode: u32) -> Result<(), Errno>;

    /// The same, for a path **below a directory an unprivileged uid owns**.
    ///
    /// # Why this is a second method and not a hardening of the first
    ///
    /// [`Kernel::write_file`] opens by path and follows what it finds, which is
    /// correct for every caller that writes into a root-owned directory and is
    /// a hole for the one that does not. The audit of 2026-09-03 named it
    /// (`PLAN.md` §123.2 item 2): `crate::runtime` renders an application's
    /// settings into ~~`<volume>/app/<name>/data/settings`~~, and `data/` is
    /// owned by that application **on purpose** (`ADR-038` §8) so the program
    /// can read its own file. So the application can delete that file and put a
    /// symlink there — to `/state/config/<document>`, to a secret under
    /// `/state/secrets`, to another application's data, to the journal — and root will truncate
    /// the target, write settings text into it, and chmod it `0644`. A FIFO at
    /// the same path blocks the open and freezes the supervisor.
    ///
    /// **The settings file left `data/` on 2026-09-04** (`ADR-038` §8.1), so the
    /// caller that motivated this method no longer needs it. The paragraph above
    /// is kept because it is the clearest statement of the hazard this method
    /// exists for, and the method is kept for two reasons: the guarded write is
    /// still the correct one for **any** future write below a directory an
    /// unprivileged uid owns, and having it means the next such caller does not
    /// have to rediscover it. `crate::runtime` still calls it, and says there
    /// why it is now belt-and-braces rather than load-bearing.
    ///
    /// **Hardening `write_file` itself was the first instinct and is wrong.**
    /// `crate::devices` writes `0` to `/sys/class/vtconsole/*/bind`, and sysfs
    /// does not support `ftruncate` — the guards below would refuse a write
    /// that has to succeed. Two methods, each honest about where it may be
    /// used, beats one method with an exception.
    ///
    /// # The three guards
    ///
    /// * `O_NOFOLLOW`, so a symlink at the final component is `ELOOP` rather
    ///   than a write somewhere else.
    /// * a regular-file check, so a FIFO, a device node or a directory is
    ///   `EINVAL`. With `O_NONBLOCK` the FIFO case cannot hang first.
    /// * `nlink == 1`, which is what refuses a **hard link** — the attack
    ///   `O_NOFOLLOW` does not see, because a hard link is not a symlink and
    ///   the kernel cannot tell which name is the "real" one.
    ///
    /// And the truncation is `ftruncate` after those checks rather than
    /// `O_TRUNC` before them, and the mode is `fchmod` on the descriptor rather
    /// than `chmod` on the path — so nothing between the open and the write can
    /// change what is being written to.
    fn write_file_nofollow(
        &mut self,
        path: &str,
        contents: &[u8],
        mode: u32,
    ) -> Result<(), Errno>;

    // -- the journal's two (2026-08-24, `ADR-022` §8) ------------------------
    //
    // On this port for the reason the `/etc` projection's four and the clock's
    // four are: the journal is reached through `Kernel::emit`, and two `&mut
    // dyn` borrows of one adapter is not a thing Rust permits. It is also the
    // right home on the merits — a journal that could not describe its own
    // failure to write is the defect `ADR-017` §8 item 2 rules against.

    /// **Append**, creating the file with `mode` if it is not there.
    ///
    /// Distinct from [`Kernel::write_file`], which replaces. A journal that
    /// replaced would lose the previous boot every time a machine came up,
    /// which is the boot most worth reading (`ADR-018` §5.3.3's rollback was
    /// diagnosed from a boot that had already ended).
    ///
    /// **No `fsync`, deliberately** — `ADR-022` §8 rules out one per record.
    /// What this does promise is that the bytes have reached the kernel, so
    /// the supervisor dying or the machine restarting does not lose them; only
    /// a power cut does. `crate::journal`'s header argues the trade.
    ///
    /// The mode applies on creation only, which is `open(2)`'s behaviour and is
    /// what a caller wants: a journal file whose mode an operator has changed
    /// is the operator's business.
    fn append_file(&mut self, path: &str, contents: &[u8], mode: u32) -> Result<(), Errno>;

    /// How many bytes are in a file. `Err(Errno::ENOENT)` if it is not there,
    /// which a caller reads as "nothing has been written yet" rather than as a
    /// fault.
    ///
    /// Read **once**, at open, rather than per record: this process is the only
    /// writer, so it can count. A `stat` per record would be a syscall per
    /// record for a number it already knows.
    fn file_size(&mut self, path: &str) -> Result<u64, Errno>;

    /// When a path was last written, in **seconds since the epoch**, or nothing
    /// if that cannot be answered. `crate::retention`.
    ///
    /// # It is a tie-break, and the type says so
    ///
    /// `Option` rather than `Result` because there is exactly one caller and
    /// every way of failing is the same answer to it: retention orders a
    /// product's libraries by **version**, and this only separates two that a
    /// version cannot (`SPEC-02` §12, `ADR-052` §7's argument for the same
    /// ordering in the artifact store). A caller that could not learn a
    /// directory's age is not blocked; it is left ordering by the reference,
    /// which is total and deterministic and merely arbitrary.
    ///
    /// **Seconds, and `ADR-021` is why that is enough and no more.** A Pi has
    /// no clock across a power cut and restores a floor that never goes
    /// backwards, so this orders two events within one machine's life and says
    /// nothing about a date. It is never compared across machines.
    fn modified_secs(&mut self, path: &str) -> Option<u64>;

    // -- the crash-safe write's four (2026-08-23, `ADR-021` §3) --------------
    //
    // They are here for the reason the `/etc` projection's four are: the thing
    // that performs them has to *journal* what it did, and the journal is
    // reached through `Kernel::emit`. A second port would mean a second adapter
    // object and a clock restore that could not report its own failure, which
    // `ADR-017` §8 item 2 rules against.
    //
    // Together with `Kernel::write_file` and `Kernel::read_file` they are
    // exactly `rivet_store::Filesystem`, which is the point: `crate::clock`
    // adapts this port to that one and **reuses `rivet-store`'s own
    // crash-safe sequence** rather than writing a second copy of it. See
    // `crate::document`, which does the same thing read-only.

    /// Flush one file's contents to the medium — `fsync(2)`.
    ///
    /// Until it returns, a rename over the target can put a directory entry in
    /// place pointing at blocks the card has never been told to write, and a
    /// power cut then produces a file that is present, the right length, and
    /// full of zeroes.
    fn sync_file(&mut self, path: &str) -> Result<(), Errno>;

    /// Rename within one filesystem — `rename(2)`, atomic with respect to a
    /// concurrent reader and to a crash.
    fn rename(&mut self, from: &str, to: &str) -> Result<(), Errno>;

    /// Flush a *directory*, so a rename inside it survives a power cut.
    ///
    /// Distinct from [`Kernel::sync_file`]: flushing the file makes its contents
    /// durable, flushing the directory makes the **name** pointing at them
    /// durable.
    fn sync_dir(&mut self, path: &str) -> Result<(), Errno>;

    /// Remove a file. Used only to clear a temp a save has just failed to write.
    fn remove_file(&mut self, path: &str) -> Result<(), Errno>;

    /// The kernel's mount table.
    ///
    /// A default method rather than a separate syscall, so the real and the
    /// fake reach it by one route and this body is covered by the tests.
    fn read_mounts(&mut self) -> Result<String, Errno> {
        self.read_file(MOUNTS_PATH)
    }

    /// Block `SIGTERM`, `SIGINT`, `SIGCHLD` and `SIGHUP` and route them to a
    /// descriptor.
    ///
    /// **`SIGHUP` joined the set 2026-08-26** (`ADR-032` §3.2), and blocking it
    /// is the load-bearing half rather than a detail: its default disposition
    /// is to TERMINATE the process. A supervisor that reached `signalfd` for
    /// three signals and left the fourth at its default would die the first
    /// time an operator asked it to reload — and on a board that is pid 1
    /// losing its only child.
    ///
    /// A refusal is survivable and must be *said*: without it there is no
    /// shutdown path and no child-exit wakeup, so the supervisor degrades to
    /// polling and default signal dispositions.
    fn arm_signals(&mut self) -> Result<(), Errno>;

    /// Open `/dev/kmsg` and start delivering [`Event::KernelMessage`].
    ///
    /// `ADR-022` §4.1. A refusal is survivable and must be *said*, exactly as
    /// [`Kernel::arm_signals`]'s is: without it the kernel's own messages reach
    /// the console and nothing else, which is the last thing an HDMI capture
    /// shows that `rivet get log` does not.
    ///
    /// **Opening reads the buffer from the beginning**, which is `/dev/kmsg`'s
    /// documented behaviour and is what makes this worth doing: the supervisor
    /// starts after the kernel has already said everything interesting about
    /// bringing the hardware up, and gets all of it.
    fn open_kmsg(&mut self) -> Result<(), Errno>;

    /// Bind the journal ingest socket and start delivering [`Event::Ingest`].
    ///
    /// `ADR-022` §11. A refusal is survivable and must be *said*, exactly as
    /// [`Kernel::arm_signals`]'s and [`Kernel::open_kmsg`]'s are: without it
    /// every process that is not a child of the supervisor falls back to its own
    /// stderr, which is a machine whose `rivet get log` is missing the
    /// operator's own edits.
    ///
    /// **The path is an argument** rather than being read from the constant
    /// here, so that a test can bind one in a temporary directory and drive the
    /// whole thing on the build host (`AGENTS.md` §8.3 — paths are arguments,
    /// never constants). `crate::main` passes
    /// `rivet_core::journal::INGEST_SOCKET_PATH`.
    fn open_journal_ingest(&mut self, path: &str) -> Result<(), Errno>;

    /// Bind `ADR-032` §3.2's query socket and start delivering
    /// [`Event::Queried`]. Added 2026-08-26.
    ///
    /// **`0600`, which is §3.3 item 2's ruling and the difference from the
    /// socket next door.** The ingest socket is `0666` because filing a record
    /// is something any process legitimately does; reading the supervisor's
    /// state is not the same act, and §3.4 is a whole section on why widening
    /// that one instead is the trap.
    ///
    /// A refusal is survivable and must be *said*: without it `rivet get status`
    /// answers "the supervisor is not answering", which is true and is the one
    /// case §3.3 item 5 wants legible.
    fn open_query(&mut self, path: &str) -> Result<(), Errno>;

    /// Bind `rivet_core::kiosk`'s socket and give it to `owner`.
    ///
    /// **Chowned, not `0666`.** The grant is `ADR-015` §5.3's — declare, chown,
    /// one owner at a time — which is the same mechanism `crate::endpoints` uses
    /// for an application-to-application endpoint and `§6.2` uses for a device
    /// node. `None` leaves it root-owned, which is a machine with no launcher
    /// and therefore nothing that may ask.
    fn open_kiosk(
        &mut self,
        path: &str,
        owner: Option<rivet_core::service::AppIdentity>,
    ) -> Result<(), Errno>;

    /// Answer the request that produced the last [`Event::KioskAsked`].
    fn answer_kiosk(&mut self, answer: &str);

    /// **Bind `rivet_core::facts`'s socket and start delivering
    /// [`Event::FactsAsked`]** — `ADR-057`.
    ///
    /// **`0666`, and it is the ingest socket's argument rather than the query
    /// socket's.** Asking a machine what version it is running is something any
    /// application on it legitimately does: the answer carries no secret, no
    /// credential and no other application's prose, and `/run/rivet` is
    /// root-owned so nothing can put a socket of its own beside this one. The
    /// alternative — `ADR-015` §5's declare-and-chown, which is how the kiosk
    /// socket is granted — is **one consumer per endpoint**, and a platform
    /// surface every future application inherits cannot have one consumer.
    ///
    /// A refusal is survivable and must be *said*: an application whose facts
    /// call goes unanswered concludes the supervisor has stopped, so a socket
    /// that was never bound would make every dashboard on the machine report a
    /// wedge that has not happened.
    fn open_facts(&mut self, path: &str) -> Result<(), Errno>;

    /// **Answer the oldest request that produced an [`Event::FactsAsked`]**.
    ///
    /// A call with nothing pending is a no-op, and a send that fails is a
    /// caller that went away — ordinary, and nowhere to report it that the
    /// asker would read.
    fn answer_facts(&mut self, answer: &str);

    /// **Answer the request that produced the last [`Event::Queried`]**, and
    /// close the connection.
    ///
    /// The kernel holds the accepted socket and the caller holds the state, so
    /// the answer has to travel one of the two ways; this is the direction that
    /// keeps [`Event`] plain data and testable. A call with nothing pending is a
    /// no-op rather than an error — the connection may have gone away between
    /// the request and the answer, which is ordinary.
    fn answer_query(&mut self, answer: &str);

    /// Wait until something happens or `timeout_ms` elapses. `None` waits
    /// indefinitely — which is what a supervisor with nothing pending must do,
    /// rather than spinning.
    fn wait(&mut self, timeout_ms: Option<u64>) -> Result<Vec<Event>, Errno>;

    /// Create the `sd_notify` socket for a service and return the path to put
    /// in its `NOTIFY_SOCKET`.
    /// Create this service's notification socket and return its path.
    ///
    /// `owner` is the identity that will **write** to it — `None` for a service
    /// that runs as root, which is every service from the image.
    ///
    /// # Why the owner is a parameter, added 2026-08-25
    ///
    /// The socket is bound by this process, which is root, and a datagram
    /// socket needs **write** permission to send to. `ADR-015` §5.1's
    /// per-application identity arrived on 2026-08-24 and nothing connected the
    /// two, so every application got a socket it could not write to:
    ///
    /// ```text
    /// rivet-todo: could not declare readiness detail="Permission denied (os error 13)"
    /// ```
    ///
    /// Read off a board. `ready = "notify"` could therefore never succeed for
    /// **any** application — the service came up *running but never ready*, for
    /// ever, and the supervisor reported exactly that without anyone noticing it
    /// was unreachable rather than merely unreported.
    ///
    /// **Chowned to the service, not widened to everyone.** The journal ingest
    /// socket is `0666` and its reasoning does not carry here: that one exists
    /// so *anything with a record to file* can send (`ADR-022` §11.4), while
    /// this one has exactly one legitimate writer. A world-writable notify
    /// socket would let any local process declare another service ready, which
    /// is a small lie with a large blast radius — readiness is what a dependent
    /// service would wait on.
    fn open_notify(
        &mut self,
        service: &str,
        owner: Option<rivet_core::service::AppIdentity>,
    ) -> Result<String, Errno>;

    /// Drop a service's notify socket. Idempotent.
    fn close_notify(&mut self, service: &str);

    /// **Open every socket a service's manifest declares, and hold them.**
    /// `ADR-015` §5.2, `ADR-017` §3.2 and §3.4 — built 2026-08-31.
    ///
    /// Returns one `LISTEN_FDNAMES` token per socket, in the manifest's order,
    /// which is also the order the descriptors are handed over in. An empty
    /// `listen` returns an empty vector and opens nothing.
    ///
    /// # A token may not contain a colon, and the manifest's spelling does
    ///
    /// `LISTEN_FDNAMES` is colon-separated, so `tcp:80` and `unix:api` — which
    /// is how `ADR-015` §3.2 writes them and how a refusal should quote them —
    /// would arrive at an application as **four** names rather than two. The
    /// tokens are therefore `tcp-<port>` and the declared socket name, and the
    /// manifest's spelling is kept for anything a person reads.
    ///
    /// Found by running it: `socket-demo` on a board reported
    /// `LISTEN_FDNAMES tcp, 80, unix, api`.
    ///
    /// # Why the supervisor opens these and not the application
    ///
    /// `ADR-015` §5.2: *"Rather than granting `CAP_NET_BIND_SERVICE`, Rivet
    /// opens the listening socket and passes the file descriptor to the
    /// application. The application never holds the privilege and need not know
    /// the port."* An application runs as its own unprivileged uid (§5.1.1), so
    /// **a port below 1024 is unreachable to it** — `bind(2)` answers `EACCES`.
    /// This call is what makes one reachable, and the privilege is spent here,
    /// in a process that already has it, rather than granted to one that does
    /// not.
    ///
    /// For a **unix** socket the argument is different and is §5.3's: the path
    /// is not the application's to choose, because it is the thing another
    /// application will be granted access to.
    ///
    /// A **udp** socket takes the same argument as tcp — port 53 is as
    /// unreachable to an unprivileged uid as port 53/tcp is — and nothing
    /// more: `PLAN.md` §150.1 added the variant for a plain datagram service
    /// and deliberately stops there. See [`rivet_core::package::application::Socket::Udp`].
    ///
    /// # Where a tcp or udp socket is bound
    ///
    /// `proxied` is whether the service's signed manifest declares a
    /// `proxy-path`, and [`rivet_core::package::application::Socket::bind_address`]
    /// turns it and the socket into the address: `127.0.0.1` for a proxied
    /// application's port unless it is `:direct`, every address otherwise
    /// (`ADR-046` §7, `.rulings/` R-188). The implementation asks that function
    /// and decides nothing itself, so the address a test records and the one a
    /// board binds are one computation.
    ///
    /// # It is NOT socket activation
    ///
    /// The socket is opened when the service starts and closed when it stops.
    /// Nothing starts a service *because* a connection arrived —
    /// `ProgramKind::SocketActivated` remains a token in a manifest with
    /// nothing behind it, and `ADR-017` §3.2's ordering argument is untouched.
    /// What is built is the descriptor handoff, which is what an application
    /// author needs and what §5.2 is about.
    fn open_listeners(
        &mut self,
        service: &str,
        sockets: &[rivet_core::package::application::Socket],
        proxied: bool,
        owner: Option<rivet_core::service::AppIdentity>,
    ) -> Result<Vec<String>, Errno>;

    /// Close a service's listening sockets and remove any unix socket files.
    /// Idempotent.
    fn close_listeners(&mut self, service: &str);

    /// **Bind this application's answering socket and hand it over with the
    /// listening ones** — `ADR-045` §3, 2026-09-08.
    ///
    /// Returns the `LISTEN_FDNAMES` token to APPEND, which is
    /// `rivet_core::health::CHANNEL_NAME`. It is appended to whatever
    /// [`Kernel::open_listeners`] returned and the descriptor is appended to
    /// the same set, so this is one more socket handed over the way
    /// `tcp-8080` is rather than a sixth way of handing something over.
    ///
    /// **It must be called AFTER `open_listeners` for the same service**, which
    /// re-binds from scratch on every start and would otherwise discard this
    /// one.
    ///
    /// # Errors
    ///
    /// Whatever `bind(2)` or `chmod(2)` said. The caller treats it as
    /// `open_listeners` treats a port in use: the service is not started,
    /// because an application whose `LISTEN_FDS` and `LISTEN_FDNAMES` disagree
    /// is worse than one that did not start.
    ///
    /// # Why the answer is an `Option`, and why the default is `None`
    ///
    /// A `Kernel` that offers no health channel says so, and a supervisor built
    /// on one then appends no name — rather than advertising a socket that was
    /// never bound, which is the one failure an application cannot recover
    /// from. `crate::testkernel::FakeKernel` overrides it; every service under
    /// such a kernel is `unknown`, which is the honest no-op.
    fn open_health(&mut self, service: &str) -> Result<Option<String>, Errno> {
        let _ = service;
        Ok(None)
    }

    /// **Ask this service how it is** — `ADR-045` §3, 2026-09-08.
    ///
    /// Connects to `rivet_core::health::channel_path(service)`, writes
    /// `rivet_core::health::REQUEST`, and returns. **It does not wait**: the
    /// answer arrives at a later [`Kernel::wait`] as [`Event::Answered`], and
    /// its absence arrives as [`Event::Unanswered`] once
    /// `rivet_core::health::PATIENCE_MS` has passed.
    ///
    /// # Why this is not a blocking call with a timeout
    ///
    /// Because the supervisor is single-threaded and is PID 1's only child. A
    /// blocking ask would put the convergence pass behind whichever application
    /// is slowest to answer — and the case this mechanism exists to catch is
    /// precisely an application that never answers, so the blocking shape would
    /// stall the machine on exactly the machines the poll is for. `ADR-045`
    /// §3.1.1 gives the supervisor the cadence and the timeout; owning them
    /// means never being held by the thing being asked.
    ///
    /// A second call while one is in flight is a no-op: there is one question
    /// outstanding per service, and asking again would be the tight retry loop
    /// the ADR forbids.
    ///
    /// # Errors
    ///
    /// Whatever `connect(2)` or `write(2)` said. **A failure here is already an
    /// answer** — it is *this service is not answering* — so the caller reports
    /// it rather than retrying.
    ///
    /// # Why this has a default
    ///
    /// [`Kernel::open_display`]'s reason. A `Kernel` written for a test that
    /// says nothing about health asks nothing and answers nothing, which leaves
    /// every service `unknown` — the honest no-op rather than a silent
    /// `healthy`.
    fn ask_health(&mut self, service: &str) -> Result<(), Errno> {
        let _ = service;
        Ok(())
    }

    /// Drop any in-flight health question for this service, and its socket.
    /// Idempotent. Called beside [`Kernel::close_listeners`] when a service
    /// stops, so a question outlives neither the process it was asked of nor
    /// the socket it was asked on.
    fn close_health(&mut self, service: &str) {
        let _ = service;
    }

    /// Start a service. The returned pid is a child of this process.
    ///
    /// The child is given `crate::definitions::oom_score_adj(service)` before
    /// it executes anything, so no service inherits the supervisor's own.
    fn spawn(&mut self, service: &str, program: &Program) -> Result<i32, Errno>;

    /// **Set this process's own `oom_score_adj`** — how readily the kernel's
    /// OOM killer chooses it, `-1000` (never) to `1000`. `ADR-017` §3.
    ///
    /// # Errors
    ///
    /// Whatever writing `/proc/self/oom_score_adj` said: `ENOENT` with no
    /// `/proc`, `EACCES` for a value below what this process may set.
    fn set_own_oom_score_adj(&mut self, value: i32) -> Result<(), Errno>;

    /// **Run a blocking transaction off this process's only thread**, in a
    /// forked child rather than a spawned one — `ADR-039` §8.
    ///
    /// `transaction` is the whole of the work; it runs once, in the child,
    /// called as `transaction(lit)`. The child never execs: it reports
    /// `transaction`'s `Result` down the pipe [`Kernel::cec_poll`] reads and
    /// calls `_exit(2)` without returning, so it runs no destructor and
    /// flushes nothing it shares with this process.
    ///
    /// # Why a fork rather than a thread
    ///
    /// A thread shares this process's heap. If it is holding the allocator's
    /// lock at the moment something else forks — [`Kernel::spawn`], starting
    /// a service — that lock's state is copied into the service's child and
    /// nothing in the child will ever release it: the child deadlocks before
    /// it can `exec`, and [`Kernel::spawn`]'s caller blocks on it forever. A
    /// fork shares nothing: the child gets its own copy of the allocator,
    /// unlocked, and `transaction` may allocate as freely as it needs to.
    ///
    /// This is safe only because forking never runs where a second thread
    /// could exist — see [`Kernel::spawn`]'s own note on the same invariant,
    /// and `tests::no_non_test_source_spawns_a_thread`, which is what keeps it
    /// true.
    ///
    /// # Errors
    ///
    /// Whatever `pipe(2)` or `fork(2)` said. The transaction did not run at
    /// all, and the caller reports that exactly as it would report
    /// [`Kernel::cec_poll`] answering `Err`.
    fn cec_send(&mut self, lit: bool, transaction: CecTransaction) -> Result<CecWorker, Errno>;

    /// Non-blocking: `None` while `worker` has not answered yet.
    ///
    /// **The one default on this trait that is a real implementation rather
    /// than a stub.** Resolving a [`CecWorker`] means the same thing
    /// regardless of which [`Kernel`] dispatched it — reap [`CecWorker::Child`]
    /// without blocking and read what it wrote, or take [`CecWorker::Resolved`]'s
    /// answer — so there is one implementation rather than one per `impl`.
    fn cec_poll(&mut self, worker: &mut CecWorker) -> Option<Result<String, String>> {
        match worker {
            CecWorker::Resolved(slot) => slot.take(),
            CecWorker::Child { pid, read } => poll_cec_child(*pid, read),
        }
    }

    /// **Delete, in a short-lived forked helper, every entry of `dir` whose
    /// name begins with `prefix`** — the deleting half of renaming a tree
    /// aside, `ADR-017` §3.
    ///
    /// Deleting a tree costs time in proportion to what is in it, and the
    /// supervisor's loop is single threaded; a rename costs one syscall. So
    /// the caller renames the tree aside under `prefix` inside the loop, where
    /// the name it wanted is free again at once, and this takes the time out
    /// of it. Every entry carrying `prefix` is deleted, not only the newest,
    /// so one a power cut left behind is reclaimed by the next helper.
    ///
    /// Returns once the helper is forked; the outcome arrives later as
    /// [`Event::Reclaimed`] carrying `service`. The helper is forked for
    /// [`Kernel::cec_send`]'s reasons and under its invariant, and is reaped
    /// by this kernel rather than by [`Kernel::reap_ready`]'s caller.
    ///
    /// # Errors
    ///
    /// Whatever `fork(2)` said. Nothing was deleted, and the entries stay for
    /// the next helper.
    fn reclaim_in_child(&mut self, service: &str, dir: &str, prefix: &str) -> Result<(), Errno>;

    /// **Issue one leaf from this machine's fleet root, in a short-lived forked
    /// helper, and write it where `order` says** — `ADR-063` §4.
    ///
    /// The helper reads the root's key, signs, writes and exits; the root's key
    /// is read only after the fork, so it is never in this process's memory.
    /// This call waits for the helper, which takes milliseconds: what it
    /// writes is what the next start of the applications it serves reads, so
    /// the caller must not go on before it is there. It is forked for
    /// [`Kernel::cec_send`]'s reasons and under its invariant.
    ///
    /// # Returns
    ///
    /// `Ok(Ok(summary))` when every file is in place, `Ok(Err(why))` when the
    /// helper refused or failed — in which case each file it had not replaced is
    /// as it was — and `Err` when the helper could not be forked.
    ///
    /// # Errors
    ///
    /// Whatever `fork(2)` or `pipe2(2)` said. Nothing was issued.
    fn issue_in_child(&mut self, order: &IssueOrder) -> Result<Result<String, String>, Errno>;

    /// **What Landlock this kernel offers** — the ABI version, or `0` for none,
    /// with the errno that said so. `ADR-042` §5.
    ///
    /// Asked **at every start** rather than once at boot, because
    /// `AGENTS.md` §5 makes the kernel the authority for system state and
    /// forbids caching its verdicts. A supervisor that read this once would
    /// answer for the kernel it started under, and an A/B update changes the
    /// kernel underneath `/state` without changing this process.
    ///
    /// Defaulted to `(0, ENOSYS)` so that a `Kernel` written for a test which
    /// says nothing about Landlock says *there is none*, which is the value
    /// that refuses rather than the one that pretends.
    fn landlock_abi(&mut self) -> (u32, i32) {
        (0, libc::ENOSYS)
    }

    /// **Replace Rivet's nftables table with `policy`** and read back what the
    /// kernel then holds — `ADR-023` §9, `rivet_adapter_net::firewall`.
    ///
    /// The real adapter only opens a `NETLINK_NETFILTER` socket and hands it
    /// over: the transaction, its acknowledgements and the read-back are the
    /// adapter crate's, tested there against a fake socket. A socket that
    /// cannot be opened, or a step the kernel refuses, is the error text; the
    /// kernel applies a refused transaction not at all.
    ///
    /// Defaulted to a refusal, so a `Kernel` that says nothing about a firewall
    /// says it applied none rather than reporting a table it never made.
    ///
    /// # Errors
    ///
    /// The socket, or the step the kernel refused, in words.
    fn apply_firewall(
        &mut self,
        policy: &rivet_adapter_net::firewall::Policy,
    ) -> Result<rivet_adapter_net::firewall::Applied, String> {
        let _ = policy;
        Err("this kernel adapter applies no firewall".to_string())
    }

    /// **Take the display lease that came with the last kiosk request.**
    ///
    /// `PLAN.md` §106.2. `None` when the request carried none — `return` and
    /// `restart` never do, and nor does a `select` from a launcher still using
    /// the drop-master path (`§104.41`), which keeps working while this is
    /// built.
    ///
    /// Taking it transfers ownership: a caller that takes and drops it closes
    /// it, which is the correct outcome for a lease nothing will use.
    ///
    /// Defaulted for the reason [`Kernel::open_display`] gives.
    fn take_kiosk_lease(&mut self) -> Option<std::os::fd::OwnedFd> {
        None
    }

    /// **Open this service's display channel** — `PLAN.md` §106.2.
    ///
    /// A `socketpair`: the supervisor keeps one end, the child is handed the
    /// other at [`Kernel::spawn`] and told its number in `RIVET_DISPLAY`. Every
    /// display descriptor the application ever gets arrives on it, including
    /// the first — which is what makes re-issuance possible at all
    /// (`ADR-036` §8.1).
    ///
    /// Replacing an existing channel closes the old one. A restarted
    /// application gets a fresh pair, because the old child's end died with it.
    ///
    /// # Errors
    ///
    /// The errno `socketpair` gave.
    ///
    /// # Why this has a default
    ///
    /// So the test double is not obliged to model a socket it never reads. A
    /// `Kernel` that does nothing here produces an application with no
    /// `RIVET_DISPLAY`, which is exactly the state of every service that is not
    /// a kiosk one — the honest no-op rather than a silent hole.
    fn open_display(&mut self, service: &str) -> Result<(), Errno> {
        let _ = service;
        Ok(())
    }

    /// **Send one display descriptor to this service**, over the channel
    /// [`Kernel::open_display`] made.
    ///
    /// Consuming: the descriptor is the application's once this returns, and
    /// this process closes its copy.
    ///
    /// # Errors
    ///
    /// [`Errno::ENOENT`] when no channel is open for that service, or whatever
    /// `sendmsg` said.
    fn send_display(
        &mut self,
        service: &str,
        lease: std::os::fd::OwnedFd,
    ) -> Result<(), Errno> {
        let (_, _) = (service, lease);
        Ok(())
    }

    /// **Close this service's display channel**, when it stops.
    fn close_display(&mut self, service: &str) {
        let _ = service;
    }

    /// Reap every child that has exited, without blocking.
    ///
    /// The supervisor is not PID 1 but it is a parent, and a child it does not
    /// reap is a zombie for as long as the supervisor lives. Infallible by
    /// design: there is no useful way to respond to "the wait failed" other
    /// than to try again on the next wakeup, which is what returning an empty
    /// list already causes.
    fn reap_ready(&mut self) -> Vec<(i32, ExitStatus)>;

    fn signal_process(&mut self, pid: i32, sig: Sig) -> Result<(), Errno>;

    /// Ask the kernel what is in a service's cgroup (`ADR-017` §3.1.3).
    fn observe(&mut self, service: &str) -> Observation;

    /// Ask whether the cgroup2 hierarchy exists at all (`ADR-017` §3.1.3.1).
    ///
    /// Separate from [`Kernel::observe`] for the same reason
    /// [`Kernel::place_in_cgroup`] is separate from [`Kernel::spawn`]: it is a
    /// different fact with a different consequence, and one call cannot report
    /// two. `observe` answers about one service; this answers about the
    /// machine, and only their *combination* says whether a supervisor is
    /// blind.
    ///
    /// **Never cached.** `AGENTS.md` §5 — the kernel is the authority for
    /// system state and its verdicts are read at the point of use. It costs one
    /// `read(2)` on a pseudo-filesystem, and only on the path where an
    /// observation already failed.
    fn cgroup_hierarchy(&mut self) -> Hierarchy;

    /// Put a freshly spawned process into its service's cgroup.
    ///
    /// Separate from [`Kernel::spawn`] because it can fail on its own, and a
    /// failure means the *observation* above stops being authoritative while
    /// the service itself is perfectly fine. Those are different facts and the
    /// supervisor reports them differently.
    fn place_in_cgroup(&mut self, service: &str, pid: i32) -> Result<(), Errno>;

    /// Kill every process a service's cgroup holds, in one step, if this
    /// kernel can — `ADR-017` §3, S-1's fix.
    ///
    /// Writes `1` to `<cgroup>/cgroup.kill` (Linux ≥ 5.14), which the kernel
    /// documents as delivering `SIGKILL` to every process the cgroup holds,
    /// atomically — including a helper a stopped service forked that this
    /// supervisor never signalled directly, because it never knew the pid.
    ///
    /// # Errors
    ///
    /// `Err` when this kernel has no `cgroup.kill` (older than 5.14) or the
    /// cgroup itself is not there. The caller reads either as "fall back to
    /// walking [`Kernel::observe`]'s members by hand" — this method does not
    /// do that itself, so it needs no grace period and no journal.
    fn cgroup_kill(&mut self, service: &str) -> Result<(), Errno>;

    // -- the 2026-08-24 split's two ----------------

    /// What is in a directory, as names rather than paths. **Not recursive.**
    ///
    /// `Err(Errno::ENOENT)` for a directory that is not there, which
    /// `crate::applications` reads as *no applications are installed* — a
    /// perfectly ordinary machine — rather than as a fault. Every other error is
    /// a fault: a card returning `EIO` must not be indistinguishable from an
    /// empty machine, or a supervisor would silently run none of what an
    /// operator installed (`AGENTS.md` §7, in the direction that costs most).
    ///
    /// Only entries that are **directories** are returned. An application is a
    /// directory under `rivet_core::service::APP_ROOT` (`ADR-015` §7), so a
    /// stray file there is not a half-installed application and reporting it as
    /// one would produce a diagnostic about a service that was never meant to
    /// exist.
    fn list_dir(&mut self, path: &str) -> Result<Vec<String>, Errno>;

    /// Every entry in a directory, **whatever type it is**.
    ///
    /// # Why this is not [`Kernel::list_dir`] with the filter removed
    ///
    /// That one answers **directories only**, and the filter is a security
    /// property rather than a convenience: it discovers installed applications
    /// under `/state/app`, where an entry is `<name>/`, and following or
    /// accepting a link there would let a symlink into `/system` present
    /// itself as an installed application.
    ///
    /// This one exists for `/dev`, where **nothing is a directory** — a
    /// framebuffer and an evdev node are character devices — so the same filter
    /// would answer "there is nothing here" about a directory full of things.
    /// That is not hypothetical: `crate::devices` was written against
    /// `list_dir` first, and it would have granted **zero** input nodes on
    /// every real board while every test passed.
    ///
    /// **It has two callers and both are closed constructions.** It is not a
    /// general directory walker and must not become the easy way to have one;
    /// what keeps that true is not the count but the fact that neither caller
    /// takes its path from anything an operator or a file supplies:
    ///
    /// 1. `ADR-015` §6.2's device grant, whose paths are
    ///    `rivet_core::service::device_globs`'s compile-time constants;
    /// 2. `crate::deploy_request`'s resolution of an artifact digest, added
    ///    2026-09-12, whose path is `rivet_core::artifact::Digest::object_dir`
    ///    — and a `Digest` cannot be built from anything but 64 hex characters,
    ///    so the directory it names is inside the store by construction.
    ///
    /// The second is why the filter matters here in the other direction from
    /// [`Kernel::list_dir`]: an object directory holds a **file**, so the
    /// directories-only answer would report an empty store on every machine.
    fn list_dir_entries(&mut self, path: &str) -> Result<Vec<String>, Errno>;

    /// Cap a service's memory — cgroups v2 `memory.max`, in bytes.
    ///
    /// **The operator's half of the 2026-08-24 ruling, as a syscall.** The
    /// document owns the cap (`ADR-007` §7.2) and `ADR-015` §5.1 names cgroups
    /// v2 as the mechanism; without this the ruling would be a number in a
    /// journal record.
    ///
    /// Separate from [`Kernel::place_in_cgroup`] for the reason that is
    /// separate from [`Kernel::spawn`]: it fails on its own, for its own reason,
    /// and the consequence is different. A failed placement means the supervisor
    /// cannot *observe* the service; a failed cap means the kernel is not
    /// *limiting* it, and a cap reported and not enforced is worse than no cap.
    ///
    /// **`ENODEV` is the answer that matters and it is a real one on a board.**
    /// `ADR-024` §8.1 found the `memory` controller absent on the Raspberry Pi,
    /// and §8.2 found the same kernel offering it under an emulator — so this
    /// call succeeds under QEMU and may not on hardware, which is exactly why it
    /// returns a result rather than being assumed.
    fn apply_memory_limit(&mut self, service: &str, bytes: u64) -> Result<(), Errno>;

    /// Cap a service's CPU — cgroups v2 `cpu.max`, in microseconds of quota
    /// against `rivet_core::service::CpuLimit::PERIOD_US`.
    ///
    /// **[`Kernel::apply_memory_limit`]'s sibling, mirrored rather than
    /// designed afresh** — `ADR-007` §7 and `ADR-015` §3's 2026-09-09
    /// amendment gave `cpu` the identical split `memory` already has, and
    /// this is that ruling's second syscall. Every reason
    /// [`Kernel::apply_memory_limit`] gives for its own shape — separate from
    /// [`Kernel::place_in_cgroup`], a result rather than an assumption —
    /// applies here unchanged.
    ///
    /// # Unlike memory, this build's own audit found no missing controller
    ///
    /// `ADR-024` §8's board read `/sys/fs/cgroup/cgroup.controllers` as
    /// `cpuset cpu io pids` **before** the memory fix — `cpu` was present the
    /// whole time the `memory` controller was not. This method still returns
    /// a `Result` rather than assuming success, because `AGENTS.md` §5 —
    /// *refuse rather than guess* — does not get to relax on the strength of
    /// one board's reading, and a machine that somehow lacks it must say so
    /// exactly as a memory-refusing one does.
    fn apply_cpu_limit(&mut self, service: &str, quota_us: u64) -> Result<(), Errno>;

    /// Grow `/state` to fill the device it was written to. `ADR-018` §10.3.
    ///
    /// # Why the whole operation is one method rather than seven primitives
    ///
    /// `ADR-018` §10.7.3 rules this the supervisor's and describes the
    /// implementor as *"four syscalls at the supervisor's own `libc` wall"*.
    /// Every **decision** already lives in `rivet_adapter_image::grow`, behind
    /// its own `GrowMedium` port, with a thousand lines of tests over a fake and
    /// a loop-device run behind it (§10.7). Putting seven raw primitives on
    /// *this* trait would oblige `crate::testkernel::FakeKernel` to grow a
    /// second, worse simulation of a GPT and an `ext4` superblock in order to
    /// test decisions that are not this crate's — and `ADR-011` would then have
    /// two answers to "what does a grow do".
    ///
    /// So the seam is at the operation. `LibcKernel` builds a `GrowMedium` over
    /// real descriptors and hands it to `grow_state`; the fake answers with a
    /// canned outcome, which is all the *call site* has to branch on.
    ///
    /// # It cannot fail
    ///
    /// [`Grew`] is total: refusals and syscall failures are values, not errors.
    /// That is `ADR-018` §10.7.3's ruling made structural — *"a failure degrades
    /// the boot and never withholds the confirmation: a rollback cannot repair a
    /// `/state` an update never touches."* There is no `Result` here to be
    /// tempted into propagating.
    fn grow_state(&mut self) -> Grew;
}

// ---------------------------------------------------------------------------
// The real one. Thin by policy: nothing here is reachable from a test.
// ---------------------------------------------------------------------------

/// One `signalfd_siginfo`. The kernel writes whole structures; the first
/// `u32` is `ssi_signo`, which is all the supervisor reads — `waitpid` asks
/// the authoritative question about *which* child anyway.
const SIGINFO_SIZE: usize = std::mem::size_of::<libc::signalfd_siginfo>();

/// **Every signal the supervisor blocks and reads through `signalfd(2)`.**
///
/// Written once and read by [`Kernel::arm_signals`], so the mask and the list a
/// test asserts cannot disagree — this was two spellings until 2026-08-26, when
/// `SIGHUP` had to be added to a list that also appeared in two comments.
///
/// **Blocking is the load-bearing half for `SIGHUP`, not a detail.** Its default
/// disposition is to **terminate the process**. A supervisor that took three
/// signals through `signalfd` and left the fourth at its default would die the
/// first time an operator asked it to reload — and on a board this process is
/// pid 1's only child, so that is the machine losing its supervisor in response
/// to a request to reconfigure it.
/// **The query socket's mode.** `ADR-032` §3.3 item 2, ruled 2026-08-25:
/// `0600`, root only.
///
/// Written here rather than beside `rivet_core::journal::INGEST_SOCKET_MODE`
/// because the two are not the same fact and must not look like one: that mode
/// is a ruled consequence of any process legitimately filing a record, and
/// §3.4 is a whole section on why inheriting it here would be wrong.
/// **Widening later is a decision; narrowing later is a break**, and the
/// trigger §3.3 names is a requester that is not root — `rivet-gitd` is the
/// first candidate and does not exist.
const QUERY_SOCKET_MODE: u32 = 0o600;

const BLOCKED_SIGNALS: [i32; 4] = [libc::SIGTERM, libc::SIGINT, libc::SIGCHLD, libc::SIGHUP];

/// **Which shutdown was asked for.** `ADR-022` §12 item 7, and `ADR-017`
/// §3.1.10's table read from this end.
///
/// The supervisor does not perform either — pid 1 does (`ADR-017` §3.1.5 item
/// 6) — so this is what it was *told*, and the record says so in those words.
/// It exists because §12 item 7 requires the shutdown record to carry *whether a
/// reboot or a power-off*, and until 2026-08-26 `Event::Terminate` collapsed the
/// two signals into one variant, which made that field unwritable rather than
/// merely unwritten.
///
/// **`SIGUSR1` is absent and that is correct**: §3.1.10 gives it to pid 1 and
/// pid 1 stops this process with `SIGTERM` regardless, so from here a trial
/// restart and a power-off arrive identically. The record says what this process
/// was told, never what it inferred (`AGENTS.md` §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Terminating {
    /// `SIGTERM`. `ADR-017` §3.1.10: shut down and stay off.
    PowerOff,
    /// `SIGINT`. Shut down and come back — the kernel also delivers this for
    /// Ctrl-Alt-Delete, which is why it has been handled since Stage 0.
    Restart,
}

impl Terminating {
    /// The wire form, for the record's own field.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PowerOff => "power-off",
            Self::Restart => "restart",
        }
    }

    /// The signal that means it, named because the record's reader is usually
    /// looking at a console that printed the number.
    pub const fn signal(self) -> &'static str {
        match self {
            Self::PowerOff => "SIGTERM",
            Self::Restart => "SIGINT",
        }
    }
}

/// How long the degraded loop sleeps when `signalfd(2)` was refused.
pub const DEGRADED_POLL_MS: u64 = 200;

/// Split Unix epoch milliseconds into a `timespec`'s two fields, or `None` if
/// this platform's `timespec` cannot hold them.
///
/// A free function rather than the two lines inline, for the reason this module
/// opens with: `LibcKernel` is where nothing is reachable from a test, and the
/// **conversion** is the part that can be wrong. It is also the part whose
/// correctness varies by target — `time_t` is 64-bit on `aarch64` and 32-bit on
/// the armv6 of `AGENTS.md` §10.2's deferred compatibility target, where a
/// perfectly ordinary date overflows it and must be refused rather than
/// truncated into some other year.
///
/// Deliberately **not** tested by calling `clock_settime`: a test that succeeded
/// would have set the clock of the machine running the suite.
fn timespec_parts(unix_ms: u64) -> Option<(libc::time_t, libc::c_long)> {
    let secs = <libc::time_t>::try_from(unix_ms / 1000).ok()?;
    let nanos = <libc::c_long>::try_from((unix_ms % 1000) * 1_000_000).ok()?;
    Some((secs, nanos))
}

/// How the supervisor renders records to the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleForm {
    /// `ADR-022` §3 — the on-disk form, and the authoritative one.
    Jsonl,
    /// A *view* of it (`ADR-022` §2), for a human at the HDMI console.
    Text,
}

/// The operating system, via `libc` and `std`.
pub struct LibcKernel {
    sigfd: Option<std::fs::File>,
    children: BTreeMap<i32, std::process::Child>,
    notify: BTreeMap<String, std::os::unix::net::UnixDatagram>,
    /// One entry per service with a `listen` in its manifest — `ADR-015` §5.2.
    ///
    /// **The supervisor owns these and the child never does** — it is handed a
    /// duplicate across `exec` and nothing else.
    ///
    /// They live exactly as long as the running service does: opened before the
    /// spawn, closed beside the notify socket when it stops. **A restart
    /// therefore re-binds rather than inheriting**, which is the same lifetime
    /// `open_notify` has and is deliberately not systemd's — keeping a socket
    /// bound across a restart so connections queue is socket activation, and
    /// `ADR-017` §3.2's ordering question is not answered by this change.
    listeners: BTreeMap<String, Listeners>,
    form: ConsoleForm,
    /// One per open pipe — `ADR-022` §4's capture. Two per running service.
    captures: Vec<Capture>,
    /// `/dev/kmsg`, once [`Kernel::open_kmsg`] has opened it — `ADR-022` §4.1.
    kmsg: Option<std::fs::File>,
    /// The journal ingest socket, once [`Kernel::open_journal_ingest`] has bound
    /// it — `ADR-022` §11.
    ingest: Option<std::os::unix::net::UnixListener>,
    /// `ADR-032` §3.2's query socket, and the one connection accepted on it.
    ///
    /// **One at a time, and that is enough.** A status request is answered
    /// inside the pass that read it, so a second caller waits one `poll` — and
    /// the listener leaves the set while an answer is outstanding, for the
    /// reason `accepting_ingest` gives: a listening socket with a connection
    /// waiting is permanently readable, and polling it while refusing to accept
    /// spins pid 1's only child.
    query: Option<std::os::unix::net::UnixListener>,
    /// The connection accepted on [`Self::query`], while its request is still
    /// arriving. See [`QueryIncoming`].
    query_incoming: Option<QueryIncoming>,
    /// `rivet_core::kiosk`'s socket, and the one connection accepted on it.
    /// Held apart from `query` so neither can answer the other's caller.
    kiosk: Option<std::os::unix::net::UnixListener>,
    kiosk_answering: Option<std::os::unix::net::UnixStream>,
    /// The connection accepted on [`Self::kiosk`], while its request is still
    /// arriving. See [`KioskIncoming`].
    kiosk_incoming: Option<KioskIncoming>,
    /// **`rivet_core::facts`'s socket** — `ADR-057`, and the one socket here
    /// that is a **datagram**.
    ///
    /// There is no listener and no accepted connection to hold, which is the
    /// whole reason it is a datagram: a stream socket's backlog is what the
    /// 2026-09-18 incident wedged this supervisor on (`STATE.md` §0,
    /// `unix_wait_for_peer`), and a surface built to make that failure visible
    /// must not be built on the mechanism that causes it.
    facts: Option<std::os::unix::net::UnixDatagram>,
    /// **Where to send each pending answer**, oldest first.
    ///
    /// A caller binds its own socket under `RIVET_APP_TMP` and this is the
    /// address `recv_from` reported — the sender's own, never anything out of
    /// the payload, so an answer cannot be aimed at a third party. A queue
    /// rather than one slot because `drain_facts` reads up to
    /// `FACTS_PER_WAKEUP` requests in a pass and `main` answers each in turn.
    facts_replies: std::collections::VecDeque<std::path::PathBuf>,
    /// **Per-application display channels** — `PLAN.md` §106.2.
    ///
    /// Keyed by service name. The value is the supervisor's end; the child's
    /// end lives beside it only until the spawn that hands it over, because a
    /// parent holding the child's end open would keep the socket alive after
    /// the child died and make a dead application look reachable.
    displays: std::collections::HashMap<String, DisplayChannel>,
    /// **A display lease that arrived with the last kiosk request**, waiting
    /// to be taken — `PLAN.md` §106.2, `Kernel::take_kiosk_lease`.
    ///
    /// Beside `kiosk_answering` and for its reason: both belong to one
    /// in-flight request, and neither is a value an `Event` can carry.
    kiosk_lease: Option<std::os::fd::OwnedFd>,
    answering: Option<std::os::unix::net::UnixStream>,
    /// Connections accepted on it and not yet finished. Bounded by
    /// `rivet_core::journal::INGEST_MAX_IN_FLIGHT`; see [`Incoming`].
    incoming: Vec<Incoming>,
    /// Lines drained after a child exited, waiting for the next [`Kernel::wait`]
    /// to hand them over.
    ///
    /// **`reap_ready` cannot return them**, because its signature is a list of
    /// exits, and a child's *last words* arrive after it has died — a service
    /// that fails writes the reason and then exits, and reading its pipe only
    /// while it is alive loses exactly the line worth having.
    pending: std::collections::VecDeque<Event>,
    /// **One outstanding health question per service** — `ADR-045` §3.
    ///
    /// Keyed by service name, so a second ask while one is in flight is a
    /// no-op rather than a second connection: the ADR forbids a tight retry
    /// loop, and the cheapest way to hold that is to make a duplicate question
    /// unrepresentable.
    asking: BTreeMap<String, Asking>,
    /// `rivet_core::health::CHANNEL_DIR`, always, outside this file's tests.
    health_dir: std::path::PathBuf,
    /// **Processes this kernel started and reaps itself**, by pid. A reclaim
    /// helper carries the service it reports for and the directory and prefix
    /// it was sent to; a one-shot program that [`Kernel::run_within`] killed at
    /// its deadline and could not reap at once carries `None`, and is reaped in
    /// silence. Never in `children`, so [`Kernel::reap_ready`] never hands one
    /// to the supervisor as a service exit; it reaps them beside the services.
    helpers: Vec<(i32, Option<Reclaiming>)>,
}

/// What a reclaim helper was sent to do, kept until it is reaped so that its
/// [`Event::Reclaimed`] can say.
#[derive(Debug)]
struct Reclaiming {
    service: String,
    dir: String,
    prefix: String,
}

/// A health question that has been sent, and whose answer has not arrived.
struct Asking {
    stream: std::os::unix::net::UnixStream,
    /// `now_ms` when the question went out. The deadline is this plus
    /// `rivet_core::health::PATIENCE_MS`, and it is stored as a start rather
    /// than as a deadline so that a reader can see what was waited for.
    since_ms: u64,
    /// Bytes read so far. An answer is one short line; the read stops at the
    /// first newline or at `rivet_core::health::ANSWER_MAX_BYTES`, so an
    /// application that writes without ever stopping cannot make the supervisor
    /// allocate.
    buffer: Vec<u8>,
}

/// One open pipe from one service, and the partial line it has written so far.
struct Capture {
    pid: i32,
    service: String,
    stream: Stream,
    file: std::fs::File,
    /// Bytes since the last newline. A service is under no obligation to write
    /// whole lines, and a record is per line.
    partial: Vec<u8>,
    /// The current partial line already hit [`MAX_CAPTURED_LINE`], so the rest
    /// of it is discarded up to the next newline. Set on the record that *was*
    /// emitted, so the truncation is visible where it happened.
    overflowed: bool,
}

/// `chmod(2)` one path.
///
/// `std::fs::set_permissions` rather than `libc::chmod`, because it is the same
/// syscall with no `unsafe` around it — the only reason this file reaches for
/// `libc` at all is calls `std` does not have.
fn set_mode(path: &str, mode: u32) -> Result<(), Errno> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| Errno::from_io(&e))
}

/// Ask the kernel who is on the other end of a connected `AF_UNIX` socket.
///
/// **`SO_PEERCRED`, and this is the whole security argument of `ADR-022` §11.3
/// in one call.** The kernel fills the structure in at `connect(2)` from the
/// connecting process's own credentials; nothing the sender writes on the socket
/// can change them, and a sender cannot choose them. So the uid, gid and pid on
/// an ingested record are *observed* rather than claimed — which is a different
/// thing from `rivet_core::deploy::Principal`, and that type's own
/// documentation carries the comparison.
///
/// **It attributes; it does not authorise.** Knowing which process sent a record
/// says nothing about whether the record is true, and this function is not a
/// permission check. `ADR-028` §4.5.
fn peer_credentials(
    stream: &std::os::unix::net::UnixStream,
) -> Result<rivet_core::journal::Peer, Errno> {
    use std::os::fd::AsRawFd as _;

    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = u32::try_from(size_of::<libc::ucred>()).unwrap_or(0);
    // SAFETY: `getsockopt` writes at most `len` bytes through the pointer, into
    // a live local of exactly that size, and reads nothing else. The descriptor
    // is owned by the caller and open for the whole call. `SO_PEERCRED` is valid
    // on a connected `AF_UNIX` stream socket, which is the only kind reaching
    // here — it is an `accept(2)` result.
    let rc = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut cred).cast::<libc::c_void>(),
            &raw mut len,
        )
    };
    if rc != 0 {
        return Err(Errno::last());
    }
    Ok(rivet_core::journal::Peer {
        uid: cred.uid,
        gid: cred.gid,
        pid: cred.pid,
    })
}

/// One accepted connection on the journal ingest socket — `ADR-022` §11.
///
/// **The credentials are taken once, at `accept(2)`.** They belong to the
/// process that connected, and that process may have exited by the time the
/// supervisor gets round to reading; asking later would ask about a pid that has
/// been reused. This is the same reason `Capture` keeps a `pid`.
struct Incoming {
    stream: std::os::unix::net::UnixStream,
    peer: rivet_core::journal::Peer,
    /// When it was accepted, on the monotonic clock. A connection that has not
    /// reached EOF by `INGEST_DEADLINE_MS` after this is closed.
    accepted_ms: u64,
    /// Bytes since the last newline. A sender is under no obligation to write
    /// its line in one `write(2)`.
    ///
    /// **Not stripped of `\r`, unlike [`Capture`]'s.** A captured line comes
    /// from a C daemon writing to what it thinks is a console; this is JSONL,
    /// where `rivet_core::journal::write_json_string` escapes every control
    /// character — so a raw carriage return on this socket is a malformed record
    /// and must read as one rather than being quietly repaired.
    partial: Vec<u8>,
}

/// The buffering rule for one ingest connection, with no socket in it.
///
/// Split out for [`Capture`]'s reason and tested the same way: a sender that
/// writes half a line, a line longer than the cap, and a line with no trailing
/// newline before EOF are three states that are awkward to produce through a
/// real socket and trivial to produce here.
impl Incoming {
    /// Take bytes off the wire. Returns `false` when the connection must be
    /// closed.
    fn feed(&mut self, bytes: &[u8], out: &mut Vec<Event>) -> bool {
        for byte in bytes {
            if *byte == b'\n' {
                self.emit_partial(out);
                continue;
            }
            if self.partial.len() >= rivet_core::journal::INGEST_MAX_BYTES {
                out.push(Event::Ingest {
                    peer: self.peer,
                    what: Ingest::TooLarge {
                        held: self.partial.len(),
                    },
                });
                self.partial.clear();
                return false;
            }
            self.partial.push(*byte);
        }
        true
    }

    /// The sender closed. Whatever is in hand is a complete line — EOF
    /// terminates one exactly as a newline does.
    fn at_eof(&mut self, out: &mut Vec<Event>) {
        self.emit_partial(out);
    }

    /// The deadline passed with the connection still open. Complete lines have
    /// already been delivered; an unterminated tail is refused, because a prefix
    /// of a record is not a record.
    fn expired(&mut self, out: &mut Vec<Event>) {
        if self.partial.is_empty() {
            return;
        }
        out.push(Event::Ingest {
            peer: self.peer,
            what: Ingest::Unfinished {
                held: self.partial.len(),
            },
        });
        self.partial.clear();
    }

    fn emit_partial(&mut self, out: &mut Vec<Event>) {
        if self.partial.is_empty() {
            return;
        }
        // Lossy rather than refusing, for `Capture::emit`'s reason: a sender's
        // bytes are bytes, and a record that could not be written because
        // somebody emitted a stray byte would be the journal failing at exactly
        // its job. The replacement character says so in the record, and
        // `rivet_core::journal::ingest` will refuse the line and keep it.
        let line = String::from_utf8_lossy(&self.partial).into_owned();
        self.partial.clear();
        out.push(Event::Ingest {
            peer: self.peer,
            what: Ingest::Line(line),
        });
    }
}

/// [`KIOSK_PATIENCE`] in the unit every deadline in this file compares
/// against [`LibcKernel::now_ms`] in.
///
/// Not a second constant: `Duration::as_millis` is the conversion, spelled
/// once here rather than at every comparison, so [`KIOSK_PATIENCE`] stays the
/// one place the value itself is written.
fn kiosk_patience_ms() -> u64 {
    u64::try_from(KIOSK_PATIENCE.as_millis()).unwrap_or(u64::MAX)
}

/// One accepted connection on the kiosk socket, while its request is still
/// arriving.
///
/// Exists for the reason [`Incoming`] exists for the ingest socket, and the
/// in-code comment [`LibcKernel::drain_kiosk`] used to carry named exactly
/// this restructuring: a read that used to happen inside one blocking call,
/// bounded per `read(2)` by [`KIOSK_PATIENCE`], is now spread across
/// [`Kernel::wait`]'s wakeups and bounded by it from `accepted_ms` — the
/// WHOLE request, not each read of it.
struct KioskIncoming {
    stream: std::os::unix::net::UnixStream,
    accepted_ms: u64,
    /// What [`recv_with_fd`] returned, once it has succeeded, and `None`
    /// until then: every wakeup before that retries it, because the
    /// descriptor arrives with the FIRST message and an ordinary `read`
    /// would discard it — the reason this socket's first read is
    /// `recv_with_fd` and not [`std::io::Read::read`].
    first: Option<String>,
    lease: Option<std::os::fd::OwnedFd>,
    /// Bytes read after the first message, unvalidated until the read
    /// completes.
    ///
    /// `Read::read_to_string`'s own contract, replicated by hand: it decodes
    /// the whole tail once, strictly, when the read is complete — never one
    /// chunk at a time, because a byte sequence a non-blocking `read(2)`
    /// split across two calls is not one character short of valid twice
    /// over, it is one character split in half.
    tail: Vec<u8>,
}

/// One accepted connection on the query socket, while its request is still
/// arriving — [`KioskIncoming`]'s shape, without the descriptor this socket
/// never carries.
struct QueryIncoming {
    stream: std::os::unix::net::UnixStream,
    accepted_ms: u64,
    asked: Vec<u8>,
}

impl LibcKernel {
    pub fn new(form: ConsoleForm) -> Self {
        Self {
            sigfd: None,
            children: BTreeMap::new(),
            notify: BTreeMap::new(),
            listeners: BTreeMap::new(),
            form,
            captures: Vec::new(),
            kmsg: None,
            ingest: None,
            query: None,
            query_incoming: None,
            kiosk: None,
            kiosk_answering: None,
            kiosk_incoming: None,
            facts: None,
            facts_replies: std::collections::VecDeque::new(),
            kiosk_lease: None,
            displays: std::collections::HashMap::new(),
            answering: None,
            incoming: Vec::new(),
            pending: std::collections::VecDeque::new(),
            asking: BTreeMap::new(),
            health_dir: std::path::PathBuf::from(rivet_core::health::CHANNEL_DIR),
            helpers: Vec::new(),
        }
    }

    /// Reap every helper that has exited, without blocking, and queue what
    /// each reclaim helper reported as an [`Event::Reclaimed`] for the next
    /// [`Kernel::wait`].
    fn reap_helpers(&mut self) {
        let mut running = Vec::with_capacity(self.helpers.len());
        for (pid, service) in std::mem::take(&mut self.helpers) {
            let mut status: libc::c_int = 0;
            // SAFETY: `pid` is this process's own child, started by
            // `reclaim_in_child` or `run_within` and reaped nowhere else — it
            // was never added to `children`. `WNOHANG` never blocks.
            let waited = unsafe { libc::waitpid(pid, &raw mut status, libc::WNOHANG) };
            let why = (waited < 0).then(Errno::last);
            if waited == 0 || why == Some(Errno::EINTR) {
                running.push((pid, service));
                continue;
            }
            // A killed one-shot program: reaped, and nothing to report.
            let Some(Reclaiming {
                service,
                dir,
                prefix,
            }) = service
            else {
                continue;
            };
            let failure = if let Some(why) = why {
                Some(format!("the helper could not be waited for: {why}"))
            } else if libc::WIFEXITED(status) {
                match libc::WEXITSTATUS(status) {
                    0 => None,
                    255 => Some("a deletion failed without an error number".to_string()),
                    code => Some(Errno(code).to_string()),
                }
            } else {
                Some(format!("the helper ended without exiting: wait status {status}"))
            };
            self.pending.push_back(Event::Reclaimed {
                service,
                dir,
                prefix,
                failure,
            });
        }
        self.helpers = running;
    }

    /// Where this service's answering socket is bound and asked on.
    /// `rivet_core::health::channel_path` on a machine; a scratch directory in
    /// a test, which is why the directory is a field rather than a constant.
    fn health_path(&self, service: &str) -> std::path::PathBuf {
        self.health_dir.join(service)
    }

    /// **Read what an in-flight health question has produced, and retire the
    /// ones that are finished or out of time** — `ADR-045` §3's three
    /// outcomes, as one function.
    ///
    /// Called after every `poll`, whether or not one of these descriptors was
    /// the reason it returned: the timeout branch has no descriptor to become
    /// readable, and a deadline nothing checks is not a deadline.
    fn drain_health(&mut self, out: &mut Vec<Event>) {
        use std::io::Read as _;

        let now = self.now_ms();
        let mut finished: Vec<String> = Vec::new();
        for (service, asking) in &mut self.asking {
            let mut buf = [0u8; 256];
            loop {
                // **`Ok(0)` is end-of-file and is a complete answer**, not an
                // error: an application that writes its line and closes is
                // behaving correctly, and the SDK's answering loop does exactly
                // that. `WouldBlock` is *nothing yet*, which is the ordinary
                // state of a question asked a microsecond ago.
                match asking.stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        asking.buffer.extend_from_slice(&buf[..n]);
                        if asking.buffer.contains(&b'\n')
                            || asking.buffer.len() >= rivet_core::health::ANSWER_MAX_BYTES
                        {
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => {
                        // A read that failed on a connected socket is a service
                        // that went away mid-answer, which is *it did not
                        // answer* and not a supervisor fault.
                        out.push(Event::Unanswered {
                            service: service.clone(),
                            why: e.to_string(),
                        });
                        finished.push(service.clone());
                        break;
                    }
                }
            }
            if finished.last().is_some_and(|s| s == service) {
                continue;
            }

            let complete = asking.buffer.contains(&b'\n')
                || asking.buffer.len() >= rivet_core::health::ANSWER_MAX_BYTES;
            if complete {
                let text = String::from_utf8_lossy(&asking.buffer).into_owned();
                // **The first line and nothing after it.** An answer is one
                // line; a service that wrote more has said something this
                // protocol does not carry, and reading past the newline would
                // be inventing a second message out of it.
                out.push(Event::Answered {
                    service: service.clone(),
                    line: text.lines().next().unwrap_or_default().to_string(),
                });
                finished.push(service.clone());
                continue;
            }

            // **The deadline, and it is the point of the whole design.** A
            // wedged application holds a listening socket that accepts nothing;
            // the connection sat in its backlog, the request was written into a
            // buffer nobody will read, and this is the only thing that ever
            // concludes anything about it.
            if now.saturating_sub(asking.since_ms) >= rivet_core::health::PATIENCE_MS {
                // Bytes with no newline are a half sentence, and a prefix of an
                // answer is not an answer — `AGENTS.md` §5, refuse rather than
                // guess at the rest.
                let held = asking.buffer.len();
                out.push(Event::Unanswered {
                    service: service.clone(),
                    why: format!(
                        "no answer within {} ms (ADR-045 §3); {held} byte(s) held, none of \
                         them a whole line",
                        rivet_core::health::PATIENCE_MS
                    ),
                });
                finished.push(service.clone());
            }
        }
        for service in finished {
            self.asking.remove(&service);
        }
    }

    /// When the soonest outstanding health question runs out of time.
    ///
    /// **The sleep is shortened by it**, for the reason an open ingest
    /// connection shortens it: a deadline is not a descriptor and `poll` cannot
    /// wake for one, so a supervisor with nothing else pending would hold a
    /// silent application's question until the next unrelated event — which on
    /// a quiet machine is never.
    fn next_health_deadline_ms(&mut self) -> Option<u64> {
        let now = self.now_ms();
        self.asking
            .values()
            .map(|a| {
                (a.since_ms + rivet_core::health::PATIENCE_MS).saturating_sub(now)
            })
            .min()
    }

    /// Accept what is waiting on the ingest socket, read what the accepted
    /// connections have written, and retire the ones that are done.
    ///
    /// **Accept is bounded by `INGEST_MAX_IN_FLIGHT` rather than draining the
    /// backlog**, and that is the whole answer to *many senders at once*: a
    /// connection the supervisor does not accept stays in the kernel's listen
    /// queue, and when that fills a further `connect(2)` is refused at the
    /// sender rather than costing this process anything. The sender then falls
    /// back to its own stderr, which is `ADR-022` §11.5's floor. Back-pressure,
    /// not a drop.
    fn drain_ingest(&mut self, out: &mut Vec<Event>) {
        let now = self.now_ms();
        if let Some(listener) = self.ingest.as_ref() {
            while self.incoming.len() < rivet_core::journal::INGEST_MAX_IN_FLIGHT {
                let Ok((stream, _)) = listener.accept() else {
                    break;
                };
                let Ok(peer) = peer_credentials(&stream) else {
                    // A connection whose credentials cannot be read is one the
                    // record could not honestly attribute. Dropped rather than
                    // attributed to nobody — `AGENTS.md` §5.
                    continue;
                };
                if stream.set_nonblocking(true).is_err() {
                    continue;
                }
                self.incoming.push(Incoming {
                    stream,
                    peer,
                    accepted_ms: now,
                    partial: Vec::new(),
                });
            }
        }

        let deadline = rivet_core::journal::INGEST_DEADLINE_MS;
        let mut buf = [0u8; 4096];
        let mut keep = Vec::with_capacity(self.incoming.len());
        for mut connection in std::mem::take(&mut self.incoming) {
            // **At most `DRAIN_BUDGET_BYTES` per connection per pass.** A
            // sender that writes without stopping still reaches its deadline,
            // and every other connection, pipe and signal is served between
            // its passes.
            let mut stream = &connection.stream;
            let mut said = Vec::new();
            let how = drain_within_budget(&mut stream, &mut buf, |bytes| {
                said.extend_from_slice(bytes);
                true
            });
            let mut open = connection.feed(&said, out);
            if open && matches!(how, Drained::Ended | Drained::Failed) {
                connection.at_eof(out);
                open = false;
            }
            if open && now.saturating_sub(connection.accepted_ms) >= deadline {
                connection.expired(out);
                open = false;
            }
            if open {
                keep.push(connection);
            }
        }
        self.incoming = keep;
    }

    /// **Accept one kiosk request, or make progress reading one already
    /// accepted** — `rivet_core::kiosk`, the narrow kiosk socket (`ADR-036`
    /// §5.4).
    ///
    /// `drain_query`'s shape exactly, with one difference that matters: this
    /// socket is chowned to an application's uid, so the caller is **not**
    /// root. That changes nothing about how a line is read and everything about
    /// what may be asked, which is why the vocabulary is `rivet_core::kiosk`'s
    /// three verbs and not `ADR-032` §3.2's operations.
    ///
    /// **Bounded by [`KIOSK_PATIENCE`] from `accept`, not per `read(2)`.**
    /// This socket belongs to an unprivileged uid, this supervisor is single
    /// threaded, and everything here used to read to EOF inside one blocking
    /// call with the deadline re-armed by every byte that arrived — so a
    /// caller trickling one byte just under it, or saying nothing at all,
    /// held this process for the request cap times the timeout: no reaping,
    /// no convergence, SIGTERM unread, and `cmd/rivet-initd` SIGKILLing this
    /// daemon and rebooting with every service orphaned. `Incoming` closes
    /// exactly this gap for the ingest socket, and this is that shape: the
    /// accepted connection is [`KioskIncoming`], parked across [`Kernel::
    /// wait`]'s wakeups in [`Self::kiosk_incoming`] with `accepted_ms` fixed
    /// at `accept`, and every wakeup either advances it or, once
    /// `kiosk_patience_ms` has passed, drops it.
    ///
    /// A line this build cannot read produces [`Event::KioskUnreadable`] rather
    /// than silence. The launcher is the only caller and it is a program in this
    /// repository, so a malformed request means the two halves disagree about
    /// the protocol — which is a defect worth a record, not a hostile client to
    /// be ignored.
    fn drain_kiosk(&mut self, out: &mut Vec<Event>) {
        use std::io::Read as _;

        if self.kiosk_answering.is_some() {
            return;
        }

        if self.kiosk_incoming.is_none()
            && let Some(listener) = self.kiosk.as_ref()
            && let Ok((stream, _)) = listener.accept()
            && stream.set_nonblocking(true).is_ok()
        {
            self.kiosk_incoming = Some(KioskIncoming {
                stream,
                accepted_ms: self.now_ms(),
                first: None,
                lease: None,
                tail: Vec::new(),
            });
        }
        let Some(mut connection) = self.kiosk_incoming.take() else {
            return;
        };

        if self.now_ms().saturating_sub(connection.accepted_ms) >= kiosk_patience_ms() {
            // Dropped in silence, matching what a `read` failure here has
            // always done: this socket has no refusal shape, and the launcher
            // — the only caller — is a program in this repository rather than
            // one an operator is waiting on.
            return;
        }

        // **The FIRST read must be a `recvmsg`**, because a descriptor arrives
        // with the bytes it was sent beside and with nothing else — read those
        // bytes with `read` and the ancillary data is discarded by the kernel,
        // silently. `PLAN.md` §106.2. Retried on `WouldBlock` rather than read
        // once and blocked on: nothing has arrived, so nothing was consumed,
        // and the descriptor — if the sender's first write has not landed yet
        // — is still waiting to be delivered with it on a later call.
        if connection.first.is_none() {
            match recv_with_fd(&connection.stream) {
                Ok((text, lease)) => {
                    connection.first = Some(text);
                    connection.lease = lease;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    self.kiosk_incoming = Some(connection);
                    return;
                }
                // A read that failed is a caller that hung up or sent bytes
                // that are not text; both are the same answer and neither is
                // worth two arms.
                Err(_) => return,
            }
        }

        // Then to EOF, as before. A stream socket may split a write, and the
        // close is still the frame. The descriptor came with the first
        // fragment or not at all, so nothing here can arrive late.
        let cap = rivet_core::kiosk::REQUEST_MAX_BYTES + 1;
        let mut buf = [0u8; 4096];
        loop {
            if connection.tail.len() >= cap {
                break;
            }
            match connection.stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let take = (cap - connection.tail.len()).min(n);
                    connection.tail.extend_from_slice(&buf[..take]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    self.kiosk_incoming = Some(connection);
                    return;
                }
                Err(_) => return,
            }
        }

        let Some(mut asked) = connection.first else {
            return;
        };
        match String::from_utf8(connection.tail) {
            Ok(more) => asked.push_str(&more),
            // `read_to_string`'s own refusal, replicated: a byte sequence
            // that is not valid UTF-8 is a caller sending bytes that are not
            // text, the same answer a failed `read` gets above.
            Err(_) => return,
        }

        self.kiosk_answering = Some(connection.stream);
        // **Held here, not carried in the event** — see `Event::KioskAsked`.
        // Replacing rather than adding: a previous request's lease that nobody
        // took is dropped and closed now, which is the only moment this code
        // can be sure it is finished with.
        self.kiosk_lease = connection.lease;
        match rivet_core::kiosk::Request::parse(&asked) {
            Ok(request) => out.push(Event::KioskAsked { request }),
            Err(why) => out.push(Event::KioskUnreadable { why }),
        }
    }

    /// **Accept one query, or make progress reading one already accepted, and
    /// decide whether to answer it.** `ADR-032` §3.2.
    ///
    /// (See [`recv_with_fd`] for the kiosk socket's out-of-band half; this
    /// socket has none and does not want one.)
    ///
    /// The request is read to EOF — the client closes its write half, so the
    /// close IS the frame, exactly as `ADR-022` §11's ingest socket works and
    /// `cmd/rivet`'s client does.
    ///
    /// **Bounded by [`KIOSK_PATIENCE`] from `accept`, not per `read(2)`,
    /// [`Self::drain_kiosk`]'s reason exactly** — this is that same
    /// restructuring, with [`QueryIncoming`] in [`Self::query_incoming`] in
    /// place of `KioskIncoming`. Root is not hostile, but root's client can be
    /// killed between its connect and its write, and a supervisor that
    /// stopped for ever because an operator pressed Ctrl-C at the wrong
    /// moment would be a defect whatever this socket's `0600` mode says —
    /// `0600` bounds who may hold the wait, not how long it lasts.
    ///
    /// **A request this build does not answer is refused at the door**, before
    /// any `Event` is emitted, and the connection is closed with a sentence
    /// rather than dropped: §3.3 item 1 rules the socket carries an operation,
    /// and a socket that silently ignored an unknown one would be a second
    /// command language whose vocabulary nobody could discover. The same
    /// sentence answers a request that timed out short of complete: the
    /// caller is owed a word either way, and this socket has exactly one
    /// sentence to give.
    fn drain_query(&mut self, out: &mut Vec<Event>) {
        use std::io::{Read as _, Write as _};

        const TOO_SLOW_OR_UNREADABLE: &[u8] =
            b"this build could not read the request, or it was longer than a request may be\n";

        if self.answering.is_some() {
            return;
        }

        if self.query_incoming.is_none()
            && let Some(listener) = self.query.as_ref()
            && let Ok((stream, _)) = listener.accept()
            && stream.set_nonblocking(true).is_ok()
        {
            self.query_incoming = Some(QueryIncoming {
                stream,
                accepted_ms: self.now_ms(),
                asked: Vec::new(),
            });
        }
        let Some(mut connection) = self.query_incoming.take() else {
            return;
        };

        if self.now_ms().saturating_sub(connection.accepted_ms) >= kiosk_patience_ms() {
            let _ = connection.stream.write_all(TOO_SLOW_OR_UNREADABLE);
            return;
        }

        let cap = rivet_core::operation::status::REQUEST_MAX_BYTES + 1;
        let mut buf = [0u8; 4096];
        loop {
            if connection.asked.len() >= cap {
                break;
            }
            match connection.stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let take = (cap - connection.asked.len()).min(n);
                    connection.asked.extend_from_slice(&buf[..take]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    self.query_incoming = Some(connection);
                    return;
                }
                Err(_) => return,
            }
        }

        let within_cap = connection.asked.len() <= rivet_core::operation::status::REQUEST_MAX_BYTES;
        let text = if within_cap {
            String::from_utf8(connection.asked).ok()
        } else {
            None
        };
        let Some(text) = text else {
            let _ = connection.stream.write_all(TOO_SLOW_OR_UNREADABLE);
            return;
        };
        // **The domain parses it, not this adapter.** `ADR-014` §3: the request
        // grammar is `rivet_core::operation::service_restart::Request`, with the
        // client's renderer beside it, so the two cannot disagree about what a
        // command looks like. This moves bytes and decides nothing.
        let request = match rivet_core::operation::service_restart::Request::parse(&text) {
            Ok(request) => request,
            Err(refusal) => {
                let _ = connection.stream.write_all(format!("{refusal}\n").as_bytes());
                return;
            }
        };
        self.answering = Some(connection.stream);
        out.push(Event::Queried { request });
    }

    /// **Read the facts requests that are waiting, and refuse the ones that
    /// are not requests** — `ADR-057`.
    ///
    /// There is no `accept` here, no connection to keep and no deadline to
    /// arm, because there is nothing to wait for: a datagram is either in the
    /// receive queue or it is not, the socket is non-blocking, and this
    /// returns at the first `WouldBlock`. **That is the property that lets this
    /// socket be open to every application on the machine** — a caller cannot
    /// connect and say nothing, because there is no connection, so there is no
    /// state in which one application is holding the supervisor.
    ///
    /// Bounded at `FACTS_PER_WAKEUP` so that a flood costs a fixed amount of
    /// work per pass rather than an unbounded one; the socket stays readable
    /// and the next pass takes the next few, which is `drain_kmsg`'s rule for
    /// the same hazard.
    ///
    /// **A request this build does not answer is refused to the caller and
    /// filed nowhere.** `ADR-032` §3's item 1 argument — a socket that
    /// silently ignored an unknown request would be a vocabulary nobody could
    /// discover — with one amendment this socket needs and the query socket
    /// does not: the refusal goes back to the address `recv_from` reported,
    /// which is the sender's own, and **no journal record is written**, because
    /// a record per bad datagram is an unprivileged process choosing how fast
    /// this machine's journal fills.
    fn drain_facts(&mut self, out: &mut Vec<Event>) {
        /// The most requests read in one pass. See the note above.
        const FACTS_PER_WAKEUP: usize = 8;

        let Some(socket) = self.facts.as_ref() else {
            return;
        };
        let mut buf = [0_u8; rivet_core::facts::REQUEST_MAX_BYTES + 1];
        for _ in 0..FACTS_PER_WAKEUP {
            let Ok((n, from)) = socket.recv_from(&mut buf) else {
                return;
            };
            // **An unnamed sender gets nothing, and cannot.** A caller that did
            // not bind a socket of its own has given no address to answer to,
            // so there is no refusal to send either. `rivet_sdk::facts` binds
            // one under `RIVET_APP_TMP`, which is a directory the machine made
            // and told the application about.
            let Some(reply) = from.as_pathname().map(std::path::Path::to_path_buf) else {
                continue;
            };
            let asked = String::from_utf8_lossy(&buf[..n]);
            // **The domain reads it, not this adapter.** `ADR-014` §3: the
            // request is `rivet_core::facts::read_request` and the refusal is
            // that module's own sentence, so the two halves cannot disagree
            // about what the question is. This moves bytes and decides nothing.
            match rivet_core::facts::read_request(&asked) {
                Ok(()) => {
                    self.facts_replies.push_back(reply);
                    out.push(Event::FactsAsked);
                }
                Err(refusal) => {
                    let _ = socket.send_to(format!("{refusal}\n").as_bytes(), &reply);
                }
            }
        }
    }

    /// Whether a further ingest connection can be accepted — and therefore
    /// whether the listener belongs in the `poll(2)` set at all.
    ///
    /// **The second half is the load-bearing one.** A listening socket with a
    /// connection waiting is permanently readable, so a supervisor that polled
    /// it while refusing to accept would wake immediately, refuse again, and
    /// spin PID 1's only child at 100% for as long as the queue stayed full.
    /// Leaving it out is what turns a full queue into an ordinary sleep; the
    /// accepted connections stay in the set, and one of them finishing or
    /// expiring re-opens the door.
    fn accepting_ingest(&self) -> bool {
        self.ingest.is_some()
            && self.incoming.len() < rivet_core::journal::INGEST_MAX_IN_FLIGHT
    }

    /// How long until the oldest open ingest connection must be retired, or
    /// `None` when there is none.
    fn next_ingest_deadline_ms(&mut self) -> Option<u64> {
        let oldest = self.incoming.iter().map(|c| c.accepted_ms).min()?;
        let now = self.now_ms();
        Some(
            rivet_core::journal::INGEST_DEADLINE_MS
                .saturating_sub(now.saturating_sub(oldest)),
        )
    }

    /// How long until the open kiosk connection must be retired, or `None`
    /// when there is none — [`Self::next_ingest_deadline_ms`]'s reason
    /// exactly: the deadline is not a descriptor, `poll(2)` cannot wake for
    /// it on its own, and a supervisor with nothing else pending would
    /// otherwise sleep past it, holding a silent caller for however long the
    /// next unrelated event takes to arrive.
    fn next_kiosk_deadline_ms(&mut self) -> Option<u64> {
        let accepted_ms = self.kiosk_incoming.as_ref()?.accepted_ms;
        let now = self.now_ms();
        Some(kiosk_patience_ms().saturating_sub(now.saturating_sub(accepted_ms)))
    }

    /// [`Self::next_kiosk_deadline_ms`], for the query socket's one
    /// connection.
    fn next_query_deadline_ms(&mut self) -> Option<u64> {
        let accepted_ms = self.query_incoming.as_ref()?.accepted_ms;
        let now = self.now_ms();
        Some(kiosk_patience_ms().saturating_sub(now.saturating_sub(accepted_ms)))
    }

    /// Read whatever `/dev/kmsg` has ready, one record per `read(2)`.
    ///
    /// **A bounded number per call.** The kernel's ring buffer holds a whole
    /// boot's messages and opening the descriptor starts at the beginning of
    /// it, so the first drain has hundreds of records waiting; taking them all
    /// in one go would keep the supervisor out of its loop for as long as that
    /// took, with services unreaped and a shutdown unanswered. A cap turns that
    /// into "several wakeups", and the loop wakes immediately because
    /// [`Kernel::wait`] returns what it has rather than polling on top of it.
    fn drain_kmsg(&mut self, out: &mut Vec<Event>) {
        const PER_WAKEUP: usize = 64;
        // The ABI documents 8 KiB as the largest a record can be. It divides
        // `DRAIN_BUDGET_BYTES`, so the budget never cuts a read short of a
        // whole record — a short read here is `EINVAL`, not half a record.
        let mut buf = [0u8; 8192];
        let Some(file) = self.kmsg.as_mut() else {
            return;
        };
        // Whichever comes first: `PER_WAKEUP` records, or
        // `DRAIN_BUDGET_BYTES` of them.
        //
        // A failed read ends the pass. `EPIPE` means the kernel overwrote
        // records before this process read them — the buffer wrapped. It is
        // not an error to stop for: the next read resumes at the oldest
        // surviving record, which is exactly what a reader wants. Nothing is
        // reported because the *sequence numbers* already say it, and a record
        // about lost records that is itself written into the thing losing them
        // would be the wrong place to say it.
        let mut records = 0_usize;
        drain_within_budget(file, &mut buf, |record| {
            out.push(Event::KernelMessage {
                record: String::from_utf8_lossy(record).into_owned(),
            });
            records += 1;
            records < PER_WAKEUP
        });
    }

    /// Make a pipe non-blocking and keep it.
    ///
    /// **Non-blocking is not an optimisation here.** These descriptors go in the
    /// same `poll(2)` set as the signal descriptor, and a blocking read on a
    /// pipe whose writer is alive and quiet would stop the supervisor — no
    /// reaping, no convergence, no shutdown — until the service said something.
    fn capture<F>(&mut self, pid: i32, service: &str, stream: Stream, file: Option<F>)
    where
        F: std::os::fd::IntoRawFd,
    {
        use std::os::fd::FromRawFd as _;

        let Some(file) = file else { return };
        let fd = file.into_raw_fd();
        // SAFETY: `fd` was just taken from a `ChildStdout`/`ChildStderr` by
        // `into_raw_fd`, which gives up ownership, so this `File` is its sole
        // owner and cannot double-close. `fcntl` reads and sets flags on it and
        // touches no memory of this process.
        let file = unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            if flags >= 0 {
                let _ = libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            }
            std::fs::File::from_raw_fd(fd)
        };
        self.captures.push(Capture {
            pid,
            service: service.to_string(),
            stream,
            file,
            partial: Vec::new(),
            overflowed: false,
        });
    }

    /// Read whatever is buffered on every open pipe, turning complete lines
    /// into events.
    ///
    /// Called at the top of every [`Kernel::wait`] as well as when `poll`
    /// reports a pipe readable, because a pipe that filled while the supervisor
    /// was busy is one whose *writer is blocked* — and a blocked `rivet-mcpd`
    /// is a machine nobody can reach. Draining unconditionally costs one
    /// `read(2)` per pipe per wakeup and removes that failure entirely.
    fn drain_captures(&mut self, out: &mut Vec<Event>) {
        for capture in &mut self.captures {
            capture.drain(false, out);
        }
    }

    /// Drain a dead child's pipes to EOF and drop them.
    ///
    /// **The last line is the one that matters**, and it is written after the
    /// last time anything polled. A service that fails prints the reason and
    /// exits; closing its pipes on reap without this would lose precisely that.
    fn close_captures(&mut self, pid: i32) {
        let mut out = Vec::new();
        for capture in &mut self.captures {
            if capture.pid == pid {
                capture.drain(true, &mut out);
            }
        }
        self.captures.retain(|c| c.pid != pid);
        self.pending.extend(out);
    }
}

/// Why one budgeted drain of one source stopped — [`drain_within_budget`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drained {
    /// The source would block: everything it held has been read.
    Dry,
    /// A read returned zero bytes — end of file on a stream, or an empty
    /// datagram.
    Ended,
    /// [`DRAIN_BUDGET_BYTES`] were read this pass. The source may hold more,
    /// and the next pass takes it.
    Budget,
    /// The caller's `take` said to stop.
    Stopped,
    /// A read failed with something other than `WouldBlock` or `EINTR`.
    Failed,
}

/// **Read one source until it is dry, ends, or has given up
/// [`DRAIN_BUDGET_BYTES`] this pass** — the one loop every drain in this file
/// is built on, so the budget is applied in exactly one place.
///
/// `take` is handed each read's bytes and returns `false` to stop early, which
/// is how a caller with a rule of its own (a line too long, a record count)
/// ends the pass.
///
/// **Each read is cut to the budget that remains**, so a pass never reads more
/// than the budget. Every caller's buffer divides the budget exactly, so for a
/// source whose reads are whole records — a datagram, a `/dev/kmsg` entry —
/// the cut never shortens a read below the buffer and never truncates one.
fn drain_within_budget<R: std::io::Read>(
    source: &mut R,
    buf: &mut [u8],
    mut take: impl FnMut(&[u8]) -> bool,
) -> Drained {
    let mut taken = 0_usize;
    loop {
        let room = DRAIN_BUDGET_BYTES.saturating_sub(taken);
        if room == 0 {
            return Drained::Budget;
        }
        let want = buf.len().min(room);
        match source.read(&mut buf[..want]) {
            Ok(0) => return Drained::Ended,
            Ok(n) => {
                taken += n;
                if !take(&buf[..n]) {
                    return Drained::Stopped;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Drained::Dry,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Drained::Failed,
        }
    }
}

/// A notify socket as a [`std::io::Read`], so [`drain_within_budget`] can
/// drain it: one `read` is one `recv`, which is one datagram.
struct Datagrams<'a>(&'a std::os::unix::net::UnixDatagram);

impl std::io::Read for Datagrams<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.recv(buf)
    }
}

impl Capture {
    /// Read until the pipe would block, ends, or has given up
    /// [`DRAIN_BUDGET_BYTES`] this pass.
    ///
    /// `to_eof` also flushes a trailing partial line, because a process that
    /// exited without a final newline has still said its piece and dropping it
    /// would be the journal disagreeing with the console.
    fn drain(&mut self, to_eof: bool, out: &mut Vec<Event>) {
        let mut buf = [0u8; 4096];
        let mut file = &self.file;
        let mut said = Vec::new();
        drain_within_budget(&mut file, &mut buf, |bytes| {
            said.extend_from_slice(bytes);
            true
        });
        self.absorb(&said, out);
        if to_eof && !self.partial.is_empty() {
            self.emit(out);
        }
    }

    fn absorb(&mut self, bytes: &[u8], out: &mut Vec<Event>) {
        for byte in bytes {
            if *byte == b'\n' {
                self.emit(out);
                continue;
            }
            // A carriage return before a newline is the mechanism's, not a
            // line of its own. Dropped rather than escaped into `\r` in the
            // record, which is what a console shows and what a reader means.
            if *byte == b'\r' {
                continue;
            }
            if self.partial.len() >= MAX_CAPTURED_LINE {
                self.overflowed = true;
                continue;
            }
            self.partial.push(*byte);
        }
    }

    fn emit(&mut self, out: &mut Vec<Event>) {
        // Lossy rather than refusing: a mechanism's output is bytes, not
        // guaranteed UTF-8, and a record that could not be written because a
        // C daemon emitted a stray byte would be the journal failing at
        // exactly its job. The replacement character says so in the record.
        let text = String::from_utf8_lossy(&self.partial).into_owned();
        self.partial.clear();
        let truncated = self.overflowed;
        self.overflowed = false;
        // A bare newline is a blank line, and a blank line is not a record.
        if text.is_empty() && !truncated {
            return;
        }
        out.push(Event::Output {
            service: self.service.clone(),
            stream: self.stream,
            text,
            truncated,
        });
    }
}

impl LibcKernel {
    /// Drain one notify socket, turning `sd_notify` text into events.
    ///
    /// The convention is systemd's (`ADR-017` §3.4): newline-separated
    /// `KEY=VALUE` in a datagram. Unknown keys are ignored rather than refused,
    /// because the convention is explicitly extensible and a service sending
    /// `WATCHDOG=1` to a supervisor that has no watchdog is not an error.
    ///
    /// **At most [`DRAIN_BUDGET_BYTES`] per socket per pass**, so a service
    /// that sends without stopping cannot keep the loop from everything else.
    /// The buffer divides the budget, so no datagram is cut by it.
    fn drain_notify(service: &str, sock: &std::os::unix::net::UnixDatagram, out: &mut Vec<Event>) {
        let mut buf = [0u8; 4096];
        drain_within_budget(&mut Datagrams(sock), &mut buf, |datagram| {
            Self::notified(service, datagram, out);
            true
        });
    }

    /// One `sd_notify` datagram's lines, as events.
    fn notified(service: &str, datagram: &[u8], out: &mut Vec<Event>) {
        let text = String::from_utf8_lossy(datagram).into_owned();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "READY" if value.trim() == "1" => out.push(Event::Ready {
                    service: service.to_string(),
                }),
                "STATUS" => out.push(Event::Status {
                    service: service.to_string(),
                    text: value.to_string(),
                }),
                // **`ADR-045` §3: *ask me now*, and nothing more.**
                // The value is deliberately DISCARDED — a push is a request
                // for a poll, not a health report, and reading the word out
                // of it would hand an application back the power over its
                // own cadence that §3.1.1 took away.
                //
                // The key stays on this socket because `sd_notify`'s
                // protocol is untouched (`ADR-017` §3.4): a foreign daemon
                // that speaks it keeps working, and this is one more
                // extensible key rather than a second channel.
                _ if key == rivet_core::health::KEY => out.push(Event::HealthHint {
                    service: service.to_string(),
                }),
                _ => {}
            }
        }
    }
}

impl Kernel for LibcKernel {
    fn now_ms(&mut self) -> u64 {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `clock_gettime` writes one `timespec` through a pointer to a
        // live local and reads nothing else. `CLOCK_MONOTONIC` is a valid
        // clock id on every Linux this targets.
        let rc = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut ts) };
        if rc != 0 {
            return 0;
        }
        let secs = u64::try_from(ts.tv_sec).unwrap_or(0);
        let nanos = u64::try_from(ts.tv_nsec).unwrap_or(0);
        secs.saturating_mul(1000).saturating_add(nanos / 1_000_000)
    }

    fn wall_ms(&mut self) -> Option<u64> {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|d| u64::try_from(d.as_millis()).ok())
    }

    fn set_wall_ms(&mut self, unix_ms: u64) -> Result<(), Errno> {
        let Some((tv_sec, tv_nsec)) = timespec_parts(unix_ms) else {
            // A value this platform's `timespec` cannot hold. `EINVAL` is what
            // `clock_settime(2)` itself returns for an out-of-range time, so
            // reporting the same number keeps one vocabulary.
            return Err(Errno::EINVAL);
        };
        let ts = libc::timespec { tv_sec, tv_nsec };
        // SAFETY: `clock_settime` reads one `timespec` through a pointer to a
        // live local and retains nothing. `CLOCK_REALTIME` is a valid, settable
        // clock id on every Linux this targets, and `tv_nsec` is in range by
        // construction — `timespec_parts` returns `None` otherwise.
        let rc = unsafe { libc::clock_settime(libc::CLOCK_REALTIME, &raw const ts) };
        if rc == 0 { Ok(()) } else { Err(Errno::last()) }
    }

    fn local_time(&mut self) -> Option<LocalReading> {
        let now = i64::try_from(self.wall_ms()? / 1000).ok()?;
        let t: libc::time_t = now.try_into().ok()?;
        // SAFETY: `localtime_r` writes one `struct tm` through a pointer to a
        // live local and reads one `time_t` through a pointer to another. It is
        // the reentrant form deliberately: `localtime` returns a pointer to
        // static storage, which is a hazard in a process that has threads.
        let mut tm: libc::tm = unsafe { core::mem::zeroed() };
        let filled = unsafe { libc::localtime_r(&raw const t, &raw mut tm) };
        if filled.is_null() {
            return None;
        }
        // SAFETY: `tm_zone` points into the library's own zone data, which
        // outlives this call, and musl always sets it to a NUL-terminated
        // abbreviation. A null is not documented and is handled anyway, because
        // dereferencing one here would take down PID 1's only child.
        let abbreviation = if tm.tm_zone.is_null() {
            String::new()
        } else {
            unsafe { core::ffi::CStr::from_ptr(tm.tm_zone) }
                .to_string_lossy()
                .into_owned()
        };
        Some(LocalReading {
            offset_s: i32::try_from(tm.tm_gmtoff).unwrap_or(0),
            abbreviation,
            local: format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
                tm.tm_year.saturating_add(1900),
                tm.tm_mon.saturating_add(1),
                tm.tm_mday,
                tm.tm_hour,
                tm.tm_min,
                tm.tm_sec
            ),
        })
    }

    fn set_hostname(&mut self, name: &str) -> Result<(), Errno> {
        // SAFETY: `sethostname` reads `len` bytes through a pointer to a live
        // slice and retains nothing. `Hostname` is at most 63 bytes of ASCII
        // (`rivet_core::hostname::MAX_BYTES`), so the length fits every
        // platform's `HOST_NAME_MAX` and the cast cannot truncate.
        let rc = unsafe {
            libc::sethostname(name.as_ptr().cast::<libc::c_char>(), name.len())
        };
        if rc == 0 { Ok(()) } else { Err(Errno::last()) }
    }

    fn emit(&mut self, record: &Record) {
        let line = match self.form {
            ConsoleForm::Jsonl => record.to_jsonl(),
            ConsoleForm::Text => format!("{}\n", record.to_text()),
        };
        // A closed console must not become the reason the supervisor dies, so
        // both results are dropped deliberately.
        let mut out = std::io::stdout();
        let _ = out.write_all(line.as_bytes());
        let _ = out.flush();
    }

    fn read_file(&mut self, path: &str) -> Result<String, Errno> {
        std::fs::read_to_string(path).map_err(|e| Errno::from_io(&e))
    }

    fn read_file_bytes(&mut self, path: &str, max: usize) -> Result<Vec<u8>, Errno> {
        // The length is asked for BEFORE the read, so an oversized file is
        // refused rather than read and then rejected — the whole point of the
        // bound is not to allocate it.
        let meta = std::fs::metadata(path).map_err(|e| Errno::from_io(&e))?;
        if meta.len() > max as u64 {
            return Err(Errno::EFBIG);
        }
        let bytes = std::fs::read(path).map_err(|e| Errno::from_io(&e))?;
        // Checked again after the read, because the file could have grown
        // between the two calls. This is a time-of-check race that matters:
        // the caller is about to hash these bytes and say a signature covers
        // them.
        if bytes.len() > max {
            return Err(Errno::EFBIG);
        }
        Ok(bytes)
    }

    fn sync_file(&mut self, path: &str) -> Result<(), Errno> {
        // Reopened rather than kept open across the write. `fsync(2)` flushes
        // the *inode*, not the descriptor, so a read-only descriptor is enough
        // — and holding a handle across two port calls would put state in the
        // adapter, which is the thing this seam exists to avoid.
        // `rivet_store::StdFilesystem` does the identical thing; this is the
        // same call reached through this crate's one port.
        let file = std::fs::File::open(path).map_err(|e| Errno::from_io(&e))?;
        file.sync_all().map_err(|e| Errno::from_io(&e))
    }

    fn rename(&mut self, from: &str, to: &str) -> Result<(), Errno> {
        std::fs::rename(from, to).map_err(|e| Errno::from_io(&e))
    }

    fn sync_dir(&mut self, path: &str) -> Result<(), Errno> {
        // Opening a directory read-only and calling `fsync` on it is the POSIX
        // way to make a rename durable, and it is what every database does.
        let dir = std::fs::File::open(path).map_err(|e| Errno::from_io(&e))?;
        dir.sync_all().map_err(|e| Errno::from_io(&e))
    }

    fn remove_file(&mut self, path: &str) -> Result<(), Errno> {
        std::fs::remove_file(path).map_err(|e| Errno::from_io(&e))
    }

    fn mkdir_p(&mut self, path: &str) -> Result<(), Errno> {
        // Already idempotent for a directory that exists. A path that exists as
        // a *file* is an error, and which error it is is the platform's answer,
        // not ours to assume (`AGENTS.md` §5).
        std::fs::create_dir_all(path).map_err(|e| Errno::from_io(&e))
    }

    fn mkdir_mode(&mut self, path: &str, mode: u32) -> Result<bool, Errno> {
        use std::os::unix::fs::PermissionsExt as _;

        match std::fs::create_dir(path) {
            Ok(()) => {
                // `mkdir(2)` masks the mode with the process umask, so the mode
                // is set explicitly rather than assumed to have been applied.
                // A `0700` directory that came out `0700 & ~umask` would depend
                // on what PID 1 happened to inherit, and a program that later
                // refuses a group- or other-writable component says so with a
                // message about permissions that names no mode.
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
                    .map_err(|e| Errno::from_io(&e))?;
                Ok(true)
            }
            Err(e) => {
                // Already there is not an error — but *what* is already there
                // matters: a regular file at this path would make every later
                // step fail somewhere else, naming neither this path nor this
                // precondition.
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    return match std::fs::metadata(path) {
                        Ok(m) if m.is_dir() => Ok(false),
                        Ok(_) => Err(Errno::ENOTDIR),
                        Err(e) => Err(Errno::from_io(&e)),
                    };
                }
                Err(Errno::from_io(&e))
            }
        }
    }

    fn chown(&mut self, path: &str, uid: u32, gid: u32) -> Result<(), Errno> {
        // **`lchown`, not `chown`.** The path is under `/state`, which is the
        // one writable filesystem; a symlink planted there by anything able to
        // write it would otherwise let this call hand ownership of a file
        // somewhere else entirely to an application's uid. There is nothing to
        // follow on a correct machine — `rivet_core::service::data_dir` is a
        // directory the supervisor itself creates — and that is the case where
        // the difference is free.
        let c = std::ffi::CString::new(path).map_err(|_| Errno::EINVAL)?;
        // SAFETY: `c` is a NUL-terminated C string that outlives the call, and
        // `lchown` reads it and nothing else.
        let rc = unsafe { libc::lchown(c.as_ptr(), uid, gid) };
        if rc == 0 { Ok(()) } else { Err(Errno::last()) }
    }

    fn chmod(&mut self, path: &str, mode: u32) -> Result<(), Errno> {
        use std::os::unix::fs::PermissionsExt as _;

        // `set_permissions` and not a `chmod` through libc, because the mode
        // wanted here has a setgid bit and `std`'s `from_mode` carries all
        // twelve bits through unchanged. The umask does not apply to
        // `chmod(2)` at all, which is the whole reason this is a separate call
        // from `mkdir_mode`.
        //
        // It follows symlinks, and there is no `lchmod` on Linux to reach for:
        // the mode of a symlink is not a thing Linux stores. Every caller
        // passes a path this program created.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(|e| Errno::from_io(&e))
    }

    fn share_tree(&mut self, path: &str, gid: u32) -> Result<(), Errno> {
        use std::os::unix::fs::PermissionsExt as _;

        // `u32::MAX` is `chown(2)`'s "leave it alone" for the uid — the owner
        // is root and stays root; only the group is being changed.
        const KEEP: u32 = u32::MAX;

        let meta = std::fs::symlink_metadata(path).map_err(|e| Errno::from_io(&e))?;
        if meta.file_type().is_symlink() {
            // Not followed, and not an error either: a symlink inside a
            // repository is git's (`objects/info/alternates` aside, git makes
            // none), and refusing the whole walk over one would leave the tree
            // half-converged. Its own target is reached by the walk if it is
            // inside the tree and is nothing to do with this machine if it is
            // not.
            return Ok(());
        }

        self.chown(path, KEEP, gid)?;
        if meta.is_dir() {
            // After the chown, because `chown(2)` clears setgid.
            let mode = meta.permissions().mode() & 0o7777;
            self.chmod(path, mode | 0o2070)?;
            let entries = std::fs::read_dir(path).map_err(|e| Errno::from_io(&e))?;
            for entry in entries {
                let entry = entry.map_err(|e| Errno::from_io(&e))?;
                let child = entry.path();
                let Some(child) = child.to_str() else {
                    // A name that is not UTF-8 is not one this program wrote,
                    // and `Kernel`'s paths are `&str` throughout.
                    return Err(Errno::EINVAL);
                };
                self.share_tree(child, gid)?;
            }
        }
        Ok(())
    }

    fn run_to_completion(&mut self, program: &Program) -> Result<Completed, Errno> {
        let mut cmd = one_shot(program);
        // `output()` spawns, reads both streams to EOF and waits. SIGCHLD is
        // blocked and delivered through `signalfd`, so this `waitpid` is not
        // competing with a handler — and the child is reaped here, so
        // `reap_ready` never sees a pid it cannot account for.
        //
        // It reads before it waits, so a generator that filled a pipe cannot
        // deadlock against a parent that is waiting for it to exit — which is
        // the classic version of this mistake and the reason this is `output()`
        // rather than `status()` plus two reads.
        let done = cmd.output().map_err(|e| Errno::from_io(&e))?;
        Ok(Completed {
            status: decode_status(done.status),
            output: captured_lines(&done.stdout, &done.stderr),
        })
    }

    fn run_within(&mut self, program: &Program, limit_ms: u64) -> Result<Completed, Errno> {
        use std::os::fd::AsRawFd as _;

        let mut cmd = one_shot(program);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| Errno::from_io(&e))?;
        let deadline = self.now_ms().saturating_add(limit_ms);

        // Both streams, read without blocking as they fill, so a program that
        // writes more than a pipe holds is not stalled against a parent that
        // is waiting for it to exit — `output()`'s property, kept. `None` once
        // a stream has ended.
        let mut stdout = child
            .stdout
            .take()
            .map(|p| std::fs::File::from(std::os::fd::OwnedFd::from(p)));
        let mut stderr = child
            .stderr
            .take()
            .map(|p| std::fs::File::from(std::os::fd::OwnedFd::from(p)));
        for pipe in [stdout.as_ref(), stderr.as_ref()].into_iter().flatten() {
            set_nonblocking(pipe.as_raw_fd());
        }
        let (mut said_out, mut said_err) = (Vec::new(), Vec::new());
        let mut exited = None;
        loop {
            if stdout.as_ref().is_some_and(|p| !read_available(p, &mut said_out)) {
                stdout = None;
            }
            if stderr.as_ref().is_some_and(|p| !read_available(p, &mut said_err)) {
                stderr = None;
            }
            if exited.is_none() {
                exited = child.try_wait().ok().flatten();
            }
            let now = self.now_ms();
            if let Some(status) = exited
                && ((stdout.is_none() && stderr.is_none()) || now >= deadline)
            {
                // Finished — and, past the deadline, something it left behind
                // holds its streams; what arrived is what it said.
                return Ok(Completed {
                    status: decode_status(status),
                    output: captured_lines(&said_out, &said_err),
                });
            }
            if now >= deadline {
                // **Out of time.** Killed, and reaped now if the kernel has
                // already finished it — otherwise handed to the helpers this
                // kernel reaps, so a process stuck where `SIGKILL` cannot
                // reach it costs a pid and not this loop.
                let _ = child.kill();
                if !matches!(child.try_wait(), Ok(Some(_)))
                    && let Ok(pid) = i32::try_from(child.id())
                {
                    self.helpers.push((pid, None));
                }
                return Err(Errno::ETIMEDOUT);
            }
            // Sleep until a stream has something or a short step passes, the
            // step being what notices an exit with both streams already shut.
            let mut fds: Vec<libc::pollfd> = [stdout.as_ref(), stderr.as_ref()]
                .into_iter()
                .flatten()
                .map(|p| libc::pollfd {
                    fd: p.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                })
                .collect();
            let step = i32::try_from((deadline - now).min(ONE_SHOT_STEP_MS)).unwrap_or(1);
            // SAFETY: `poll` reads and writes exactly `fds.len()` `pollfd`
            // structures through a pointer to a live, correctly sized `Vec`,
            // and every descriptor in it is a pipe this call owns. With no
            // descriptors it is a plain sleep of `step` milliseconds.
            unsafe {
                libc::poll(
                    fds.as_mut_ptr(),
                    libc::nfds_t::try_from(fds.len()).unwrap_or(0),
                    step,
                );
            }
        }
    }

    fn mount(
        &mut self,
        source: &str,
        target: &str,
        fstype: &str,
        flags: MountFlags,
        data: Option<&str>,
    ) -> Result<(), Errno> {
        let (Ok(source), Ok(target), Ok(fstype)) = (
            std::ffi::CString::new(source),
            std::ffi::CString::new(target),
            std::ffi::CString::new(fstype),
        ) else {
            return Err(Errno::EINVAL);
        };
        let data = match data.map(std::ffi::CString::new) {
            None => None,
            Some(Ok(c)) => Some(c),
            Some(Err(_)) => return Err(Errno::EINVAL),
        };
        let data_ptr = data
            .as_ref()
            .map_or(std::ptr::null(), |c| c.as_ptr())
            .cast::<libc::c_void>();

        // SAFETY: `mount(2)` reads four borrowed pointers and returns an int; it
        // retains none of them. `source`, `target` and `fstype` are
        // NUL-terminated and owned by `CString`s alive for the whole call.
        // `data_ptr` is either NULL — which `mount(2)` documents as "no
        // filesystem-specific options" — or borrowed from `data`, likewise
        // alive here. `flags` is a bitmask of `MS_*` constants and cannot be an
        // invalid value.
        let rc = unsafe {
            libc::mount(
                source.as_ptr(),
                target.as_ptr(),
                fstype.as_ptr(),
                flags.0 as libc::c_ulong,
                data_ptr,
            )
        };
        if rc == 0 { Ok(()) } else { Err(Errno::last()) }
    }

    fn file_exists(&mut self, path: &str) -> Result<bool, Errno> {
        match std::fs::metadata(path) {
            Ok(_) => Ok(true),
            // The two errnos that mean "the platform looked and there is
            // nothing there". Everything else — a permission problem, an I/O
            // error — is the question failing rather than being answered, and
            // the caller is told so rather than handed a `false`.
            Err(e) => {
                let errno = Errno::from_io(&e);
                if errno == Errno::ENOENT || errno == Errno::ENOTDIR {
                    Ok(false)
                } else {
                    Err(errno)
                }
            }
        }
    }

    fn is_symlink(&mut self, path: &str) -> Result<bool, Errno> {
        // `symlink_metadata` is `lstat(2)`: unlike `metadata`'s `stat(2)`, it
        // reports on the final component itself rather than on whatever it
        // points at.
        match std::fs::symlink_metadata(path) {
            Ok(meta) => Ok(meta.file_type().is_symlink()),
            Err(e) => {
                let errno = Errno::from_io(&e);
                if errno == Errno::ENOENT || errno == Errno::ENOTDIR {
                    Ok(false)
                } else {
                    Err(errno)
                }
            }
        }
    }

    fn write_file(&mut self, path: &str, contents: &[u8], mode: u32) -> Result<(), Errno> {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(mode)
            .open(path)
            .map_err(|e| Errno::from_io(&e))?;
        file.write_all(contents).map_err(|e| Errno::from_io(&e))?;
        // `mode()` above applies only when the file is *created*. On a re-render
        // — which `ADR-017` §3.1 expects, because the supervisor is restarted —
        // the file already exists and keeps whatever mode it had, so the mode is
        // set explicitly rather than assumed to have been applied.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(|e| Errno::from_io(&e))
    }

    fn write_file_nofollow(
        &mut self,
        path: &str,
        contents: &[u8],
        mode: u32,
    ) -> Result<(), Errno> {
        use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};

        // `truncate(false)`: the file is emptied AFTER the checks below, not by
        // the open. `O_TRUNC` here would have destroyed the target's contents
        // before anything had established that the target is the right file —
        // which for a symlink to the configuration document is most of the
        // damage, whether or not the write then goes ahead.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .mode(mode)
            .open(path)
            .map_err(|e| Errno::from_io(&e))?;

        let meta = file.metadata().map_err(|e| Errno::from_io(&e))?;
        if !meta.file_type().is_file() {
            return Err(Errno::EINVAL);
        }
        if meta.nlink() != 1 {
            return Err(Errno::EMLINK);
        }

        file.set_len(0).map_err(|e| Errno::from_io(&e))?;
        file.write_all(contents).map_err(|e| Errno::from_io(&e))?;
        // `File::set_permissions` is `fchmod` on this descriptor. The path is
        // not consulted again, so nothing can have been swapped underneath it
        // since the open.
        file.set_permissions(std::fs::Permissions::from_mode(mode))
            .map_err(|e| Errno::from_io(&e))
    }

    fn append_file(&mut self, path: &str, contents: &[u8], mode: u32) -> Result<(), Errno> {
        use std::os::unix::fs::OpenOptionsExt as _;

        // Opened and closed per record rather than kept open across calls.
        //
        // The reason is the same one `sync_file` gives for reopening: holding a
        // descriptor across two port calls would put state in the adapter, and
        // this port is deliberately stateless about files. The cost is one
        // `open`/`close` pair per record, which is a few microseconds against a
        // journal that writes on the order of tens of records per boot — and
        // the benefit is that a supervisor restart, which `ADR-017` §3.1
        // expects, cannot leave a stale descriptor pointing at a rotated-away
        // inode and go on writing to a file nothing will ever read.
        //
        // **`O_APPEND` and no `truncate`.** With `O_APPEND` the offset is
        // resolved by the kernel at write time, so a rotation between two
        // records cannot make the second one land at the old offset in a new
        // file, leaving a megabyte of NUL bytes in front of it.
        //
        // No `fsync` — `ADR-022` §8, and `crate::journal`'s header argues it.
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .mode(mode)
            .open(path)
            .map_err(|e| Errno::from_io(&e))?;
        file.write_all(contents).map_err(|e| Errno::from_io(&e))
    }

    fn file_size(&mut self, path: &str) -> Result<u64, Errno> {
        std::fs::metadata(path)
            .map(|m| m.len())
            .map_err(|e| Errno::from_io(&e))
    }

    fn modified_secs(&mut self, path: &str) -> Option<u64> {
        std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
    }

    fn arm_signals(&mut self) -> Result<(), Errno> {
        if self.sigfd.is_some() {
            return Ok(());
        }
        // SAFETY: `set` is a `sigset_t` owned by this frame and lives across
        // every call that borrows it. `sigemptyset`/`sigaddset` only write
        // through that pointer. `pthread_sigmask` reads it and writes nothing
        // (the old-set pointer is NULL, documented as "do not report the old
        // mask"). `signalfd(-1, …)` reads the set and creates a new descriptor
        // rather than modifying an existing one, so the `File` is its sole
        // owner and cannot double-close.
        let fd = unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            if libc::sigemptyset(&raw mut set) != 0 {
                return Err(Errno::last());
            }
            for sig in BLOCKED_SIGNALS {
                if libc::sigaddset(&raw mut set, sig) != 0 {
                    return Err(Errno::last());
                }
            }
            if libc::pthread_sigmask(libc::SIG_BLOCK, &raw const set, std::ptr::null_mut()) != 0 {
                return Err(Errno::last());
            }
            let fd = libc::signalfd(-1, &raw const set, libc::SFD_CLOEXEC);
            if fd < 0 {
                return Err(Errno::last());
            }
            <std::fs::File as std::os::fd::FromRawFd>::from_raw_fd(fd)
        };
        self.sigfd = Some(fd);
        Ok(())
    }

    fn open_kmsg(&mut self) -> Result<(), Errno> {
        use std::os::unix::fs::OpenOptionsExt as _;

        if self.kmsg.is_some() {
            return Ok(());
        }
        // `O_NONBLOCK` for the same reason the capture pipes have it: this
        // descriptor goes in the same `poll(2)` set, and a blocking read on a
        // quiet kernel would stop the supervisor until something happened.
        //
        // **No seek.** `/dev/kmsg` starts at the oldest surviving record, which
        // is the whole point — the supervisor starts long after the kernel has
        // finished saying anything interesting about bringing the hardware up.
        // `SEEK_END` would give a Rivet that could describe everything except
        // its own boot.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(crate::kmsg::KMSG_PATH)
            .map_err(|e| Errno::from_io(&e))?;
        self.kmsg = Some(file);
        Ok(())
    }

    fn wait(&mut self, timeout_ms: Option<u64>) -> Result<Vec<Event>, Errno> {
        use std::os::fd::AsRawFd as _;

        // A dead child's last words, drained by `reap_ready` and handed over
        // here. Returned **without polling**: they are already in hand, and
        // sleeping on a descriptor before reporting what has been read would
        // delay a failed service's explanation by a whole timeout. The queue is
        // finite and drains, so this cannot spin.
        if !self.pending.is_empty() {
            return Ok(self.pending.drain(..).collect());
        }

        // Every open pipe, before anything blocks. A pipe that filled while the
        // supervisor was busy has a **blocked writer** on the other end, and a
        // blocked `rivet-mcpd` is a machine nobody can reach — so this does
        // not wait to be told a pipe is readable.
        let mut early = Vec::new();
        self.drain_captures(&mut early);
        // And the kernel's, for a different reason: the ring buffer holds a
        // whole boot when the descriptor is first opened, so the first several
        // wakeups have work waiting rather than pending.
        self.drain_kmsg(&mut early);
        // And the ingest socket, for the pipes' reason exactly: a sender is
        // blocked in `connect(2)` until this process accepts, and `ADR-022` §11
        // has the login shell on the other end of it. It also retires
        // connections whose deadline has passed, which is why it runs on every
        // wakeup rather than only when `poll` says the listener is readable.
        self.drain_ingest(&mut early);
        // And the query and kiosk sockets, for `next_kiosk_deadline_ms`'s
        // reason: an accepted connection's deadline is not a descriptor, so
        // it has to be checked on every wakeup rather than only when `poll`
        // says the connection itself became readable — the case a silent
        // caller never produces.
        self.drain_query(&mut early);
        self.drain_kiosk(&mut early);

        // No signal descriptor means `arm_signals` was refused. The supervisor
        // still has to make progress, so it sleeps in short steps and reports a
        // timeout — the same "does less, and says so" fallback PID 1 uses.
        let Some(sigfd) = self.sigfd.as_ref() else {
            if !early.is_empty() {
                return Ok(early);
            }
            let step = timeout_ms.map_or(DEGRADED_POLL_MS, |t| t.min(DEGRADED_POLL_MS));
            std::thread::sleep(std::time::Duration::from_millis(step));
            return Ok(vec![Event::Timeout]);
        };

        // Something was already waiting on a pipe. Report it rather than
        // sleeping on top of it — **but poll first, with no timeout**, so this
        // pass also hears every signal and notify datagram that is ready. A
        // writer that never stops refills its pipe every pass, so returning
        // here unpolled would let it keep `SIGTERM` and every other service's
        // `READY=1` unread for as long as it wrote. The sources drained above
        // have had their `DRAIN_BUDGET_BYTES` for this pass and are not drained
        // again below.
        let busy = !early.is_empty();

        let names: Vec<String> = self.notify.keys().cloned().collect();
        let capture_fds: Vec<std::os::fd::RawFd> =
            self.captures.iter().map(|c| c.file.as_raw_fd()).collect();
        let mut fds = Vec::with_capacity(names.len() + capture_fds.len() + 1);
        fds.push(libc::pollfd {
            fd: sigfd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        });
        for name in &names {
            if let Some(sock) = self.notify.get(name) {
                fds.push(libc::pollfd {
                    fd: sock.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                });
            }
        }
        // `ADR-022` §4's capture, in the same set as the signal descriptor —
        // which is why this port's wait was written as "return the events that
        // are ready" rather than as "wait for a signal" (see [`Event`]).
        for fd in &capture_fds {
            fds.push(libc::pollfd {
                fd: *fd,
                events: libc::POLLIN,
                revents: 0,
            });
        }
        // The ingest socket and everything accepted on it — `ADR-022` §11.
        // The listener and the connections go in together and are drained
        // together, so nothing has to match a `revents` index against a `Vec`
        // that `drain_ingest` is about to rebuild.
        //
        // **The listener leaves the set when every slot is full**, and that is
        // not an optimisation: a listening socket with a connection waiting is
        // permanently readable, so polling it while refusing to accept would
        // return immediately for ever and spin PID 1's only child at 100% for as
        // long as the queue stayed full. The connections themselves stay in the
        // set, and one of them finishing or expiring is what re-opens the door.
        let ingest_from = fds.len();
        if self.accepting_ingest()
            && let Some(listener) = self.ingest.as_ref()
        {
            fds.push(libc::pollfd {
                fd: listener.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
        }
        for connection in &self.incoming {
            fds.push(libc::pollfd {
                fd: connection.stream.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
        }
        let ingest_count = fds.len() - ingest_from;

        // `ADR-032` §3.2's query socket. **Out of the set while an answer is
        // outstanding, or while a connection is already being read**, for the
        // reason the ingest listener leaves it when full: a listening socket
        // with a connection waiting is permanently readable.
        let query_polled = match self.query.as_ref() {
            Some(listener) if self.answering.is_none() && self.query_incoming.is_none() => {
                fds.push(libc::pollfd {
                    fd: listener.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                });
                Some(fds.len() - 1)
            }
            _ => None,
        };
        // The one connection accepted on it and not yet complete — parked in
        // the set on `QueryIncoming`'s own terms, the way `self.incoming` is
        // for the ingest socket, so a byte arriving on it wakes this `poll`
        // rather than waiting for the next unrelated event.
        let query_incoming_polled = self.query_incoming.as_ref().map(|connection| {
            fds.push(libc::pollfd {
                fd: connection.stream.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
            fds.len() - 1
        });

        // The kiosk socket, on the query socket's terms exactly: out of the
        // set while its own answer is outstanding or a connection is already
        // being read, and its own pending connection parked the same way.
        let kiosk_polled = match self.kiosk.as_ref() {
            Some(listener) if self.kiosk_answering.is_none() && self.kiosk_incoming.is_none() => {
                fds.push(libc::pollfd {
                    fd: listener.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                });
                Some(fds.len() - 1)
            }
            _ => None,
        };
        let kiosk_incoming_polled = self.kiosk_incoming.as_ref().map(|connection| {
            fds.push(libc::pollfd {
                fd: connection.stream.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
            fds.len() - 1
        });

        // **`rivet_core::facts`'s socket, and it never leaves the set.**
        //
        // The two sockets above leave it while an answer is outstanding because
        // a *listening* socket with a connection waiting is permanently
        // readable, and polling one while refusing to accept spins pid 1's only
        // child. A datagram socket has no such state: it is readable exactly
        // when a datagram is queued, and `drain_facts` takes them.
        let facts_polled = match self.facts.as_ref() {
            Some(socket) => {
                fds.push(libc::pollfd {
                    fd: socket.as_raw_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                });
                Some(fds.len() - 1)
            }
            None => None,
        };

        // **Every outstanding health question** — `ADR-045` §3. Which of them
        // became readable is not asked, for the captures' reason:
        // `drain_health` reads every one, stops at `WouldBlock`, and checks
        // every deadline — which removes any chance of an index disagreeing
        // with a map that a retirement is about to change.
        for asking in self.asking.values() {
            fds.push(libc::pollfd {
                fd: asking.stream.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
        }

        // `/dev/kmsg` last, so its index is `fds.len() - 1` and nothing above
        // has to account for whether it is open.
        let kmsg_polled = self.kmsg.as_ref().map(|f| {
            fds.push(libc::pollfd {
                fd: f.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
            fds.len() - 1
        });

        // **An open ingest connection shortens the sleep.** Its deadline is not
        // a descriptor and `poll` cannot wake for it, so a supervisor with
        // nothing else to do would hold a silent sender's connection until the
        // next unrelated event — which on a quiet machine is `None`, meaning for
        // ever. `ADR-022` §11.5's bound is only a bound if something checks it.
        let timeout_ms = self.next_ingest_deadline_ms().map_or(timeout_ms, |until| {
            Some(timeout_ms.map_or(until, |t| t.min(until)))
        });
        // **An open query or kiosk connection shortens it too**, for the same
        // reason and the same fix: `next_kiosk_deadline_ms`'s own deadline is
        // not a descriptor either, so a supervisor with nothing else to do
        // would otherwise hold a silent caller until whatever next unrelated
        // event happens to arrive.
        let timeout_ms = self.next_query_deadline_ms().map_or(timeout_ms, |until| {
            Some(timeout_ms.map_or(until, |t| t.min(until)))
        });
        let timeout_ms = self.next_kiosk_deadline_ms().map_or(timeout_ms, |until| {
            Some(timeout_ms.map_or(until, |t| t.min(until)))
        });
        // **An outstanding health question shortens it too**, and for the same
        // reason: `ADR-045` §3's timeout is only a timeout if something
        // wakes up to apply it. A supervisor with one wedged application and
        // nothing else pending would otherwise sleep indefinitely, holding a
        // question it had already decided to give up on.
        let timeout_ms = self.next_health_deadline_ms().map_or(timeout_ms, |until| {
            Some(timeout_ms.map_or(until, |t| t.min(until)))
        });
        let timeout = if busy {
            0
        } else {
            timeout_ms.map_or(-1, |ms| i32::try_from(ms).unwrap_or(i32::MAX))
        };
        // SAFETY: `poll` reads and writes exactly `fds.len()` `pollfd`
        // structures through a pointer to a live, correctly sized `Vec`. Every
        // descriptor in it is owned by `self` and open for the whole call.
        let rc = unsafe {
            libc::poll(
                fds.as_mut_ptr(),
                libc::nfds_t::try_from(fds.len()).unwrap_or(1),
                timeout,
            )
        };
        if rc < 0 {
            let e = Errno::last();
            // Interrupted before anything was reported. Nothing happened, and
            // saying so is not an error the caller should act on.
            // What was drained before the poll is still in hand, and is
            // returned rather than dropped.
            if e == Errno::EINTR {
                return Ok(early);
            }
            return Err(e);
        }
        if rc == 0 {
            // **A timeout is where a wedged application is CONCLUDED about.**
            // Returning `Timeout` alone here was the obvious spelling and would
            // have made `ADR-045` §3's whole case unreachable on a quiet
            // machine: the deadline that matters expires precisely when nothing
            // is happening, which is when `poll` returns zero.
            let mut events = early;
            self.drain_health(&mut events);
            events.push(Event::Timeout);
            return Ok(events);
        }

        let mut events = early;
        if fds[0].revents & libc::POLLIN != 0
            && let Some(file) = self.sigfd.as_mut()
        {
            let mut buf = [0u8; SIGINFO_SIZE * 8];
            if let Ok(n) = file.read(&mut buf) {
                for chunk in buf[..n].as_chunks::<SIGINFO_SIZE>().0 {
                    let mut signo = [0u8; 4];
                    signo.copy_from_slice(&chunk[..4]);
                    match i32::try_from(u32::from_ne_bytes(signo)).unwrap_or(0) {
                        libc::SIGTERM => events.push(Event::Terminate {
                            how: Terminating::PowerOff,
                        }),
                        libc::SIGINT => events.push(Event::Terminate {
                            how: Terminating::Restart,
                        }),
                        libc::SIGCHLD => events.push(Event::ChildChanged),
                        libc::SIGHUP => events.push(Event::Reload),
                        _ => {}
                    }
                }
            }
        }
        for (i, name) in names.iter().enumerate() {
            if fds[i + 1].revents & libc::POLLIN != 0
                && let Some(sock) = self.notify.get(name)
            {
                Self::drain_notify(name, sock, &mut events);
            }
        }
        // Which pipe became readable is not asked: `drain_captures` reads every
        // one and stops at `WouldBlock` or its budget, which costs one `read(2)` per quiet
        // pipe and removes any chance of a `revents` index disagreeing with the
        // order `captures` is in after a spawn or a reap. Not on a busy pass,
        // which drained every pipe before the poll.
        if !busy
            && fds
                .iter()
                .skip(1 + names.len())
                .take(capture_fds.len())
                .any(|p| p.revents & (libc::POLLIN | libc::POLLHUP) != 0)
        {
            self.drain_captures(&mut events);
        }
        // Which of the ingest descriptors became readable is not asked, for the
        // captures' reason: `drain_ingest` accepts and reads everything and
        // stops at `WouldBlock` or its budget, which removes any chance of an index
        // disagreeing with a list it rebuilds. Not on a busy pass, for the
        // captures' reason.
        if !busy
            && fds
                .iter()
                .skip(ingest_from)
                .take(ingest_count)
                .any(|p| p.revents & (libc::POLLIN | libc::POLLHUP) != 0)
        {
            self.drain_ingest(&mut events);
        }
        // Either the listener (a new connection to accept) or the connection
        // already accepted on it (more of a request already in progress) —
        // `drain_query` itself decides which, and reads until `WouldBlock`
        // either way, so there is nothing here for an index to disagree with.
        if (query_polled.is_some_and(|i| fds.get(i).is_some_and(|p| p.revents & libc::POLLIN != 0)))
            || query_incoming_polled
                .is_some_and(|i| fds.get(i).is_some_and(|p| p.revents & libc::POLLIN != 0))
        {
            self.drain_query(&mut events);
        }
        if (kiosk_polled.is_some_and(|i| fds.get(i).is_some_and(|p| p.revents & libc::POLLIN != 0)))
            || kiosk_incoming_polled
                .is_some_and(|i| fds.get(i).is_some_and(|p| p.revents & libc::POLLIN != 0))
        {
            self.drain_kiosk(&mut events);
        }
        if let Some(i) = facts_polled
            && fds.get(i).is_some_and(|p| p.revents & libc::POLLIN != 0)
        {
            self.drain_facts(&mut events);
        }
        if !busy
            && let Some(i) = kmsg_polled
            && fds.get(i).is_some_and(|p| p.revents & libc::POLLIN != 0)
        {
            self.drain_kmsg(&mut events);
        }
        // **Unconditionally, and not behind a `revents` check.** A question's
        // deadline can expire on a pass woken by something else entirely — a
        // `SIGCHLD`, a journal record — and a supervisor on a busy machine
        // would then be the one that never notices a wedged application. It
        // costs one `read(2)` per outstanding question and there is at most one
        // per running service.
        self.drain_health(&mut events);
        Ok(events)
    }

    fn open_journal_ingest(&mut self, path: &str) -> Result<(), Errno> {
        // The parent directory, which is `/run/rivet` on a machine and a
        // temporary directory in a test. `create_dir_all` because on a real boot
        // this may run before any service has been started and therefore before
        // `open_notify` has made `/run/rivet/notify`.
        if let Some(at) = path.rfind('/')
            && at > 0
        {
            std::fs::create_dir_all(&path[..at]).map_err(|e| Errno::from_io(&e))?;
        }
        // A socket left behind by a previous supervisor run would make `bind`
        // fail with EADDRINUSE, and `ADR-017` §3.1 expects previous runs. This
        // is `open_notify`'s existing rule and the same hazard.
        let _ = std::fs::remove_file(path);
        let listener =
            std::os::unix::net::UnixListener::bind(path).map_err(|e| Errno::from_io(&e))?;
        listener.set_nonblocking(true).map_err(|e| Errno::from_io(&e))?;
        // **After the bind, which is the only order that has no window.** A
        // socket created at the default mode and widened afterwards is
        // world-writable a moment later than intended; a socket created narrow
        // and widened is *unreachable* for that moment, which costs a sender its
        // record and nothing else. `ADR-022` §11.4 argues the mode itself.
        set_mode(path, rivet_core::journal::INGEST_SOCKET_MODE)?;
        self.ingest = Some(listener);
        Ok(())
    }

    fn open_query(&mut self, path: &str) -> Result<(), Errno> {
        if let Some(at) = path.rfind('/')
            && at > 0
        {
            std::fs::create_dir_all(&path[..at]).map_err(|e| Errno::from_io(&e))?;
        }
        // A socket left behind by a previous supervisor run — `open_journal_ingest`'s
        // rule and the same hazard.
        let _ = std::fs::remove_file(path);
        let listener =
            std::os::unix::net::UnixListener::bind(path).map_err(|e| Errno::from_io(&e))?;
        listener.set_nonblocking(true).map_err(|e| Errno::from_io(&e))?;
        // **After the bind**, which is the order the ingest socket argues for —
        // except that here the two windows point the other way. A socket created
        // at the default mode and narrowed a moment later is briefly READABLE BY
        // ANYONE, which for `0600` is the thing being prevented rather than a
        // lost record. It is still the right order: the alternative is `umask`
        // games at process level, and the window is between two adjacent
        // syscalls in a process nothing else has yet been started by.
        set_mode(path, QUERY_SOCKET_MODE)?;
        self.query = Some(listener);
        Ok(())
    }

    fn open_kiosk(
        &mut self,
        path: &str,
        owner: Option<rivet_core::service::AppIdentity>,
    ) -> Result<(), Errno> {
        if let Some(at) = path.rfind('/')
            && at > 0
        {
            std::fs::create_dir_all(&path[..at]).map_err(|e| Errno::from_io(&e))?;
        }
        let _ = std::fs::remove_file(path);
        let listener =
            std::os::unix::net::UnixListener::bind(path).map_err(|e| Errno::from_io(&e))?;
        listener.set_nonblocking(true).map_err(|e| Errno::from_io(&e))?;
        // `0600` first and the chown after, so there is no window in which the
        // socket is both permissive and unowned. `open_listeners` does the same
        // in the same order and for the same reason.
        set_mode(path, QUERY_SOCKET_MODE)?;
        if let Some(identity) = owner {
            self.chown(path, identity.uid, identity.gid)?;
        }
        self.kiosk = Some(listener);
        Ok(())
    }

    fn answer_kiosk(&mut self, answer: &str) {
        use std::io::Write as _;

        let Some(mut stream) = self.kiosk_answering.take() else {
            return;
        };
        let _ = stream.write_all(answer.as_bytes());
    }

    fn open_facts(&mut self, path: &str) -> Result<(), Errno> {
        if let Some(at) = path.rfind('/')
            && at > 0
        {
            std::fs::create_dir_all(&path[..at]).map_err(|e| Errno::from_io(&e))?;
        }
        // A socket left behind by a previous supervisor run — `open_query`'s
        // rule and the same hazard: `bind` would refuse the path.
        let _ = std::fs::remove_file(path);
        let socket =
            std::os::unix::net::UnixDatagram::bind(path).map_err(|e| Errno::from_io(&e))?;
        // **Non-blocking, and it is load-bearing rather than tidy.** Every read
        // and every answer on this socket happens inside the supervisor's own
        // pass, so a blocking one would put the loop at the mercy of whichever
        // application asked — which is `.rulings/` R-195 D5's invariant, and
        // the failure this whole surface exists to let a screen report.
        socket
            .set_nonblocking(true)
            .map_err(|e| Errno::from_io(&e))?;
        // **After the bind**, which is the ingest socket's order and its
        // argument: a socket created narrow and widened is unreachable for a
        // moment, which costs a caller one retry, where the other order is
        // briefly permissive.
        set_mode(path, rivet_core::journal::INGEST_SOCKET_MODE)?;
        self.facts = Some(socket);
        Ok(())
    }

    fn answer_facts(&mut self, answer: &str) {
        let Some(socket) = self.facts.as_ref() else {
            return;
        };
        // Popped rather than peeked: the request is finished either way, and a
        // reply left in the queue would send this answer to the next caller.
        let Some(reply) = self.facts_replies.pop_front() else {
            return;
        };
        // A send that fails is a caller that went away, or one whose own
        // receive queue is full. Both are the caller's problem and neither is
        // worth a record — and the socket is non-blocking, so neither can be
        // this supervisor's.
        let _ = socket.send_to(answer.as_bytes(), &reply);
    }

    fn answer_query(&mut self, answer: &str) {
        use std::io::Write as _;

        // Taken rather than borrowed: the connection is finished either way, and
        // leaving it in place would keep the listener out of the poll set for
        // ever if a write failed.
        let Some(mut stream) = self.answering.take() else {
            return;
        };
        // A write that fails is a caller that hung up, which is ordinary and is
        // not this supervisor's problem — there is nowhere to report it that the
        // asker would read.
        let _ = stream.write_all(answer.as_bytes());
    }

    fn open_notify(
        &mut self,
        service: &str,
        owner: Option<rivet_core::service::AppIdentity>,
    ) -> Result<String, Errno> {
        std::fs::create_dir_all(NOTIFY_DIR).map_err(|e| Errno::from_io(&e))?;
        let path = format!("{NOTIFY_DIR}/{service}");
        // A socket left behind by a previous supervisor run would make `bind`
        // fail with EADDRINUSE, and ADR-017 §3.1 expects previous runs.
        let _ = std::fs::remove_file(&path);
        let sock = std::os::unix::net::UnixDatagram::bind(&path).map_err(|e| Errno::from_io(&e))?;
        sock.set_nonblocking(true).map_err(|e| Errno::from_io(&e))?;
        // **After the bind and before the spawn.** The socket exists the moment
        // `bind` returns, and the service is not started until this call has
        // returned, so there is no window in which the child could reach a
        // socket it does not yet own.
        //
        // A root service passes `None` and is left alone: it already owns
        // everything, and chowning to 0:0 would be a syscall that changes
        // nothing and can still fail.
        if let Some(id) = owner {
            self.chown(&path, id.uid, id.gid)?;
        }
        self.notify.insert(service.to_string(), sock);
        Ok(path)
    }

    fn open_listeners(
        &mut self,
        service: &str,
        sockets: &[rivet_core::package::application::Socket],
        proxied: bool,
        owner: Option<rivet_core::service::AppIdentity>,
    ) -> Result<Vec<String>, Errno> {
        use rivet_core::package::application::Socket;
        use rivet_core::service::{LISTEN_SOCKET_MODE, listen_dir};
        use std::os::unix::net::UnixListener;

        // Re-entry is a restart, and a restart must not inherit the sockets of
        // the run before it — a stale unix path would make `bind` answer
        // `EADDRINUSE` and a held TCP fd would make it answer the same.
        self.close_listeners(service);
        if sockets.is_empty() {
            return Ok(Vec::new());
        }

        let name = rivet_core::service::ServiceName::new(service).ok_or(Errno(libc::EINVAL))?;
        let dir = listen_dir(&name);

        let mut opened: Vec<std::os::fd::OwnedFd> = Vec::new();
        let mut names: Vec<String> = Vec::new();
        let mut paths: Vec<String> = Vec::new();

        for socket in sockets {
            // **Loopback for a proxied application's port** — `ADR-046` §7,
            // `.rulings/` R-188. The one function the manifest readers and the
            // test kernel ask too; `None` only for a unix socket, which the
            // arm below binds by path.
            let address = socket
                .bind_address(proxied)
                .unwrap_or(std::net::Ipv4Addr::UNSPECIFIED);
            let fd = match socket {
                Socket::Tcp(port) | Socket::TcpDirect(port) => {
                    // `SO_REUSEADDR` is deliberately NOT set. A port still in
                    // TIME_WAIT belongs to a connection this machine has not
                    // finished, and starting anyway would be this supervisor
                    // overriding the kernel's own answer about whether the
                    // address is free. The restart backoff (`ADR-017` §3.5) is
                    // the mechanism for waiting, and it already exists.
                    let listener = std::net::TcpListener::bind((address, *port))
                        .map_err(|e| Errno::from_io(&e))?;
                    names.push(format!("tcp-{port}"));
                    std::os::fd::OwnedFd::from(listener)
                }
                Socket::Udp(port) | Socket::UdpDirect(port) => {
                    // **Plain `bind()` and nothing else** — `PLAN.md` §150.1.
                    // No `SO_REUSEADDR` for `Socket::Tcp`'s reason above, and no
                    // `SO_BROADCAST` or `IP_PKTINFO` either: those exist to let a
                    // server reply to a client with no address yet (DHCP), and
                    // setting either here would be this supervisor learning what
                    // the socket is FOR — exactly the special case `ADR-017`
                    // §4.1 keeps out of it. A service that needs one binds its
                    // own socket; this is for the ordinary case, a service that
                    // replies to whoever a datagram came from.
                    let socket = std::net::UdpSocket::bind((address, *port))
                        .map_err(|e| Errno::from_io(&e))?;
                    names.push(format!("udp-{port}"));
                    std::os::fd::OwnedFd::from(socket)
                }
                Socket::Unix(socket_name) => {
                    std::fs::create_dir_all(&dir).map_err(|e| Errno::from_io(&e))?;
                    let path = format!("{dir}/{socket_name}");
                    // A socket left by a previous supervisor run, exactly as
                    // `open_notify` removes one and for the same reason.
                    let _ = std::fs::remove_file(&path);
                    let listener = UnixListener::bind(&path).map_err(|e| Errno::from_io(&e))?;
                    // **After the bind and before the spawn**, on `open_notify`'s
                    // terms: the file exists the moment `bind` returns and the
                    // service has not been started, so there is no window in
                    // which anything could reach a socket at the wrong mode.
                    self.chmod(&path, LISTEN_SOCKET_MODE)?;
                    let _ = owner; // see `LISTEN_SOCKET_MODE`: root-owned, 0600.
                    // The declared name, which `parse_socket` has already
                    // refused a `:` and a `/` in.
                    names.push(socket_name.clone());
                    paths.push(path);
                    std::os::fd::OwnedFd::from(listener)
                }
            };
            opened.push(fd);
        }

        self.listeners
            .insert(service.to_string(), Listeners { fds: opened, paths });
        Ok(names)
    }

    fn open_health(&mut self, service: &str) -> Result<Option<String>, Errno> {
        use rivet_core::service::LISTEN_SOCKET_MODE;

        // **APPENDED to whatever `open_listeners` left, never prepended.**
        // `SPEC-03` §3.1 makes the descriptors `3 .. 3+n` in the manifest's
        // order, and every consumer in this tree either takes the first
        // (`rivet-fleetd`) or looks one up by name (`rivet-proxyd`,
        // `rivet-launcher`, `rivet-bluetoothd`). Appending moves none of them;
        // prepending would have moved all of them, silently, at the next boot.
        //
        // **Root-owned `0600`, like any other unix listener here.** The
        // application never opens this path — it is handed the descriptor and
        // accepts on it — and the only process that needs to `connect(2)` to it
        // is this one, which is root. A widened mode would let any local
        // process ask a service how it is and, worse, occupy its answering
        // loop.
        let path = self.health_path(service);
        std::fs::create_dir_all(&self.health_dir).map_err(|e| Errno::from_io(&e))?;
        // A socket left behind by a previous supervisor run, exactly as
        // `open_notify` removes one and for the same reason.
        let _ = std::fs::remove_file(&path);
        let listener = bind_health_listener(&path)?;
        // After the bind and before the spawn, on `open_notify`'s terms.
        let path = path.to_string_lossy().into_owned();
        self.chmod(&path, LISTEN_SOCKET_MODE)?;

        let held = self
            .listeners
            .entry(service.to_string())
            .or_insert_with(|| Listeners {
                fds: Vec::new(),
                paths: Vec::new(),
            });
        held.fds.push(std::os::fd::OwnedFd::from(listener));
        held.paths.push(path);
        Ok(Some(rivet_core::health::CHANNEL_NAME.to_string()))
    }

    fn ask_health(&mut self, service: &str) -> Result<(), Errno> {
        use std::io::Write as _;

        // One question outstanding per service. A second ask is the tight
        // retry loop `ADR-045` forbids, so it is refused by the shape rather
        // than by a comment.
        if self.asking.contains_key(service) {
            return Ok(());
        }
        let path = self.health_path(service);
        // **Non-blocking from the first syscall, and this time the socket is
        // CREATED that way** — see [`connect_without_waiting`], which is the
        // whole of the difference and the whole of the 2026-09-18 incident.
        //
        // **`EAGAIN` here is usually the previous question still waiting**:
        // the listener's backlog is zero (see [`bind_health_listener`]), so a
        // question the application never accepted holds the only place, and
        // this one is refused rather than queued behind it. That is the same
        // verdict — *not answering* — reached without leaving a second
        // unaccepted connection in the kernel.
        let mut stream = connect_without_waiting(&path)?;
        // A short write into a socket buffer nobody is reading is still a
        // question asked: the bytes are queued, and the answer's absence is
        // what is being measured. A write that fails outright is reported.
        stream
            .write_all(rivet_core::health::REQUEST.as_bytes())
            .map_err(|e| Errno::from_io(&e))?;
        let since_ms = self.now_ms();
        self.asking.insert(
            service.to_string(),
            Asking {
                stream,
                since_ms,
                buffer: Vec::new(),
            },
        );
        Ok(())
    }

    fn close_health(&mut self, service: &str) {
        // The socket FILE is `close_listeners`'s — it is one of this service's
        // listeners and is removed with them. What is dropped here is the
        // supervisor's own end of an outstanding question, which must not
        // outlive the process it was asked of: an answer arriving after a
        // restart would be the previous run's opinion, attributed to this one.
        self.asking.remove(service);
    }

    fn close_listeners(&mut self, service: &str) {
        // Dropping the `OwnedFd`s closes them. The unix paths are removed after,
        // so a path is never removed while its socket is still open — which
        // would leave a listener nothing can reach and no file to say so.
        if let Some(held) = self.listeners.remove(service) {
            drop(held.fds);
            for path in held.paths {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    fn close_notify(&mut self, service: &str) {
        if self.notify.remove(service).is_some() {
            let _ = std::fs::remove_file(format!("{NOTIFY_DIR}/{service}"));
        }
    }

    fn landlock_abi(&mut self) -> (u32, i32) {
        crate::landlock::abi()
    }

    /// One socket per apply, closed when it returns. Every step is bounded by
    /// the adapter socket's receive timeout, so a kernel that does not answer
    /// costs this pass seconds and not the loop.
    fn apply_firewall(
        &mut self,
        policy: &rivet_adapter_net::firewall::Policy,
    ) -> Result<rivet_adapter_net::firewall::Applied, String> {
        let mut socket =
            rivet_adapter_net::socket::NetlinkSocket::open_protocol(rivet_adapter_net::nft::NETLINK_NETFILTER)
                .map_err(|e| {
                    format!("a NETLINK_NETFILTER socket could not be opened ({e}), so this kernel has no nf_tables to apply a table to")
                })?;
        rivet_adapter_net::firewall::apply(&mut socket, policy).map_err(|refused| refused.to_string())
    }

    fn set_own_oom_score_adj(&mut self, value: i32) -> Result<(), Errno> {
        std::fs::write("/proc/self/oom_score_adj", value.to_string())
            .map_err(|e| Errno::from_io(&e))
    }

    fn spawn(&mut self, service: &str, program: &Program) -> Result<i32, Errno> {
        use std::os::unix::process::CommandExt as _;

        let mut cmd = std::process::Command::new(&program.path);
        cmd.args(&program.args);

        // **The service's cgroup, created and its `cgroup.procs` opened HERE
        // — in the parent, before the fork** — S-2's fix.
        //
        // Until this existed, the child was placed in its cgroup only by
        // `Kernel::place_in_cgroup`, called by `crate::supervise::Supervisor`
        // after this function has already returned a pid: the window from
        // `execve` through a journal append to that later call is real, and
        // anything the child forks in it — at its very first instruction, if
        // it chooses to — escapes this service's memory and CPU caps
        // permanently. It is invisible to `Kernel::observe`, so a restart
        // spawns a second instance beside it, and it survives every stop.
        //
        // Opening the descriptor here and writing the self-referring pid
        // `"0"` to it inside `pre_exec`, below, moves the JOIN to before the
        // fork: the window left is the gap between `fork(2)` and the first
        // line of that closure, which is as small as `clone3`'s
        // `CLONE_INTO_CGROUP` would make it, without needing a second way to
        // start a process.
        //
        // `None` on any setup failure — a hierarchy this process cannot
        // reach, a directory it cannot create, a `cgroup.procs` it cannot
        // open — is read as "fall back to today's after-the-fork join": the
        // existing `Kernel::place_in_cgroup` call and its own warning already
        // cover that case, so a setup problem here is not this function's to
        // report a second time.
        let cgroup_procs = open_cgroup_procs_for_join(CGROUP_ROOT, service);
        let cgroup_procs_raw: libc::c_int = cgroup_procs
            .as_ref()
            .map_or(-1, std::os::fd::AsRawFd::as_raw_fd);

        // **The environment is applied in the CHILD, with `setenv`, and never
        // with `Command::env`.** Changed 2026-08-31, and one variable is the
        // reason: `ADR-017` §3.4's `LISTEN_PID`, which must hold the pid of the
        // process being exec'd. That pid does not exist until after `fork`, and
        // `Command` builds its environment before it.
        //
        // **Measured rather than assumed** (`AGENTS.md` §6), on rustc 1.97.1:
        //
        // | what was done | what the child saw |
        // | :-- | :-- |
        // | `cmd.env("A","parent")` and `setenv("A","child")` in `pre_exec` | `parent` |
        // | `cmd.env` untouched, `setenv("A","child")` in `pre_exec` | `child` |
        //
        // std installs its own `environ` **after** the `pre_exec` closures run,
        // so a `setenv` there is overwritten — unless nothing asked std to
        // change the environment at all, in which case `execvp` reads the
        // inherited `environ` and the child's own `setenv` is what the program
        // gets. It is all-or-nothing per spawn, and this takes all.
        //
        // **`setenv` is not async-signal-safe: it allocates.** That is a real
        // rule being bent, and it is bent on a checked fact rather than a hope:
        // `fork` in a **single-threaded** parent gives the child a heap no other
        // thread can have been holding, so `malloc` there cannot deadlock.
        //
        // **The invariant this rests on: no other thread exists anywhere in
        // this process at the moment of any `fork` — here, or in
        // `Kernel::cec_send`.** `rivet-sysd` is a `signalfd` and a poll loop,
        // and `tests::no_non_test_source_spawns_a_thread` is what keeps that
        // true rather than merely stated: it refuses a `thread::spawn` or
        // `thread::Builder` anywhere in this crate's non-test source, so a
        // change that reintroduced one fails that test instead of deadlocking
        // a board.
        //
        // Everything else in the closure remains allocation-free.
        let child_env: Vec<(std::ffi::CString, std::ffi::CString)> = program
            .env
            .iter()
            .filter_map(|(key, value)| {
                Some((
                    std::ffi::CString::new(key.as_str()).ok()?,
                    std::ffi::CString::new(value.as_str()).ok()?,
                ))
            })
            .collect();

        // The descriptors this service's sockets are on, in the manifest's
        // order. **Raw rather than owned**: the parent keeps ownership in
        // `self.listeners` so the socket outlives one start, and the child only
        // ever `dup2`s them.
        let listeners: Vec<std::os::fd::RawFd> = self
            .listeners
            .get(service)
            .map(|held| {
                held.fds
                    .iter()
                    .map(std::os::fd::AsRawFd::as_raw_fd)
                    .collect()
            })
            .unwrap_or_default();

        // **The display end, if this service was selected for the screen** —
        // `PLAN.md` §106.2.
        //
        // TAKEN rather than borrowed: the parent must not keep the child's end
        // open, or the socket outlives the child and a write into the
        // supervisor's end succeeds against nobody. `DisplayChannel` carries
        // the longer form of that argument.
        //
        // It is taken on the FIRST spawn only. A service that crashes and is
        // restarted gets no display until the launcher selects it again, which
        // is correct: the lease it had was cut for a process that no longer
        // exists, and the kernel revoked it when that process died.
        let display: Option<std::os::fd::OwnedFd> = self
            .displays
            .get_mut(service)
            .and_then(|channel| channel.theirs.take());
        let display_raw: std::os::fd::RawFd = display
            .as_ref()
            .map_or(-1, std::os::fd::AsRawFd::as_raw_fd);

        // **A null stdin, made without a filesystem** — `PLAN.md` §123, Wave 2.4.
        //
        // The comment further down records why `Stdio::null()` was tried and
        // reverted: it opens `/dev/null` in the parent, and under
        // `tools/qemu/run.sh` the root is an initramfs with no devtmpfs, so
        // there is no `/dev/null` to open and three services failed to spawn.
        // Moving that `open` into the closure moves WHERE it fails, not
        // whether — on that same image every spawn would fail in the child.
        //
        // A pipe needs no filesystem. The parent drops both ends as soon as
        // `spawn` returns, so the child's descriptor 0 is readable, permanently
        // at end-of-file, and connected to nothing — which is what `/dev/null`
        // gives a reader, on a machine that has no `/dev`.
        //
        // **Made before the lift below, so its read end is lifted too.** It is
        // a fresh descriptor and the kernel gives out the lowest free number,
        // which is very likely to be inside the range the listeners are about
        // to be placed into.
        let mut ends: [libc::c_int; 2] = [-1, -1];
        // SAFETY: `ends` is two `c_int`s on this function's stack and `pipe2`
        // writes exactly two. `O_CLOEXEC` so neither end survives the `exec`;
        // the child's fd 0 is made by `dup2` in the closure, which clears it.
        #[allow(unsafe_code)]
        if unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return Err(Errno::from_io(&std::io::Error::last_os_error()));
        }
        // SAFETY: both are fresh descriptors `pipe2` just returned, owned here
        // and by nothing else.
        #[allow(unsafe_code)]
        let (nothing_to_read, write_end) = unsafe {
            use std::os::fd::FromRawFd as _;
            (
                std::os::fd::OwnedFd::from_raw_fd(ends[0]),
                std::os::fd::OwnedFd::from_raw_fd(ends[1]),
            )
        };
        // **Closed at once**, so the read end is at end-of-file from the first
        // instruction the child runs rather than from whenever this function
        // returns. Nothing is ever written into this pipe; it exists to be an
        // empty, read-only descriptor.
        //
        // And read-only is the other half of matching `/dev/null`: a program
        // that writes to its own stdin gets `EBADF` here, which is what it
        // would get from a `/dev/null` opened `O_RDONLY` — and is what it did
        // NOT get from the inherited `/dev/console`, which was `O_RDWR`.
        drop(write_end);

        // **Every source descriptor is lifted ABOVE the range it is about to be
        // placed into** — `PLAN.md` §123, Wave 2.5.
        //
        // The placement in the closure below is `dup2(source, 3 + i)` in order,
        // with only the `source == target` case handled. Until 2026-09-03 a
        // source that happened to sit *inside* `3 ..= 3 + n` and belonged to a
        // LATER position was overwritten before it had been copied — so the
        // application was handed a descriptor that is not the socket its
        // manifest names. The display end is the worst case: `RIVET_DISPLAY`
        // then names a listening socket, and the program blocks in `recvmsg`
        // for ever against something that will never send it a lease. Silent,
        // and dependent on whatever numbers the parent's opens happened to get.
        //
        // **Lifted here, in the parent, and not in the closure.** `pre_exec`
        // must be async-signal-safe, and relocating n descriptors there needs
        // somewhere to put n new numbers. Here there is a heap and a `Vec`; the
        // duplicates are `O_CLOEXEC`, so they are open across the fork — which
        // is when the closure needs them — and gone at `exec`.
        // **The Landlock ruleset, built here in the parent** — `ADR-042` §3.
        //
        // Before the fork, because every rule needs a directory *opened* and
        // the closure below may not allocate the vector that would take. The
        // descriptor it produces is `FD_CLOEXEC` — `landlock_create_ruleset`
        // sets that itself — so it is open across the fork, which is when the
        // child needs it, and gone at `exec`.
        //
        // **The ABI is asked again here rather than taken from the caller.**
        // `crate::supervise` asks it too, to say what is enforced before it
        // starts anything; this call is the one the ruleset is actually built
        // against, and two readings of the same authority are cheaper than one
        // reading carried between them (`AGENTS.md` §5).
        // **The capabilities kept, as one mask** — `Program::capabilities`.
        // With an identity they are kept across the uid change; without one
        // the program stays uid 0 and they are its whole bounding set.
        let kept: u64 = capability_mask(&program.capabilities);

        let ruleset: Option<crate::landlock::Ruleset> = match &program.confinement {
            None => None,
            Some(policy) => {
                // **A confinement without an identity would be applied
                // nowhere.** The closure below runs the whole privilege drop
                // inside `if let Some(identity)`, because `PR_SET_NO_NEW_PRIVS`
                // is `landlock_restrict_self`'s precondition and is set there.
                // A `Program` carrying a policy and no identity is therefore a
                // program that would start looking confined and be confined by
                // nothing — refused here rather than left as a comment.
                if program.identity.is_none() {
                    return Err(Errno(libc::EINVAL));
                }
                let (abi, errno) = crate::landlock::abi();
                if abi < crate::landlock::MINIMUM_ABI {
                    // The supervisor refuses before it reaches here, so this is
                    // the second gate rather than the first — and it is kept
                    // because a spawn that fell through it would run an
                    // application the machine has just promised is confined.
                    return Err(Errno(if errno == 0 { libc::ENOSYS } else { errno }));
                }
                Some(crate::landlock::build(policy, abi).map_err(|fault| fault.errno)?)
            }
        };
        let ruleset_raw: std::os::fd::RawFd =
            ruleset.as_ref().map_or(-1, crate::landlock::Ruleset::raw);

        let mut lifted: Vec<std::os::fd::OwnedFd> = Vec::new();
        // One past the last target: fd 0 through 2 are the standard streams,
        // `3 .. 3 + n` the listeners, and `3 + n` the display end.
        let ceiling = 3 + listeners.len() as std::os::fd::RawFd + 1;
        let mut lift = |fd: std::os::fd::RawFd| -> Result<std::os::fd::RawFd, Errno> {
            if fd < 0 || fd >= ceiling {
                return Ok(fd);
            }
            // SAFETY: `fd` is a descriptor this process owns — a listener held
            // in `self.listeners`, or the display end taken above. `F_DUPFD_CLOEXEC`
            // reads it and returns a new one; it does not close or consume it.
            #[allow(unsafe_code)]
            let high = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, ceiling) };
            if high < 0 {
                return Err(Errno::from_io(&std::io::Error::last_os_error()));
            }
            // SAFETY: `high` is a fresh descriptor `fcntl` just returned and
            // nothing else holds it, so this `OwnedFd` is its only owner.
            #[allow(unsafe_code)]
            lifted.push(unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(high) });
            Ok(high)
        };
        let listeners: Vec<std::os::fd::RawFd> = listeners
            .into_iter()
            .map(&mut lift)
            .collect::<Result<Vec<_>, Errno>>()?;
        let display_raw = lift(display_raw)?;
        let nothing_to_read_raw = lift(std::os::fd::AsRawFd::as_raw_fd(&nothing_to_read))?;
        // **Lifted like every other descriptor, and the reason is the same
        // one.** The `dup2` loop below writes over `3 ..= 3 + n`; a ruleset
        // descriptor that happened to land in that range would be overwritten
        // by a listening socket before the closure could use it, and
        // `landlock_restrict_self` would then be handed a socket. That is
        // `EBADFD` on a good day and a ruleset applied to the wrong object on
        // a bad one.
        let ruleset_raw = lift(ruleset_raw)?;
        // `ADR-017` §7: "a Rivet service does not open a log file; the
        // supervisor captures its output." `ADR-022` §4 is the same rule from
        // the journal's side. Until 2026-08-24 the streams were inherited, so a
        // service's words went to the console and never to `/state/log` —
        // closed the same day, `ADR-022` §9.2.
        //
        // **stdin was left INHERITED until 2026-09-03, and the reason it was
        // is preserved here because it is what makes the replacement the shape
        // it is.** The first version set it to `Stdio::null()`, on the argument
        // that a mechanism blocking on a read nobody will answer looks exactly
        // like one that hung. That argument is still sound and the
        // implementation was wrong: `Stdio::null()` opens `/dev/null` in the
        // parent *before* the fork, so a machine without one fails to spawn
        // **the service** with `ENOENT` — reported against the program's path,
        // which is present.
        //
        // That is not hypothetical. Under `tools/qemu/run.sh` the root is an
        // initramfs, the kernel does not auto-mount devtmpfs onto one, and this
        // line took `rivet-netd`, `rivet-timed` and `rivet-mcpd` down on the
        // first boot after it was written — each refused with "this image cannot
        // run the program this service names", naming a binary that was there.
        // Found by running, not by reading (`AGENTS.md` §6).
        //
        // That paragraph said the answer was to do it in the `pre_exec` closure
        // instead, after the fork. **It is half the answer**: it moves where the
        // failure lands and not whether there is one, because on that same
        // initramfs there is still no `/dev/null` to open. The other half is
        // the pipe made above — a descriptor at end-of-file, with no filesystem
        // anywhere in it.
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Measured, not assumed: on rustc 1.97.1, `std::process::Command` does
        // **not** reset the child's signal mask — a parent with SIGTERM and
        // SIGCHLD blocked (which this one is, for `signalfd`) hands both to
        // every child, and `SigBlk` in the child's `/proc/self/status` is
        // identical to the parent's. A service that inherited a blocked
        // SIGTERM would ignore the supervisor's own shutdown signal, so the
        // mask is cleared here.
        //
        // **The privilege drop, and the order is the whole of it.** `ADR-015`
        // §5.1: identity is the first sandbox mechanism, and this is where it
        // happens — between `fork` and `exec`, so the program has never
        // executed one instruction as uid 0.
        //
        // Copied out of the borrow before the closure, because `pre_exec` wants
        // `'static` and because a `u32` on the child's stack is the only thing
        // that is safe to touch in there anyway.
        let identity = program.identity;

        // **The earned supplementary groups, copied into a fixed array** —
        // `Program::groups`.
        //
        // A `Vec` would work, because nothing in the child allocates by merely
        // reading one. An array is used anyway: `pre_exec`'s contract is that
        // the closure touches nothing which could allocate or take a lock, and
        // the cheapest way to keep that obviously true is to carry no heap
        // pointer across the `fork` at all. `MAX_EARNED_GROUPS` is the cap and
        // anything beyond it is refused below rather than silently dropped — a
        // group quietly missing is a connection that fails with `EACCES` and
        // no explanation.
        let mut group_buf = [0u32; MAX_EARNED_GROUPS];
        if program.groups.len() > MAX_EARNED_GROUPS {
            return Err(Errno(libc::EINVAL));
        }
        for (slot, gid) in group_buf.iter_mut().zip(program.groups.iter()) {
            *slot = *gid;
        }
        let group_len = program.groups.len();

        // **The child's `oom_score_adj`, rendered here, in the parent**, so the
        // closure only writes bytes it was handed. Every child gets one, the
        // reset to `0` included: it would otherwise inherit the supervisor's
        // `-1000` and be exactly as immune to the OOM killer. `ADR-017` §3.
        let mut oom_buf = [0u8; 12];
        let oom_len = {
            use std::io::Write as _;
            let mut cursor = std::io::Cursor::new(&mut oom_buf[..]);
            let _ = write!(cursor, "{}", crate::definitions::oom_score_adj(service));
            usize::try_from(cursor.position()).unwrap_or(0)
        };

        // SAFETY: the closure runs in the child between `fork` and `exec` and
        // must be async-signal-safe. `sigemptyset` and `pthread_sigmask` are,
        // and `set` lives on the child's stack for the whole call. It
        // allocates nothing and takes no lock. The identity calls below are
        // **raw syscalls** for that reason — see the comment at them.
        unsafe {
            cmd.pre_exec(move || {
                // **First, before anything else in this closure** — S-2's
                // fix. No `dup2` below has touched this descriptor's number
                // yet, whatever it happens to be, so there is no risk of a
                // listener or the display end landing on top of it before it
                // is used. `join_cgroup` is one `write(2)` and allocates
                // nothing, so it costs this closure nothing extra to keep the
                // no-allocation rule the rest of it already documents.
                //
                // `< 0` is "no descriptor to join" — a setup failure in the
                // parent, or a machine with no cgroup2 — and this closure
                // proceeds exactly as it did before this existed. `>= 0`
                // means the parent handed over a real descriptor, and a
                // refusal to join through it fails the whole spawn: a
                // service whose child silently kept running outside its
                // cgroup is the exact escape this exists to close, so this
                // is not a fault to merely report and continue past.
                if cgroup_procs_raw >= 0 {
                    join_cgroup(cgroup_procs_raw)?;
                }

                // **The OOM killer's view of this child, before it runs a
                // single instruction of its own** — rendered above. `open`,
                // `write` and `close` are async-signal-safe and allocate
                // nothing. A machine with no `/proc` (`ENOENT`) has no
                // supervisor value to inherit either, so that alone is passed
                // over; any other refusal fails the spawn, because a child left
                // at the supervisor's `-1000` is the thing being prevented.
                let oom_fd = libc::open(
                    c"/proc/self/oom_score_adj".as_ptr(),
                    libc::O_WRONLY | libc::O_CLOEXEC,
                );
                if oom_fd < 0 {
                    let refused = std::io::Error::last_os_error();
                    if refused.raw_os_error() != Some(libc::ENOENT) {
                        return Err(refused);
                    }
                } else {
                    let wrote = libc::write(oom_fd, oom_buf.as_ptr().cast(), oom_len);
                    let refused = (wrote < 0).then(std::io::Error::last_os_error);
                    libc::close(oom_fd);
                    if let Some(refused) = refused {
                        return Err(refused);
                    }
                }

                let mut set: libc::sigset_t = std::mem::zeroed();
                if libc::sigemptyset(&raw mut set) != 0
                    || libc::pthread_sigmask(
                        libc::SIG_SETMASK,
                        &raw const set,
                        std::ptr::null_mut(),
                    ) != 0
                {
                    return Err(std::io::Error::last_os_error());
                }

                // --- fd 0, and a session of its own -----------------------
                //
                // **The application inherited the supervisor's stdin until
                // 2026-09-03, and on a booted machine that is `/dev/console`**
                // — an `O_RDWR` handle on the HDMI virtual terminal that no
                // manifest declares and nothing grants. An unprivileged program
                // could write to the screen with it, read what somebody typed
                // at the console, and issue VT ioctls (`PLAN.md` §123).
                //
                // `setsid` goes with it and is not decoration: without a new
                // session the child keeps the parent's controlling terminal and
                // can reacquire it by opening `/dev/tty`, which would put back
                // exactly what the line above takes away.
                // `dup2` clears `FD_CLOEXEC` on the new descriptor, which is
                // what keeps fd 0 across the `exec`.
                if nothing_to_read_raw >= 0 && libc::dup2(nothing_to_read_raw, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // A failure is not fatal: `setsid` returns `EPERM` when this
                // process is already a process group leader, which is a child
                // that has no controlling terminal to lose. Refusing to start a
                // service over that would be worse than the thing being
                // prevented.
                let _ = libc::setsid();

                // --- the descriptors, before the environment that describes
                //     them, so a failure here never leaves a child told it has
                //     sockets it does not have. `ADR-017` §3.4.
                for (index, source) in listeners.iter().enumerate() {
                    // 0, 1 and 2 are the streams std has already set up, so the
                    // first passed socket is 3 — which is the convention, not a
                    // choice.
                    let target = 3 + index as libc::c_int;
                    // `dup2` clears `FD_CLOEXEC` on the new descriptor, which
                    // is exactly what is wanted: the whole point is that it
                    // survives `exec`. It is a no-op when the numbers already
                    // match, and then the flag must be cleared by hand — a case
                    // that arises whenever the parent happens to have opened
                    // the socket at 3.
                    if *source == target {
                        if libc::fcntl(target, libc::F_SETFD, 0) < 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                    } else if libc::dup2(*source, target) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }

                // The display channel, immediately after the listening
                // sockets so the numbering is `3 .. 3+n` for those and the next
                // one for this. `PLAN.md` §106.2.
                let display_target = 3 + listeners.len() as libc::c_int;
                if display_raw >= 0 {
                    if display_raw == display_target {
                        if libc::fcntl(display_target, libc::F_SETFD, 0) < 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                    } else if libc::dup2(display_raw, display_target) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }

                // --- the environment, per the note above `child_env` ---------
                //
                // **Emptied first, so the child gets what it was HANDED and not
                // what the supervisor happens to be carrying.** `ADR-015`'s
                // posture is that an application is handed what it needs; until
                // 2026-09-04 it was handed those things *on top of* every
                // variable `rivet-sysd` had, which is the opposite arrangement
                // wearing the right clothes.
                //
                // **What sysd is carrying is not a short list, and it is not
                // this project's to choose.** sysd is `rivet-initd`'s child and
                // initd is PID 1, so sysd's environment is whatever the kernel
                // gave init: `HOME`, `TERM`, and **every `key=value` word on the
                // kernel command line that the kernel did not recognise**. So an
                // operator editing `cmdline.txt` sets variables in every
                // application on the machine, with nothing in any manifest to
                // say so and no way to see it.
                //
                // **What is left is the whole of what an application legitimately
                // needs, and each of the four is composed by the machine:**
                //
                // | variable | written by |
                // | :-- | :-- |
                // | `LD_LIBRARY_PATH` | `rivet_core::service::library_path`, from the manifest's declared closure and from nothing else |
                // | `RIVET_APP_DIR` | `service::current_dir` — where this application's installed files are |
                // | `RIVET_APP_DATA` | `service::data_dir` — where it may write |
                // | `LISTEN_FDS` / `LISTEN_PID` / `RIVET_DISPLAY` | this closure, below, from the descriptors actually placed |
                //
                // The first three arrive in `child_env` (`crate::applications`
                // builds them); a system service's `Program::env` is empty and
                // it needs nothing else, because every program in the image is
                // reached by an ABSOLUTE path and every Rivet binary is static
                // musl. Nothing here looks up `PATH`.
                //
                // **`TZ` is the variable that would have been load-bearing and
                // deliberately is not.** `rivet-core::etc` §5.3.1 rules that the
                // wall clock is `/etc/localtime` and that `TZ` stays UNSET,
                // precisely because musl, Go and chrono all read that path when
                // it is — so clearing the environment is what the timezone
                // design already assumed, not a risk to it.
                //
                // `clearenv` allocates in some libcs, exactly as `setenv` below
                // does; the note above `child_env` is the argument for why that
                // is sound here, and it covers this call too. `execvp` reads
                // `environ` after this closure returns — see that same note for
                // why nothing asked std to install one.
                if libc::clearenv() != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                for (key, value) in &child_env {
                    if libc::setenv(key.as_ptr(), value.as_ptr(), 1) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                if !listeners.is_empty() {
                    let mut digits = [0u8; 24];
                    if libc::setenv(
                        c"LISTEN_FDS".as_ptr(),
                        decimal(listeners.len() as u64, &mut digits),
                        1,
                    ) != 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                    // **The whole reason the environment moved into the child.**
                    // `sd_listen_fds` compares this with `getpid()` and hands
                    // back nothing when they differ, so a wrong value here is
                    // indistinguishable from no sockets at all.
                    let mut pid_digits = [0u8; 24];
                    if libc::setenv(
                        c"LISTEN_PID".as_ptr(),
                        decimal(libc::getpid() as u64, &mut pid_digits),
                        1,
                    ) != 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                }

                // **`RIVET_DISPLAY` names the descriptor, not the display.**
                // What is on the other end is the supervisor, and what arrives
                // over it is a DRM lease — the first one at start, and another
                // whenever the launcher re-issues (`ADR-036` §8.1). An
                // application reads descriptors from it; it never opens a card.
                if display_raw >= 0 {
                    let mut digits = [0u8; 24];
                    if libc::setenv(
                        c"RIVET_DISPLAY".as_ptr(),
                        decimal(display_target as u64, &mut digits),
                        1,
                    ) != 0
                    {
                        return Err(std::io::Error::last_os_error());
                    }
                }

                if let Some(id) = identity {
                    // **Before the uid drop, because two of the three cannot be
                    // done after it** — `PLAN.md` §123, Wave 2.6.
                    confine(kept)?;
                    // **Between the two, and both edges are load-bearing** —
                    // `ADR-042` §8. After `confine()`, because
                    // `landlock_restrict_self` answers `EPERM` to a caller with
                    // neither `PR_SET_NO_NEW_PRIVS` nor `CAP_SYS_ADMIN`, and
                    // `confine()` is what sets that bit. Before
                    // `drop_privilege`, so that nothing which could fail runs
                    // between the drop and the confinement.
                    if ruleset_raw >= 0 {
                        crate::landlock::restrict_self(ruleset_raw)?;
                    }
                    // **Only when something is kept.** Without it `setuid`
                    // clears the permitted set, which is what every other
                    // homed program gets and must go on getting.
                    if kept != 0 && libc::prctl(libc::PR_SET_KEEPCAPS, 1, 0, 0, 0) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    drop_privilege(id.uid, id.gid, &group_buf[..group_len])?;
                    if kept != 0 {
                        keep_capabilities(kept, true)?;
                    }
                } else if kept != 0 {
                    // **uid 0, reduced**: `NoNewPrivs` and a bounding set of
                    // exactly `kept`, then the sets this process holds cut to
                    // match. The `execve` recomputes a root program's sets
                    // from the bounding set, so this is the ceiling it and
                    // every child it starts will have.
                    reduce_root(kept)?;
                    keep_capabilities(kept, false)?;
                }
                Ok(())
            });
        }

        let mut child = cmd.spawn().map_err(|e| Errno::from_io(&e))?;
        let pid = i32::try_from(child.id()).unwrap_or(-1);
        // Taken out of the `Child` rather than left on it, so `try_wait` in
        // `reap_ready` does not also own them and the pipes outlive the
        // process — which is the whole point: a service's last line is written
        // just before it exits.
        let (out, err) = (child.stdout.take(), child.stderr.take());
        self.capture(pid, service, Stream::Stdout, out);
        self.capture(pid, service, Stream::Stderr, err);
        self.children.insert(pid, child);
        Ok(pid)
    }

    fn cec_send(&mut self, lit: bool, transaction: CecTransaction) -> Result<CecWorker, Errno> {
        use std::os::fd::{FromRawFd as _, IntoRawFd as _};

        let mut ends: [libc::c_int; 2] = [-1, -1];
        // SAFETY: `ends` is two `c_int`s on this function's stack and `pipe2`
        // writes exactly two. `O_CLOEXEC` so neither end survives a later
        // service `exec`.
        if unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return Err(Errno::from_io(&std::io::Error::last_os_error()));
        }
        // SAFETY: both are fresh descriptors `pipe2` just returned, owned
        // here and by nothing else.
        let (read, write) = unsafe {
            (
                std::os::fd::OwnedFd::from_raw_fd(ends[0]),
                std::os::fd::OwnedFd::from_raw_fd(ends[1]),
            )
        };

        // SAFETY: this process has exactly one thread at every call site —
        // `tests::no_non_test_source_spawns_a_thread` is what keeps that
        // true — which is what makes it safe for the child to allocate
        // before `_exit`, exactly as `spawn`'s `pre_exec` closure does.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return Err(Errno::from_io(&std::io::Error::last_os_error()));
        }
        if pid == 0 {
            // CHILD. Never returns: every path below ends in `_exit`, and the
            // whole of the work runs inside `catch_unwind` so that a panic —
            // `transaction`'s, or this function's own — cannot unwind back
            // through this process's inherited call stack and start dropping
            // a copy of state the parent still owns.
            drop(read);
            let write_fd = write.into_raw_fd();

            // **Everything this process had open, this child now has open
            // too — forking without an `exec` copies the whole descriptor
            // table, and `O_CLOEXEC` is an `exec`-time flag that a process
            // which never execs gets no benefit from.** That is every
            // pre-bound service listener, the signalfd, the journal's own
            // files, `/dev/kmsg`, the query/kiosk/facts sockets, and a kiosk
            // application's DRM lease if one is held — none of them opened
            // with this child in mind, and every one of them kept alive by
            // this child's copy for as long as the transaction below runs.
            // If `transaction` ever blocks (the whole reason it is not on
            // the convergence pass any more), a leaked listener stays bound
            // and a leaked lease stays alive, indefinitely, in a process
            // nothing is watching.
            //
            // `close_inherited` closes everything it can, leaving `write_fd`
            // and the standard streams — the only descriptors this child is
            // entitled to keep.
            close_inherited(Some(write_fd));

            let _ = std::panic::catch_unwind(move || {
                let (tag, text): (u8, String) = match transaction(lit) {
                    Ok(s) => (0, s),
                    Err(s) => (1, s),
                };
                let mut msg = Vec::with_capacity(1 + text.len().min(MAX_CEC_MESSAGE - 1));
                msg.push(tag);
                let mut text_bytes = text.as_bytes();
                if text_bytes.len() > MAX_CEC_MESSAGE - 1 {
                    text_bytes = &text_bytes[..MAX_CEC_MESSAGE - 1];
                    // Back off to the nearest character boundary rather than
                    // split one and hand `poll_cec_child` invalid UTF-8.
                    while !text_bytes.is_empty() && std::str::from_utf8(text_bytes).is_err() {
                        text_bytes = &text_bytes[..text_bytes.len() - 1];
                    }
                }
                msg.extend_from_slice(text_bytes);
                // SAFETY: `write_fd` is this child's own, exclusively-owned
                // descriptor; `msg` is at most `MAX_CEC_MESSAGE` bytes, well
                // under `PIPE_BUF`, so this is one atomic write. Nothing is
                // done about a failed write: `poll_cec_child` already reads a
                // silent child as "ended without an answer", which is exactly
                // what this would be.
                unsafe {
                    libc::write(write_fd, msg.as_ptr().cast::<libc::c_void>(), msg.len());
                }
            });
            // SAFETY: never returns. No destructor runs and nothing shared
            // with the parent — the journal included — is flushed.
            unsafe { libc::_exit(0) };
        }

        // PARENT.
        drop(write);
        Ok(CecWorker::Child { pid, read })
    }

    fn reclaim_in_child(&mut self, service: &str, dir: &str, prefix: &str) -> Result<(), Errno> {
        // Copied before the fork, so the child allocates nothing it has to
        // borrow from a structure the parent goes on changing.
        let (dir, prefix) = (dir.to_string(), prefix.to_string());
        // SAFETY: this process has exactly one thread at every call site —
        // `tests::no_non_test_source_spawns_a_thread` is what keeps that true —
        // which is what makes it safe for the child to allocate before
        // `_exit`, exactly as `cec_send`'s child does.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return Err(Errno::last());
        }
        if pid == 0 {
            // CHILD. Never returns, for `cec_send`'s reasons: everything runs
            // inside `catch_unwind` and ends in `_exit`, so no destructor runs
            // on a copy of state the parent still owns.
            close_inherited(None);
            let status = std::panic::catch_unwind(|| reclaim(&dir, &prefix)).unwrap_or(255);
            // SAFETY: never returns. No destructor runs and nothing shared
            // with the parent — the journal included — is flushed.
            unsafe { libc::_exit(status) };
        }
        // PARENT.
        self.helpers.push((
            pid,
            Some(Reclaiming {
                service: service.to_string(),
                dir,
                prefix,
            }),
        ));
        Ok(())
    }

    fn issue_in_child(&mut self, order: &IssueOrder) -> Result<Result<String, String>, Errno> {
        use std::os::fd::{AsRawFd as _, FromRawFd as _, IntoRawFd as _};

        let mut ends: [libc::c_int; 2] = [-1, -1];
        // SAFETY: `ends` is two `c_int`s on this function's stack and `pipe2`
        // writes exactly two.
        if unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return Err(Errno::last());
        }
        // SAFETY: both are fresh descriptors `pipe2` just returned, owned here
        // and by nothing else.
        let (read, write) = unsafe {
            (
                std::os::fd::OwnedFd::from_raw_fd(ends[0]),
                std::os::fd::OwnedFd::from_raw_fd(ends[1]),
            )
        };
        // Copied before the fork, so the child borrows nothing the parent goes
        // on changing.
        let order = order.clone();

        // SAFETY: this process has exactly one thread at every call site —
        // `tests::no_non_test_source_spawns_a_thread` is what keeps that true —
        // which is what makes it safe for the child to allocate before
        // `_exit`, exactly as `cec_send`'s child does.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            return Err(Errno::last());
        }
        if pid == 0 {
            // CHILD. Never returns, for `cec_send`'s reasons. The root's key is
            // read in here and nowhere else, and this process ends with it.
            drop(read);
            let write_fd = write.into_raw_fd();
            close_inherited(Some(write_fd));
            let _ = std::panic::catch_unwind(move || {
                let (tag, text) = match issue_certificate(&order, &mut getrandom) {
                    Ok(summary) => (0u8, summary),
                    Err(why) => (1u8, why),
                };
                let mut msg = Vec::with_capacity(MAX_ISSUE_MESSAGE);
                msg.push(tag);
                let mut text = text.as_bytes();
                if text.len() > MAX_ISSUE_MESSAGE - 1 {
                    text = &text[..MAX_ISSUE_MESSAGE - 1];
                    while !text.is_empty() && std::str::from_utf8(text).is_err() {
                        text = &text[..text.len() - 1];
                    }
                }
                msg.extend_from_slice(text);
                // SAFETY: `write_fd` is this child's own descriptor; `msg` is at
                // most `MAX_ISSUE_MESSAGE` bytes, within `PIPE_BUF`, so this is
                // one atomic write. A failed write is read by the parent as a
                // helper that ended without an answer.
                unsafe {
                    libc::write(write_fd, msg.as_ptr().cast::<libc::c_void>(), msg.len());
                }
            });
            // SAFETY: never returns. No destructor runs and nothing shared with
            // the parent — the journal included — is flushed.
            unsafe { libc::_exit(0) };
        }

        // PARENT.
        drop(write);
        let mut status: libc::c_int = 0;
        loop {
            // SAFETY: `pid` is this process's own child, forked just above and
            // added to no table, so nothing else waits for it. Blocking: the
            // helper's work is a key, a signature and three small files.
            let waited = unsafe { libc::waitpid(pid, &raw mut status, 0) };
            if waited == pid {
                break;
            }
            let why = Errno::last();
            if waited < 0 && why != Errno::EINTR {
                return Ok(Err(format!("the issuing helper could not be waited for: {why}")));
            }
        }
        let mut buf = vec![0u8; MAX_ISSUE_MESSAGE];
        // SAFETY: `read`'s descriptor is open and owned here, and `buf` is a
        // live buffer of the length given. The child has exited, so whatever
        // it wrote is already in the pipe and this cannot block.
        let n = unsafe {
            libc::read(read.as_raw_fd(), buf.as_mut_ptr().cast::<libc::c_void>(), buf.len())
        };
        let len = usize::try_from(n).unwrap_or(0);
        Ok(match buf[..len].split_first() {
            Some((0, text)) => Ok(String::from_utf8_lossy(text).into_owned()),
            Some((_, text)) => Err(String::from_utf8_lossy(text).into_owned()),
            None => Err(format!(
                "the issuing helper ended without an answer (wait status {status})"
            )),
        })
    }

    fn take_kiosk_lease(&mut self) -> Option<std::os::fd::OwnedFd> {
        self.kiosk_lease.take()
    }

    fn open_display(&mut self, service: &str) -> Result<(), Errno> {
        use std::os::fd::FromRawFd as _;

        let mut fds = [0 as libc::c_int; 2];
        // SEQPACKET, so one descriptor is one message and the receiver cannot
        // read half a hand-over. CLOEXEC on both: the child gets its end by an
        // explicit `dup2` in `spawn`, and nothing else should inherit either.
        // SAFETY: `fds` is a live two-element array the call fills in.
        let rc = unsafe {
            libc::socketpair(
                libc::AF_UNIX,
                libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC,
                0,
                fds.as_mut_ptr(),
            )
        };
        if rc != 0 {
            return Err(Errno(errno_now()));
        }
        // SAFETY: both are descriptors the kernel just created for this
        // process, and nothing else owns them.
        let (ours, theirs) = unsafe {
            (
                std::os::fd::OwnedFd::from_raw_fd(fds[0]),
                std::os::fd::OwnedFd::from_raw_fd(fds[1]),
            )
        };
        // Replacing closes any previous channel — see `DisplayChannel`.
        self.displays.insert(
            service.to_string(),
            DisplayChannel {
                ours,
                theirs: Some(theirs),
            },
        );
        Ok(())
    }

    fn send_display(
        &mut self,
        service: &str,
        lease: std::os::fd::OwnedFd,
    ) -> Result<(), Errno> {
        use std::os::fd::AsRawFd as _;

        let Some(channel) = self.displays.get(service) else {
            return Err(Errno(libc::ENOENT));
        };
        send_fd(channel.ours.as_raw_fd(), lease.as_raw_fd())
        // `lease` drops here, closing this process's copy. The receiver has its
        // own descriptor for the same open file: `SCM_RIGHTS` duplicates rather
        // than moves, so closing here does not take it away.
    }

    fn close_display(&mut self, service: &str) {
        self.displays.remove(service);
    }

    fn reap_ready(&mut self) -> Vec<(i32, ExitStatus)> {
        use std::os::unix::process::ExitStatusExt as _;

        let mut done = Vec::new();
        for (pid, child) in &mut self.children {
            // `Ok(None)` is still running and `Err` is a wait that itself
            // failed. Neither is something to report about this child on this
            // wakeup, and both are asked again on the next one.
            if let Ok(Some(status)) = child.try_wait() {
                let decoded = if let Some(code) = status.code() {
                    ExitStatus::Exited(code)
                } else if let Some(sig) = status.signal() {
                    ExitStatus::Signalled(sig)
                } else {
                    ExitStatus::Other
                };
                done.push((*pid, decoded));
            }
        }
        for (pid, _) in &done {
            self.children.remove(pid);
            // To EOF, before the descriptors go — see `close_captures`. The
            // lines land in `pending` and the next `wait` hands them over.
            self.close_captures(*pid);
        }
        // The helpers, on the same pass and by the same route: their outcome
        // lands in `pending` too.
        self.reap_helpers();
        done
    }

    fn signal_process(&mut self, pid: i32, sig: Sig) -> Result<(), Errno> {
        if pid <= 0 {
            // `kill(0, …)` signals the whole process group and `kill(-1, …)`
            // signals everything the caller may signal. Neither is ever what
            // the supervisor means, and as PID 1's child it would be a very
            // bad accident.
            return Err(Errno::EINVAL);
        }
        // SAFETY: `kill` takes two integers and returns one. It cannot read or
        // write this process's memory.
        let rc = unsafe { libc::kill(pid, sig.number()) };
        if rc == 0 { Ok(()) } else { Err(Errno::last()) }
    }

    fn observe(&mut self, service: &str) -> Observation {
        let path = format!("{CGROUP_ROOT}/{service}/cgroup.procs");
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let pids: Vec<i32> = text.lines().filter_map(|l| l.trim().parse().ok()).collect();
                if pids.is_empty() {
                    Observation::Empty
                } else {
                    Observation::Members(pids)
                }
            }
            Err(e) => Observation::Unavailable(Errno::from_io(&e)),
        }
    }

    fn cgroup_hierarchy(&mut self) -> Hierarchy {
        // Deliberately NOT `/proc/self/mounts`. `rivet_core::mount`'s note on
        // `CGROUP2_MOUNT` records that a cgroup2 mounted before sysfs stays in
        // the kernel's mount table while every path lookup under it lands in
        // sysfs instead — so the table can say "mounted" about a hierarchy no
        // path reaches. Reading the root's own control file exercises the same
        // lookup `observe` does, which is the only probe whose answer is about
        // the thing that actually matters.
        match std::fs::read_to_string(CGROUP_CONTROLLERS) {
            Ok(_) => Hierarchy::Present,
            Err(e) => Hierarchy::Unreachable(Errno::from_io(&e)),
        }
    }

    fn place_in_cgroup(&mut self, service: &str, pid: i32) -> Result<(), Errno> {
        let dir = format!("{CGROUP_ROOT}/{service}");
        std::fs::create_dir_all(&dir).map_err(|e| Errno::from_io(&e))?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(format!("{dir}/cgroup.procs"))
            .map_err(|e| Errno::from_io(&e))?;
        file.write_all(pid.to_string().as_bytes())
            .map_err(|e| Errno::from_io(&e))
    }

    fn cgroup_kill(&mut self, service: &str) -> Result<(), Errno> {
        // No `create_dir_all`, unlike `place_in_cgroup`: a cgroup that is not
        // there has nothing to kill, and `ENOENT` is exactly the right answer
        // for that rather than one this call should paper over by creating an
        // empty cgroup for nobody.
        std::fs::write(format!("{CGROUP_ROOT}/{service}/cgroup.kill"), b"1")
            .map_err(|e| Errno::from_io(&e))
    }

    fn list_dir(&mut self, path: &str) -> Result<Vec<String>, Errno> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(path).map_err(|e| Errno::from_io(&e))? {
            let entry = entry.map_err(|e| Errno::from_io(&e))?;
            // `file_type()` does not follow a symlink and `ADR-015` §7's
            // `current` is one — but that is *inside* an application's
            // directory, not here. What is here is `<name>/`, a real directory,
            // so `metadata()` (which does follow) is not needed and not used:
            // following a link at this level would let a link into `/system`
            // present itself as an installed application.
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_dir() {
                continue;
            }
            // Lossy is wrong for a path and right for a *name that must be a
            // service name*: `ServiceName::new` refuses anything that is not
            // ASCII anyway, so a name this mangles was already going to be
            // refused, and the alternative is silently skipping it.
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    fn list_dir_entries(&mut self, path: &str) -> Result<Vec<String>, Errno> {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(path).map_err(|e| Errno::from_io(&e))? {
            let entry = entry.map_err(|e| Errno::from_io(&e))?;
            // **No type filter, which is the whole point** — see the trait. And
            // no `metadata()`, so nothing here follows a symlink: a name is
            // matched against a constant pattern and then chowned by path, and
            // a link that matched would be chowned as the link rather than
            // followed to whatever it points at.
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
        names.sort();
        Ok(names)
    }

    fn apply_memory_limit(&mut self, service: &str, bytes: u64) -> Result<(), Errno> {
        // The controller has to be delegated down to where the service's own
        // cgroup is before `memory.max` exists there at all. Two writes, both
        // idempotent, both harmless when the controller is already on:
        //
        // 1. the cgroup2 root's `subtree_control`, so `/sys/fs/cgroup/rivet`
        //    gains the memory files. The root is exempt from the "no internal
        //    processes" rule, so this works with processes still in it.
        // 2. `rivet`'s own, so each service's directory gains them.
        //
        // A failure at either is the whole call's failure, and `ENODEV` on the
        // first is what a machine without the controller answers — `ADR-024`
        // §8.1's board.
        enable_controller(&format!("{}/cgroup.subtree_control", cgroup2_root()), "memory")?;
        std::fs::create_dir_all(CGROUP_ROOT).map_err(|e| Errno::from_io(&e))?;
        enable_controller(&format!("{CGROUP_ROOT}/cgroup.subtree_control"), "memory")?;

        let dir = format!("{CGROUP_ROOT}/{service}");
        std::fs::create_dir_all(&dir).map_err(|e| Errno::from_io(&e))?;
        std::fs::write(format!("{dir}/memory.max"), bytes.to_string().as_bytes())
            .map_err(|e| Errno::from_io(&e))
    }

    fn apply_cpu_limit(&mut self, service: &str, quota_us: u64) -> Result<(), Errno> {
        // Same two-write delegation as `apply_memory_limit`, the same
        // controller-per-write ordering, and the same reason: `cpu.max` does
        // not exist in a child cgroup until the parent delegates it.
        enable_controller(&format!("{}/cgroup.subtree_control", cgroup2_root()), "cpu")?;
        std::fs::create_dir_all(CGROUP_ROOT).map_err(|e| Errno::from_io(&e))?;
        enable_controller(&format!("{CGROUP_ROOT}/cgroup.subtree_control"), "cpu")?;

        let dir = format!("{CGROUP_ROOT}/{service}");
        std::fs::create_dir_all(&dir).map_err(|e| Errno::from_io(&e))?;
        // **`cpu.max` takes TWO numbers, space-separated: `"<quota> <period>"`,
        // both in microseconds** — unlike `memory.max`, which takes one. The
        // worked arithmetic lives on `rivet_core::service::CpuLimit`'s own
        // documentation; what matters here is only that the period is the
        // same constant on every write, because a quota is meaningless
        // without the period it is measured against.
        let value = format!("{quota_us} {}", rivet_core::service::CpuLimit::PERIOD_US);
        std::fs::write(format!("{dir}/cpu.max"), value.as_bytes()).map_err(|e| Errno::from_io(&e))
    }

    fn grow_state(&mut self) -> Grew {
        match SyscallGrowMedium::open() {
            Ok(mut medium) => rivet_adapter_image::grow::grow_state(&mut medium),
            // **`ReadDeviceSize`, and not a new `Refusal` variant.**
            // `GrowRefusal` is a closed vocabulary about *the partition table*
            // — not GPT, bad checksum, `/state` mislabelled — and "the device
            // node is not there" is not a statement about a table at all.
            // `Step::ReadDeviceSize` already means the first thing that touches
            // the device, its `is_after_a_write()` is false, and its own
            // sentence ends "Nothing was written", which is exactly true.
            //
            // Under QEMU this is the ordinary answer, every boot: `ADR-026` §4
            // — the guest runs from an initramfs and has no `/dev/mmcblk0` at
            // all. It must therefore read as "not applicable here" and never as
            // damage, which `ENOENT` against a named device does.
            Err((_what, errno)) => Grew::Failed {
                at: GrowStep::ReadDeviceSize,
                errno,
            },
        }
    }
}

/// Become `uid`/`gid` and nothing else, in the child, before `exec`.
///
/// # The order, which is the classic way to get this wrong
///
/// **Groups, then gid, then uid — and never any other order.** Each of the
/// three is privileged, so each must happen while the process is still uid 0:
///
/// 1. `setgroups(0, NULL)` — **drop the supplementary group list.** The
///    supervisor is PID 1's child and inherits root's list; a child that kept it
///    would carry every group root is in past the point where it could be
///    removed, and `setgroups` requires `CAP_SETGID`, which is gone after step
///    3. The list is emptied rather than set to the application's own gid,
///    because `rivet_core::service::AppIdentity` makes the primary gid the
///    application's own group already and a supplementary copy of it would say
///    nothing new.
/// 2. `setgid(gid)` — the group, **before** the user.
/// 3. `setuid(uid)` — last, because it is the one that cannot be undone.
///
/// Doing 3 before 1 or 2 is the textbook privilege-drop bug: `setuid` to a
/// non-zero uid discards every capability, so the `setgid` and `setgroups`
/// that follow fail with `EPERM`, and a caller that did not check would have a
/// process running as an unprivileged **user** that is still in root's
/// **groups**. **Every return value is checked**, and a failure aborts the
/// child rather than continuing — the failure mode of not checking is a service
/// that starts, works, and is not sandboxed, which is exactly the shape
/// `AGENTS.md` §5 calls a wrong guess succeeding.
///
/// # Why raw syscalls rather than the libc wrappers
///
/// musl's POSIX `setuid`/`setgid`/`setgroups` are process-wide: they broadcast
/// to every thread through `__synccall`, which sends signals and takes a lock.
/// **Neither is async-signal-safe**, and this runs after `fork` in a child that
/// has exactly one thread and must touch nothing that could deadlock. The raw
/// syscall changes the calling thread only, which in a post-`fork` child is the
/// whole process — so the raw form is both the safe one and the correct one.
///
/// # The read-back
///
/// After the three calls the child asks the **kernel** who it is, rather than
/// trusting that three zero returns mean what they should. It is four extra
/// syscalls in a path that runs once per service start, and it is the only
/// thing here that could catch a kernel where one of the three silently did
/// less than it claimed (`AGENTS.md` §5: report what the system said).
///
/// # SAFETY
///
/// Called only from a `pre_exec` closure. `syscall`, `getuid`, `getgid`,
/// `geteuid` and `getegid` are all async-signal-safe, nothing here allocates,
/// and nothing here takes a lock.
/// The listening sockets held on one service's behalf.
///
/// The descriptors and the filesystem paths are kept together because closing
/// one without removing the other is the failure mode: a unix path left behind
/// makes the next `bind` answer `EADDRINUSE`, and a path removed while its
/// socket is still open leaves a listener nothing can reach and no file to say
/// so.
struct Listeners {
    fds: Vec<std::os::fd::OwnedFd>,
    /// Unix socket paths, to unlink on close. Empty for a TCP-only service.
    paths: Vec<String>,
}

/// A `u64` as a NUL-terminated decimal C string, written into `buffer`.
///
/// **Allocation-free, because this runs between `fork` and `exec`.** The one
/// place the child is allowed to allocate is `setenv` — see the note above
/// `child_env` in [`Kernel::spawn`] — and formatting a number into it would be
/// a second, avoidable one. `u64::MAX` is 20 digits, so a 24-byte buffer cannot
/// overflow and the bound is asserted by the caller's array size rather than
/// checked here.
fn decimal(mut value: u64, buffer: &mut [u8; 24]) -> *const libc::c_char {
    let mut at = buffer.len() - 1;
    buffer[at] = 0;
    if value == 0 {
        at -= 1;
        buffer[at] = b'0';
        return buffer[at..].as_ptr().cast();
    }
    while value > 0 && at > 0 {
        at -= 1;
        buffer[at] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    buffer[at..].as_ptr().cast()
}

/// What a child gives up **before** it gives up being root.
///
/// # Why this is a floor and not a sandbox
///
/// `ADR-015` §6 is candid that this build has no mount namespace, no network
/// namespace and no seccomp filter, and none of that changes here. What changes
/// is that the three cheapest confinements now happen, each of which has to
/// happen while the process is still privileged — which is why this runs before
/// [`drop_privilege`] and not after.
///
/// 1. **`PR_SET_NO_NEW_PRIVS`.** Application volumes are mounted `exec`
///    (`ADR-031` §5), so a setuid binary that an application can write and then
///    execute would hand root straight back. With this set, `execve` never
///    grants privilege again for this process or anything it forks — and the
///    bit cannot be cleared, which is what makes it worth setting first.
/// 2. **The capability bounding set, emptied — or reduced to `kept`.**
///    `setuid` alone drops the effective and permitted sets, but the BOUNDING
///    set survives, and it is the ceiling on what any future `execve` of a
///    file with capabilities could raise. Emptying it means the ceiling is
///    nothing; a program that keeps capabilities ([`Program::capabilities`])
///    has a ceiling of exactly those.
/// 3. **`RLIMIT_NPROC`.** A bound on processes for this uid. Nothing else in
///    this build stops a program forking until the machine cannot start a new
///    one — and a supervisor that cannot fork cannot restart anything, so an
///    application that does it takes the whole machine rather than itself.
///
///    **Per-uid, which in this design is per-application.** `ADR-015` §5.1
///    gives every application its own uid, so the limit lands almost exactly
///    where a cgroup `pids.max` would. It is not identical — two services
///    sharing an identity would share the budget — and `pids.max` is still the
///    stronger form, but it would require every service to be placed in a
///    cgroup at start, which today only happens when `memory` is declared. That
///    is a change to how every service starts and it is not this one.
///
/// # Every call here is async-signal-safe
///
/// This runs in the child between `fork` and `exec`. `prctl` and `setrlimit`
/// are plain syscalls: no allocation, no lock, no libc state.
///
/// # Errors
///
/// Any of the three failing fails the spawn. **Deliberately**: this is the
/// privilege boundary, and a service that started with it half-applied would be
/// less confined than the manifest and the journal both say it is.
///
/// # Safety
///
/// The caller must be the child of a `fork`, before `exec`, and must not have
/// dropped privilege yet.
#[allow(unsafe_code)]
unsafe fn confine(kept: u64) -> std::io::Result<()> {
    unsafe {
        reduce_root(kept)?;

        // Generous, because this is a ceiling and not a budget: an application
        // with a worker per core and a few helpers is nowhere near it, and an
        // application forking without bound passes it immediately. Both limits
        // are set, so the process cannot raise its own soft limit back.
        let limit = libc::rlimit { rlim_cur: 256, rlim_max: 256 };
        if libc::setrlimit(libc::RLIMIT_NPROC, &raw const limit) != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// **`PR_SET_NO_NEW_PRIVS`, and the bounding set reduced to `kept`** — the
/// first two of [`confine`]'s three, and the whole of what a program that
/// stays uid 0 is given ([`Program::capabilities`] without an identity).
///
/// A root program's sets are recomputed from the bounding set at every
/// `execve`, so this is what makes a uid-0 program's capability list a
/// ceiling rather than a description: neither it nor any child it starts can
/// hold a capability outside `kept` again. `RLIMIT_NPROC` is [`confine`]'s
/// alone, because the kernel does not apply it to uid 0.
///
/// # Safety
///
/// [`confine`]'s: the child of a `fork`, before `exec`, still privileged.
#[allow(unsafe_code)]
unsafe fn reduce_root(kept: u64) -> std::io::Result<()> {
    unsafe {
        if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
            return Err(std::io::Error::last_os_error());
        }

        // **Walked upward until the kernel says the capability does not
        // exist**, rather than to a constant. `CAP_LAST_CAP` grows with the
        // kernel, and a number compiled in here would silently stop dropping
        // the newest capabilities the day the kernel gained one.
        let mut cap: libc::c_int = 0;
        while libc::prctl(libc::PR_CAPBSET_READ, cap, 0, 0, 0) >= 0 {
            let is_kept = cap < 64 && kept & (1u64 << cap) != 0;
            if !is_kept && libc::prctl(libc::PR_CAPBSET_DROP, cap, 0, 0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            cap += 1;
            // A guard against a kernel that answers `>= 0` for ever. 64 is far
            // above `CAP_LAST_CAP` on anything Rivet runs on (40 on 6.x).
            if cap > 128 {
                break;
            }
        }
    }
    Ok(())
}

unsafe fn drop_privilege(uid: u32, gid: u32, groups: &[u32]) -> std::io::Result<()> {
    unsafe {
        // 1. The supplementary list, while `CAP_SETGID` is still held.
        //
        // **Set to exactly what the manifest earned, which is usually
        // nothing.** `groups` holds one gid per SHARED endpoint this service
        // was granted (`.rulings/` R-206) and is empty for every service that
        // was granted none — which is the overwhelming majority, and for them
        // this is the `setgroups(0, NULL)` it has always been.
        //
        // **Dropping an INHERITED list and granting an EARNED group are
        // different acts**, and conflating them is what made a shared endpoint
        // look impossible for a while. The supervisor is PID 1's child and
        // inherits root's list; none of that may survive, and none of it does
        // — this call REPLACES the list rather than adding to it, so anything
        // not granted is gone whether or not `groups` is empty.
        if libc::syscall(
            libc::SYS_setgroups,
            groups.len(),
            if groups.is_empty() {
                std::ptr::null::<libc::gid_t>()
            } else {
                groups.as_ptr().cast::<libc::gid_t>()
            },
        ) != 0
        {
            return Err(std::io::Error::last_os_error());
        }
        // 2. The group, before the user.
        if libc::syscall(libc::SYS_setgid, gid) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // 3. The user, last.
        if libc::syscall(libc::SYS_setuid, uid) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // And the kernel's own account of what just happened. `EPERM` is the
        // closest errno to "the calls returned success and the identity is not
        // what they said"; there is no better one, and refusing is the point.
        if libc::getuid() != uid
            || libc::geteuid() != uid
            || libc::getgid() != gid
            || libc::getegid() != gid
        {
            return Err(std::io::Error::from_raw_os_error(libc::EPERM));
        }
    }
    Ok(())
}

/// `struct __user_cap_header_struct`, `linux/capability.h`.
#[repr(C)]
struct CapabilityHeader {
    version: u32,
    pid: libc::c_int,
}

/// `struct __user_cap_data_struct`: one 32-bit word of each set. Version 3
/// takes two of them, low word first.
#[repr(C)]
#[derive(Clone, Copy)]
struct CapabilityData {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

/// `_LINUX_CAPABILITY_VERSION_3`, the 64-bit form every kernel Rivet runs on
/// accepts.
const CAPABILITY_VERSION_3: u32 = 0x2008_0522;

/// Hold exactly `kept`, in the child, after [`drop_privilege`] and before
/// `exec` — [`Program::capabilities`].
///
/// # The sequence
///
/// `PR_SET_KEEPCAPS` was set before the `setuid`, so the permitted set
/// survived it and the effective set did not. Then:
///
/// 1. **`capset`** — permitted, effective and inheritable all become `kept`.
///    Lowering permitted needs no privilege; raising it is impossible, so a
///    capability the supervisor did not hold fails here with `EPERM` and the
///    spawn fails rather than starting a program that cannot do its work.
/// 2. **`PR_CAP_AMBIENT_RAISE`** for each — the set `execve` carries into a
///    program file with no file capabilities. It needs each capability in
///    both permitted and inheritable, which step 1 made true.
/// 3. **The read-back**, `capget` and `PR_CAP_AMBIENT_IS_SET`, so the claim is
///    the kernel's rather than the return codes' — the same reason
///    [`drop_privilege`] asks who it became.
///
/// The bounding set was reduced to `kept` by [`confine`], so after `execve`
/// the program holds these and can never hold another.
///
/// **`ambient` is `false` for a program that stays uid 0** ([`reduce_root`]):
/// steps 2 and its read-back are skipped, because the `execve` of a root
/// program grants it its bounding set whatever the ambient set says. Step 1
/// still runs, so the inheritable set cannot carry anything past the bounding
/// set either.
///
/// # SAFETY
///
/// Called only from a `pre_exec` closure. `syscall` and `prctl` are
/// async-signal-safe; nothing here allocates or takes a lock, and the two
/// structures live on this function's stack.
#[allow(unsafe_code, clippy::cast_possible_truncation)]
unsafe fn keep_capabilities(kept: u64, ambient: bool) -> std::io::Result<()> {
    let low = kept as u32;
    let high = (kept >> 32) as u32;
    let mut header = CapabilityHeader { version: CAPABILITY_VERSION_3, pid: 0 };
    let sets = [
        CapabilityData { effective: low, permitted: low, inheritable: low },
        CapabilityData { effective: high, permitted: high, inheritable: high },
    ];
    unsafe {
        if libc::syscall(libc::SYS_capset, &raw mut header, sets.as_ptr()) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        for cap in 0..64 as libc::c_ulong {
            if ambient
                && kept & (1u64 << cap) != 0
                && libc::prctl(libc::PR_CAP_AMBIENT, libc::PR_CAP_AMBIENT_RAISE as libc::c_ulong, cap, 0, 0)
                    != 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }

        let mut header = CapabilityHeader { version: CAPABILITY_VERSION_3, pid: 0 };
        let mut held = [CapabilityData { effective: 0, permitted: 0, inheritable: 0 }; 2];
        if libc::syscall(libc::SYS_capget, &raw mut header, held.as_mut_ptr()) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let word = |f: fn(&CapabilityData) -> u32| u64::from(f(&held[0])) | (u64::from(f(&held[1])) << 32);
        if word(|d| d.effective) != kept || word(|d| d.permitted) != kept {
            return Err(std::io::Error::from_raw_os_error(libc::EPERM));
        }
        for cap in 0..64 as libc::c_ulong {
            let wanted = ambient && kept & (1u64 << cap) != 0;
            let set =
                libc::prctl(libc::PR_CAP_AMBIENT, libc::PR_CAP_AMBIENT_IS_SET as libc::c_ulong, cap, 0, 0);
            if wanted && set != 1 {
                return Err(std::io::Error::from_raw_os_error(libc::EPERM));
            }
        }
    }
    Ok(())
}

/// The cgroup2 mount point, derived from the one path this crate already names.
///
/// [`CGROUP_CONTROLLERS`] is `<root>/cgroup.controllers`, and spelling the root
/// again here would be a second answer to where cgroup2 is mounted (`ADR-011`).
fn cgroup2_root() -> String {
    CGROUP_CONTROLLERS.rsplit_once('/').map_or_else(
        || CGROUP_CONTROLLERS.to_string(),
        |(root, _)| root.to_string(),
    )
}

/// Delegate one controller through one `cgroup.subtree_control`.
///
/// **Generalised to take the controller's name 2026-09-09**, when `cpu`
/// joined `memory` as a resource this crate caps — the write this function
/// makes is identical for both, and a second copy differing only in the
/// literal `+memory`/`+cpu` it wrote would be `ADR-011`'s failure in a
/// four-line function.
///
/// Writing `+memory` or `+cpu` where it is already enabled is not an error,
/// so this is safe to run on every capped start rather than once — which
/// matters, because "once" would need state, and `ADR-017` §3 has the
/// supervisor **observe rather than remember**.
fn enable_controller(path: &str, controller: &str) -> Result<(), Errno> {
    std::fs::write(path, format!("+{controller}").as_bytes()).map_err(|e| Errno::from_io(&e))
}

/// Open `<root>/<service>/cgroup.procs` for writing, creating the service's
/// cgroup first (`mkdir -p`, idempotent) — [`Kernel::spawn`]'s S-2 fix.
///
/// `None` on any failure: a hierarchy this process cannot reach, a directory
/// it cannot create, or a `cgroup.procs` it cannot open. The caller reads
/// `None` as "fall back to the after-the-fork join" — [`Kernel::place_in_cgroup`]
/// and its own warning already cover that case, so a setup problem here is
/// not reported a second time.
///
/// `root` rather than [`CGROUP_ROOT`] taken as a parameter, so a test can
/// point this at a temporary directory without touching the real cgroup2
/// hierarchy — the production call site passes the constant.
fn open_cgroup_procs_for_join(root: &str, service: &str) -> Option<std::fs::File> {
    let dir = format!("{root}/{service}");
    std::fs::create_dir_all(&dir).ok()?;
    std::fs::OpenOptions::new()
        .write(true)
        .open(format!("{dir}/cgroup.procs"))
        .ok()
}

/// Move the calling process into whatever cgroup `fd` is that cgroup's own
/// `cgroup.procs` — by writing the self-referring pid `"0"`. `ADR-017` §3's
/// cgroup authority, and [`Kernel::spawn`]'s S-2 fix: a child joins its
/// cgroup before it `exec`s, so nothing it forks in turn can escape the cap
/// that cgroup carries.
///
/// # Why this is its own function
///
/// So the failure half — the write refused — can be proven with a
/// descriptor chosen to fail, rather than only by forking a real process
/// and hoping the test environment's cgroupfs cooperates.
///
/// # SAFETY
///
/// `fd` must be a valid, open file descriptor for the duration of the call.
/// One `write(2)` and no allocation: async-signal-safe, which is what lets
/// [`Kernel::spawn`]'s `pre_exec` closure call it between `fork` and `exec`.
fn join_cgroup(fd: libc::c_int) -> std::io::Result<()> {
    let pid = b"0";
    // SAFETY: `fd` is valid per this function's own contract, and `pid` is a
    // one-byte buffer on this function's own stack.
    #[allow(unsafe_code)]
    let rc = unsafe { libc::write(fd, pid.as_ptr().cast::<libc::c_void>(), pid.len()) };
    if rc < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}


// ---------------------------------------------------------------------------
// `/state`'s grow — `ADR-018` §10.3, and the syscalls it needs
// ---------------------------------------------------------------------------

// **All three are `u32` and every call site casts with `as _`, and that is not
// laziness.** `libc::ioctl`'s request argument is `c_ulong` on glibc x86-64 and
// `c_int` on `aarch64-unknown-linux-musl` — so a constant typed for either one
// fails to compile against the other, and this crate has to build for both: the
// board is the second and the test host is the first. Measured rather than
// assumed; the cross-check caught it and the host build never would have.
//
// An ioctl request **is** a 32-bit encoding (`linux/ioctl.h`: 2 direction bits,
// 14 size, 8 type, 8 number), so `u32` is the honest width and the cast is a
// reinterpretation rather than a narrowing. `BLKGETSIZE64` has the top bit set
// and therefore arrives at the kernel as a negative `c_int` on the board; the
// bit pattern is what is compared, which is why that is correct and not merely
// tolerated.

/// `BLKGETSIZE64` — the size of a block device in bytes. `_IOR(0x12, 114, size_t)`.
///
/// Written out rather than taken from `libc`, which does not define the block
/// ioctls on musl. The encoding is `linux/ioctl.h`'s and is checked by
/// `the_ioctl_numbers_are_the_ones_linux_defines` below, because a wrong
/// constant here is a syscall against **a different device's command** and the
/// kernel would be within its rights to do something else entirely.
const BLKGETSIZE64: u32 = 0x8008_1272;

/// `BLKPG` — `_IO(0x12, 105)`. The container for [`BLKPG_RESIZE_PARTITION`].
const BLKPG: u32 = 0x0000_1269;

/// `BLKPG_RESIZE_PARTITION` — `linux/blkpg.h`. Tells the running kernel a
/// partition's length changed **without** re-reading the whole table, which
/// `BLKRRPART` would do and which needs every partition on the disk to be
/// unused. `/system` is mounted from this disk, so `BLKRRPART` could never
/// succeed here (`rivet_adapter_image::grow`'s `resize_partition`).
const BLKPG_RESIZE_PARTITION: libc::c_int = 3;

/// `EXT4_IOC_RESIZE_FS` — `_IOW('f', 16, __u64)`. The argument is the wanted
/// size in **filesystem blocks**.
const EXT4_IOC_RESIZE_FS: u32 = 0x4008_6610;

/// `BLKPG_DEVNAMELTH` / `BLKPG_VOLNAMELTH` from `linux/blkpg.h`.
const BLKPG_NAMELTH: usize = 64;

/// `struct blkpg_ioctl_arg`. Field order and types are the kernel's.
#[repr(C)]
struct BlkpgIoctlArg {
    op: libc::c_int,
    flags: libc::c_int,
    datalen: libc::c_int,
    data: *mut libc::c_void,
}

/// `struct blkpg_partition`. `start` and `length` are **bytes**, not sectors —
/// the one thing in this block that is easy to get wrong and impossible to
/// notice, because a sector-valued length is a plausible number that resizes the
/// partition to 1/512th of what was asked.
#[repr(C)]
struct BlkpgPartition {
    start: libc::c_longlong,
    length: libc::c_longlong,
    pno: libc::c_int,
    devname: [libc::c_char; BLKPG_NAMELTH],
    volname: [libc::c_char; BLKPG_NAMELTH],
}

/// What [`SyscallGrowMedium::open`] names when the DISK it discovered refuses.
///
/// A `&'static str` rather than the path, because the caller's error type is
/// `&'static str` and the path is only known at run time. It names the step
/// rather than the device, which is honest: `devices_in_use` carries the
/// reasoning and the journal carries the errno.
const DISK_IN_USE: &str = "the disk /state is on";

/// [`GrowMedium`] over two real descriptors.
///
/// **Two, and that is the whole reason this type exists rather than one file
/// and an offset.** `rivet_adapter_image::grow::GrowMedium::read_state_lbas`
/// carries the measurement: `/state` is *mounted* while this runs, so its
/// writes go through the **partition** device's page cache, and the whole-disk
/// device has a different one. A resize read back through the wrong device
/// reports the old size and the next boot decides to grow again. Harmless in
/// the instance that found it, and a component acting on a stale copy of a fact
/// either way.
struct SyscallGrowMedium {
    /// The whole disk, `O_RDWR` — the GPT lives here and so does the partition
    /// table the kernel is told about.
    disk: std::fs::File,
    /// `/state`'s own partition, `O_RDONLY`, for the **superblock read**.
    ///
    /// Read through the partition rather than the whole disk because a mounted
    /// filesystem's writes land in *this* device's page cache — the measurement
    /// in `GrowMedium::read_state_lbas`.
    state: std::fs::File,
    /// **The mount point**, `O_RDONLY | O_DIRECTORY`, and the only thing
    /// `EXT4_IOC_RESIZE_FS` may be issued against.
    ///
    /// # This field exists because a board said `EINVAL`
    ///
    /// It was the [`Self::state`] descriptor until 2026-08-25, on the reasoning
    /// — written into this file in as many words — that *"the descriptor is
    /// `/state`'s partition; the kernel resolves it to the mounted
    /// filesystem"*. **It does not.** An `ioctl` is dispatched through the
    /// open file's own `f_op`, and a block-device descriptor's is the block
    /// layer's, which has never heard of `fs/ext4/`. The request never reached
    /// ext4 at all.
    ///
    /// The first board to run it answered `EINVAL` at exactly that step, with
    /// the GPT already rewritten and `BLKPG_RESIZE_PARTITION` already done —
    /// so the partition really was 5.67 GiB and the filesystem really was still
    /// 232 MiB. `resize2fs` growing a mounted filesystem opens the **mount
    /// point**, and that is the whole of the difference.
    ///
    /// `AGENTS.md` §6, with nothing to soften it: this was reviewed, documented,
    /// cross-compiled, unit-tested and wrong, and only a card could say so.
    mount: std::fs::File,
}

impl SyscallGrowMedium {
    /// **Which partition `/state` is on, and which disk that partition belongs
    /// to** — both read from the running kernel, neither composed from a
    /// constant.
    ///
    /// # Why this does not fall back the way the mount does
    ///
    /// `cmd/rivet-initd`'s `/state` mount looks the partition up by its GPT
    /// label and **falls back** to `/dev/mmcblk0p5` when it cannot (`PLAN.md`
    /// §62.3), on the grounds that a machine which cannot read `/sys` should
    /// still boot the way it always did. **The same fall-back would be
    /// indefensible here**, and that asymmetry is the whole design:
    ///
    /// | | mounting the wrong device | rewriting the wrong device's GPT |
    /// | :-- | :-- | :-- |
    /// | likely outcome | `ENOENT` or `EINVAL`, and nothing happens | a partition table is overwritten |
    /// | recoverable | yes, trivially | **no** |
    ///
    /// So this **refuses** rather than guessing, and the refusal is cheap: the
    /// caller turns it into `Grew::Failed { at: ReadDeviceSize }`, whose own
    /// sentence ends *"Nothing was written"*, and `/state` stays the size it
    /// is. A card that did not grow is a card somebody can still repair.
    ///
    /// It also starts from **the partition that is mounted** rather than from a
    /// label search, so the only disk this can reach is the one this machine's
    /// `/state` is already on. The two-cards case `PLAN.md` §62.3 refuses to
    /// choose between cannot arise here at all: the second card is not what is
    /// mounted.
    fn devices_in_use() -> Result<(String, String), (&'static str, GrowErrno)> {
        use rivet_core::mount::{MOUNT_TABLE, source_mounted_at};
        use rivet_core::partlabel::{
            DEVNAME, SYS_BLOCK_DIR, containing_disk_uevent_path, device_node, entry_of_device,
            field,
        };
        use rivet_core::state::STATE_MOUNT_POINT;

        let table =
            std::fs::read_to_string(MOUNT_TABLE).map_err(|e| (MOUNT_TABLE, grow_errno(&e)))?;
        // Nothing mounted at /state is not a fault anywhere else in this
        // program and it is not one here: there is simply nothing to grow.
        let state = source_mounted_at(&table, STATE_MOUNT_POINT)
            .ok_or((STATE_MOUNT_POINT, GrowErrno::ENOENT))?;
        // **Not the line the emulator reaches**, measured 2026-08-28: without
        // `--card` the initramfs `/state` is a directory on rootfs rather than
        // a mount, so the lookup above returns `None` and this is never
        // evaluated. It guards the case where `/state` IS mounted and is not a
        // block device — a tmpfs, an overlay — where `tmpfs` is not a node
        // under /dev, there is no disk behind it, and there is no partition
        // table to touch.
        let entry = entry_of_device(state).ok_or((STATE_MOUNT_POINT, GrowErrno::ENODEV))?;
        let uevent = std::fs::read_to_string(containing_disk_uevent_path(entry))
            .map_err(|e| (SYS_BLOCK_DIR, grow_errno(&e)))?;
        let devname = field(&uevent, DEVNAME).ok_or((SYS_BLOCK_DIR, GrowErrno::ENODEV))?;
        Ok((state.to_string(), device_node(devname)))
    }

    /// Open both descriptors, or say which one refused.
    fn open() -> Result<Self, (&'static str, GrowErrno)> {
        // ~~`BOOT_DISK`, and `state_device(BOOT_DISK)`.~~ **`PLAN.md` §62.7.**
        // Both were right for a card in a Pi's own slot and wrong for the same
        // card anywhere else, and this is the process that writes a partition
        // table — see `devices_in_use` for why it refuses instead of assuming.
        let (state_path, disk_path) = Self::devices_in_use()?;
        let disk = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&disk_path)
            .map_err(|e| (DISK_IN_USE, grow_errno(&e)))?;
        let state = std::fs::File::open(&state_path).map_err(|e| {
            // A leaked path would be a second spelling of something discovered
            // once; the caller reports the mount point instead.
            (rivet_core::state::STATE_MOUNT_POINT, grow_errno(&e))
        })?;
        // The mount point, for the resize ioctl alone. `File::open` on a
        // directory is `O_RDONLY` and succeeds on Linux, which is all the ioctl
        // needs: it is a request to the filesystem, not a write to it.
        let mount = std::fs::File::open(rivet_core::state::STATE_MOUNT_POINT)
            .map_err(|e| (rivet_core::state::STATE_MOUNT_POINT, grow_errno(&e)))?;
        Ok(Self {
            disk,
            state,
            mount,
        })
    }

    /// `pread` a whole number of sectors from one of the two descriptors.
    fn read_at(file: &std::fs::File, lba: u64, count: u64) -> Result<Vec<u8>, GrowErrno> {
        use std::os::unix::fs::FileExt as _;
        let bytes = usize::try_from(count.saturating_mul(LBA_BYTES)).map_err(|_| GrowErrno::EINVAL)?;
        let offset = lba.checked_mul(LBA_BYTES).ok_or(GrowErrno::EINVAL)?;
        let mut buffer = vec![0u8; bytes];
        // `read_exact_at` rather than `read_at`: a short read of a GPT is a
        // truncated structure that would then fail a checksum somewhere less
        // obvious. `grow`'s `Refusal::ShortRead` exists for a medium that really
        // is short, which is a different thing from a read that stopped early.
        file.read_exact_at(&mut buffer, offset)
            .map_err(|e| grow_errno(&e))?;
        Ok(buffer)
    }
}

impl GrowMedium for SyscallGrowMedium {
    fn device_lbas(&mut self) -> Result<u64, GrowErrno> {
        let mut bytes: u64 = 0;
        // SAFETY: `BLKGETSIZE64` writes one `u64` through the pointer, which
        // points at a live local of exactly that type. The descriptor is a
        // block device this process opened; a non-block device answers ENOTTY
        // rather than writing anything.
        let rc = unsafe {
            libc::ioctl(
                std::os::unix::io::AsRawFd::as_raw_fd(&self.disk),
                BLKGETSIZE64 as _,
                &raw mut bytes,
            )
        };
        if rc < 0 {
            return Err(grow_errno_last());
        }
        Ok(bytes / LBA_BYTES)
    }

    fn read_lbas(&mut self, lba: u64, count: u64) -> Result<Vec<u8>, GrowErrno> {
        Self::read_at(&self.disk, lba, count)
    }

    fn write_lbas(&mut self, lba: u64, bytes: &[u8]) -> Result<(), GrowErrno> {
        use std::os::unix::fs::FileExt as _;
        let offset = lba.checked_mul(LBA_BYTES).ok_or(GrowErrno::EINVAL)?;
        self.disk
            .write_all_at(bytes, offset)
            .map_err(|e| grow_errno(&e))
    }

    fn read_state_lbas(&mut self, lba: u64, count: u64) -> Result<Vec<u8>, GrowErrno> {
        Self::read_at(&self.state, lba, count)
    }

    fn flush(&mut self) -> Result<(), GrowErrno> {
        // `sync_all`, not `sync_data`: the GPT write order in
        // `rivet_adapter_image::grow` is a claim about what reaches the medium
        // in what order, and a Pi has no battery.
        self.disk.sync_all().map_err(|e| grow_errno(&e))
    }

    fn resize_partition(
        &mut self,
        number: u32,
        start_bytes: u64,
        len_bytes: u64,
    ) -> Result<(), GrowErrno> {
        let mut part = BlkpgPartition {
            start: libc::c_longlong::try_from(start_bytes).map_err(|_| GrowErrno::EINVAL)?,
            length: libc::c_longlong::try_from(len_bytes).map_err(|_| GrowErrno::EINVAL)?,
            pno: libc::c_int::try_from(number).map_err(|_| GrowErrno::EINVAL)?,
            // The kernel identifies the partition by `pno` for a resize; the
            // name fields are for add/remove. Zeroed rather than filled, so
            // nothing here can disagree with what the kernel already believes.
            devname: [0; BLKPG_NAMELTH],
            volname: [0; BLKPG_NAMELTH],
        };
        let datalen = libc::c_int::try_from(std::mem::size_of::<BlkpgPartition>())
            .map_err(|_| GrowErrno::EINVAL)?;
        let mut arg = BlkpgIoctlArg {
            op: BLKPG_RESIZE_PARTITION,
            flags: 0,
            datalen,
            data: (&raw mut part).cast::<libc::c_void>(),
        };
        // SAFETY: `arg` and `part` are live for the call and `arg.data` points
        // at `part`, which is `#[repr(C)]` and laid out as `struct
        // blkpg_partition`. `datalen` is that structure's own size, so the
        // kernel copies exactly what is there. The descriptor is the whole-disk
        // device, which is what BLKPG requires — issued against a partition it
        // answers EINVAL.
        let rc = unsafe {
            libc::ioctl(
                std::os::unix::io::AsRawFd::as_raw_fd(&self.disk),
                BLKPG as _,
                &raw mut arg,
            )
        };
        if rc < 0 {
            return Err(grow_errno_last());
        }
        Ok(())
    }

    fn resize_filesystem(&mut self, blocks: u64) -> Result<(), GrowErrno> {
        let mut wanted = blocks;
        // SAFETY: the argument is one `u64` passed by pointer, per
        // `_IOW('f', 16, __u64)`, and it points at a live local of that type.
        // **The descriptor is the MOUNT POINT and must be** — see
        // [`SyscallGrowMedium::mount`]. An `ioctl` is dispatched through the
        // open file's own `f_op`; a block device's is the block layer's and
        // never reaches `fs/ext4/resize.c`, which is what a board answered
        // `EINVAL` to say.
        let rc = unsafe {
            libc::ioctl(
                std::os::unix::io::AsRawFd::as_raw_fd(&self.mount),
                EXT4_IOC_RESIZE_FS as _,
                &raw mut wanted,
            )
        };
        if rc < 0 {
            return Err(grow_errno_last());
        }
        Ok(())
    }
}

/// 512, and it is [`rivet_adapter_image::grow`]'s own constant rather than a
/// second copy of the number.
const LBA_BYTES: u64 = rivet_adapter_image::grow::LBA_BYTES;

/// An [`std::io::Error`] as the grow port's errno.
///
/// **Two `Errno` newtypes meet in this function and neither is wrong.** The
/// supervisor's [`Errno`] is this crate's vocabulary for its own syscalls;
/// `rivet_adapter_image::port::Errno` is that crate's, and `ADR-014` §3 keeps
/// an adapter's port independent of any one caller. They are both `i32` and the
/// conversion is total, so the honest thing is a named function that says so
/// rather than a `From` impl that makes the boundary invisible.
fn grow_errno(e: &std::io::Error) -> GrowErrno {
    // `raw_os_error` is `None` only for an error `std` synthesised rather than
    // one a syscall returned. Nothing in the medium above can produce one — every
    // error there comes from `open`, `pread`, `pwrite` or `fsync` — so the
    // fallback is unreachable rather than approximate, and `EIO` is the right
    // shape for "the device failed and did not say how" if it ever is reached.
    GrowErrno(e.raw_os_error().unwrap_or(libc::EIO))
}

/// `errno` after a failed `ioctl`, as the grow port's. See [`grow_errno`].
fn grow_errno_last() -> GrowErrno {
    GrowErrno(Errno::last().0)
}
/// How many bytes of control data the two `SCM_RIGHTS` paths reserve.
///
/// One descriptor needs `CMSG_SPACE(4)` — twenty-four bytes on a 64-bit target,
/// a sixteen-byte header plus four rounded up to the alignment. Sixty-four
/// leaves room for a sender that attached more than the protocol asks for,
/// which is what makes the extras closable rather than silently truncated.
const CONTROL_BYTES: usize = 64;

/// A control-message buffer **with the alignment `struct cmsghdr` requires.**
///
/// **Added 2026-09-04 for the audit of 2026-09-03.** Both `SCM_RIGHTS` paths
/// used a bare `[u8; 64]`, whose alignment is **one**. `msg_control` is read as
/// a `struct cmsghdr` — by the kernel, and by `CMSG_FIRSTHDR` / `CMSG_NXTHDR`
/// / `CMSG_DATA`, which dereference it — and that struct's alignment is
/// `size_t`'s: eight on every target this project builds for.
///
/// **Nothing was observed to go wrong, and that is the point.** aarch64 and
/// x86-64 both tolerate unaligned loads of a `size_t` in practice, so the code
/// worked; it was undefined behaviour that happened to be benign, which is the
/// class of defect that stops being benign when a compiler gets a new
/// optimisation or somebody builds for a target that traps. `#[repr(C,
/// align(8))]` makes the guarantee the code was already assuming.
///
/// **Deliberately not `[u64; 8]`.** The bytes are addressed as bytes — the
/// `CMSG_*` macros take a `*mut u8` — so the element type would be a cast at
/// every use, and the thing being fixed is the alignment rather than the shape.
#[repr(C, align(8))]
struct ControlBuffer([u8; CONTROL_BYTES]);

impl ControlBuffer {
    const fn zeroed() -> Self {
        Self([0u8; CONTROL_BYTES])
    }
}

/// How many descriptors a `SCM_RIGHTS` control message carries.
///
/// `cmsg_len` counts the **header as well as the data**, so the header has to
/// come off before the division — and it comes off with `checked_sub`, which is
/// the audit of 2026-09-03's second finding here.
///
/// **A `cmsg_len` shorter than a header is not a thing a working kernel
/// produces**, and the subtraction was written as if that settled it. It does
/// not: the value is read out of a buffer through a raw pointer after a
/// `recvmsg` that may have set `MSG_CTRUNC`, an underflow on `usize` is a
/// number near `2^64`, and the loop that follows would then read descriptors
/// out of memory past the end of the control buffer. Refusing to guess at a
/// header that is too short — `AGENTS.md` §5 — is one call.
fn descriptors_in(cmsg_len: usize, header_bytes: usize) -> usize {
    cmsg_len
        .checked_sub(header_bytes)
        .map_or(0, |payload| payload / core::mem::size_of::<libc::c_int>())
}

/// **Read one message and any descriptor sent beside it** — `PLAN.md` §106.2.
///
/// A file descriptor crosses a unix socket as *ancillary data*: `SCM_RIGHTS`
/// control bytes attached to an ordinary message. It is attached to a
/// particular message, and **an ordinary `read` on that message discards it** —
/// the kernel drops the control data and closes the sender's gift, with no
/// error anywhere. That is why the kiosk socket's first read is this and not
/// `Read::read`.
///
/// Exactly one descriptor is accepted. The protocol sends one — a display
/// lease — and a caller that sent several would be speaking a protocol this
/// build does not have; the extras are **closed** rather than leaked, because a
/// descriptor PID 1 forgets is one it holds until the machine stops.
///
/// # Errors
///
/// Whatever `recvmsg` said. A zero-length read is not an error here: it is a
/// caller that connected and hung up, and the empty string it produces fails
/// `Request::parse` one layer up, where the record is written.
///
/// **`EMSGSIZE` is this function's own**, for a message the kernel marked
/// `MSG_CTRUNC` — see below. Every descriptor that did arrive is closed before
/// it is returned.
fn recv_with_fd(
    stream: &std::os::unix::net::UnixStream,
) -> std::io::Result<(String, Option<std::os::fd::OwnedFd>)> {
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};

    let mut buf = [0u8; rivet_core::kiosk::REQUEST_MAX_BYTES];
    // Space for exactly one descriptor, and room to see that a sender attached
    // more. `CMSG_SPACE` includes the header and the alignment padding;
    // `CMSG_LEN` would not, and sizing this with the wrong one is how a cmsg is
    // silently truncated. **Aligned** — see [`ControlBuffer`].
    let mut control = ControlBuffer::zeroed();
    debug_assert!(CONTROL_BYTES as u32 >= unsafe { libc::CMSG_SPACE(4) });

    let mut iov = libc::iovec {
        iov_base: buf.as_mut_ptr().cast::<libc::c_void>(),
        iov_len: buf.len(),
    };
    // SAFETY: `msghdr` is a plain C struct with no invariants beyond the
    // pointers below, which are filled in immediately.
    let mut msg: libc::msghdr = unsafe { core::mem::zeroed() };
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.0.as_mut_ptr().cast::<libc::c_void>();
    msg.msg_controllen = CONTROL_BYTES as _;

    // SAFETY: `stream` is an open socket this process owns; `msg` points at
    // live buffers that outlive the call.
    let read = unsafe { libc::recvmsg(stream.as_raw_fd(), &raw mut msg, libc::MSG_CMSG_CLOEXEC) };
    if read < 0 {
        return Err(std::io::Error::last_os_error());
    }

    // **Did the control data fit?** The kernel sets `MSG_CTRUNC` when it did
    // not, and says nothing else about it: `recvmsg` still returns the byte
    // count of the ordinary data, so a caller that only looks at the return
    // value reads a truncated message as a whole one. That is the audit of
    // 2026-09-03's third finding here, and it matters more than it looks —
    // the descriptors that did not fit are **closed by the kernel** and lost,
    // so what a truncated read produces is a request that appears to have
    // arrived with a partial or missing lease.
    //
    // Read before the walk and acted on after it, so that whatever *did*
    // arrive is closed rather than leaked into PID 1 for the life of the
    // machine.
    let truncated = msg.msg_flags & libc::MSG_CTRUNC != 0;

    let text = String::from_utf8_lossy(&buf[..read as usize]).into_owned();

    // Walk the control messages. There is at most one in this protocol, and
    // anything else present is closed rather than ignored.
    let mut lease: Option<OwnedFd> = None;
    // SAFETY: `msg` was filled by `recvmsg`, which is what these macros read.
    let mut cmsg = unsafe { libc::CMSG_FIRSTHDR(&raw const msg) };
    while !cmsg.is_null() {
        // SAFETY: `cmsg` is non-null and points into `control`, which is live.
        let (level, kind, len) = unsafe { ((*cmsg).cmsg_level, (*cmsg).cmsg_type, (*cmsg).cmsg_len) };
        if level == libc::SOL_SOCKET && kind == libc::SCM_RIGHTS {
            // SAFETY: `CMSG_DATA` on a cmsg `recvmsg` produced.
            let data = unsafe { libc::CMSG_DATA(cmsg) };
            // How many descriptors this cmsg actually carries. Derived rather
            // than assumed — a sender may attach several in one message — and
            // derived with a subtraction that cannot wrap; [`descriptors_in`]
            // is where that is argued.
            //
            // **The cast is kept and the lint allowed**, not the other way
            // round: `cmsg_len`'s Rust type is the `libc` crate's rendering of
            // whatever the target's header says — `size_t` on the two targets
            // this builds for, and not everywhere — so removing it because it
            // is a no-op *here* is how a cross build stops compiling.
            //
            // SAFETY: `CMSG_LEN` is a pure computation over a constant.
            #[allow(clippy::unnecessary_cast)]
            let count = descriptors_in(len as usize, unsafe { libc::CMSG_LEN(0) } as usize);
            for i in 0..count {
                let mut raw: libc::c_int = -1;
                // SAFETY: reading `count` descriptors out of the data area the
                // header says holds exactly that many, unaligned because the
                // control buffer has no alignment guarantee for `c_int`.
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        data.add(i * core::mem::size_of::<libc::c_int>()),
                        (&raw mut raw).cast::<u8>(),
                        core::mem::size_of::<libc::c_int>(),
                    );
                }
                if raw < 0 {
                    continue;
                }
                // SAFETY: `raw` is a descriptor the kernel just created in this
                // process; nothing else owns it.
                let owned = unsafe { OwnedFd::from_raw_fd(raw) };
                if lease.is_none() {
                    lease = Some(owned);
                }
                // Any second descriptor is dropped here, which closes it.
            }
        }
        // SAFETY: as above.
        cmsg = unsafe { libc::CMSG_NXTHDR(&raw const msg, cmsg) };
    }

    if truncated {
        // `lease` is dropped here, which closes it. **Refuse rather than
        // guess** (`AGENTS.md` §5): a message whose control data did not fit is
        // not this protocol, and returning its text with whichever descriptors
        // happened to survive would hand the caller a lease it cannot reason
        // about. `EMSGSIZE` is the platform's own word for it.
        return Err(std::io::Error::from_raw_os_error(libc::EMSGSIZE));
    }

    Ok((text, lease))
}

/// **One application's display channel** — `PLAN.md` §106.2.
///
/// A `socketpair`, kept apart so the two ends can have different lifetimes:
/// `ours` lives as long as the service is selected, and `theirs` only until the
/// spawn that hands it to the child.
///
/// **`theirs` is dropped after the first spawn, deliberately.** A parent holding
/// the child's end open keeps the socket alive after the child dies, so a write
/// into `ours` would succeed against nobody and a dead application would look
/// reachable. Closing it makes the kernel tell the truth: once the child is
/// gone, `ours` reports `EPIPE`.
struct DisplayChannel {
    /// The supervisor's end. Display descriptors are written into this.
    ours: std::os::fd::OwnedFd,
    /// The child's end, waiting for a spawn to hand it over.
    theirs: Option<std::os::fd::OwnedFd>,
}

/// Send one file descriptor over a unix socket, with no payload but a byte.
///
/// `SCM_RIGHTS` is attached to a **message**, and a message with no data is not
/// sent at all — so one byte goes with it. The receiver reads that byte and the
/// descriptor together; without it there would be nothing to attach the
/// ancillary data to and nothing for `recvmsg` to return.
fn send_fd(socket: std::os::fd::RawFd, fd: std::os::fd::RawFd) -> Result<(), Errno> {
    let mut payload = [b'\n'];
    let mut iov = libc::iovec {
        iov_base: payload.as_mut_ptr().cast::<libc::c_void>(),
        iov_len: payload.len(),
    };
    // **Aligned** — see [`ControlBuffer`]. The kernel writes a `struct cmsghdr`
    // through this pointer and the `CMSG_*` macros below dereference it as one.
    let mut control = ControlBuffer::zeroed();

    // SAFETY: a plain C struct; every pointer below outlives the call.
    let mut msg: libc::msghdr = unsafe { core::mem::zeroed() };
    msg.msg_iov = &raw mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.0.as_mut_ptr().cast::<libc::c_void>();
    // SAFETY: `CMSG_SPACE` is a pure computation over a constant.
    let space = unsafe { libc::CMSG_SPACE(core::mem::size_of::<libc::c_int>() as u32) } as usize;
    debug_assert!(space <= CONTROL_BYTES);
    msg.msg_controllen = space as _;

    // SAFETY: `msg.msg_control` points at `control`, which is large enough for
    // one descriptor — the `debug_assert` above says so, and `CMSG_FIRSTHDR`
    // answers null if `msg_controllen` were ever smaller than a header, which
    // is the branch below.
    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&raw const msg);
        if cmsg.is_null() {
            return Err(Errno(libc::EINVAL));
        }
        (*cmsg).cmsg_level = libc::SOL_SOCKET;
        (*cmsg).cmsg_type = libc::SCM_RIGHTS;
        (*cmsg).cmsg_len = libc::CMSG_LEN(core::mem::size_of::<libc::c_int>() as u32) as _;
        core::ptr::copy_nonoverlapping(
            (&raw const fd).cast::<u8>(),
            libc::CMSG_DATA(cmsg),
            core::mem::size_of::<libc::c_int>(),
        );
    }

    // SAFETY: `socket` is an open socket this process owns.
    let sent = unsafe { libc::sendmsg(socket, &raw const msg, libc::MSG_NOSIGNAL) };
    if sent < 0 {
        return Err(Errno(errno_now()));
    }
    Ok(())
}

/// `errno`, for the raw calls above.
fn errno_now() -> libc::c_int {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(libc::EIO)
}

/// How many connections an application's answering socket may hold that it
/// has not accepted. **Zero, which the kernel reads as one** — see
/// [`bind_health_listener`].
const HEALTH_BACKLOG: libc::c_int = 0;

/// **Bind an application's answering socket with room for exactly one
/// unaccepted question** — `ADR-045` §3.
///
/// # Why the backlog is the bound, and not a drain
///
/// Every connect to a unix listener creates the server's half of the
/// connection in the kernel at once and queues it, with the bytes written to
/// it, until somebody calls `accept`. The supervisor abandons a question after
/// `rivet_core::health::PATIENCE_MS` by closing its own half; **the queued half
/// stays until it is accepted or the listener is closed.** An application that
/// never accepts on this socket therefore gains one orphaned socket and one
/// buffer of unreclaimable kernel memory per interval, until the backlog fills
/// — and `UnixListener::bind` asks for the largest backlog the kernel allows.
///
/// The obvious repair is for the supervisor to accept and discard what is
/// queued, on its own copy of the listener. **It cannot do that without
/// waiting**, and waiting is the one thing this supervisor may not do:
///
/// * the descriptor shares ONE open file description with the application's
///   copy, so `O_NONBLOCK` set here is set for the application — whose
///   `rivet_sdk::health::Answering::serve` loop ends at the first `accept`
///   that fails, and would stop answering for good;
/// * `accept4`'s `SOCK_NONBLOCK` governs the socket it RETURNS, not the call;
/// * `poll` then `accept` races the application's own `accept`, and losing
///   that race leaves the supervisor asleep in `accept` until somebody else
///   connects — and nobody else connects to this socket;
/// * a thread to absorb the wait is not available, because this crate has
///   none and a test holds that.
///
/// **So the queue is bounded where it is created instead.** `listen(2)` again
/// on a listening socket changes only its backlog, and a unix listener refuses
/// a connect once its queue is LONGER than the backlog — so zero admits one.
/// The one is the question just asked; a second ask finds it still waiting and
/// is refused with `EAGAIN`, which `ask_health` already reports as *not
/// answering*. Nothing about an application that does accept changes: it
/// empties the queue as it answers, and the next question finds room.
///
/// Only the supervisor connects to this socket (it is root-owned `0600`), and
/// it holds one question per service at a time, so a backlog of one is the
/// whole demand rather than a limit on it.
///
/// # Errors
///
/// Whatever `bind(2)` or `listen(2)` said.
fn bind_health_listener(path: &std::path::Path) -> Result<std::os::unix::net::UnixListener, Errno> {
    use std::os::fd::AsRawFd as _;

    let listener = std::os::unix::net::UnixListener::bind(path).map_err(|e| Errno::from_io(&e))?;
    // SAFETY: `listen` reads the descriptor number and an integer; the
    // descriptor is owned by `listener`, which outlives the call.
    let rc = unsafe { libc::listen(listener.as_raw_fd(), HEALTH_BACKLOG) };
    if rc != 0 {
        return Err(Errno::last());
    }
    Ok(listener)
}

/// **Connect to a unix socket without ever waiting for the peer** — the repair
/// of 2026-09-18, and `.rulings/` R-195 D5 in one function.
///
/// # What went wrong, so that it is not undone
///
/// This replaced two lines that read:
///
/// ```ignore
/// let stream = UnixStream::connect(&path)?;   // blocking connect
/// stream.set_nonblocking(true)?;              // one line too late
/// ```
///
/// with a comment above them asserting *"non-blocking from the first
/// syscall"*. It was not: `UnixStream::connect` creates a **blocking** socket
/// and connects on it, and `set_nonblocking` runs afterwards — so the very
/// first syscall was the one that could sleep.
///
/// The comment's justification was worse than its claim, because it is
/// **inverted**: it argued that a wedged application leaves backlog room
/// *because* it is not accepting. An application that is not accepting is
/// precisely what **fills** a backlog — every unaccepted connection stays
/// queued in it — so the case the comment named as safe is the case that wedges
/// the supervisor.
///
/// On 2026-09-18 at `14:12:43Z` it did, for the second time in five days.
/// `rivet-01`'s supervisor was captured in `wchan` `unix_wait_for_peer`,
/// syscall `203` — `connect` — and stayed there. Everything it had already
/// started kept running, so the machine looked healthy from outside while
/// nothing supervised it. `docs/design-candidates/
/// Supervisor-Blocking-Audit-2026-09-16.md` §11 is the incident.
///
/// # Why this is small
///
/// **A unix-domain `connect` is synchronous.** With backlog room it succeeds
/// immediately; with a full backlog and `O_NONBLOCK` it fails immediately with
/// `EAGAIN`. There is no `EINPROGRESS` half-open state to poll for, the way
/// there would be on TCP, so nothing downstream of this changes and the caller
/// needs no new path: `crate::supervise`'s `ask_health_where_due` already
/// treats an `Err` as *this service is not answering*, records it, and pushes
/// the next ask out by a whole interval.
///
/// # `SOCK_CLOEXEC` is not optional
///
/// `UnixStream::connect` sets it; hand-rolling the socket drops it unless it is
/// asked for. A supervisor descriptor without `CLOEXEC` is inherited by every
/// application spawned while it is open — one application holding another's
/// health channel, which is a worse defect than the one this function fixes.
/// `LibcKernel::spawn`'s `socketpair` carries the same flag for the same reason.
///
/// # Errors
///
/// `EAGAIN` when the peer's backlog is full — *the application is not
/// accepting*, which is a real answer and the one that used to hang.
/// `ENOENT` when nothing is listening, `EACCES` when the mode refuses, and
/// whatever else the platform says. **Never a wait.**
fn connect_without_waiting(path: &std::path::Path) -> Result<std::os::unix::net::UnixStream, Errno> {
    use std::os::unix::ffi::OsStrExt as _;

    let bytes = path.as_os_str().as_bytes();
    // **Refused rather than truncated.** `sun_path` is 108 bytes and must hold
    // a trailing NUL; a path that does not fit would otherwise be silently cut
    // and this would connect to a DIFFERENT socket than the one named.
    // `rivet_core::service` already sizes service names against this limit.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= std::mem::size_of_val(&address.sun_path) {
        return Err(Errno(libc::ENAMETOOLONG));
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (slot, byte) in address.sun_path.iter_mut().zip(bytes) {
        *slot = *byte as libc::c_char;
    }

    // `SOCK_NONBLOCK` in the CREATION flags is the fix. `SOCK_CLOEXEC` is the
    // property `UnixStream::connect` was giving us for free.
    let fd = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        return Err(Errno::last());
    }
    // Owned from here, so every path below closes it exactly once.
    let owned = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) };

    let length = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
    loop {
        let rc = unsafe {
            libc::connect(
                fd,
                std::ptr::addr_of!(address).cast::<libc::sockaddr>(),
                length,
            )
        };
        if rc == 0 {
            return Ok(std::os::unix::net::UnixStream::from(owned));
        }
        let e = Errno::last();
        // A signal arriving during a connect that has not yet completed. The
        // retry is bounded by the socket being non-blocking: it cannot sleep,
        // so this cannot spin on anything but signals.
        if e == Errno::EINTR {
            continue;
        }
        return Err(e);
    }
}

#[cfg(test)]
mod tests {
    // A test that cannot fail loudly is not a test.
    //
    // **Added 2026-08-24, and it should have been here on 2026-08-24.** The
    // capture tests below are the first in this module to `panic!`, and this
    // crate denies `panic` rather than warning on it (see `Cargo.toml`) — so
    // they were **twelve `cargo clippy --all-targets` errors** from the moment
    // they were written, and the change that wrote them was reported as
    // "clippy zero".
    //
    // The verification was the defect, not the code: the check that was run
    // grepped clippy's output for `^warning` and read no output as clean,
    // which cannot tell an error from a true negative. That is `AGENTS.md` §7
    // exactly — *"never conclude absence from a single probe, especially one
    // with stderr suppressed"* — arriving through a filter rather than through
    // a redirect. Found by a subagent running `--all-targets` for its own
    // reasons and reporting it as somebody else's problem.
    #![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn errno_zero_says_so_rather_than_inventing_a_message() {
        assert_eq!(Errno(0).to_string(), "error with no OS error number");
    }

    #[test]
    fn errno_display_carries_the_number_the_platform_used() {
        assert!(
            Errno::EPERM
                .to_string()
                .contains(&Errno::EPERM.0.to_string())
        );
    }

    #[test]
    fn only_a_clean_exit_counts_as_success_for_on_failure() {
        assert!(ExitStatus::Exited(0).is_success());
        assert!(!ExitStatus::Exited(1).is_success());
        assert!(!ExitStatus::Signalled(libc::SIGSEGV).is_success());
        assert!(!ExitStatus::Other.is_success());
    }

    #[test]
    fn exit_status_renders_what_happened_rather_than_a_number() {
        assert_eq!(ExitStatus::Exited(3).to_string(), "exit 3");
        assert_eq!(ExitStatus::Signalled(9).to_string(), "killed by signal 9");
        assert_eq!(ExitStatus::Other.to_string(), "stopped or continued");
    }

    #[test]
    fn the_two_sendable_signals_are_the_platform_numbers_they_name() {
        assert_eq!(Sig::Term.number(), libc::SIGTERM);
        assert_eq!(Sig::Kill.number(), libc::SIGKILL);
        assert_ne!(Sig::Term.number(), Sig::Kill.number());
    }


    // -----------------------------------------------------------------------
    // `connect_without_waiting` — the repair of 2026-09-18, and the invariant
    // `.rulings/` R-195 D5 states: NOTHING THIS SUPERVISOR SUPERVISES CAN MAKE
    // IT WAIT FOREVER.
    // -----------------------------------------------------------------------

    /// A listening unix socket that **never accepts**, with the smallest
    /// backlog the platform will give — which is what an application whose
    /// answering loop has stopped looks like from the supervisor's side.
    ///
    /// `listen(fd, 0)` does not mean *no* queue on Linux; it means a very
    /// small one. The test fills it by connecting until a connect refuses,
    /// rather than by assuming a depth.
    fn a_peer_that_never_accepts(path: &std::path::Path) -> std::os::fd::OwnedFd {
        use std::os::unix::ffi::OsStrExt as _;
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (slot, byte) in address.sun_path.iter_mut().zip(path.as_os_str().as_bytes()) {
            *slot = *byte as libc::c_char;
        }
        let fd = unsafe {
            libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0)
        };
        assert!(fd >= 0, "socket: {}", std::io::Error::last_os_error());
        let length = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
        let rc = unsafe {
            libc::bind(fd, std::ptr::addr_of!(address).cast::<libc::sockaddr>(), length)
        };
        assert_eq!(rc, 0, "bind: {}", std::io::Error::last_os_error());
        let rc = unsafe { libc::listen(fd, 0) };
        assert_eq!(rc, 0, "listen: {}", std::io::Error::last_os_error());
        unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) }
    }

    /// The test's own non-blocking connect, used to FILL a backlog.
    ///
    /// **Deliberately not [`connect_without_waiting`].** A harness that fills
    /// the queue with the function under test cannot report that function
    /// regressing — it hangs in the setup instead, which is exactly what
    /// happened when this test was first written and then checked against the
    /// reverted repair.
    fn a_connection_that_cannot_wait(path: &std::path::Path) -> Result<std::os::fd::OwnedFd, Errno> {
        use std::os::unix::ffi::OsStrExt as _;
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        address.sun_family = libc::AF_UNIX as libc::sa_family_t;
        for (slot, byte) in address.sun_path.iter_mut().zip(path.as_os_str().as_bytes()) {
            *slot = *byte as libc::c_char;
        }
        let fd = unsafe {
            libc::socket(
                libc::AF_UNIX,
                libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                0,
            )
        };
        assert!(fd >= 0, "socket: {}", std::io::Error::last_os_error());
        let owned = unsafe { <std::os::fd::OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(fd) };
        let length = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
        let rc = unsafe {
            libc::connect(fd, std::ptr::addr_of!(address).cast::<libc::sockaddr>(), length)
        };
        if rc == 0 { Ok(owned) } else { Err(Errno::last()) }
    }

    /// **The regression test for the incident of 2026-09-18.**
    ///
    /// An application that has stopped accepting fills its backlog, and the
    /// supervisor's next `connect` used to sleep in `unix_wait_for_peer`
    /// forever — taking journal ingest, health polling, service supervision and
    /// every control-plane answer down with it, on a loop with one thread.
    ///
    /// **This must FAIL rather than HANG if the repair is undone**, which is
    /// why the connect runs on its own thread and the assertion is on a
    /// `recv_timeout`. A test that hangs tells a reader nothing and blocks the
    /// suite; `std::thread` here is test scaffolding, and the crate's
    /// no-threads property is about `LibcKernel` itself.
    #[test]
    fn a_peer_that_is_not_accepting_is_refused_rather_than_waited_for() {
        let dir = std::env::temp_dir().join(format!("rivet-health-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let path = dir.join("full-backlog.sock");
        let _ = std::fs::remove_file(&path);
        let listener = a_peer_that_never_accepts(&path);

        // **Filled with the test's OWN non-blocking connect, never with the
        // function under test.** The first version of this test used
        // `connect_without_waiting` here, and when the repair was reverted to
        // check that this test catches it, the test HUNG in this loop instead
        // of failing at the assertion below — the regression it exists to
        // report was the thing that stopped it reporting. A harness must not
        // share the mechanism it is testing.
        //
        // Every connection is held open deliberately: dropping them would let
        // the queue drain and the test would prove nothing.
        let mut queued = Vec::new();
        let mut refused = None;
        for _ in 0..512 {
            match a_connection_that_cannot_wait(&path) {
                Ok(fd) => queued.push(fd),
                Err(e) => {
                    refused = Some(e);
                    break;
                }
            }
        }
        let refused = refused.expect(
            "the backlog never filled in 512 connections, so this test is not exercising \
             the case it exists for",
        );
        assert_eq!(
            refused,
            Errno(libc::EAGAIN),
            "a full backlog must refuse with EAGAIN; `crate::supervise`'s ask_health_where_due \
             reads any Err as `this service is not answering` and pushes the next ask out by a \
             whole interval, which is the behaviour ADR-045 wants"
        );

        // And now the property itself, on the clock: with the queue full, a
        // further connect must RETURN, not sleep.
        let (tell, heard) = std::sync::mpsc::channel();
        let again = path.clone();
        std::thread::spawn(move || {
            let _ = tell.send(connect_without_waiting(&again).map(|_| ()));
        });
        match heard.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(outcome) => assert!(
                outcome.is_err(),
                "the peer is not accepting and its backlog is full, so this cannot have succeeded"
            ),
            Err(_) => panic!(
                "connect_without_waiting DID NOT RETURN within 5 s against a peer that is not \
                 accepting. That is the 2026-09-18 incident exactly: rivet-01's supervisor was \
                 captured in wchan `unix_wait_for_peer`, syscall 203, and stayed there until it \
                 was power-cycled. Whatever change caused this test to reach this line has put \
                 an unbounded wait back on the supervisor's loop"
            ),
        }

        drop(queued);
        drop(listener);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    /// **`CLOEXEC`, which is the property hand-rolling the socket loses.**
    ///
    /// `UnixStream::connect` sets it; `libc::socket` does not unless asked. A
    /// supervisor descriptor without it is inherited by every application
    /// spawned while it is open — one application holding another's health
    /// channel — which is a worse defect than the wait this function removes.
    #[test]
    fn the_connected_descriptor_is_not_inherited_by_anything_the_supervisor_spawns() {
        use std::os::fd::AsRawFd as _;
        let dir = std::env::temp_dir().join(format!("rivet-health-cloexec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let path = dir.join("accepting.sock");
        let _ = std::fs::remove_file(&path);
        let listener = a_peer_that_never_accepts(&path);

        let stream = connect_without_waiting(&path).expect("the backlog has room for one");
        let flags = unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFD) };
        assert!(flags >= 0, "fcntl: {}", std::io::Error::last_os_error());
        assert_ne!(
            flags & libc::FD_CLOEXEC,
            0,
            "the health descriptor would survive an execve and reach the next application \
             this supervisor starts"
        );

        // And the other half of the creation flags, for the same reason: this
        // is the line the incident turned on.
        let status = unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFL) };
        assert!(status >= 0, "fcntl: {}", std::io::Error::last_os_error());
        assert_ne!(
            status & libc::O_NONBLOCK,
            0,
            "the socket must be non-blocking from its creation, not from a set_nonblocking \
             call after the connect — which is what made 2026-09-18 possible"
        );

        drop(stream);
        drop(listener);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    /// A `LibcKernel` whose answering sockets live in a scratch directory of
    /// their own, so these tests need no `/run` and cannot meet each other.
    fn a_kernel_asking_in(tag: &str) -> (LibcKernel, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("rivet-health-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        k.health_dir = dir.clone();
        (k, dir)
    }

    /// Accept, on a second descriptor for the service's answering socket,
    /// everything waiting in its queue, and say how many there were.
    ///
    /// **Non-blocking on the test's side**, so a queue that is empty ends the
    /// count rather than the test.
    fn connections_waiting_on(k: &LibcKernel, service: &str) -> usize {
        let held = k.listeners.get(service).expect("the service's listeners");
        let fd = held.fds.last().expect("the answering socket").try_clone().expect("a dup");
        let listener = std::os::unix::net::UnixListener::from(fd);
        listener.set_nonblocking(true).expect("non-blocking");
        let mut n = 0;
        loop {
            match listener.accept() {
                Ok(_) => n += 1,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return n,
                Err(e) => panic!("accept on the answering socket: {e}"),
            }
        }
    }

    /// **An application that never accepts holds ONE question in the kernel,
    /// however often it is asked** — the leak `bind_health_listener` bounds.
    ///
    /// Each ask is abandoned with `close_health`, which drops the supervisor's
    /// half exactly as the patience deadline does. Before the bound, every
    /// abandoned question stayed queued on the listener: this test counted
    /// eight, and a board counted one per interval per silent application,
    /// each an orphaned socket and a buffer of kernel memory nothing freed.
    #[test]
    fn an_application_that_never_accepts_holds_one_question_however_often_it_is_asked() {
        let (mut k, dir) = a_kernel_asking_in("silent");
        k.open_health("silent").expect("the answering socket binds");

        let mut refused = Vec::new();
        for _ in 0..8 {
            if let Err(e) = k.ask_health("silent") {
                refused.push(e);
            }
            k.close_health("silent");
        }

        assert_eq!(
            connections_waiting_on(&k, "silent"),
            1,
            "every abandoned question is still queued on the answering socket, so an \
             application that does not accept leaks one socket per interval"
        );
        assert_eq!(
            refused,
            vec![Errno(libc::EAGAIN); 7],
            "the first question waits and every later one is refused with EAGAIN — which \
             `ask_health_where_due` reports as not answering"
        );

        // And once the application does accept, the next question has room.
        k.ask_health("silent").expect("room again after the queue is emptied");
        assert_eq!(connections_waiting_on(&k, "silent"), 1);

        k.close_health("silent");
        k.close_listeners("silent");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **An application that accepts still gets every question, and the
    /// supervisor gets every answer** — the bound must cost an answering
    /// application nothing.
    #[test]
    fn an_application_that_accepts_answers_every_question_under_the_bound() {
        use std::io::{Read as _, Write as _};

        let (mut k, dir) = a_kernel_asking_in("answering");
        k.open_health("answering").expect("the answering socket binds");
        let fd = k.listeners["answering"]
            .fds
            .last()
            .expect("the answering socket")
            .try_clone()
            .expect("a dup, as the application is handed one");
        let application = std::os::unix::net::UnixListener::from(fd);
        const ROUNDS: usize = 4;
        let answering = std::thread::spawn(move || {
            for _ in 0..ROUNDS {
                let (mut stream, _) = application.accept().expect("a question");
                let mut asked = [0_u8; 64];
                let _ = stream.read(&mut asked);
                stream.write_all(b"HEALTH=healthy\n").expect("the answer goes out");
            }
        });

        for round in 0..ROUNDS {
            k.ask_health("answering")
                .unwrap_or_else(|e| panic!("round {round}: the question was refused: {e:?}"));
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let answered = loop {
                let mut out = Vec::new();
                k.drain_health(&mut out);
                if let Some(line) = out.iter().find_map(|e| match e {
                    Event::Answered { line, .. } => Some(line.clone()),
                    _ => None,
                }) {
                    break line;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "round {round}: no answer within 5 s"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            assert_eq!(answered, "HEALTH=healthy", "round {round}");
        }
        answering.join().expect("the application thread");

        k.close_health("answering");
        k.close_listeners("answering");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `kill(0, …)` hits the whole process group and `kill(-1, …)` hits
    /// everything signallable. As PID 1's child, either would be a very bad
    /// accident, so the adapter refuses before reaching the syscall.
    #[test]
    fn a_non_positive_pid_is_refused_before_it_reaches_kill() {
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.signal_process(0, Sig::Term), Err(Errno::EINVAL));
        assert_eq!(k.signal_process(-1, Sig::Kill), Err(Errno::EINVAL));
    }

    /// The kernel writes whole `signalfd_siginfo` structures and the reader
    /// walks the buffer in that stride. If the constant were wrong the walk
    /// would misread every signal after the first.
    #[test]
    fn the_siginfo_stride_is_the_platform_structure_size() {
        assert_eq!(SIGINFO_SIZE, std::mem::size_of::<libc::signalfd_siginfo>());
        // A compile-time check, so a platform where this stopped holding would
        // fail the build rather than misread every signal after the first.
        const { assert!(SIGINFO_SIZE >= 4) }
    }

    /// The one place a real filesystem is used. What the platform answers for
    /// an absent cgroup is the platform's answer, and `AGENTS.md` §5 says read
    /// it rather than reproduce it from documentation.
    #[test]
    fn an_absent_cgroup_is_unavailable_rather_than_empty() {
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        let observation = k.observe("a-service-that-does-not-exist-rivet-test");
        // Printed rather than only asserted, so `cargo test -- --nocapture`
        // shows what this platform actually answered.
        eprintln!("observing an absent cgroup answered: {observation:?}");
        assert!(
            matches!(observation, Observation::Unavailable(e) if e.0 != 0),
            "an absent cgroup must not read as {observation:?}"
        );
    }

    // -- ADR-021 §3's restore: the conversion, not the syscall ---------------

    /// The split that `clock_settime` is handed. Whole seconds down, the
    /// remainder in nanoseconds, and never a `tv_nsec` outside `0..1_000_000_000`
    /// — which the kernel rejects with `EINVAL`.
    #[test]
    fn epoch_milliseconds_split_into_whole_seconds_and_a_nanosecond_remainder() {
        assert_eq!(timespec_parts(0), Some((0, 0)));
        assert_eq!(timespec_parts(1), Some((0, 1_000_000)));
        assert_eq!(timespec_parts(999), Some((0, 999_000_000)));
        assert_eq!(timespec_parts(1_000), Some((1, 0)));
        assert_eq!(
            timespec_parts(1_700_000_000_123),
            Some((1_700_000_000, 123_000_000))
        );
        for ms in [
            0,
            1,
            999,
            1_000,
            1_001,
            1_700_000_000_123,
            u64::from(u32::MAX),
        ] {
            let Some((secs, nanos)) = timespec_parts(ms) else {
                unreachable!("{ms} is representable and was refused")
            };
            assert!(
                (0..1_000_000_000).contains(&nanos),
                "{ms} gave tv_nsec {nanos}"
            );
            assert!(secs >= 0);
        }
    }

    /// A value this platform cannot represent is **refused**, not truncated into
    /// some other year. `AGENTS.md` §5 — refuse rather than guess, and a silently
    /// wrapped clock is the most expensive possible guess.
    ///
    /// **Written against the platform's own limit rather than a value**, and the
    /// reason is that on every target Rivet builds for today the limit cannot
    /// be reached at all: `time_t` is 64-bit, and no `u64` count of milliseconds
    /// divided by a thousand overflows an `i64`. Asserting `None` for some
    /// literal would therefore be asserting something untrue here, and a test
    /// pinned to a 32-bit `time_t` would be one nobody runs (`AGENTS.md` §4 — a
    /// suite that cannot be run rots). The answer this platform gives is printed
    /// either way.
    #[test]
    fn a_time_this_platform_cannot_hold_is_refused_rather_than_wrapped() {
        let most_seconds = u64::MAX / 1000;
        let representable = <libc::time_t>::try_from(most_seconds).is_ok();
        eprintln!(
            "this platform's time_t is {} bytes and {} hold {most_seconds} seconds",
            std::mem::size_of::<libc::time_t>(),
            if representable { "can" } else { "cannot" }
        );
        assert_eq!(
            timespec_parts(u64::MAX).is_some(),
            representable,
            "the refusal must follow the platform's own limit rather than a guess"
        );
    }

    /// **The claim no fake can make**, and the one `PLAN.md` §53.2 rests on:
    /// the walk reaches every directory in a tree, sets the group on all of
    /// them, adds group-write and setgid to directories, and leaves file modes
    /// alone.
    ///
    /// **The group is this process's own gid**, not `rivet_core::etc::REPO_GID`
    /// — an unprivileged `chown` may only give a file to a group the caller is
    /// in, and a test that needed root would be one that never runs
    /// (`AGENTS.md` §4). What is being proved here is the *walk* and the
    /// *modes*; that `repo` is the right gid is `crate::repos`' claim, and that
    /// a real board can set it is the board's.
    /// **The guarded write, against a real filesystem — because no fake can
    /// make this claim.**
    ///
    /// `PLAN.md` §123.2 item 2. `crate::runtime` ~~renders an application's
    /// settings into `<volume>/app/<name>/data/settings`, and `data/` belongs to
    /// that application: it can delete the file and leave something else at the
    /// path.~~ — **the settings file moved out of `data/` on 2026-09-04**
    /// (`ADR-038` §8.1), so this no longer has a caller whose safety depends on
    /// it. It asserts, against the kernel and not against a model of it, that
    /// root does not write through what it finds — which is what any future
    /// caller below an application-owned directory will need, and is why the
    /// method and this test outlive the defect that prompted them.
    ///
    /// Every case is checked for the SAME two things: the write was refused,
    /// **and the thing it was pointed at is untouched.** The second is the one
    /// that matters — a refusal that had already truncated the target would be
    /// most of the damage done.
    #[test]
    fn the_guarded_write_refuses_a_symlink_a_hard_link_and_a_fifo() {
        use std::os::unix::fs::symlink;

        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-nofollow", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.mkdir_p(&root), Ok(()));

        // What an application would aim at: somebody else's file.
        let secret = format!("{root}/somebody-elses-file");
        assert_eq!(k.write_file(&secret, b"the original contents\n", 0o600), Ok(()));
        let untouched = || {
            assert_eq!(
                std::fs::read_to_string(&secret).expect("the target is still there"),
                "the original contents\n",
                "the target was written through, which is the whole defect"
            );
        };

        // 1. An ordinary file is written, so the guards are not simply refusing
        //    everything — without this the three below prove nothing.
        let ordinary = format!("{root}/settings");
        assert_eq!(k.write_file_nofollow(&ordinary, b"key = value\n", 0o644), Ok(()));
        assert_eq!(
            std::fs::read_to_string(&ordinary).expect("written"),
            "key = value\n"
        );
        // ...and a second write replaces rather than appends, which is what
        // `set_len(0)` after the checks has to still do.
        assert_eq!(k.write_file_nofollow(&ordinary, b"k = v\n", 0o644), Ok(()));
        assert_eq!(std::fs::read_to_string(&ordinary).expect("written"), "k = v\n");

        // 2. A SYMLINK to somebody else's file: ELOOP, and that file untouched.
        let planted = format!("{root}/planted-symlink");
        symlink(&secret, &planted).expect("a symlink");
        assert_eq!(
            k.write_file_nofollow(&planted, b"settings from root\n", 0o644),
            Err(Errno::ELOOP)
        );
        untouched();

        // 3. A HARD LINK to it: EMLINK. `O_NOFOLLOW` does not see this one —
        //    a hard link is not a symlink and the open succeeds — so this is
        //    the case the `nlink` check exists for and the case that would be
        //    missed by a fix that only added the flag.
        let hard = format!("{root}/planted-hardlink");
        std::fs::hard_link(&secret, &hard).expect("a hard link");
        assert_eq!(
            k.write_file_nofollow(&hard, b"settings from root\n", 0o644),
            Err(Errno::EMLINK)
        );
        untouched();

        // 4. A FIFO: refused, and — the part that is not about confidentiality
        //    — refused WITHOUT BLOCKING. This supervisor is single threaded, so
        //    an open that waited for a reader would freeze the machine, not
        //    just this write.
        let fifo = format!("{root}/planted-fifo\0");
        let made = unsafe { libc::mkfifo(fifo.as_ptr().cast(), 0o600) };
        assert_eq!(made, 0, "mkfifo: {}", std::io::Error::last_os_error());
        let fifo = fifo.trim_end_matches('\0').to_string();
        let answer = k.write_file_nofollow(&fifo, b"settings from root\n", 0o644);
        assert!(
            matches!(answer, Err(Errno::EINVAL) | Err(Errno(libc::ENXIO))),
            "a FIFO must be refused and must not block; got {answer:?}"
        );

        // 5. And the ordinary file still works after all of that, so nothing
        //    above left the adapter in a state that refuses everything.
        assert_eq!(k.write_file_nofollow(&ordinary, b"still fine\n", 0o644), Ok(()));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// **`is_symlink` against a real filesystem** — the primitive
    /// `crate::applications::deliver_staged` checks every path with before it
    /// is handed to `mkdir_mode`, `file_exists` or a write. `ADR-049`.
    ///
    /// The FakeKernel used elsewhere cannot demonstrate this: it records
    /// which paths a test marked as links rather than resolving one against a
    /// real inode, so the claim that `lstat` and not `stat` decides the
    /// answer needs a real directory, a real file and a real symlink to each.
    #[test]
    fn is_symlink_answers_by_lstat_and_never_follows() {
        use std::os::unix::fs::symlink;

        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-is-symlink", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.mkdir_p(&root), Ok(()));

        // Nothing there at all.
        let missing = format!("{root}/never-created");
        assert_eq!(k.is_symlink(&missing), Ok(false));

        // An ordinary directory and an ordinary file: real entries, neither a
        // link, which is what makes the two `Ok(true)` cases below meaningful
        // rather than a fake that answers `true` for everything.
        let real_dir = format!("{root}/a-real-directory");
        assert_eq!(k.mkdir_p(&real_dir), Ok(()));
        assert_eq!(k.is_symlink(&real_dir), Ok(false));

        let real_file = format!("{root}/a-real-file");
        assert_eq!(k.write_file(&real_file, b"not a link\n", 0o644), Ok(()));
        assert_eq!(k.is_symlink(&real_file), Ok(false));

        // A link to the directory: `Ok(true)` from `lstat` on the link
        // itself, even though `stat`-following `metadata`/`is_dir` would call
        // this a directory too — which is exactly the confusion this method
        // exists to avoid for its caller.
        let link_to_dir = format!("{root}/link-to-directory");
        symlink(&real_dir, &link_to_dir).expect("a symlink to the directory");
        assert_eq!(k.is_symlink(&link_to_dir), Ok(true));
        assert!(
            std::path::Path::new(&link_to_dir).is_dir(),
            "the link's target really is a directory, so a `stat`-following check would miss this"
        );

        // A link to the file, and a link to nothing at all: both symlinks,
        // whether or not their target exists.
        let link_to_file = format!("{root}/link-to-file");
        symlink(&real_file, &link_to_file).expect("a symlink to the file");
        assert_eq!(k.is_symlink(&link_to_file), Ok(true));

        let dangling = format!("{root}/link-to-nothing");
        symlink(format!("{root}/does-not-exist"), &dangling).expect("a dangling symlink");
        assert_eq!(k.is_symlink(&dangling), Ok(true));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// **The descriptor shuffle, against a real child process.**
    ///
    /// `PLAN.md` §123, Wave 2.5. The placement is `dup2(source, 3 + i)` in
    /// order. When a source sits at a number that is a LATER position's target,
    /// the old code overwrote it before it had been copied, and the child was
    /// handed the same socket twice — silently, and dependent on whatever
    /// numbers the parent's opens happened to get.
    ///
    /// This arranges exactly that collision and then asks the child what it
    /// actually received. Two pipes carry different bytes; the listener list is
    /// ordered so that the second entry's descriptor is the first entry's
    /// target. Under the old code the child reads the same byte twice.
    #[test]
    fn two_passed_sockets_stay_two_when_one_sits_on_the_other_s_target() {
        use std::io::Read as _;
        use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};

        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-shuffle", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.mkdir_p(&root), Ok(()));

        // Two pipes, each with one distinguishing byte already in it. Whatever
        // numbers the kernel hands out, `low` is the smaller pair because it is
        // made first — which is what puts a later entry's source on an earlier
        // entry's target below.
        let pipe = |byte: u8| -> (OwnedFd, OwnedFd) {
            let mut ends = [-1i32; 2];
            // SAFETY: two `c_int`s on this stack; `pipe2` writes exactly two.
            #[allow(unsafe_code)]
            let made = unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) };
            assert_eq!(made, 0, "pipe2: {}", std::io::Error::last_os_error());
            // SAFETY: fresh descriptors, owned only here.
            #[allow(unsafe_code)]
            let (r, w) = unsafe { (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1])) };
            let written = std::io::Write::write(&mut std::fs::File::from(w.try_clone().expect("dup")), &[byte]);
            assert_eq!(written.expect("a byte into a pipe"), 1);
            (r, w)
        };
        let (low_read, low_write) = pipe(b'A');
        let (high_read, high_write) = pipe(b'B');
        eprintln!(
            "pipe read ends: low={} high={}",
            low_read.as_raw_fd(),
            high_read.as_raw_fd()
        );

        // **The order is the test.** `high_read` goes first, so it is placed at
        // `3`; `low_read` goes second, and its own number is very likely to be
        // `3` — the target that was just written. That is the collision.
        k.listeners.insert(
            "shuffle".to_string(),
            Listeners { fds: vec![high_read, low_read], paths: Vec::new() },
        );

        let out = format!("{root}/got");
        let program = Program {
            path: "/bin/sh".to_string(),
            args: vec![
                "-c".to_string(),
                format!("head -c 1 <&3 > {out}.3; head -c 1 <&4 > {out}.4"),
            ],
            env: Vec::new(),
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        };
        let pid = k.spawn("shuffle", &program).expect("the child started");
        // The parent's copies of the write ends must go, or `head` waits.
        drop(low_write);
        drop(high_write);
        // SAFETY: `pid` is this process's own child, just spawned.
        #[allow(unsafe_code)]
        unsafe {
            libc::waitpid(pid, std::ptr::null_mut(), 0);
        }

        let read_one = |suffix: &str| -> String {
            let mut s = String::new();
            std::fs::File::open(format!("{out}.{suffix}"))
                .expect("the child wrote it")
                .read_to_string(&mut s)
                .expect("readable");
            s
        };
        let three = read_one("3");
        let four = read_one("4");
        eprintln!("the child read {three:?} on fd 3 and {four:?} on fd 4");
        assert_eq!(three, "B", "fd 3 must be the FIRST declared socket");
        assert_eq!(four, "A", "fd 4 must be the SECOND declared socket");
        assert_ne!(three, four, "the child was handed the same socket twice");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// **A started service gets an empty stdin of its own, and not the
    /// supervisor's.**
    ///
    /// `PLAN.md` §123, Wave 2.4. On a booted machine the supervisor's own
    /// descriptor 0 is `/dev/console` — an `O_RDWR` handle on the HDMI virtual
    /// terminal — and every application inherited it: undeclared, ungranted,
    /// and enough to write to the screen and read what somebody typed.
    ///
    /// The child is asked what its fd 0 actually is, because that is the only
    /// answer that settles it. A pipe with no writer is what it must be, and
    /// reading it must give nothing.
    #[test]
    fn a_child_is_handed_an_empty_stdin_and_not_the_supervisors() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-stdin", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.mkdir_p(&root), Ok(()));

        let mine = std::fs::read_link("/proc/self/fd/0")
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "(unreadable)".to_string());
        eprintln!("this process's own fd 0 is {mine}");

        let out = format!("{root}/fd0");
        let program = Program {
            path: "/bin/sh".to_string(),
            args: vec![
                "-c".to_string(),
                format!("readlink /proc/self/fd/0 > {out}.what; cat <&0 > {out}.read"),
            ],
            env: Vec::new(),
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        };
        let pid = k.spawn("stdin-probe", &program).expect("the child started");
        // SAFETY: `pid` is this process's own child, just spawned.
        #[allow(unsafe_code)]
        unsafe {
            libc::waitpid(pid, std::ptr::null_mut(), 0);
        }

        let what = std::fs::read_to_string(format!("{out}.what")).expect("the child answered");
        let read = std::fs::read_to_string(format!("{out}.read")).expect("the child answered");
        eprintln!("the child's fd 0 is {} and reading it gave {read:?}", what.trim());
        assert!(
            what.trim().starts_with("pipe:"),
            "the child's stdin is {}, not a pipe of its own — it inherited the supervisor's",
            what.trim()
        );
        assert!(read.is_empty(), "the child's stdin was not empty: {read:?}");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// **A started program gets the environment it was HANDED, and nothing
    /// else.**
    ///
    /// `ADR-015`'s posture is that an application is handed what it needs. Until
    /// 2026-09-04 [`Kernel::spawn`] handed it those things on top of every
    /// variable the supervisor was carrying — and sysd's own environment is
    /// initd's, which is the kernel's, which includes every unrecognised
    /// `key=value` on the kernel command line.
    ///
    /// **The child is asked what it received**, because the parent's opinion
    /// about `environ` is not evidence. A variable this test process really has
    /// is the witness; `RIVET_APP_DATA` is the handed one that must survive.
    #[test]
    fn a_child_gets_the_environment_it_was_handed_and_not_the_supervisor_s() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-env", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.mkdir_p(&root), Ok(()));

        // `env(1)` by absolute path, reached through `sh` only for the
        // redirection — so no part of this test depends on a `PATH` that the
        // change under test is removing.
        let Some(env_bin) = ["/usr/bin/env", "/bin/env"]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())
        else {
            eprintln!("no env(1) on this host: the child cannot be asked, so this proves nothing");
            return;
        };

        // **Named from the live environment rather than invented**, because
        // `std::env::set_var` is unsound in a process running tests on several
        // threads. The excluded names are the ones a POSIX shell sets or
        // defaults for itself, which would make an inherited value
        // indistinguishable from a manufactured one.
        let Some(witness) = std::env::vars().map(|(k, _)| k).find(|k| {
            !matches!(
                k.as_str(),
                "PATH" | "PWD" | "OLDPWD" | "IFS" | "PS1" | "PS2" | "PS4" | "OPTIND" | "SHLVL" | "_"
            )
        }) else {
            eprintln!("this process has no environment to leak, so this proves nothing");
            return;
        };
        eprintln!("the witness variable is {witness}");

        let out = format!("{root}/env");
        let handed = format!("{root}/data");
        let program = Program {
            path: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), format!("{env_bin} > {out}")],
            env: vec![(
                rivet_core::service::APP_DATA_ENV.to_string(),
                handed.clone(),
            )],
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        };
        let pid = k.spawn("env-probe", &program).expect("the child started");
        // SAFETY: `pid` is this process's own child, just spawned.
        #[allow(unsafe_code)]
        unsafe {
            libc::waitpid(pid, std::ptr::null_mut(), 0);
        }

        let saw = std::fs::read_to_string(&out).expect("the child answered");
        eprintln!("the child's environment was:\n{saw}");
        let names: Vec<&str> = saw
            .lines()
            .filter_map(|line| line.split('=').next())
            .collect();
        assert!(
            !names.contains(&witness.as_str()),
            "the child inherited {witness} from the supervisor:\n{saw}"
        );
        assert!(
            saw.lines()
                .any(|line| line == format!("{}={handed}", rivet_core::service::APP_DATA_ENV)),
            "the child did not get the one variable it was handed:\n{saw}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// **An application starts with no way back to root**, and this asks the
    /// kernel rather than the code.
    ///
    /// `PLAN.md` §123, Wave 2.6. `ADR-015` §6 records that this build has no
    /// namespaces and no seccomp, and that is unchanged — what is asserted here
    /// is the floor beneath it: `NoNewPrivs` is set, so a setuid file on an
    /// `exec`-mounted application volume (`ADR-031` §5) cannot hand privilege
    /// back, and the capability bounding set is empty, so no `execve` of a file
    /// with capabilities can raise any either.
    ///
    /// Read out of `/proc/self/status` in the child, because the child is the
    /// only process whose answer means anything.
    #[test]
    fn an_application_starts_with_no_new_privs_and_an_empty_capability_ceiling() {
        if unsafe_getuid() != 0 {
            eprintln!("not root: the identity drop cannot be exercised, so this proves nothing");
            return;
        }

        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-confine", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.mkdir_p(&root), Ok(()));
        // The child runs as somebody else and still has to write its answer.
        assert_eq!(k.chmod(&root, 0o777), Ok(()));

        let out = format!("{root}/status");
        let program = Program {
            path: "/bin/sh".to_string(),
            args: vec![
                "-c".to_string(),
                format!("grep -E '^(NoNewPrivs|CapBnd|Uid):' /proc/self/status > {out}"),
            ],
            env: Vec::new(),
            // `nobody` on every distribution this could run on, and it does not
            // have to exist in `/etc/passwd` for `setuid` to reach it.
            identity: Some(rivet_core::service::AppIdentity { uid: 65534, gid: 65534 }),
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        };
        let pid = k.spawn("confined", &program).expect("the child started");
        // SAFETY: `pid` is this process's own child, just spawned.
        #[allow(unsafe_code)]
        unsafe {
            libc::waitpid(pid, std::ptr::null_mut(), 0);
        }

        let status = std::fs::read_to_string(&out).expect("the child answered");
        eprintln!("the child reported:\n{status}");
        assert!(
            status.contains("NoNewPrivs:\t1"),
            "PR_SET_NO_NEW_PRIVS is not set on the child:\n{status}"
        );
        let bounding = status
            .lines()
            .find_map(|l| l.strip_prefix("CapBnd:"))
            .map(str::trim)
            .expect("CapBnd is in /proc/self/status");
        assert_eq!(
            u64::from_str_radix(bounding, 16).expect("a hexadecimal mask"),
            0,
            "the capability bounding set is {bounding}, not empty:\n{status}"
        );
        assert!(status.contains("65534"), "the child did not become the identity:\n{status}");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// **A spawned service does not inherit the supervisor's `oom_score_adj`**,
    /// read out of the child's own `/proc` — `ADR-017` §3.
    ///
    /// The test process raises its own value to `500`, which needs no
    /// privilege, stands in for a supervisor whose value is not `0`, and puts
    /// it back afterwards. A child spawned for an ordinary service must answer
    /// `0`. Hand-composed: the supervisor's real `-1000` needs
    /// `CAP_SYS_RESOURCE`, which the build container does not grant.
    #[test]
    fn a_spawned_service_is_given_its_own_oom_score_adj_rather_than_the_supervisors() {
        if std::fs::read_to_string("/proc/self/oom_score_adj").is_err() {
            eprintln!("no /proc: there is no oom_score_adj to inherit, so this proves nothing");
            return;
        }
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-oom", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.mkdir_p(&root), Ok(()));
        let out = format!("{root}/adj");
        let program = Program {
            path: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), format!("cat /proc/self/oom_score_adj > {out}")],
            env: Vec::new(),
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        };

        assert_eq!(k.set_own_oom_score_adj(500), Ok(()));
        let spawned = k.spawn("ordinary", &program);
        let _ = k.set_own_oom_score_adj(0);
        let pid = spawned.expect("the child started");
        // SAFETY: `pid` is this process's own child, just spawned.
        #[allow(unsafe_code)]
        unsafe {
            libc::waitpid(pid, std::ptr::null_mut(), 0);
        }

        let adj = std::fs::read_to_string(&out).expect("the child answered");
        assert_eq!(adj.trim(), "0", "the child inherited its parent's oom_score_adj");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The numbers are the kernel's: `CAP_SYS_TIME` is bit 25, which is
    /// `0000000002000000` in `/proc/<pid>/status`, and nothing past the two
    /// words of a version 3 set is accepted.
    #[test]
    fn a_capability_is_the_kernels_number_and_its_mask_is_the_status_files() {
        assert_eq!(Capability::SYS_TIME.number(), 25);
        assert_eq!(capability_mask(&[Capability::SYS_TIME]), 0x0000_0000_0200_0000);
        // `linux/capability.h`: 10, 12 and 13, which `/proc` prints as 3400.
        assert_eq!(
            capability_mask(&[
                Capability::NET_BIND_SERVICE,
                Capability::NET_ADMIN,
                Capability::NET_RAW
            ]),
            0x0000_0000_0000_3400
        );
        assert_eq!(capability_mask(&[]), 0);
        assert_eq!(Capability::from_number(63).map(Capability::bit), Some(1 << 63));
        assert_eq!(Capability::from_number(64), None);
    }

    /// The bits set in one `Cap*:` line of a `/proc/<pid>/status`.
    fn status_mask(status: &str, field: &str) -> u64 {
        let raw = status
            .lines()
            .find_map(|l| l.strip_prefix(field))
            .map(str::trim)
            .unwrap_or_else(|| panic!("{field} is not in:\n{status}"));
        u64::from_str_radix(raw, 16).expect("a hexadecimal mask")
    }

    /// **A program that stays uid 0 and names its capabilities holds exactly
    /// those, and its bounding set is reduced to them.** Read out of the
    /// child's own `/proc/self/status` after a second `execve`, because the
    /// kernel recomputes a root process's sets at every `execve` and the
    /// answer that matters is the one the program's own children inherit.
    ///
    /// The capabilities kept are the two lowest this test process holds, for
    /// the homed test's reason: the build container does not grant every
    /// capability, and the mechanism is the same for every number.
    #[test]
    fn a_root_program_keeps_uid_0_and_only_the_capabilities_it_names() {
        if unsafe_getuid() != 0 {
            eprintln!("not root: there is no root bounding set to reduce, so this proves nothing");
            return;
        }
        let mine = std::fs::read_to_string("/proc/self/status").expect("this process's status");
        let held = status_mask(&mine, "CapEff:");
        let kept: Vec<Capability> = (0..64u8)
            .filter(|n| held & (1 << n) != 0)
            .filter_map(Capability::from_number)
            .take(2)
            .collect();
        if kept.len() < 2 {
            eprintln!("this root holds fewer than two capabilities, so this proves nothing");
            return;
        }
        let want = capability_mask(&kept);
        if want == held {
            eprintln!("keeping these two drops nothing this process holds, so this proves nothing");
            return;
        }

        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-root-keep", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.mkdir_p(&root), Ok(()));

        let out = format!("{root}/status");
        let program = Program {
            path: "/bin/sh".to_string(),
            args: vec![
                "-c".to_string(),
                format!("grep -E '^(NoNewPrivs|Cap[A-Za-z]+|Uid):' /proc/self/status > {out}"),
            ],
            identity: None,
            capabilities: kept.clone(),
            ..Program::default()
        };
        let pid = k.spawn("root-keeps-two", &program).expect("the child started");
        // SAFETY: `pid` is this process's own child, just spawned.
        #[allow(unsafe_code)]
        unsafe {
            libc::waitpid(pid, std::ptr::null_mut(), 0);
        }

        let status = std::fs::read_to_string(&out).expect("the child answered");
        eprintln!("the child reported:\n{status}");
        for field in ["CapPrm:", "CapEff:", "CapBnd:"] {
            assert_eq!(
                status_mask(&status, field),
                want,
                "{field} is not exactly the capabilities named:\n{status}"
            );
        }
        assert!(status.contains("NoNewPrivs:\t1"), "NoNewPrivs is not set:\n{status}");
        let uid = status
            .lines()
            .find_map(|l| l.strip_prefix("Uid:"))
            .map(|l| l.split_whitespace().collect::<Vec<_>>())
            .expect("Uid is in /proc/self/status");
        assert!(uid.iter().all(|u| *u == "0"), "the child is not uid 0:\n{status}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **A ruleset without an identity is still refused**, capabilities or
    /// not: the reduced-root shape is uid 0 with a smaller bounding set and
    /// nothing else. Runs as any user: the refusal happens before the fork.
    #[test]
    fn a_root_program_naming_capabilities_and_a_ruleset_is_refused_before_the_fork() {
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        let program = Program {
            path: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), "exit 0".to_string()],
            identity: None,
            confinement: Some(rivet_core::confinement::Policy::default()),
            capabilities: vec![Capability::NET_ADMIN],
            ..Program::default()
        };
        assert_eq!(k.spawn("confined-root", &program), Err(Errno(libc::EINVAL)));
    }

    /// **A homed service keeps exactly the capability it names, and nothing
    /// else, across its change of uid** — permitted, effective, inheritable,
    /// ambient and bounding all equal to that one bit, `NoNewPrivs` still set.
    ///
    /// The capability kept is the lowest one this test process holds, rather
    /// than `CAP_SYS_TIME`, because the build container does not grant that
    /// one; the mechanism is the same for every number. The child is asked
    /// through `/bin/sh` and then `grep`, so the answer is read after a
    /// second `execve` — which is the one ambient capabilities exist to
    /// survive.
    #[test]
    fn a_homed_program_keeps_exactly_the_capability_it_names_across_the_uid_change() {
        if unsafe_getuid() != 0 {
            eprintln!("not root: the identity drop cannot be exercised, so this proves nothing");
            return;
        }
        let mine = std::fs::read_to_string("/proc/self/status").expect("this process's status");
        let held = status_mask(&mine, "CapEff:");
        let Some(kept) = (0..64u8)
            .find(|n| held & (1 << n) != 0)
            .and_then(Capability::from_number)
        else {
            eprintln!("this root holds no capability at all, so this proves nothing");
            return;
        };
        eprintln!("keeping capability {} of this process's {held:016x}", kept.number());

        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-keep", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.mkdir_p(&root), Ok(()));
        assert_eq!(k.chmod(&root, 0o777), Ok(()));

        let out = format!("{root}/status");
        let program = Program {
            path: "/bin/sh".to_string(),
            args: vec![
                "-c".to_string(),
                format!("grep -E '^(NoNewPrivs|Cap[A-Za-z]+|Uid):' /proc/self/status > {out}"),
            ],
            identity: Some(rivet_core::service::AppIdentity { uid: 65534, gid: 65534 }),
            capabilities: vec![kept],
            ..Program::default()
        };
        let pid = k.spawn("keeps-one", &program).expect("the child started");
        // SAFETY: `pid` is this process's own child, just spawned.
        #[allow(unsafe_code)]
        unsafe {
            libc::waitpid(pid, std::ptr::null_mut(), 0);
        }

        let status = std::fs::read_to_string(&out).expect("the child answered");
        eprintln!("the child reported:\n{status}");
        let want = kept.bit();
        for field in ["CapPrm:", "CapEff:", "CapInh:", "CapAmb:", "CapBnd:"] {
            assert_eq!(
                status_mask(&status, field),
                want,
                "{field} is not exactly capability {}:\n{status}",
                kept.number()
            );
        }
        assert!(status.contains("NoNewPrivs:\t1"), "NoNewPrivs is not set:\n{status}");
        assert!(status.contains("65534"), "the child did not become the identity:\n{status}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **A capability the supervisor does not hold cannot be kept, and the
    /// spawn fails rather than starting the program without it.** A service
    /// that started and then met `EPERM` at its first privileged call would
    /// report a running state the machine cannot honour.
    #[test]
    fn a_capability_the_supervisor_does_not_hold_fails_the_spawn() {
        if unsafe_getuid() != 0 {
            eprintln!("not root: the identity drop cannot be exercised, so this proves nothing");
            return;
        }
        let mine = std::fs::read_to_string("/proc/self/status").expect("this process's status");
        let held = status_mask(&mine, "CapPrm:");
        let bounding = status_mask(&mine, "CapBnd:");
        // A number the kernel knows (it is in the bounding set's range) and
        // this process does not hold.
        let last = (0..64u8).rev().find(|n| bounding & (1 << n) != 0).unwrap_or(40).max(40);
        let Some(missing) = (0..=last)
            .find(|n| held & (1 << n) == 0)
            .and_then(Capability::from_number)
        else {
            eprintln!("this process holds every capability, so this proves nothing");
            return;
        };
        eprintln!("asking to keep capability {}, which this process lacks", missing.number());

        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        let program = Program {
            path: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), "exit 0".to_string()],
            identity: Some(rivet_core::service::AppIdentity { uid: 65534, gid: 65534 }),
            capabilities: vec![missing],
            ..Program::default()
        };
        let spawned = k.spawn("keeps-what-it-lacks", &program);
        if let Ok(pid) = spawned {
            // SAFETY: `pid` is this process's own child, just spawned.
            #[allow(unsafe_code)]
            unsafe {
                libc::waitpid(pid, std::ptr::null_mut(), 0);
            }
        }
        assert_eq!(spawned, Err(Errno(libc::EPERM)));
    }

    /// `getuid`, for the one test that has to know whether it can prove
    /// anything. A test that quietly passed as an ordinary user would be
    /// asserting nothing about the privilege drop.
    fn unsafe_getuid() -> u32 {
        // SAFETY: `getuid` takes nothing, cannot fail, and touches no memory.
        #[allow(unsafe_code)]
        unsafe {
            libc::getuid()
        }
    }

    /// **A caller that connects to the kiosk socket and says nothing does not
    /// stop the machine, and one `drain_kiosk` call does not wait to find
    /// out.** `PLAN.md` §123, Wave 2.2. The socket is chowned to the
    /// launcher's uid and this supervisor is single threaded, so a `connect`
    /// with no write is reachable from any process running as the launcher.
    ///
    /// `drain_kiosk` is non-blocking now: the accepted connection is parked
    /// in [`LibcKernel::kiosk_incoming`] rather than read to end-of-file
    /// inside this call, so a single call on a silent caller returns at
    /// once — the assertion below is generous only because it is a ceiling,
    /// not a measurement of what this call actually costs. The connection's
    /// own deadline is a SEPARATE property, asserted on a real trickling
    /// peer by the test below this one.
    #[test]
    fn a_kiosk_caller_that_says_nothing_costs_one_slow_pass_and_not_the_machine() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-kiosk", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        let path = format!("{root}/kiosk.sock");
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.mkdir_p(&root), Ok(()));
        assert_eq!(k.open_kiosk(&path, None), Ok(()));

        // A caller that connects and then does nothing at all. Held open for
        // the length of the test, which is the whole point.
        let _silent = std::os::unix::net::UnixStream::connect(&path).expect("connected");

        let started = std::time::Instant::now();
        let mut events = Vec::new();
        k.drain_kiosk(&mut events);
        let waited = started.elapsed();
        eprintln!("draining a silent caller took {waited:?}");

        assert!(
            waited < KIOSK_PATIENCE * 4,
            "the supervisor waited {waited:?} on a caller that said nothing; the deadline is \
             {KIOSK_PATIENCE:?}"
        );
        assert!(
            events.is_empty(),
            "a caller that said nothing produced an event: {events:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// **S1, the regression test.** `drain_kiosk` used to arm
    /// `KIOSK_PATIENCE` as a PER-`read(2)` timeout, not a deadline on the
    /// whole request: `read_to_string` on a `Take` looped until EOF or the
    /// byte cap, re-arming the timeout on every byte that arrived. A peer
    /// trickling one byte just under it — this one, at one every 200 ms, well
    /// inside the old 500 ms window — could hold this single-threaded
    /// supervisor for the request cap times the timeout.
    ///
    /// **Drives a real socket with a real peer thread**, because the defect
    /// was in the actual `read(2)` sequence, which a fake kernel does not
    /// perform. The peer's own observation of being cut off — its next write
    /// failing — is what is timed, so the assertion is about what the
    /// supervisor actually did to the connection rather than about how this
    /// test happened to poll it.
    #[test]
    fn a_kiosk_caller_trickling_one_byte_at_a_time_is_dropped_at_the_deadline_not_its_whole_message()
     {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-kiosk-trickle", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        let path = format!("{root}/kiosk.sock");
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.mkdir_p(&root), Ok(()));
        assert_eq!(k.open_kiosk(&path, None), Ok(()));

        let (cut_off, was_cut_off) = std::sync::mpsc::channel();
        let trickle_path = path.clone();
        let trickle = std::thread::spawn(move || {
            use std::io::Write as _;
            let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&trickle_path) else {
                return;
            };
            let started = std::time::Instant::now();
            for _ in 0..rivet_core::kiosk::REQUEST_MAX_BYTES {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if stream.write_all(b"x").is_err() {
                    let _ = cut_off.send(started.elapsed());
                    return;
                }
            }
            // Every byte was accepted without the connection ever being
            // closed under it: the bound did not hold. Reported rather than
            // left to time the test out, so the failure names what happened.
            let _ = cut_off.send(started.elapsed());
        });

        // Stands in for `Kernel::wait`'s repeated wakeups; bounds this test's
        // own wall time regardless of what the code under test does.
        let deadline = std::time::Instant::now() + KIOSK_PATIENCE * 6;
        let mut events = Vec::new();
        let mut cut_off_at = None;
        while cut_off_at.is_none() && std::time::Instant::now() < deadline {
            k.drain_kiosk(&mut events);
            if let Ok(elapsed) = was_cut_off.try_recv() {
                cut_off_at = Some(elapsed);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        let elapsed = cut_off_at.unwrap_or_else(|| {
            panic!(
                "the trickling peer was never cut off within {:?}",
                KIOSK_PATIENCE * 6
            )
        });
        assert!(
            elapsed < KIOSK_PATIENCE * 2,
            "the connection was held for {elapsed:?}, not dropped at its deadline \
             ({KIOSK_PATIENCE:?}) — the whole-request bound did not hold"
        );
        assert!(
            elapsed >= KIOSK_PATIENCE,
            "the connection was dropped before its own deadline: {elapsed:?}"
        );
        assert!(
            events.is_empty(),
            "a trickling caller that was never a complete request produced an event: {events:?}"
        );

        let _ = trickle.join();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **The other half of S1's fix: an ordinary caller is still answered.**
    /// `drain_kiosk` is non-blocking now and spread across calls the way
    /// `Kernel::wait`'s poll loop drives it, so this drives it the same way —
    /// repeated calls standing in for repeated wakeups, bounded well under
    /// `KIOSK_PATIENCE` so a regression that stopped answering promptly
    /// fails this test rather than merely being slow.
    #[test]
    fn a_kiosk_caller_that_sends_a_complete_request_at_once_is_still_answered() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-kiosk-normal", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        let path = format!("{root}/kiosk.sock");
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.mkdir_p(&root), Ok(()));
        assert_eq!(k.open_kiosk(&path, None), Ok(()));

        let caller_path = path.clone();
        let caller = std::thread::spawn(move || {
            use std::io::Write as _;
            let mut stream =
                std::os::unix::net::UnixStream::connect(&caller_path).expect("connected");
            stream
                .write_all(rivet_core::kiosk::Request::Return.render().as_bytes())
                .expect("wrote the whole request");
            // Closing both halves on drop is this protocol's frame end.
        });

        let deadline = std::time::Instant::now() + KIOSK_PATIENCE;
        let mut events = Vec::new();
        while events.is_empty() && std::time::Instant::now() < deadline {
            k.drain_kiosk(&mut events);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert_eq!(
            events,
            vec![Event::KioskAsked {
                request: rivet_core::kiosk::Request::Return
            }],
            "an ordinary request was not answered within {:?}",
            KIOSK_PATIENCE
        );

        let _ = caller.join();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **S1's other socket.** `drain_query` had the identical shape and the
    /// identical defect: `KIOSK_PATIENCE` bounded each `read(2)`, not the
    /// request. This is
    /// `a_kiosk_caller_trickling_one_byte_at_a_time_is_dropped_at_the_deadline_not_its_whole_message`
    /// on the query socket.
    #[test]
    fn a_query_caller_trickling_one_byte_at_a_time_is_dropped_at_the_deadline_not_its_whole_message()
     {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-query-trickle", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        let path = format!("{root}/query.sock");
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.mkdir_p(&root), Ok(()));
        assert_eq!(k.open_query(&path), Ok(()));

        let (cut_off, was_cut_off) = std::sync::mpsc::channel();
        let trickle_path = path.clone();
        let trickle = std::thread::spawn(move || {
            use std::io::Write as _;
            let Ok(mut stream) = std::os::unix::net::UnixStream::connect(&trickle_path) else {
                return;
            };
            let started = std::time::Instant::now();
            for _ in 0..rivet_core::operation::status::REQUEST_MAX_BYTES {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if stream.write_all(b"x").is_err() {
                    let _ = cut_off.send(started.elapsed());
                    return;
                }
            }
            // Every byte was accepted without the connection ever being
            // closed under it: the bound did not hold. Reported rather than
            // left to time the test out, so the failure names what happened.
            let _ = cut_off.send(started.elapsed());
        });

        let deadline = std::time::Instant::now() + KIOSK_PATIENCE * 6;
        let mut events = Vec::new();
        let mut cut_off_at = None;
        while cut_off_at.is_none() && std::time::Instant::now() < deadline {
            k.drain_query(&mut events);
            if let Ok(elapsed) = was_cut_off.try_recv() {
                cut_off_at = Some(elapsed);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        let elapsed = cut_off_at.unwrap_or_else(|| {
            panic!(
                "the trickling peer was never cut off within {:?}",
                KIOSK_PATIENCE * 6
            )
        });
        assert!(
            elapsed < KIOSK_PATIENCE * 2,
            "the connection was held for {elapsed:?}, not dropped at its deadline \
             ({KIOSK_PATIENCE:?}) — the whole-request bound did not hold"
        );
        assert!(
            elapsed >= KIOSK_PATIENCE,
            "the connection was dropped before its own deadline: {elapsed:?}"
        );
        assert!(
            events.is_empty(),
            "a trickling caller that was never a complete request produced an event: {events:?}"
        );

        let _ = trickle.join();
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The other half, on the query socket: an ordinary caller is still
    /// answered, promptly — [`Self::a_kiosk_caller_that_sends_a_complete_request_at_once_is_still_answered`]'s
    /// reason, mirrored.
    #[test]
    fn a_query_caller_that_sends_a_complete_request_at_once_is_still_answered() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-query-normal", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        let path = format!("{root}/query.sock");
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.mkdir_p(&root), Ok(()));
        assert_eq!(k.open_query(&path), Ok(()));

        let caller_path = path.clone();
        let caller = std::thread::spawn(move || {
            use std::io::Write as _;
            let mut stream =
                std::os::unix::net::UnixStream::connect(&caller_path).expect("connected");
            stream
                .write_all(rivet_core::operation::service_restart::Request::GetStatus.render().as_bytes())
                .expect("wrote the whole request");
        });

        let deadline = std::time::Instant::now() + KIOSK_PATIENCE;
        let mut events = Vec::new();
        while events.is_empty() && std::time::Instant::now() < deadline {
            k.drain_query(&mut events);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert_eq!(
            events,
            vec![Event::Queried {
                request: rivet_core::operation::service_restart::Request::GetStatus
            }],
            "an ordinary request was not answered within {:?}",
            KIOSK_PATIENCE
        );

        let _ = caller.join();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn share_tree_reaches_every_directory_and_leaves_files_alone() {
        use std::os::unix::fs::MetadataExt as _;
        use std::os::unix::fs::PermissionsExt as _;

        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-share", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        let nested = format!("{root}/objects/pack");
        let file = format!("{root}/objects/loose");

        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.mkdir_p(&nested), Ok(()));
        assert_eq!(k.write_file(&file, b"an object\n", 0o444), Ok(()));
        // The arrangement a repository created before the ruling is in.
        for d in [&root, &format!("{root}/objects"), &nested] {
            assert_eq!(k.chmod(d, 0o755), Ok(()));
        }

        let gid = std::fs::metadata(&root).expect("the root is there").gid();
        assert_eq!(k.share_tree(&root, gid), Ok(()));

        for d in [&root, &format!("{root}/objects"), &nested] {
            let m = std::fs::metadata(d).expect("a directory the walk made");
            assert_eq!(m.gid(), gid, "{d} kept the wrong group");
            assert_eq!(
                m.permissions().mode() & 0o7777,
                0o2775,
                "{d} is not group-writable and setgid"
            );
        }

        // **Deliberately untouched.** A push creates entries in directories; it
        // never rewrites a loose object, and every file that does not become
        // group-writable is one an account cannot corrupt.
        let m = std::fs::metadata(&file).expect("the file is there");
        assert_eq!(m.permissions().mode() & 0o7777, 0o444, "the file mode moved");
        assert_eq!(m.gid(), gid);

        // Idempotent: a second pass is what every boot after the first one is.
        assert_eq!(k.share_tree(&root, gid), Ok(()));
        assert_eq!(
            std::fs::metadata(&nested).expect("still there").permissions().mode() & 0o7777,
            0o2775
        );

        assert!(std::fs::remove_dir_all(&root).is_ok());
    }

    /// A path that is not there is `ENOENT` and not a panic — `crate::repos`
    /// reads that answer as "the `create` already failed and said so".
    #[test]
    fn share_tree_on_a_missing_path_is_enoent() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-share-absent", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.share_tree(&root, 0), Err(Errno::ENOENT));
    }

    /// **The crash-safe write's four calls, against a real filesystem.**
    ///
    /// `crate::clock`'s own tests drive every failure path through
    /// [`crate::testkernel::FakeKernel`], which proves the *decisions*. It
    /// cannot prove that `sync_dir` works at all — and that one is the trap:
    /// `fsync` on a directory needs a **read-only** descriptor, because opening
    /// a directory for writing fails with `EISDIR` on Linux. A `sync_dir` that
    /// always returned an error would leave every clock write reported as
    /// `NotDurable` on a perfectly healthy board, and no fake would notice.
    ///
    /// Runs wherever the suite runs, so it is checked on the development host,
    /// under WSL, and on the target (`AGENTS.md` §4 — a suite that cannot be run
    /// rots). A path is an argument, never a constant (`AGENTS.md` §8.3), and
    /// the pid keeps two concurrent runs apart.
    #[test]
    fn the_real_adapter_writes_syncs_renames_and_flushes_a_directory() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-atomic", std::process::id()));
        let dir = dir.to_string_lossy().into_owned();
        let temp = format!("{dir}/clock.new");
        let target = format!("{dir}/clock");

        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.mkdir_p(&dir), Ok(()));
        assert_eq!(
            k.write_file(&temp, b"1700000000000 unsynchronised\n", 0o644),
            Ok(())
        );
        assert_eq!(k.sync_file(&temp), Ok(()));
        assert_eq!(k.rename(&temp, &target), Ok(()));
        // The one that cannot be proven by a fake.
        assert_eq!(k.sync_dir(&dir), Ok(()));
        assert_eq!(
            k.read_file(&target),
            Ok("1700000000000 unsynchronised\n".to_string())
        );
        // The rename moved it: nothing is left at the temp path.
        assert_eq!(k.read_file(&temp), Err(Errno::ENOENT));
        assert_eq!(k.remove_file(&target), Ok(()));
        assert_eq!(k.remove_file(&target), Err(Errno::ENOENT));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Both paths this module composes must lie under the mount point PID 1
    /// actually creates.
    ///
    /// They are written here as literals rather than derived, because a `const`
    /// cannot `format!`. This is what makes the duplication a *checked* one:
    /// a `CGROUP2_MOUNT` that moved and left these behind would give a
    /// supervisor that reported UNREACHABLE for ever on a perfectly healthy
    /// machine, which is the precise failure `ADR-017` §3.1.3.1 exists to stop.
    /// `cmd/rivet-initd` pins its own copy of the controllers path the same
    /// way.
    #[test]
    fn the_cgroup_paths_are_under_the_mount_point_pid_1_creates() {
        use rivet_core::mount::CGROUP2_MOUNT;
        assert_eq!(
            CGROUP_CONTROLLERS,
            format!("{CGROUP2_MOUNT}/cgroup.controllers")
        );
        assert_eq!(CGROUP_ROOT, format!("{CGROUP2_MOUNT}/rivet"));
    }

    /// The discrimination, against a real filesystem rather than a fake.
    ///
    /// On a machine with cgroup2 mounted — which every Linux host running this
    /// suite is, and which `ADR-024` §8.1 confirmed the board is — the two
    /// probes must answer `Present` and `ENOENT`. **That pair is the benign
    /// case**, and it is exactly what a Raspberry Pi 4 produced on 2026-08-23
    /// while the supervisor was calling it a double-start hazard.
    ///
    /// Guarded rather than unconditional, because a host with no cgroup2 would
    /// otherwise fail a test about something else entirely — and the answer is
    /// printed either way, so a skipped assertion is visible rather than
    /// silent (`AGENTS.md` §7).
    #[test]
    fn on_a_machine_with_cgroup2_an_absent_service_cgroup_reads_as_the_benign_case() {
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        let hierarchy = k.cgroup_hierarchy();
        let observation = k.observe("a-service-that-does-not-exist-rivet-test");
        eprintln!("this host answered hierarchy={hierarchy:?} observation={observation:?}");
        if hierarchy == Hierarchy::Present {
            assert_eq!(
                observation,
                Observation::Unavailable(Errno::ENOENT),
                "a readable hierarchy with an absent service cgroup must be ENOENT, or the \
                 supervisor's benign branch is unreachable on this platform"
            );
        }
    }

    // -- the capture's line splitting — ADR-022 §4 ---------------------------
    //
    // `Capture::absorb` and `Capture::emit` are what turn a mechanism's bytes
    // into records, and they are the half that has to be right whatever the
    // mechanism does. Driven directly rather than through a pipe, because the
    // cases that matter are ones a real mechanism produces rarely and a test
    // must produce on demand: a write cut in the middle of a line, a line
    // longer than the cap, and bytes that are not UTF-8.

    /// A `Capture` with no descriptor, for the byte-level half.
    ///
    /// `/dev/null` rather than a fake, because `Capture` owns a `File` and the
    /// tests below never read from it — every one of them calls `absorb`
    /// directly, which is the function under test.
    fn splitter() -> Capture {
        Capture {
            pid: 1,
            service: "rivet-netd".to_string(),
            stream: Stream::Stderr,
            file: std::fs::File::open("/dev/null").unwrap_or_else(|e| {
                panic!(
                    "this host has no /dev/null, which is not a thing this test can work round: {e}"
                )
            }),
            partial: Vec::new(),
            overflowed: false,
        }
    }

    fn lines(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Output { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn each_line_becomes_one_event_in_the_services_name() {
        let mut c = splitter();
        let mut out = Vec::new();
        c.absorb(b"first\nsecond\n", &mut out);
        assert_eq!(lines(&out), ["first", "second"]);
        assert!(matches!(
            out.first(),
            Some(Event::Output { service, stream, truncated: false, .. })
                if service == "rivet-netd" && *stream == Stream::Stderr
        ));
    }

    /// **A write is not a line.** A pipe delivers whatever the writer flushed,
    /// which for a C daemon using `stdio` is a buffer boundary rather than a
    /// newline. A capture that emitted per read would split one of a
    /// mechanism's messages into two records at an arbitrary character.
    #[test]
    fn a_line_split_across_two_reads_is_one_record() {
        let mut c = splitter();
        let mut out = Vec::new();
        c.absorb(b"Exit before auth: ", &mut out);
        assert!(out.is_empty(), "half a line became a record: {out:?}");
        c.absorb(b"Disconnect received\n", &mut out);
        assert_eq!(lines(&out), ["Exit before auth: Disconnect received"]);
    }

    /// And the other direction: a partial line is **not lost** when the process
    /// exits without a final newline. This is the last-words case — a service
    /// that dies mid-sentence still said it.
    #[test]
    fn a_trailing_partial_line_is_flushed_at_eof() {
        let mut c = splitter();
        let mut out = Vec::new();
        c.absorb(b"no newline after this", &mut out);
        assert!(out.is_empty());
        c.emit(&mut out); // what `drain(to_eof: true)` does
        assert_eq!(lines(&out), ["no newline after this"]);
    }

    #[test]
    fn a_carriage_return_before_a_newline_is_not_a_line_of_its_own() {
        let mut c = splitter();
        let mut out = Vec::new();
        c.absorb(b"crlf line\r\nnext\r\n", &mut out);
        assert_eq!(lines(&out), ["crlf line", "next"]);
    }

    /// A blank line is not a record. A mechanism that separates its output with
    /// them would otherwise fill `/state/log` with empty messages, which is
    /// `ADR-022` §8's cap being spent on nothing.
    #[test]
    fn a_blank_line_produces_no_record() {
        let mut c = splitter();
        let mut out = Vec::new();
        c.absorb(b"\n\n\nsomething\n\n", &mut out);
        assert_eq!(lines(&out), ["something"]);
    }

    /// **The cap, and it says so.** A service writing without a newline must
    /// not be able to grow this buffer without bound — same hazard as
    /// `ADR-017` §3.5's restart loop, arriving through a pipe.
    #[test]
    fn a_line_longer_than_the_cap_is_cut_and_the_record_says_it_was() {
        let mut c = splitter();
        let mut out = Vec::new();
        let long = vec![b'x'; MAX_CAPTURED_LINE * 2];
        c.absorb(&long, &mut out);
        c.absorb(b"\n", &mut out);
        match out.first() {
            Some(Event::Output {
                text, truncated, ..
            }) => {
                assert_eq!(text.len(), MAX_CAPTURED_LINE);
                assert!(*truncated, "a cut line did not say it was cut");
            }
            other => panic!("{other:?}"),
        }
        // And the buffer is empty again, so the *next* line is not also flagged.
        c.absorb(b"short\n", &mut out);
        assert!(
            matches!(
                out.get(1),
                Some(Event::Output {
                    truncated: false,
                    ..
                })
            ),
            "{out:?}"
        );
    }

    /// A mechanism's output is bytes. A record that could not be written
    /// because a C daemon emitted a stray byte would be the journal failing at
    /// exactly its job, so the bytes are replaced and the record is written.
    #[test]
    fn bytes_that_are_not_utf8_become_a_record_rather_than_stopping_one() {
        let mut c = splitter();
        let mut out = Vec::new();
        c.absorb(b"before \xff\xfe after\n", &mut out);
        let got = lines(&out);
        assert_eq!(got.len(), 1);
        assert!(got[0].starts_with("before "), "{got:?}");
        assert!(got[0].ends_with(" after"), "{got:?}");
        assert!(got[0].contains('\u{fffd}'), "{got:?}");
    }

    /// **End to end through the real kernel**: a spawned process's output
    /// arrives as events, in order, from both streams.
    ///
    /// `/bin/sh` rather than a Rivet binary, because what is under test is
    /// this adapter's pipes and not anything Rivet wrote — and because the
    /// interesting property is that a program that knows nothing about the
    /// journal has its output captured anyway, which is `ADR-017` §7's whole
    /// claim.
    #[test]
    fn a_spawned_program_has_both_its_streams_captured() {
        if !std::path::Path::new("/bin/sh").exists() {
            eprintln!("no /bin/sh on this host; the end-to-end capture test is blind");
            return;
        }
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        let program = Program {
            path: "/bin/sh".to_string(),
            args: vec![
                "-c".to_string(),
                "echo to-stdout; echo to-stderr >&2".to_string(),
            ],
            env: Vec::new(),
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        };
        let pid = match k.spawn("test-service", &program) {
            Ok(pid) => pid,
            Err(e) => panic!("could not spawn /bin/sh: {e}"),
        };

        // Reap first, which drains to EOF — the path a short-lived service
        // actually takes, and the one that would lose the output if the pipes
        // were closed with the `Child`.
        let mut waited = 0;
        while k.reap_ready().is_empty() && waited < 200 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            waited += 1;
        }
        let events = k.wait(Some(0)).unwrap_or_default();
        let mut got: Vec<(String, String)> = events
            .iter()
            .filter_map(|e| match e {
                Event::Output {
                    service,
                    stream,
                    text,
                    ..
                } => Some((format!("{stream}"), format!("{service}:{text}"))),
                _ => None,
            })
            .collect();
        got.sort();
        assert_eq!(
            got,
            [
                ("stderr".to_string(), "test-service:to-stderr".to_string()),
                ("stdout".to_string(), "test-service:to-stdout".to_string()),
            ],
            "the spawned program's output did not arrive as events (pid {pid})"
        );
    }

    /// **The privilege drop against a real kernel**, both directions, and which
    /// one this run took depends on who is running it.
    ///
    /// `ADR-015` §5.1's identity is the one part of this change that a fake
    /// cannot settle: `FakeKernel::spawn` records a `Program` and never forks,
    /// so it can show that the field was *set* and nothing about whether
    /// `setuid` took. This forks for real and asks the child.
    ///
    /// * **As root** it asserts the whole thing: the child prints the uid and
    ///   gid the kernel gave it, and they are the ones asked for. That is the
    ///   only assertion here that proves the mechanism.
    /// * **As anybody else** it asserts the failure direction, which is the one
    ///   that matters more: an unprivileged supervisor **cannot** drop
    ///   privilege, and the spawn must **fail** rather than quietly starting the
    ///   program as whoever the supervisor already was. A `setuid` whose return
    ///   value went unchecked would produce exactly that, and this is the test
    ///   that fails when somebody stops checking it.
    ///
    /// Neither branch is a skip, so the test says something wherever it runs
    /// (`AGENTS.md` §7). The one thing it cannot do on a developer's machine is
    /// the first branch, and `PLAN.md` §0.11 says what closed that instead.
    #[test]
    fn a_spawn_with_an_identity_either_becomes_it_or_refuses_to_start() {
        if !std::path::Path::new("/bin/sh").exists() {
            eprintln!("no /bin/sh on this host; the privilege-drop test is blind");
            return;
        }
        // A uid nothing on a developer's machine is likely to be, and one this
        // scheme could really produce — inside `APP_UID_BASE..=APP_UID_LAST`.
        let (uid, gid) = (54_321_u32, 54_321_u32);
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        let program = Program {
            path: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), "id -u; id -g".to_string()],
            env: Vec::new(),
            identity: Some(rivet_core::service::AppIdentity { uid, gid }),
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        };

        // SAFETY: `getuid` reads and does nothing else.
        let running_as_root = unsafe { libc::getuid() } == 0;
        let spawned = k.spawn("identity-test", &program);

        if !running_as_root {
            assert_eq!(
                spawned,
                Err(Errno::EPERM),
                "an unprivileged supervisor started a program it could NOT drop privilege for, \
                 which means it ran as this test's own uid with a sandbox nobody applied"
            );
            return;
        }

        let pid = match spawned {
            Ok(pid) => pid,
            Err(e) => panic!("root could not drop privilege: {e}"),
        };
        let mut waited = 0;
        while k.reap_ready().is_empty() && waited < 200 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            waited += 1;
        }
        let events = k.wait(Some(0)).unwrap_or_default();
        let said: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                Event::Output {
                    stream: Stream::Stdout,
                    text,
                    ..
                } => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            said,
            [uid.to_string(), gid.to_string()],
            "the child's own account of who it is disagrees with what was asked (pid {pid})"
        );
    }

    /// **`/dev/kmsg` end to end, against the build host's own kernel.**
    ///
    /// `ADR-026` §4's table says what the emulator cannot settle, and this is a
    /// case it does not cover: `tools/qemu/run.sh` boots an initramfs and the
    /// kernel does not auto-mount devtmpfs onto one, so there is **no
    /// `/dev/kmsg` under QEMU at all**. The only Linux this path can be
    /// exercised on before a board is the one the tests run on.
    ///
    /// So it is exercised here, and the guard is a skip rather than a failure:
    /// a host with no `/dev/kmsg`, or one whose kernel refuses a reader, is not
    /// this test's finding to report. It prints what it did either way, so a
    /// skipped assertion is visible rather than silent (`AGENTS.md` §7).
    ///
    /// What it proves: the descriptor opens, `poll` and a non-blocking read
    /// deliver **whole records**, and `crate::kmsg` parses what a real kernel
    /// hands over — which is the half a fixture cannot cover, because a fixture
    /// is bytes somebody already framed into records.
    #[test]
    fn on_a_host_with_dev_kmsg_the_records_arrive_whole_and_parse() {
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        let opened = k.open_kmsg();
        if let Err(e) = opened {
            eprintln!(
                "this host refused {}: {e}; the kmsg path is blind here",
                crate::kmsg::KMSG_PATH
            );
            return;
        }
        // Idempotent: `run` calls it once, and reopening would restart at the
        // oldest surviving record and journal the whole boot a second time.
        assert!(k.open_kmsg().is_ok());

        let events = k.wait(Some(0)).unwrap_or_default();
        let records: Vec<&String> = events
            .iter()
            .filter_map(|e| match e {
                Event::KernelMessage { record } => Some(record),
                _ => None,
            })
            .collect();
        eprintln!("this host delivered {} kernel records", records.len());
        if records.is_empty() {
            eprintln!("…none of them, so only the open was exercised");
            return;
        }
        for record in &records {
            let parsed = crate::kmsg::parse(record);
            assert!(
                parsed.is_ok(),
                "a real kernel wrote {record:?} and crate::kmsg refused it: {parsed:?}"
            );
        }
        // Whole records, not fragments: every one has its `;` and a header
        // before it, which is what `read(2)` on this descriptor promises and
        // what a buffered reader would break.
        assert!(records.iter().all(|r| r.contains(';')));
    }

    /// `Kernel::wait` hands over what a reap drained **before** it sleeps.
    /// Otherwise a failed service's explanation waits for the next timeout,
    /// which for a supervisor with nothing else pending is for ever.
    #[test]
    fn a_dead_childs_last_words_are_returned_without_polling() {
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        k.pending.push_back(Event::Output {
            service: "s".to_string(),
            stream: Stream::Stderr,
            text: "last words".to_string(),
            truncated: false,
        });
        // A timeout that would block for ever if this slept.
        let events = k.wait(None).unwrap_or_default();
        assert_eq!(lines(&events), ["last words"]);
        assert!(k.pending.is_empty(), "the queue was not drained");
    }

    // -- the journal ingest socket (`ADR-022` §11) ---------------------------

    /// An [`Incoming`] over a descriptor nothing writes to, so the buffering
    /// rule can be driven directly — [`splitter`]'s arrangement, one socket
    /// over.
    fn receiver() -> Incoming {
        let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap_or_else(|e| {
            panic!("this host cannot make a socketpair, which nothing here can work round: {e}")
        });
        Incoming {
            stream: a,
            peer: rivet_core::journal::Peer {
                uid: 1000,
                gid: 1000,
                pid: 4242,
            },
            accepted_ms: 0,
            partial: Vec::new(),
        }
    }

    fn ingested(events: &[Event]) -> Vec<Ingest> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Ingest { what, .. } => Some(what.clone()),
                _ => None,
            })
            .collect()
    }

    /// One line, one event — the ordinary case, and JSONL over a stream is the
    /// file format unchanged.
    #[test]
    fn each_ingested_line_becomes_one_event() {
        let mut r = receiver();
        let mut out = Vec::new();
        assert!(r.feed(b"{\"a\":1}\n{\"b\":2}\n", &mut out));
        assert_eq!(
            ingested(&out),
            [
                Ingest::Line("{\"a\":1}".to_string()),
                Ingest::Line("{\"b\":2}".to_string())
            ]
        );
    }

    /// **A write is not a line**, here for the same reason it is not one on a
    /// pipe: a sender may split its record across `write(2)`s, and an event per
    /// read would cut a record in half at an arbitrary byte.
    #[test]
    fn a_record_split_across_two_writes_is_one_ingest_event() {
        let mut r = receiver();
        let mut out = Vec::new();
        assert!(r.feed(b"{\"sev\":\"in", &mut out));
        assert!(out.is_empty(), "half a record became an event: {out:?}");
        assert!(r.feed(b"fo\"}\n", &mut out));
        assert_eq!(ingested(&out), [Ingest::Line("{\"sev\":\"info\"}".to_string())]);
    }

    /// EOF terminates a line exactly as a newline does — a sender that closes
    /// without one still said what it said.
    #[test]
    fn a_line_with_no_trailing_newline_is_delivered_at_eof() {
        let mut r = receiver();
        let mut out = Vec::new();
        assert!(r.feed(b"no newline after this", &mut out));
        assert!(out.is_empty());
        r.at_eof(&mut out);
        assert_eq!(ingested(&out), [Ingest::Line("no newline after this".to_string())]);
    }

    /// **An enormous record is refused rather than cut.** A prefix of a record
    /// is not a record, and the connection is closed rather than being allowed
    /// to keep spending memory — `feed` says so by returning `false`.
    #[test]
    fn a_line_past_the_cap_closes_the_connection_and_reports_what_was_dropped() {
        let mut r = receiver();
        let mut out = Vec::new();
        let huge = vec![b'x'; rivet_core::journal::INGEST_MAX_BYTES + 1];
        assert!(
            !r.feed(&huge, &mut out),
            "an oversized line left the connection open"
        );
        assert_eq!(
            ingested(&out),
            [Ingest::TooLarge {
                held: rivet_core::journal::INGEST_MAX_BYTES
            }]
        );
        assert!(r.partial.is_empty(), "the bytes were kept");
    }

    /// **A sender that connects and never finishes** costs one descriptor and
    /// one deadline. Complete lines it already sent are kept; the unterminated
    /// tail is refused.
    #[test]
    fn a_connection_that_stops_mid_line_keeps_what_was_whole_and_refuses_the_rest() {
        let mut r = receiver();
        let mut out = Vec::new();
        assert!(r.feed(b"{\"whole\":1}\nhalf of ano", &mut out));
        r.expired(&mut out);
        assert_eq!(
            ingested(&out),
            [
                Ingest::Line("{\"whole\":1}".to_string()),
                Ingest::Unfinished {
                    held: "half of ano".len()
                }
            ]
        );
    }

    /// **A connection that says nothing is not a record**, so it produces no
    /// event at all — at EOF or at the deadline. Manufacturing one would hand
    /// anybody who can `connect(2)` a free write into `ADR-022` §8's capped
    /// storage.
    #[test]
    fn a_connection_that_delivers_no_bytes_produces_no_record() {
        let mut r = receiver();
        let mut out = Vec::new();
        r.at_eof(&mut out);
        r.expired(&mut out);
        assert!(out.is_empty(), "{out:?}");
    }

    /// **The whole path, over a real socket on this host.** Bind, connect,
    /// `SO_PEERCRED`, read, deliver — none of which a fake can prove.
    ///
    /// The credentials asserted are **this process's own**, read back out of
    /// the kernel rather than compared against a constant: that is the claim
    /// `ADR-022` §11.3 makes, and comparing it against anything else would be
    /// checking the test's arithmetic instead of the kernel's answer.
    ///
    /// It binds under the host's temporary directory and nowhere near
    /// `/run/rivet` (`AGENTS.md` §9), which is why
    /// [`Kernel::open_journal_ingest`] takes the path as an argument.
    #[test]
    fn the_real_socket_accepts_a_record_and_reports_the_kernels_own_credentials() {
        use std::io::Write as _;

        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-ingest", std::process::id()));
        let dir = dir.to_string_lossy().into_owned();
        let _ = std::fs::remove_dir_all(&dir);
        let path = format!("{dir}/journal");

        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.open_journal_ingest(&path), Ok(()));

        // The mode is what makes this reachable by an account that is not this
        // one (`ADR-022` §11.4). Read back off the filesystem rather than
        // assumed.
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path)
                .unwrap_or_else(|e| panic!("the socket is not there: {e}"))
                .permissions()
                .mode()
                & 0o7777;
            assert_eq!(mode, rivet_core::journal::INGEST_SOCKET_MODE);
        }

        let record = rivet_core::journal::Record {
            ts: None,
            boot: "the-senders-idea-of-a-boot".to_string(),
            mono: 1.5,
            sev: rivet_core::journal::Severity::Info,
            svc: "rivet".to_string(),
            corr: "senders-chain".to_string(),
            actor: "operator".to_string(),
            msg: "replace /state/x: 7 bytes to 6".to_string(),
            fields: vec![],
        };
        let mut client = std::os::unix::net::UnixStream::connect(&path)
            .unwrap_or_else(|e| panic!("nothing could connect to the socket that was bound: {e}"));
        client
            .write_all(record.to_jsonl().as_bytes())
            .unwrap_or_else(|e| panic!("the record could not be written: {e}"));
        drop(client);

        // One wakeup is enough: `drain_ingest` runs before anything blocks, for
        // the captures' reason — a sender waiting to be accepted is a sender
        // that is blocked.
        let events = k.wait(Some(0)).unwrap_or_default();
        match ingested(&events).first() {
            Some(Ingest::Line(line)) => assert_eq!(line.as_str(), record.to_jsonl().trim_end()),
            other => panic!("{other:?} — events were {events:?}"),
        }
        let peer = events
            .iter()
            .find_map(|e| match e {
                Event::Ingest { peer, .. } => Some(*peer),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no ingest event at all"));
        // SAFETY: `getuid`, `getgid` and `getpid` take no arguments, read no
        // memory and cannot fail.
        let (uid, gid, pid) = unsafe { (libc::getuid(), libc::getgid(), libc::getpid()) };
        assert_eq!(peer.uid, uid, "the uid is not this process's");
        assert_eq!(peer.gid, gid, "the gid is not this process's");
        assert_eq!(peer.pid, pid, "the pid is not this process's");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **Many senders at once, over real sockets.** More connect than there are
    /// slots; the supervisor accepts its cap and leaves the rest in the kernel's
    /// listen queue, which is back-pressure rather than a drop — a sender that
    /// cannot connect takes its own stderr floor.
    ///
    /// It also pins the thing that would otherwise be a livelock: with every
    /// slot full the listener must **leave** the `poll(2)` set, because a
    /// listening socket with a connection waiting is permanently readable and
    /// polling it while refusing to accept spins the supervisor at 100%.
    #[test]
    fn more_senders_than_there_are_slots_are_held_by_the_kernel_rather_than_dropped() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-ingest-many", std::process::id()));
        let dir = dir.to_string_lossy().into_owned();
        let _ = std::fs::remove_dir_all(&dir);
        let path = format!("{dir}/journal");

        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.open_journal_ingest(&path), Ok(()));

        let cap = rivet_core::journal::INGEST_MAX_IN_FLIGHT;
        // Held open and silent, which is the awkward shape: nothing to read, so
        // nothing retires them until their deadline.
        let clients: Vec<std::os::unix::net::UnixStream> = (0..cap + 4)
            .map(|n| {
                std::os::unix::net::UnixStream::connect(&path)
                    .unwrap_or_else(|e| panic!("connection {n} was refused: {e}"))
            })
            .collect();

        let events = k.wait(Some(0)).unwrap_or_default();
        assert!(
            ingested(&events).is_empty(),
            "a connection that said nothing produced a record: {events:?}"
        );
        assert_eq!(k.incoming.len(), cap, "the cap was not applied");

        // **And the listener must now be out of the poll set.** A listening
        // socket with a connection waiting is permanently readable, so polling
        // it while refusing to accept returns immediately for ever.
        //
        // This asserts the *decision* rather than the absence of the spin, and
        // the difference is worth stating: driving the real `poll(2)` would need
        // `arm_signals`, which blocks every signal in `BLOCKED_SIGNALS` for the whole
        // test binary. So what is pinned is that `wait` builds its set from this
        // predicate — which it does, one line — and not that a spinning build
        // would be observed here.
        assert!(
            !k.accepting_ingest(),
            "with every slot full the listener is still offered to poll(2)"
        );

        // The surplus is still connected — the kernel is holding them, and none
        // of them has been told anything is wrong.
        drop(clients);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The signals the supervisor takes over, as a contract.**
    ///
    /// `SIGHUP` is the one worth asserting and the reason is its DEFAULT
    /// DISPOSITION: it terminates the process. A build that read it as an event
    /// without blocking it would pass every test in this file and die on the
    /// first reload, because nothing here arms signals (see the note in
    /// `an_ingest_backlog_stops_being_offered_to_poll`).
    #[test]
    fn the_supervisor_blocks_every_signal_it_reads_and_sighup_is_one_of_them() {
        assert!(
            BLOCKED_SIGNALS.contains(&libc::SIGHUP),
            "SIGHUP is read as Event::Reload and not blocked, so it kills this process instead"
        );
        assert_eq!(
            BLOCKED_SIGNALS,
            [libc::SIGTERM, libc::SIGINT, libc::SIGCHLD, libc::SIGHUP],
            "the set changed; ADR-017 §3.1.10 and ADR-032 §3.2 are the contracts it implements"
        );
    }

    // -- the drain budget (`DRAIN_BUDGET_BYTES`) -----------------------------
    //
    // Every fixture below writes 64-byte lines, so a budget is a whole number
    // of them and a count of lines is a count of bytes. All hand-composed.

    /// One 64-byte line: 63 bytes of text and its newline.
    fn flood_line() -> Vec<u8> {
        let mut line = vec![b'x'; 63];
        line.push(b'\n');
        line
    }

    /// The lines a budget's worth of [`flood_line`]s makes.
    const LINES_PER_BUDGET: usize = DRAIN_BUDGET_BYTES / 64;

    /// A source that never runs dry — every read is filled, as by a writer
    /// that refills faster than the reader empties it.
    ///
    /// **It panics once it has served far more than any budget**, so a drain
    /// with no budget fails this suite loudly rather than hanging it.
    struct Endless {
        served: usize,
        reads: Vec<usize>,
    }

    impl std::io::Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            assert!(
                self.served < 64 * DRAIN_BUDGET_BYTES,
                "one pass read {} bytes from a source that never stops and had not stopped \
                 reading — nothing else in the loop would have run",
                self.served
            );
            for (i, byte) in buf.iter_mut().enumerate() {
                *byte = if (self.served + i) % 64 == 63 { b'\n' } else { b'x' };
            }
            self.served += buf.len();
            self.reads.push(buf.len());
            Ok(buf.len())
        }
    }

    /// **A source that never stops writing ends the pass at the budget**, and
    /// the next pass takes the next budget — the whole of the rule, at the one
    /// function every drain is built on.
    #[test]
    fn a_source_that_never_runs_dry_gives_up_one_budget_per_pass() {
        let mut source = Endless {
            served: 0,
            reads: Vec::new(),
        };
        let mut buf = [0u8; 4096];
        for pass in 1..=3 {
            let mut taken = 0;
            let how = drain_within_budget(&mut source, &mut buf, |bytes| {
                taken += bytes.len();
                true
            });
            assert_eq!(how, Drained::Budget, "pass {pass}");
            assert_eq!(taken, DRAIN_BUDGET_BYTES, "pass {pass}");
            assert_eq!(source.served, pass * DRAIN_BUDGET_BYTES, "pass {pass}");
        }
    }

    /// **The budget never cuts a whole-record read.** A datagram longer than
    /// the buffer it is received into is truncated, and a `/dev/kmsg` read
    /// shorter than its record is `EINVAL` — so both drains depend on every
    /// read being the full buffer, which holds because both buffers divide
    /// the budget.
    #[test]
    fn a_record_sized_buffer_is_read_whole_up_to_the_budget() {
        for size in [4096_usize, 8192] {
            assert_eq!(DRAIN_BUDGET_BYTES % size, 0, "{size} does not divide the budget");
            let mut source = Endless {
                served: 0,
                reads: Vec::new(),
            };
            let mut buf = vec![0u8; size];
            let how = drain_within_budget(&mut source, &mut buf, |_| true);
            assert_eq!(how, Drained::Budget);
            assert!(
                source.reads.iter().all(|&n| n == size),
                "a read was cut short of the buffer: {:?}",
                source.reads
            );
        }
    }

    /// A caller's own stop still ends the pass where it says.
    #[test]
    fn a_drain_stops_where_its_caller_says() {
        let mut source = Endless {
            served: 0,
            reads: Vec::new(),
        };
        let mut buf = [0u8; 4096];
        let mut reads = 0;
        let how = drain_within_budget(&mut source, &mut buf, |_| {
            reads += 1;
            reads < 3
        });
        assert_eq!(how, Drained::Stopped);
        assert_eq!(source.served, 3 * 4096);
    }

    /// A pipe holding `bytes` of [`flood_line`]s, **its capacity raised past
    /// one budget** — the state a pipe is in when its writer refills it faster
    /// than the supervisor empties it. The read end is non-blocking, as
    /// [`LibcKernel::capture`] makes it; the write end is returned and must be
    /// held, so an emptied pipe answers `WouldBlock` rather than EOF.
    fn a_full_pipe(bytes: usize) -> (std::fs::File, std::fs::File) {
        use std::os::fd::FromRawFd as _;

        let mut ends: [libc::c_int; 2] = [-1, -1];
        // SAFETY: two `c_int`s on this stack; `pipe2` writes exactly two.
        let made = unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) };
        assert_eq!(made, 0, "pipe2: {}", std::io::Error::last_os_error());
        // SAFETY: both are fresh descriptors `pipe2` just returned, owned here
        // and nowhere else. `fcntl` sets flags on them and touches no memory.
        let (read, mut write) = unsafe {
            let sized = libc::fcntl(ends[1], libc::F_SETPIPE_SZ, 1024 * 1024);
            assert!(
                sized >= 0 && usize::try_from(sized).unwrap_or(0) >= bytes,
                "this host would not raise a pipe to 1 MiB, so the fixture cannot hold \
                 more than one budget: {}",
                std::io::Error::last_os_error()
            );
            let flags = libc::fcntl(ends[0], libc::F_GETFL);
            assert!(flags >= 0);
            assert_eq!(libc::fcntl(ends[0], libc::F_SETFL, flags | libc::O_NONBLOCK), 0);
            (
                std::fs::File::from_raw_fd(ends[0]),
                std::fs::File::from_raw_fd(ends[1]),
            )
        };
        let line = flood_line();
        for _ in 0..bytes / line.len() {
            write
                .write_all(&line)
                .unwrap_or_else(|e| panic!("the fixture pipe would not take its lines: {e}"));
        }
        (read, write)
    }

    /// **A pipe holding more than a budget gives up one budget per pass**, and
    /// keeps the rest for the next.
    #[test]
    fn a_pipe_holding_more_than_the_budget_gives_up_one_budget_per_pass() {
        let (read, _write) = a_full_pipe(4 * DRAIN_BUDGET_BYTES);
        let mut c = splitter();
        c.file = read;
        for pass in 1..=4 {
            let mut out = Vec::new();
            c.drain(false, &mut out);
            assert_eq!(lines(&out).len(), LINES_PER_BUDGET, "pass {pass}");
        }
        let mut out = Vec::new();
        c.drain(false, &mut out);
        assert!(out.is_empty(), "the pipe held more than was written: {}", out.len());
    }

    /// **An ingest connection holding more than a budget gives up one budget
    /// per pass**, and stays open for the next.
    #[test]
    fn an_ingest_connection_holding_more_than_the_budget_gives_up_one_budget_per_pass() {
        let (ours, theirs) = std::os::unix::net::UnixStream::pair().unwrap_or_else(|e| {
            panic!("this host cannot make a socketpair, which nothing here can work round: {e}")
        });
        assert!(ours.set_nonblocking(true).is_ok());
        assert!(theirs.set_nonblocking(true).is_ok());
        // As much as the socket will hold, in whole lines and in 4 KiB
        // writes: a socket accounts every write as a buffer of its own, so
        // 64-byte writes would fill it with overhead long before one budget.
        let line = flood_line();
        let chunk = line.repeat(64);
        let mut written = 0;
        let mut sender = &theirs;
        while written < 4 * DRAIN_BUDGET_BYTES {
            match sender.write(&chunk) {
                Ok(n) => written += n,
                Err(_) => break,
            }
        }
        assert!(
            written > DRAIN_BUDGET_BYTES + line.len(),
            "this host's socket buffer held only {written} bytes, so this test cannot \
             put more than one budget in front of the drain"
        );

        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        let accepted_ms = k.now_ms();
        k.incoming.push(Incoming {
            stream: ours,
            peer: rivet_core::journal::Peer {
                uid: 1000,
                gid: 1000,
                pid: 4242,
            },
            accepted_ms,
            partial: Vec::new(),
        });
        let mut out = Vec::new();
        k.drain_ingest(&mut out);
        assert_eq!(ingested(&out).len(), LINES_PER_BUDGET);
        assert_eq!(k.incoming.len(), 1, "a connection with more to say was closed");
        drop(theirs);
    }

    /// **A notify socket gives up one budget per pass.** Driven through
    /// `drain_notify`'s own two halves — [`drain_within_budget`] and
    /// [`LibcKernel::notified`] — over a source that never stops sending,
    /// because a real datagram socket cannot be filled past a budget for a
    /// test: the kernel's per-socket queue length refuses the sender first.
    /// What it cannot refuse is a sender that refills as fast as it is read.
    #[test]
    fn a_notify_socket_that_never_stops_gives_up_one_budget_per_pass() {
        let mut source = Endless {
            served: 0,
            reads: Vec::new(),
        };
        let mut buf = [0u8; 4096];
        let mut out = Vec::new();
        let how = drain_within_budget(&mut source, &mut buf, |datagram| {
            LibcKernel::notified("loud", datagram, &mut out);
            true
        });
        assert_eq!(how, Drained::Budget);
        assert_eq!(source.reads.len(), DRAIN_BUDGET_BYTES / 4096);
    }

    /// **The case the budget exists for, through `wait` itself.** One service's
    /// pipe holds four budgets, as a writer that never stops leaves it; a
    /// `SIGTERM` is waiting and another service has said `READY=1`. One pass
    /// must hear all three: a budget of the flood, the signal, and the other
    /// service.
    ///
    /// The signal descriptor is a pipe holding one `signalfd_siginfo` with
    /// `SIGTERM` in it, because arming the real one would block those signals
    /// for the whole test binary; what `wait` does with the bytes is the same.
    #[test]
    fn a_flooding_pipe_cannot_keep_a_pass_from_a_signal_or_another_service() {
        use std::os::fd::FromRawFd as _;

        let mut k = LibcKernel::new(ConsoleForm::Jsonl);

        let mut ends: [libc::c_int; 2] = [-1, -1];
        // SAFETY: two `c_int`s on this stack; `pipe2` writes exactly two.
        let made = unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) };
        assert_eq!(made, 0, "pipe2: {}", std::io::Error::last_os_error());
        // SAFETY: fresh descriptors `pipe2` just returned, owned only here.
        let (signals, mut raise) = unsafe {
            (
                std::fs::File::from_raw_fd(ends[0]),
                std::fs::File::from_raw_fd(ends[1]),
            )
        };
        let mut siginfo = [0u8; SIGINFO_SIZE];
        siginfo[..4].copy_from_slice(&u32::try_from(libc::SIGTERM).unwrap_or(0).to_ne_bytes());
        assert!(raise.write_all(&siginfo).is_ok());
        k.sigfd = Some(signals);

        let (flood, _writer) = a_full_pipe(4 * DRAIN_BUDGET_BYTES);
        k.captures.push(Capture {
            pid: 1,
            service: "loud".to_string(),
            stream: Stream::Stdout,
            file: flood,
            partial: Vec::new(),
            overflowed: false,
        });

        let (theirs, ours) = std::os::unix::net::UnixDatagram::pair().unwrap_or_else(|e| {
            panic!("this host cannot make a datagram pair: {e}")
        });
        assert!(ours.set_nonblocking(true).is_ok());
        assert!(theirs.send(b"READY=1\n").is_ok());
        k.notify.insert("quiet".to_string(), ours);

        // `None`: a pass that slept here would never return.
        let events = k.wait(None).unwrap_or_default();
        assert_eq!(
            lines(&events).len(),
            LINES_PER_BUDGET,
            "the flooding pipe was not held to one budget"
        );
        assert!(
            events.iter().any(|e| matches!(e, Event::Terminate { .. })),
            "a SIGTERM went unread for a pass because one service would not stop writing"
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Ready { service } if service == "quiet")),
            "another service's READY=1 went unread for a pass because one service would \
             not stop writing"
        );
        drop(raise);
        drop(theirs);
    }

    /// **A silent connection cannot hold the supervisor asleep.** Its deadline
    /// is not a descriptor, so `poll` cannot wake for it; the wait has to
    /// shorten itself. A supervisor with nothing else pending waits for ever
    /// otherwise, which would make `ADR-022` §11.5's bound unreachable.
    #[test]
    fn an_open_ingest_connection_shortens_a_wait_that_would_otherwise_block_for_ever() {
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.next_ingest_deadline_ms(), None);
        k.incoming.push(receiver());
        let until = k
            .next_ingest_deadline_ms()
            .unwrap_or_else(|| panic!("an open connection reported no deadline"));
        assert!(
            until <= rivet_core::journal::INGEST_DEADLINE_MS,
            "{until}"
        );
    }

    // -- passing a descriptor (`PLAN.md` §106.2) -----------------------------
    //
    // Real syscalls over a real `socketpair`. A fake cannot demonstrate any of
    // this: the whole subject is what the kernel writes into a control buffer
    // and what it says about the write.

    /// The control buffer really is aligned for the struct the kernel writes
    /// through it.
    ///
    /// **This is what the audit's first finding is, stated as an assertion.**
    /// A bare `[u8; 64]` has alignment one; `struct cmsghdr` needs `size_t`'s.
    /// aarch64 and x86-64 both tolerate the unaligned access in practice, so
    /// **no round-trip test can catch this** — the code worked. What can be
    /// checked is the property the code is entitled to assume, and it is
    /// checked here rather than trusted to a `#[repr]` attribute nobody reads.
    #[test]
    fn the_control_buffer_is_aligned_for_a_cmsghdr() {
        let control = ControlBuffer::zeroed();
        let at = control.0.as_ptr() as usize;
        assert_eq!(
            at % core::mem::align_of::<libc::cmsghdr>(),
            0,
            "the control buffer is at {at:#x}, which a cmsghdr may not be read from"
        );
        assert!(core::mem::align_of::<ControlBuffer>() >= core::mem::align_of::<libc::cmsghdr>());
        // And it is still big enough for the descriptor the protocol sends.
        // SAFETY: `CMSG_SPACE` is a pure computation over a constant.
        assert!(CONTROL_BYTES as u32 >= unsafe { libc::CMSG_SPACE(4) });
    }

    /// **The length arithmetic cannot wrap**, which is the audit's second
    /// finding. A `cmsg_len` shorter than a header is not something a working
    /// kernel writes; it is read through a raw pointer out of a buffer a
    /// truncated `recvmsg` touched, and `len - CMSG_LEN(0)` on a `usize` would
    /// have produced a count near `2^62` and a loop reading past the buffer.
    #[test]
    fn a_control_message_shorter_than_its_own_header_carries_no_descriptors() {
        // SAFETY: `CMSG_LEN` is a pure computation over a constant.
        let header = unsafe { libc::CMSG_LEN(0) } as usize;
        let fd = core::mem::size_of::<libc::c_int>();

        for short in [0, 1, header / 2, header - 1] {
            assert_eq!(descriptors_in(short, header), 0, "cmsg_len {short}");
        }
        assert_eq!(descriptors_in(header, header), 0);
        assert_eq!(descriptors_in(header + fd, header), 1);
        assert_eq!(descriptors_in(header + fd * 3, header), 3);
        // And a partial trailing descriptor is not counted as a whole one.
        assert_eq!(descriptors_in(header + fd + 1, header), 1);
    }

    /// The ordinary case, end to end over a real socket: one descriptor is sent
    /// and the same open file arrives at the other end.
    #[test]
    fn a_descriptor_sent_with_a_message_arrives_with_it() {
        use std::os::fd::AsRawFd as _;

        let (a, b) = std::os::unix::net::UnixStream::pair()
            .unwrap_or_else(|e| panic!("this host cannot make a socketpair: {e}"));
        // Any descriptor will do; a third socket is the cheapest one that needs
        // no filesystem.
        let (gift, _keep) = std::os::unix::net::UnixStream::pair()
            .unwrap_or_else(|e| panic!("this host cannot make a socketpair: {e}"));

        send_fd(a.as_raw_fd(), gift.as_raw_fd()).unwrap_or_else(|e| panic!("send: {e:?}"));
        let (text, lease) = recv_with_fd(&b).unwrap_or_else(|e| panic!("recv: {e}"));

        assert_eq!(text, "\n", "the carrier byte is what `send_fd` documents");
        let lease = lease.unwrap_or_else(|| panic!("no descriptor arrived"));
        // A different number in this process, and the same open file: writing
        // into it reaches `_keep`.
        assert_ne!(lease.as_raw_fd(), gift.as_raw_fd());
    }

    /// **A truncated control message is refused rather than read as a whole
    /// one** — the audit's third finding.
    ///
    /// `recvmsg` returns the byte count of the ordinary data whether or not the
    /// control data fitted, so a caller that looks only at the return value
    /// cannot tell. The kernel says so in `msg_flags`, and it drops the
    /// descriptors that did not fit — so what an unchecked read produces is a
    /// request that looks complete and whose lease is missing or partial.
    ///
    /// Driven by sending more descriptors than [`CONTROL_BYTES`] can hold,
    /// which is the only way to make a real kernel set the flag.
    #[test]
    fn a_control_message_that_did_not_fit_is_refused() {
        use std::os::fd::AsRawFd as _;

        let (a, b) = std::os::unix::net::UnixStream::pair()
            .unwrap_or_else(|e| panic!("this host cannot make a socketpair: {e}"));

        // Enough descriptors that `CMSG_SPACE(4 * n)` is past the buffer.
        let many: Vec<std::os::unix::net::UnixStream> = (0..32)
            .map(|_| {
                let (one, _) = std::os::unix::net::UnixStream::pair()
                    .unwrap_or_else(|e| panic!("this host cannot make a socketpair: {e}"));
                one
            })
            .collect();
        let raw: Vec<libc::c_int> = many.iter().map(std::os::fd::AsRawFd::as_raw_fd).collect();
        send_fds(a.as_raw_fd(), &raw).unwrap_or_else(|e| panic!("send: {e:?}"));

        let answer = recv_with_fd(&b);
        let Err(e) = answer else {
            panic!("a truncated control message was accepted as a whole one");
        };
        assert_eq!(
            e.raw_os_error(),
            Some(libc::EMSGSIZE),
            "the refusal does not name the platform's own word for it: {e}"
        );
    }

    /// Send several descriptors in one `SCM_RIGHTS`, which nothing in the
    /// supervisor does and which a hostile or confused peer can.
    ///
    /// A test-only sender: [`send_fd`] deliberately sends exactly one, and
    /// widening it so a test could drive this would be changing the program to
    /// suit the test.
    fn send_fds(socket: std::os::fd::RawFd, fds: &[libc::c_int]) -> Result<(), Errno> {
        let mut payload = *b"\n";
        let mut iov = libc::iovec {
            iov_base: payload.as_mut_ptr().cast::<libc::c_void>(),
            iov_len: payload.len(),
        };
        let bytes = core::mem::size_of_val(fds);
        // SAFETY: `CMSG_SPACE` is a pure computation over a length.
        let space = unsafe { libc::CMSG_SPACE(bytes as u32) } as usize;
        let mut control = vec![0u8; space];

        // SAFETY: a plain C struct; every pointer below outlives the call.
        let mut msg: libc::msghdr = unsafe { core::mem::zeroed() };
        msg.msg_iov = &raw mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast::<libc::c_void>();
        msg.msg_controllen = space as _;

        // SAFETY: `control` is `space` bytes, which is what `msg_controllen`
        // says, so `CMSG_FIRSTHDR` and `CMSG_DATA` stay inside it. `Vec`'s
        // allocation is at least pointer-aligned, which is `cmsghdr`'s.
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&raw const msg);
            assert!(!cmsg.is_null());
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN(bytes as u32) as _;
            core::ptr::copy_nonoverlapping(
                fds.as_ptr().cast::<u8>(),
                libc::CMSG_DATA(cmsg),
                bytes,
            );
        }

        // SAFETY: `socket` is an open socket this process owns.
        let sent = unsafe { libc::sendmsg(socket, &raw const msg, libc::MSG_NOSIGNAL) };
        if sent < 0 {
            return Err(Errno(errno_now()));
        }
        Ok(())
    }

    // -- no thread anywhere this process might fork ---------------------

    /// **No thread lives in this process at any point it might fork** — the
    /// invariant `spawn`'s `pre_exec` closure and `cec_send` both depend on
    /// to allocate safely in a child. Stated here as an assertion about this
    /// crate's own source rather than only in its documentation, because a
    /// convention nobody enforces erodes: `display_hours.rs`'s `Television`
    /// once carried a `std::thread::spawn` this test would have refused.
    ///
    /// **Non-test source only.** A source file's own `mod tests`, and
    /// anything after it, is excluded: a test binary is not this process —
    /// `cargo test` already runs many of them on many threads — and this very
    /// module uses a real thread deliberately, in
    /// `a_peer_that_is_not_accepting_is_refused_rather_than_waited_for`, to
    /// give a test a way to fail loudly instead of hanging the suite.
    ///
    /// The cut point is the LAST occurrence of the test-module attribute that
    /// is actually followed by `mod tests`, searched from the end of the file
    /// backward rather than taking the single last occurrence unconditionally
    /// — a file may name the attribute again after its real test module: a
    /// single function gated the same way, or a doc comment (this one
    /// included) that merely quotes it in prose.
    #[test]
    fn no_non_test_source_spawns_a_thread() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let Ok(entries) = std::fs::read_dir(&root) else {
            unreachable!("this crate has no src directory")
        };
        let mut checked = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let Ok(body) = std::fs::read_to_string(&path) else {
                continue;
            };
            checked += 1;

            // A plain literal, deliberately: the search below finds the LAST
            // occurrence that is actually followed by `mod tests` rather than
            // trusting there is only one, so it does not matter that this
            // file's own source — this line included — mentions the
            // attribute again afterward.
            let marker = "#[cfg(test)]";
            let mut cutoff = body.len();
            let mut search_end = body.len();
            while let Some(at) = body[..search_end].rfind(marker) {
                if body[at + marker.len()..].trim_start().starts_with("mod tests") {
                    cutoff = at;
                    break;
                }
                search_end = at;
            }

            for (n, line) in body[..cutoff].lines().enumerate() {
                let code = line.trim_start();
                if code.starts_with("//") {
                    continue;
                }
                assert!(
                    !code.contains("thread::spawn") && !code.contains("thread::Builder"),
                    "{}:{}: a thread was spawned outside this crate's own tests: {line}",
                    path.display(),
                    n + 1
                );
            }
        }
        assert!(checked >= 20, "only {checked} source files were scanned; the guard is unchecked");
    }

    // -- LISTEN_PID names the child, not the parent -----------------------
    //
    // Real syscalls through the real kernel port, for `spawn`'s own note: a
    // fake cannot demonstrate an unspecified `std` behaviour.

    /// **`LISTEN_PID` names the child, not the parent** — `ADR-017` §3, and
    /// the half of `spawn`'s environment hand-over a fake cannot prove.
    /// `std::process::Command` does not document that it reads the child's
    /// own `environ` at `execvp` rather than the one it built before `fork`;
    /// `spawn`'s `pre_exec` closure depends on that measured-not-assumed
    /// behaviour to set `LISTEN_PID` after the child's pid exists. This
    /// drives a real spawn through the real kernel port and reads what the
    /// CHILD believed about itself, not what this process handed it.
    #[test]
    fn listen_pid_names_the_child_the_pre_exec_closure_set_it_to() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("rivet-sysd-{}-listen-pid", std::process::id()));
        let root = dir.to_string_lossy().into_owned();
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.mkdir_p(&root), Ok(()));

        // One descriptor is enough for `spawn` to take the `LISTEN_FDS`
        // branch that also sets `LISTEN_PID`; what is at the other end of it
        // is not this test's subject.
        let (placeholder, _keep) = std::os::unix::net::UnixStream::pair()
            .unwrap_or_else(|e| panic!("this host cannot make a socketpair: {e}"));
        k.listeners.insert(
            "listen-pid-probe".to_string(),
            Listeners {
                fds: vec![std::os::fd::OwnedFd::from(placeholder)],
                paths: Vec::new(),
            },
        );

        let out = format!("{root}/pids");
        let program = Program {
            path: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), format!("echo \"$LISTEN_PID $$\" > {out}")],
            env: Vec::new(),
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        };
        let pid = k
            .spawn("listen-pid-probe", &program)
            .unwrap_or_else(|e| panic!("the child did not start: {e}"));
        // SAFETY: `pid` is this process's own child, just spawned, and
        // nothing else reaps it.
        unsafe {
            libc::waitpid(pid, std::ptr::null_mut(), 0);
        }

        let reported = std::fs::read_to_string(&out)
            .unwrap_or_else(|e| panic!("the child did not answer: {e}"));
        let mut words = reported.split_whitespace();
        let listen_pid: i32 = words
            .next()
            .unwrap_or_else(|| panic!("the child printed no LISTEN_PID: {reported:?}"))
            .parse()
            .unwrap_or_else(|e| panic!("LISTEN_PID was not a number ({reported:?}): {e}"));
        let own_pid: i32 = words
            .next()
            .unwrap_or_else(|| panic!("the child printed no pid of its own: {reported:?}"))
            .parse()
            .unwrap_or_else(|e| panic!("$$ was not a number ({reported:?}): {e}"));

        assert_eq!(
            listen_pid, own_pid,
            "LISTEN_PID named {listen_pid}, but the child's own pid was {own_pid} — \
             sd_listen_fds compares the two and hands back nothing when they differ"
        );
        assert_eq!(
            listen_pid, pid,
            "LISTEN_PID did not name the pid this process itself observed from `spawn`"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // -- a one-shot program under a deadline (`run_within`) ----------------

    fn sh(script: &str) -> Program {
        Program {
            path: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), script.to_string()],
            env: Vec::new(),
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        }
    }

    /// **A program still running at its deadline is killed, and the call
    /// returns at the deadline rather than when the program would have.**
    #[test]
    fn a_one_shot_program_past_its_deadline_is_stopped_and_the_call_returns() {
        if !std::path::Path::new("/bin/sh").exists() {
            eprintln!("no /bin/sh on this host; the deadline test is blind");
            return;
        }
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        let started = std::time::Instant::now();
        let answer = k.run_within(&sh("exec sleep 5"), 300);
        let took = started.elapsed();
        assert_eq!(answer.map(|c| c.status), Err(Errno::ETIMEDOUT));
        assert!(
            took < std::time::Duration::from_secs(2),
            "the call waited {took:?} for a program its deadline gave 300 ms"
        );
        // And it does not outlive the call as a zombie: reaped beside the
        // services, within moments of the kill.
        for _ in 0..200 {
            let _ = k.reap_ready();
            if k.helpers.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(k.helpers.is_empty(), "the killed program was never reaped");
        assert!(k.pending.is_empty(), "a killed one-shot program reported something");
    }

    /// **A program that finishes in time is answered as `run_to_completion`
    /// answers it** — status and both streams.
    #[test]
    fn a_one_shot_program_that_finishes_in_time_is_answered_with_what_it_said() {
        if !std::path::Path::new("/bin/sh").exists() {
            eprintln!("no /bin/sh on this host; the deadline test is blind");
            return;
        }
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        let done = k
            .run_within(&sh("echo to-stdout; echo to-stderr >&2; exit 3"), 5_000)
            .unwrap_or_else(|e| panic!("a program that finishes at once was refused: {e}"));
        assert_eq!(done.status, ExitStatus::Exited(3));
        assert_eq!(
            done.output,
            [
                (Stream::Stdout, "to-stdout".to_string()),
                (Stream::Stderr, "to-stderr".to_string())
            ]
        );
    }

    /// **A program that exits and leaves something holding its streams is
    /// answered at the deadline with what it said**, and is not reported as
    /// timed out — it finished; the straggler is not the program.
    #[test]
    fn a_program_whose_child_holds_its_streams_is_answered_at_the_deadline() {
        if !std::path::Path::new("/bin/sh").exists() {
            eprintln!("no /bin/sh on this host; the deadline test is blind");
            return;
        }
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        let started = std::time::Instant::now();
        let done = k
            .run_within(&sh("sleep 5 & echo drawn"), 300)
            .unwrap_or_else(|e| panic!("a program that exited was reported as {e}"));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert!(done.status.is_success(), "{:?}", done.status);
        assert_eq!(done.output, [(Stream::Stdout, "drawn".to_string())]);
    }

    // -- the reclaim helper: fork, delete, reap ----------------------------

    /// Reap until a reclaim helper reports, or two seconds pass.
    fn await_reclaimed(k: &mut LibcKernel) -> Vec<Event> {
        for _ in 0..200 {
            let _ = k.reap_ready();
            let reported: Vec<Event> = k
                .pending
                .drain(..)
                .filter(|e| matches!(e, Event::Reclaimed { .. }))
                .collect();
            if !reported.is_empty() {
                return reported;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Vec::new()
    }

    /// **The helper deletes every entry with the prefix, and nothing else**,
    /// off the calling thread, and reports through `wait`'s queue — a real
    /// fork over a real directory. Hand-composed fixture.
    ///
    /// A symbolic link inside a set-aside tree, pointing at a directory
    /// outside it, must cost the link and not what it points at.
    #[test]
    fn a_reclaim_helper_deletes_what_was_set_aside_and_nothing_else() {
        let root = format!(
            "{}/rivet-sysd-{}-reclaim",
            std::env::temp_dir().display(),
            std::process::id()
        );
        let _ = std::fs::remove_dir_all(&root);
        let app = format!("{root}/app");
        let outside = format!("{root}/outside");
        for dir in [
            format!("{app}/tmp.reclaim-1-1/deep/deeper"),
            format!("{app}/tmp.reclaim-7-2"),
            format!("{app}/tmp"),
            format!("{app}/data"),
            outside.clone(),
        ] {
            assert!(std::fs::create_dir_all(&dir).is_ok(), "{dir}");
        }
        assert!(std::fs::write(format!("{app}/tmp.reclaim-1-1/deep/deeper/f"), b"x").is_ok());
        assert!(std::fs::write(format!("{outside}/keep"), b"keep").is_ok());
        assert!(std::os::unix::fs::symlink(&outside, format!("{app}/tmp.reclaim-1-1/link")).is_ok());

        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        assert_eq!(k.reclaim_in_child("a", &app, "tmp.reclaim-"), Ok(()));
        let reported = await_reclaimed(&mut k);
        assert_eq!(
            reported,
            [Event::Reclaimed {
                service: "a".to_string(),
                dir: app.clone(),
                prefix: "tmp.reclaim-".to_string(),
                failure: None
            }],
            "the helper did not report, or reported trouble"
        );

        let mut left: Vec<String> = std::fs::read_dir(&app)
            .map(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        left.sort();
        assert_eq!(left, ["data", "tmp"], "the helper deleted the wrong things");
        assert!(
            std::path::Path::new(&format!("{outside}/keep")).exists(),
            "the helper followed a link out of the tree it was deleting"
        );
        assert!(k.helpers.is_empty(), "the helper was not reaped");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// **A removal, end to end on a real filesystem** — `crate::reclaim`: the
    /// target is gone from its directory the moment the call returns, the
    /// helper deletes the aside, a link inside it costs only the link, and a
    /// sibling with a name of its own is untouched. Hand-composed fixture.
    #[test]
    fn a_removal_is_gone_at_the_rename_and_deleted_by_a_helper() {
        let root = format!(
            "{}/rivet-sysd-{}-removal",
            std::env::temp_dir().display(),
            std::process::id()
        );
        let _ = std::fs::remove_dir_all(&root);
        let lib = format!("{root}/lib");
        let outside = format!("{root}/outside");
        for dir in [
            format!("{lib}/libgone/deep"),
            format!("{lib}/libkept"),
            outside.clone(),
        ] {
            assert!(std::fs::create_dir_all(&dir).is_ok(), "{dir}");
        }
        assert!(std::fs::write(format!("{lib}/libgone/deep/f"), b"x").is_ok());
        assert!(std::fs::write(format!("{outside}/keep"), b"keep").is_ok());
        assert!(std::os::unix::fs::symlink(&outside, format!("{lib}/libgone/link")).is_ok());

        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        let done = crate::reclaim::set_aside(&mut k, "rivet-sysd", &format!("{lib}/libgone"));
        assert!(matches!(&done, Ok(d) if d.unforked.is_none()), "{done:?}");
        assert!(
            !std::path::Path::new(&format!("{lib}/libgone")).exists(),
            "the removed name was still there when the call returned"
        );
        let reported = await_reclaimed(&mut k);
        assert_eq!(
            reported,
            [Event::Reclaimed {
                service: "rivet-sysd".to_string(),
                dir: lib.clone(),
                prefix: ".reclaim-".to_string(),
                failure: None
            }]
        );
        let left: Vec<String> = std::fs::read_dir(&lib)
            .map(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(left, ["libkept"], "the helper left or took the wrong things");
        assert!(
            std::path::Path::new(&format!("{outside}/keep")).exists(),
            "the helper followed a link out of the tree it was deleting"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    // -- the CEC worker: fork, pipe, reap --------------------------------
    //
    // Real forks through the real kernel port, with a transaction this test
    // wrote rather than `display_hours::send` — the seam `cec_send` takes a
    // plain function pointer for — so this proves the mechanism without any
    // CEC hardware.

    /// A negotiation the child reports, round-tripped through the pipe, for
    /// both an answer and a refusal — and gone, not merely finished, once
    /// `cec_poll` has read it.
    #[test]
    fn cec_worker_round_trips_success_and_failure_without_a_zombie() {
        fn agrees(lit: bool) -> Result<String, String> {
            Ok(if lit { "on".to_string() } else { "standby".to_string() })
        }
        fn refuses(_lit: bool) -> Result<String, String> {
            Err("simulated: no logical address granted".to_string())
        }

        let mut k = LibcKernel::new(ConsoleForm::Jsonl);

        let cases: [(CecTransaction, bool, Result<String, String>); 2] = [
            (agrees, true, Ok("on".to_string())),
            (refuses, false, Err("simulated: no logical address granted".to_string())),
        ];
        for (transaction, lit, want) in cases {
            let mut worker = k
                .cec_send(lit, transaction)
                .unwrap_or_else(|e| panic!("fork/pipe: {e}"));
            let pid = match worker {
                CecWorker::Child { pid, .. } => pid,
                CecWorker::Resolved(_) => {
                    panic!("LibcKernel::cec_send resolved without forking a child")
                }
            };

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let outcome = loop {
                if let Some(outcome) = k.cec_poll(&mut worker) {
                    break outcome;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "the worker never answered within 5s"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            };
            assert_eq!(outcome, want);

            // And the child is really gone: an independent `waitpid` on the
            // same pid must find nothing left to reap, rather than a zombie
            // `cec_poll` left behind.
            // SAFETY: `pid` was this process's own child; `cec_poll` above
            // already reaped it, so this call only observes that no zombie
            // remains.
            let again = unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
            let again_errno = errno_now();
            assert_eq!(
                again, -1,
                "waitpid found pid {pid} again after cec_poll already reaped it"
            );
            assert_eq!(
                again_errno,
                libc::ECHILD,
                "waitpid on an already-reaped pid failed with {again_errno}, not ECHILD"
            );
        }
    }

    /// **The fd `cec_send`'s test probe checks.** A plain function pointer
    /// carries no state of its own, so the number to check is set here
    /// instead, before the fork that copies it into the child.
    static PROBE_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

    /// Report whether [`PROBE_FD`] is still open — raw `fcntl(F_GETFD)`
    /// rather than anything from `std`, because this runs inside the forked
    /// child and the whole question is what `cec_send` left it holding.
    fn report_probe_fd(_lit: bool) -> Result<String, String> {
        let fd = PROBE_FD.load(std::sync::atomic::Ordering::SeqCst);
        // SAFETY: `F_GETFD` reads one descriptor flag and touches no memory
        // of its own; a closed or invalid `fd` is the ordinary `EBADF`
        // answer, not undefined behaviour.
        let rc = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if rc < 0 {
            Ok(format!("closed, fcntl said errno {}", errno_now()))
        } else {
            Err(format!("still open, F_GETFD returned {rc}"))
        }
    }

    /// **The worker closes what it inherited, not only what it opened.**
    ///
    /// Forked without an `exec`, the child starts with a full copy of this
    /// process's descriptor table: every listener, every long-lived socket,
    /// the signalfd, and — here — a pipe opened for no other reason than to
    /// be something the child must not be able to see. `O_CLOEXEC` cannot
    /// help, because nothing here ever execs; only closing the range by hand
    /// does.
    ///
    /// **Deliberately opened WITHOUT `O_CLOEXEC`** (`libc::pipe`, not
    /// `pipe2`), because the property under test is that `cec_send` closes it
    /// regardless of that flag, exactly as it would have to for a real
    /// listener or the kmsg descriptor, neither of which carries the flag
    /// either.
    #[test]
    fn the_worker_closes_a_descriptor_it_inherited_but_never_opened() {
        let mut ends: [libc::c_int; 2] = [-1, -1];
        // SAFETY: `ends` is a live two-element array on this function's
        // stack; `pipe(2)` writes exactly two descriptors into it.
        let rc = unsafe { libc::pipe(ends.as_mut_ptr()) };
        assert_eq!(rc, 0, "pipe: {}", std::io::Error::last_os_error());
        let (probe_read, probe_write) = (ends[0], ends[1]);
        PROBE_FD.store(probe_read, std::sync::atomic::Ordering::SeqCst);

        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        let mut worker = k
            .cec_send(true, report_probe_fd)
            .unwrap_or_else(|e| panic!("fork/pipe: {e}"));

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let outcome = loop {
            if let Some(outcome) = k.cec_poll(&mut worker) {
                break outcome;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the worker never answered within 5s"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        };

        // SAFETY: this process's own pipe, opened above and closed exactly
        // once, here.
        unsafe {
            libc::close(probe_read);
            libc::close(probe_write);
        }

        assert!(
            outcome.is_ok(),
            "the inherited pipe read end was still visible in the CEC worker: {outcome:?}"
        );
    }

    // -----------------------------------------------------------------------
    // S-2: the child joins its cgroup before it execs.
    // -----------------------------------------------------------------------

    /// `open_cgroup_procs_for_join` creates the directory and opens the file
    /// when a real `cgroup.procs` is there to find — modelled with an
    /// ordinary temp file standing in for the kernel-created one, so this
    /// proves the function's own logic without touching real cgroupfs.
    #[test]
    fn open_cgroup_procs_for_join_finds_a_real_file() {
        let root = std::env::temp_dir().join(format!("rivet-cgroup-root-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let service_dir = root.join("a-service");
        std::fs::create_dir_all(&service_dir).expect("a scratch service directory");
        std::fs::write(service_dir.join("cgroup.procs"), b"").expect("a stand-in cgroup.procs");

        let root_str = root.to_string_lossy().into_owned();
        assert!(
            open_cgroup_procs_for_join(&root_str, "a-service").is_some(),
            "a cgroup.procs that is really there was not opened"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The fallback signal: no `cgroup.procs` for this function to find — not
    /// real cgroupfs, or a hierarchy this process cannot reach — reads as
    /// `None`, which `Kernel::spawn` takes as "fall back to the after-the-fork
    /// join" rather than as a fault.
    #[test]
    fn open_cgroup_procs_for_join_answers_none_with_nothing_to_open() {
        let root = std::env::temp_dir().join(format!("rivet-cgroup-root-missing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let root_str = root.to_string_lossy().into_owned();

        // `create_dir_all` makes the directory; this is not real cgroupfs, so
        // nothing creates a `cgroup.procs` inside it.
        assert!(
            open_cgroup_procs_for_join(&root_str, "a-service").is_none(),
            "answered Some for a directory with no cgroup.procs in it"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// `join_cgroup` fails when the descriptor cannot be written — proven
    /// with a descriptor opened read-only, which `write(2)` refuses with
    /// `EBADF` on any real kernel, real cgroupfs or not.
    #[test]
    fn join_cgroup_fails_on_a_descriptor_that_cannot_be_written() {
        let dir = std::env::temp_dir().join(format!("rivet-join-cgroup-ro-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let path = dir.join("read-only-stand-in");
        std::fs::write(&path, b"").expect("a file to reopen read-only");
        let file = std::fs::File::open(&path).expect("opened for reading, not writing");
        #[allow(unsafe_code)]
        let fd = std::os::fd::AsRawFd::as_raw_fd(&file);

        assert!(
            join_cgroup(fd).is_err(),
            "a write to a read-only descriptor must fail"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// And the success half: a genuinely writable descriptor is written to
    /// without error.
    #[test]
    fn join_cgroup_succeeds_on_a_writable_descriptor() {
        let dir = std::env::temp_dir().join(format!("rivet-join-cgroup-rw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        let path = dir.join("writable-stand-in");
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .expect("opened for writing");
        #[allow(unsafe_code)]
        let fd = std::os::fd::AsRawFd::as_raw_fd(&file);

        assert!(join_cgroup(fd).is_ok());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap_or_default(),
            "0",
            "the self-referring pid was not what was written"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **The real-kernel proof of S-2's fix**: the `pre_exec` write of `0` to
    /// an open `cgroup.procs` descriptor really moves the child, on a REAL
    /// kernel rather than the fake's model of one.
    ///
    /// **Skips itself — passing, but proving nothing, and saying so loudly
    /// on stderr — when this container's cgroupfs will not let a new cgroup
    /// be created or written.** `AGENTS.md` §10.5 warns Docker usually mounts
    /// it read-only, and this repository's own gate is `cargo test`'s exit
    /// status — a hard failure here would fail every run in exactly the
    /// environment where it is expected, which is `AGENTS.md` §7's mistake
    /// in the other direction: reporting a probe's refusal as the thing it
    /// was trying to prove.
    #[test]
    fn a_spawned_child_really_is_in_its_cgroup_before_it_execs() {
        let service = format!("rivet-sysd-test-cgroup-join-{}", std::process::id());
        let dir = format!("{CGROUP_ROOT}/{service}");
        // Whatever an aborted previous run left.
        let _ = std::fs::remove_dir(&dir);

        if open_cgroup_procs_for_join(CGROUP_ROOT, &service).is_none() {
            eprintln!(
                "SKIPPED: this container's cgroupfs at {CGROUP_ROOT} would not open a \
                 cgroup.procs for {service:?} — S-2's fix is UNPROVEN against a real kernel in \
                 this environment. A board, or a container with a writable cgroupfs delegated \
                 to it, is what would prove it."
            );
            return;
        }

        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        let program = Program {
            path: "/bin/sh".to_string(),
            args: vec!["-c".to_string(), "sleep 5".to_string()],
            env: Vec::new(),
            identity: None,
            confinement: None,
            groups: Vec::new(),
            capabilities: Vec::new(),
        };
        let pid = match k.spawn(&service, &program) {
            Ok(pid) => pid,
            Err(e) => {
                eprintln!(
                    "SKIPPED: this container refused to spawn against a real cgroup.procs \
                     descriptor for {service:?}: {e}. S-2's fix is UNPROVEN against a real \
                     kernel in this environment."
                );
                let _ = std::fs::remove_dir(&dir);
                return;
            }
        };

        // Read the cgroup back BEFORE tearing the child down, so what is
        // asserted is what a later supervisor's own `Kernel::observe` would
        // have seen.
        let observed = k.observe(&service);

        // SAFETY: `pid` is this process's own child, just spawned above.
        #[allow(unsafe_code)]
        unsafe {
            libc::kill(pid, libc::SIGKILL);
            libc::waitpid(pid, std::ptr::null_mut(), 0);
        }
        let _ = std::fs::remove_dir(&dir);

        assert_eq!(
            observed,
            Observation::Members(vec![pid]),
            "the child's pid was not in its own cgroup immediately after spawn returned — the \
             pre_exec join did not take effect on this real kernel"
        );
    }

    // -- issuing a leaf from a resting fleet root --------------------------
    //
    // The root is minted by each test into its own scratch directory
    // (self-generated: a counter stands in for `getrandom(2)` in the mint),
    // and the leaf is written to files owned by this test's own uid, which is
    // the one owner an unprivileged test can give.

    /// A fleet root resting in a scratch directory, and an application's data
    /// directory beside it.
    fn resting_root(tag: &str) -> (std::path::PathBuf, IssueOrder) {
        let dir = std::env::temp_dir().join(format!("rivet-issue-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("data")).expect("a scratch directory");
        let request = rivet_core::fleet_root::Request::new(
            "dev-fleet",
            "dev-01",
            &["local".to_string(), "dev-fleet.home.arpa".to_string()],
        )
        .expect("a legal request");
        let mut next = 3u8;
        let mut counting = |len: usize| -> Result<Vec<u8>, String> {
            Ok((0..len)
                .map(|_| {
                    next = next.wrapping_add(29);
                    next
                })
                .collect())
        };
        let minted = rivet_fleet_root::mint(&request, &mut counting, 0).expect("mints");
        std::fs::write(dir.join("fleet-root.crt"), &minted.root.certificate).expect("writes");
        std::fs::write(dir.join("fleet-root.key"), &minted.root.key).expect("writes");
        let (uid, gid) = (unsafe_getuid(), unsafe_getgid());
        let order = IssueOrder {
            root_certificate: dir.join("fleet-root.crt").display().to_string(),
            root_key: dir.join("fleet-root.key").display().to_string(),
            names: minted.names.clone(),
            now: 0,
            targets: vec![IssueTarget {
                dir: dir.join("data").display().to_string(),
                certificate: "tls-cert".to_string(),
                key: "tls-key".to_string(),
                root: Some("tls-root".to_string()),
                uid,
                gid,
            }],
        };
        (dir, order)
    }

    fn unsafe_getgid() -> u32 {
        // SAFETY: `getgid` cannot fail and touches no memory of ours.
        unsafe { libc::getgid() }
    }

    /// **The helper writes the leaf, its key and the root, each owned by the
    /// application and mode `0600`, and the leaf chains to the root** — read
    /// back with `rivet_fleet_root::judge`, and the key read back as the key
    /// the leaf names.
    #[test]
    fn an_issued_leaf_lands_owned_by_the_application_and_mode_0600() {
        use std::os::unix::fs::MetadataExt as _;

        let (dir, order) = resting_root("owned");
        let summary = issue_certificate(&order, &mut getrandom).expect("issues");
        assert!(summary.starts_with("SHA-256 "), "{summary}");

        let data = dir.join("data");
        for name in ["tls-cert", "tls-key", "tls-root"] {
            let meta = std::fs::symlink_metadata(data.join(name)).expect("written");
            assert!(meta.file_type().is_file(), "{name}");
            assert_eq!(meta.mode() & 0o7777, 0o600, "{name}");
            assert_eq!(meta.uid(), order.targets[0].uid, "{name}");
            assert_eq!(meta.gid(), order.targets[0].gid, "{name}");
        }
        let root_pem = std::fs::read_to_string(dir.join("fleet-root.crt")).expect("reads");
        assert_eq!(std::fs::read_to_string(data.join("tls-root")).expect("reads"), root_pem);
        let root = rivet_fleet_root::judge::Root::from_pem(&root_pem).expect("the root reads");
        let leaf = std::fs::read_to_string(data.join("tls-cert")).expect("reads");
        assert!(matches!(
            rivet_fleet_root::judge::judge(&leaf, &root, 0),
            rivet_fleet_root::judge::Verdict::Current { .. }
        ));
        // Nothing is left beside the three names.
        let mut left: Vec<String> = std::fs::read_dir(&data)
            .expect("lists")
            .map(|e| e.expect("an entry").file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["tls-cert", "tls-key", "tls-root"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A link planted at a name, or at the fresh file's name, is replaced
    /// and never written through** — the data directory belongs to the
    /// application, so either could be one.
    #[test]
    fn an_issued_leaf_replaces_a_planted_link_and_does_not_follow_it() {
        let (dir, order) = resting_root("links");
        let data = dir.join("data");
        let victim = dir.join("victim");
        std::fs::write(&victim, b"untouched").expect("writes");
        std::os::unix::fs::symlink(&victim, data.join("tls-key")).expect("a link");
        std::os::unix::fs::symlink(&victim, data.join(".tls-cert.issuing")).expect("a link");

        issue_certificate(&order, &mut getrandom).expect("issues");

        assert_eq!(std::fs::read(&victim).expect("reads"), b"untouched");
        for name in ["tls-cert", "tls-key"] {
            let meta = std::fs::symlink_metadata(data.join(name)).expect("written");
            assert!(meta.file_type().is_file(), "{name} is still a link");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// **A key that is not the root's issues nothing and writes nothing.**
    #[test]
    fn an_issue_with_the_wrong_key_writes_nothing() {
        let (dir, order) = resting_root("wrong-key");
        let (other, other_order) = resting_root("wrong-key-other");
        let order = IssueOrder {
            root_key: other_order.root_key.clone(),
            ..order
        };
        let why = issue_certificate(&order, &mut getrandom).expect_err("refused");
        assert!(why.contains("not the key"), "{why}");
        assert_eq!(std::fs::read_dir(dir.join("data")).expect("lists").count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&other);
    }

    /// **Through a real fork**: the helper's answer comes back over the pipe,
    /// the child is reaped, and the files are there when the call returns.
    #[test]
    fn issue_in_child_round_trips_through_a_real_fork() {
        let (dir, order) = resting_root("fork");
        let mut k = LibcKernel::new(ConsoleForm::Jsonl);
        let answer = k.issue_in_child(&order).expect("forks");
        assert!(matches!(&answer, Ok(summary) if summary.starts_with("SHA-256 ")), "{answer:?}");
        assert!(dir.join("data").join("tls-cert").exists());

        let refused = IssueOrder {
            root_key: dir.join("absent.key").display().to_string(),
            ..order
        };
        let answer = k.issue_in_child(&refused).expect("forks");
        assert!(matches!(&answer, Err(why) if why.contains("could not be read")), "{answer:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
