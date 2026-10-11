# Supervisor blocking audit — `rivet-sysd`, 2026-09-16

* **Status:** Proposal, not a decision. Written to serve `.rulings/` R-195 D4, which
  places this audit between `ADR-056` and any instrument build.
* **Scope:** reading only. No code, configuration or document outside this file was
  changed, and nothing was run on a board. `.rulings/` R-195 D1 forbids behaviour
  changes until the failure is reproduced under instrument; §8 below describes
  repairs and builds none of them.
* **What it audits against:** `.rulings/` R-195 D5 — the supervisor cannot be
  blocked by a service, a device or a filesystem; every wait it makes is bounded,
  and nothing it supervises can make it wait forever.

---

## 1. Verdict

**The invariant is not held.** Of twenty-five distinct waits reachable from
`rivet-sysd`, **thirteen have no bound of any kind** — no timeout, no deadline, no
non-blocking descriptor, no `WNOHANG` — and four more are bounded per-operation
while being unbounded per-request or per-pass. Eight are properly bounded, and the
three that are bounded *well* (`drain_kmsg`'s per-wakeup cap, the ingest socket's
in-flight cap and deadline, `reap_ready`'s `WNOHANG`) are the pattern the rest of
the crate should be measured against. The single worst site is
`Kernel::run_to_completion` (`cmd/rivet-sysd/src/kernel.rs:2906`), which is
`std::process::Command::output()` with no timeout anywhere in it: it reads a
child's two pipes to end-of-file and then waits for the child to exit, so any
program the supervisor runs this way stops the machine for as long as it likes.
It is reached on the stop edge of a display-owning service
(`cmd/rivet-sysd/src/supervise.rs:4077` → `cmd/rivet-sysd/src/splash.rs:122`) and
when a declared git repository is created
(`cmd/rivet-sysd/src/repos.rs:321`). The second worst is that every journal record
the supervisor writes performs an unbounded `write_all` to its own stdout
(`kernel.rs:2670`) and an unbounded `write_all` to the card (`kernel.rs:3081`);
the first of those is kept harmless not by anything in this crate but by PID 1
pointing fd 1 at `/dev/null` before it spawns the supervisor
(`cmd/rivet-initd/src/kernel.rs:764`), on a branch that a trial boot does not
take.

**None of the thirteen is established as the cause of the 2026-09-16 incident**,
and §7 says for each what would confirm or kill it. Two are ruled out by the
evidence already in hand, one is ruled out by a reading taken off the board while
`ADR-056` was written, and the rest need either the button of `ADR-056` §3 or one
line read off `rivet-01`.

**§10 was written after the rest of this document, against Fleet Monitor's stored
copy of the incident, and it supersedes §7's ranking.** Three of the five
hypotheses are dead — H1, H2 and H5 — and the failure is located: **it is confined
to `rivet-sysd`, and it is total**, because every question the machine could answer
without its supervisor was answered normally for eleven minutes after the
supervisor stopped, and every question that needed the supervisor timed out. §10.6
also adds a **fourth route** to the privilege problem §6.2 leaves open, which §6.2
did not consider and which costs nothing.

**§11 SETTLES IT, AND IT IS THE SECTION TO READ FIRST.** On 2026-09-18 the failure
happened again, under instrument, and the capture names the site in one word:
**`wchan` `unix_wait_for_peer`, syscall `203` — `connect` on `aarch64`.** The
supervisor was blocked **making a connection**, on a blocking descriptor, with no
bound. That is **B10, `ask_health`'s connect**, and it kills the two hypotheses §10
left standing: H3 predicted a `write` and H4 a filesystem wait.

**This document ranked B10 fourth of five and called the repair's value cosmetic**
— §8.4 says *"the numbers make that wait nearly unreachable"*. **That estimate is
false.** The wait was reached twice in five days, at 34 h 08 m and 34 h 09 m of
uptime, ninety seconds apart. The audit reached it by accepting the same inverted
reasoning the code's own comment offers (§11.4), which is the mistake worth
carrying forward: **an audit that reads a comment's argument instead of the code's
behaviour inherits the comment's error.**

---

## 2. Method, and how much of it survived reading

Two searches, then reading.

* A syscall-shaped search over `cmd/rivet-sysd/src/` for `poll`, `read`, `write`,
  `connect`, `accept`, `recv`, `ioctl`, `waitpid`, `fsync`/`sync_all`,
  `File::open`/`OpenOptions`, `read_dir`, `Command::new`/`spawn`/`output`/`wait`,
  channel `recv`, `Mutex`/`RwLock`, `thread::spawn`, `thread::sleep`,
  `create_dir_all` and `remove_dir_all` produced **157 hits, of which 100 are
  outside a `#[cfg(test)]` module.**
* A seam search: the `Kernel` port declares **53 methods**
  (`kernel.rs:600`–`kernel.rs:1500`), and there are **159 non-test call sites** of
  the form `k.<method>(` across seventeen modules.

**Every one of the 259 candidates was read.** Twenty-five distinct blocking sites
survived; the rest were duplicate spellings of the same site, pure computation, or
test scaffolding.

Two facts about the crate make the audit tractable and are worth stating because
they are the reason the list is this short:

* **All I/O in non-test code goes through the `Kernel` port.** A search of every
  module other than `kernel.rs`, `landlock.rs` and `display_hours.rs` for
  `std::fs::`, `std::process::`, `std::net::`, `UnixStream`, `UnixListener`,
  `std::thread` and the print macros returns hits only inside test modules. The
  seam claimed by `cmd/rivet-sysd/src/main.rs:34` holds in fact.
* **There is no lock anywhere in the crate.** `Mutex`, `RwLock` and `OnceLock`
  appear nowhere, in test code or out of it. Mutex acquisition is therefore not a
  blocking shape this supervisor has, and the one place a lock could be taken is
  the child's allocator between `fork` and `exec` — `ADR-056` §7.2's suspect,
  which is site **B2** below.

`rivet-core`'s journal ingest was read as the ruling's brief asks. It is one of
the well-bounded parts: `INGEST_MAX_IN_FLIGHT` is 16, `INGEST_DEADLINE_MS` is
2000, `INGEST_MAX_BYTES` is 16 KiB per line and `INGEST_UNPARSED_MAX_BYTES` is 1
KiB (`rivet-core/src/journal.rs:1480`–`rivet-core/src/journal.rs:1520`), with a
per-uid token bucket of 64 burst and 8 per second over at most 32 tracked uids
(`rivet-core/src/journal.rs:1557`–`rivet-core/src/journal.rs:1585`). Nothing in
the ingest path can hold the supervisor. What it *can* do is amplify: every
admitted record becomes a journal record, which becomes site **B4** and site
**B5**.

---

## 3. The main loop, stated once

`cmd/rivet-sysd/src/main.rs:1043`–`main.rs:1260` is the whole of it. Per pass:

1. `main.rs:1063` computes the soonest of the supervisor's deadline and the
   clock's; `main.rs:1064` calls `Kernel::wait`, which is **B1**.
2. Each returned `Event` is dispatched. `Event::Queried` writes an answer
   (**B9**); `Event::KioskAsked` may open and write a display channel;
   `Event::Output`, `Event::KernelMessage` and `Event::Ingest` each write a
   journal record (**B4**, **B5**).
3. `main.rs:1221` reaps (**B16**).
4. `main.rs:1228`–`main.rs:1240` polls five `/state` watches — `deploy_request`,
   `uninstall_request`, `repo_delete_request`, `installed`, `carried` — which is
   **B21**.
5. `main.rs:1241` converges, which may start a service (**B2**, **B17**, **B18**,
   **B25**), stop one (**B3** through the splash), or ask a health question
   (**B10**).
6. `main.rs:1253` evaluates the display window (**B22**), `main.rs:1258` ticks the
   clock (**B6** once an hour), `main.rs:1259` reports disk trouble.

Everything before `main.rs:1043` runs once per supervisor start and is marked
*boot only* in the table.

---

## 4. Every blocking site

`State` is the letter `/proc/<pid>/status` would show while the supervisor is
stuck there. `utime`/`stime` advance only where the wait burns CPU.
`vol` is `voluntary_ctxt_switches`.

| # | Where, and in what | What it waits on, and who can make that long | Bounded? | How it looks from outside | 2026-09-16? |
| :-- | :-- | :-- | :-- | :-- | :-- |
| **B1** | `kernel.rs:3327`, `libc::poll` inside `LibcKernel::wait` | the signal descriptor, every notify socket, every capture pipe, the ingest/query/kiosk listeners, outstanding health sockets, `/dev/kmsg`; plus a timeout | **Yes, by design** — `-1` only when `main.rs:1063` yields `None`, which needs no backoff, no stop deadline, no adopted poll, no readiness deadline and **no running application** (`supervise.rs:1727`) | `S`, `wchan` `do_sys_poll`, `utime`/`stime` frozen, `vol` advances once per wakeup. **This is also what a healthy idle supervisor looks like** | **Cannot be told from a wedge by `State` alone.** §6 Q3 |
| **B2** | `kernel.rs:4224`, `cmd.spawn()` in `Kernel::spawn`, reached from `supervise.rs:3537` | the child's close-on-exec status pipe, until the child `execve`s or reports an error. A child that deadlocks in `malloc` inside `clearenv`/`setenv` (`kernel.rs:4154`, `kernel.rs:4158`), or whose `execve` stalls reading the program off `/state` | **None** | `S`, parked in a pipe read, counters frozen, `vol` frozen | **Ruled OUT for the malloc path** — see §5. Not ruled out for the `execve` path |
| **B3** | `kernel.rs:2906`, `cmd.output()` in `Kernel::run_to_completion` | the child's stdout and stderr to end-of-file, **then** `waitpid` with no options. Any program that never exits, or that forks a grandchild holding the pipes | **None at all.** No timeout, no deadline, no poll | `S`, in a pipe read then in `do_wait`, counters frozen | Reachable only if a display-owning service stopped with no arbiter running (`supervise.rs:4064`), or a repository was created. **Undecidable from reading** |
| **B4** | `kernel.rs:2670`–`kernel.rs:2672`, `write_all` + `flush` on `std::io::stdout()` in `LibcKernel::emit`, **once per journal record** | whatever PID 1 left on fd 1. `cmd/rivet-initd/src/kernel.rs:1187` spawns the supervisor with inherited stdio, so it is `/dev/null` **only if** `cmd/rivet-initd/src/splash.rs:126` took the console — which it does not on a trial boot, nor when `in_trial_boot` errors, nor if `/dev/null` could not be opened (`cmd/rivet-initd/src/kernel.rs:765`) | **None in this crate.** The bound is PID 1's redirection, taken elsewhere, conditionally | `S`, in a tty write, counters frozen | **Undecidable from reading.** One line off the board settles it — §7 H3 |
| **B5** | `kernel.rs:3081`, `write_all` in `Kernel::append_file`, **once per journal record** | the card. `ADR-022` §8 rules out an `fsync` per record, so this is a page-cache write — which still blocks when the cache must be written back | **None** | `D` if the card stalls, `S` otherwise; `vol` frozen | Weakened by the evidence — TempMon's store advanced for five hours |
| **B6** | `kernel.rs:2705`, `kernel.rs:2717`, `sync_all` in `sync_file`/`sync_dir` | the card, uninterruptibly. Reached by `durable::commit` (`durable.rs:87`) and by the hourly clock write through `clock.rs:173` and `clock.rs:181`, and directly at `deploy_request.rs:481` | **None** | **`D`**, `wchan` in the block layer, counters frozen, `vol` frozen. The one site that produces an uninterruptible state | `WRITE_INTERVAL_MS` is one hour from boot (`clock.rs:117`), so the hourly write lands at a fixed minute past each hour. **Worth checking against 20:16:46Z** — §7 H5 |
| **B7** | `kernel.rs:2237`, `read_to_string` on the accepted kiosk stream after `set_nonblocking(false)` at `kernel.rs:2196` | the launcher's uid. `SO_RCVTIMEO` is `KIOSK_PATIENCE`, 500 ms (`kernel.rs:511`) | **Per read only.** `REQUEST_MAX_BYTES` is 256, so a peer dribbling one byte per 499 ms holds the supervisor ~128 s per connection, and back-to-back connections make that indefinite | `S`, in a unix socket read, counters frozen | `rivet-01` has a launcher; nothing suggests it dribbled. **Low** |
| **B8** | `kernel.rs:2303`, the same shape on the query socket, deadline set at `kernel.rs:2299` | root's client — in practice `rivet-mcpd`. Same 500 ms per read, same 256-byte request | **Per read only**, as B7 | `S`, counters frozen | **Low**, and it cannot hold for eleven minutes without repeating |
| **B9** | `kernel.rs:3515` and `kernel.rs:3530`, `write_all` in `answer_kiosk`/`answer_query` | the peer draining the socket. **No `SO_SNDTIMEO` is ever set**, and the stream was switched to blocking at `kernel.rs:2196`/`kernel.rs:2299` | **None.** Bounded in practice only by the socket send buffer being larger than the answer — which is an assumption about `Status::render`'s length, not a mechanism | `S`, in a unix socket write, counters frozen | Requires a client that asks and stops reading. **Undecidable from reading** |
| **B10** | `kernel.rs:3704`, `UnixStream::connect` in `ask_health` — **on a blocking socket**, because `set_nonblocking(true)` is the *next* line (`kernel.rs:3705`) | the application's listen backlog. The comment above it says *"Non-blocking from the first syscall"*, and the code does not do that | **None.** The practical bound is elsewhere: one question outstanding per service (`kernel.rs:3694`) and retirement after `PATIENCE_MS` = 2000 (`kernel.rs:2057`), so at most one connection queues against a backlog of 128 | `S`, in `unix_stream_connect`, counters frozen | **Very low** on the numbers, but the site is a live contradiction between a comment and its code |
| **B11** | `kernel.rs:2226`, `recv_with_fd` — the first kiosk read, which must be a `recvmsg` to keep the ancillary descriptor | as B7 | **`SO_RCVTIMEO` applies**, 500 ms | `S` | **Low** |
| **B12** | `kernel.rs:2470`–`kernel.rs:2478`, `Capture::drain`'s `loop` over a non-blocking pipe | a service writing to its stdout faster than the supervisor reads it. The loop exits on `WouldBlock`, which is a **condition, not a deadline** | **No deadline.** The descriptor is `O_NONBLOCK` (`kernel.rs:2415`), so no single read blocks | `R`, **`utime`/`stime` advancing**, `vol` frozen or slow | **Ruled OUT.** The journal stopped; this state writes a record per line |
| **B13** | `kernel.rs:2536`, `drain_notify`'s `loop` over a non-blocking datagram socket | as B12, on `sd_notify` datagrams | **No deadline**, same shape | `R`, counters advancing | **Ruled OUT**, for B12's reason |
| **B14** | `kernel.rs:2374`, `drain_kmsg` | `/dev/kmsg`, non-blocking (`kernel.rs:3141`) | **Yes — `PER_WAKEUP` is 64.** The model the rest of the crate should follow | `R` briefly | Out |
| **B15** | `kernel.rs:2110`, `drain_ingest`'s accept loop and per-connection read | any local uid — the socket is `0666` (`rivet-core/src/journal.rs:1468`) | **Yes** — 16 in flight, 2 s deadline, non-blocking streams (`kernel.rs:2120`), and the listener leaves the poll set when full (`kernel.rs:2340`) | `R` briefly | Out |
| **B16** | `kernel.rs:4308`, `child.try_wait()` in `reap_ready` | nothing | **Yes — `WNOHANG`** | — | Out |
| **B17** | `kernel.rs:2679`, `read_file_bytes`, called three times per application launch from `verify.rs:168`, `verify.rs:179` and `verify.rs:227` | the card. The third reads the whole program to hash it — up to `MAX_PROGRAM_BYTES`, 16 MiB (`rivet-core/src/package/application.rs:130`) — on the supervisor's own thread, at **every** launch, per `ADR-015` §4.2 | **Size-bounded, not time-bounded** | `D` on a stalled card, otherwise `R` with `utime` advancing through the hash | Only on a start. No application started at 20:16 as far as the evidence says |
| **B18** | `landlock.rs:561`, `libc::open(path, O_PATH)` per confinement rule, built in the parent before `fork` (`kernel.rs:3936`) | path resolution on `/system` and `/state`. `O_PATH` means a FIFO or device node at the path cannot block — that hazard is closed — but a stalled filesystem still can | **None** | `D` on a stalled filesystem | Only on a start |
| **B19** | `kernel.rs:4946`, `kernel.rs:5012`, `kernel.rs:5034` — `libc::ioctl` on the card, and `kernel.rs:4978` `sync_all`, inside `grow_state`, called once from `main.rs:814` | the SD card and the block layer | **None** | `D` | **Boot only**, before the loop. Cannot explain a wedge twenty minutes in |
| **B20** | `kernel.rs:2968`, `libc::mount` for the `/etc` tmpfs, from `main.rs:917` | the kernel; tmpfs has no backing device | **None**, and nothing outside can lengthen it | `D` briefly | Boot only |
| **B21** | `deploy_request.rs:292`, `uninstall_request.rs:63`, `repo_delete_request.rs:70` — a `stat(2)` each; `carried.rs:255` — a `readdir` of `/state/carried`; all four **on every convergence pass** through `main.rs:1228`–`main.rs:1240` | the card | **None** | `D` on a stalled card | This is the tightest coupling between the loop and the filesystem: **the supervisor touches `/state` at least four times per pass whether or not anything is happening** |
| **B22** | `display_hours.rs:182`, `std::thread::spawn` around `rivet_adapter_cec`'s blocking ioctls | a television over an HDMI wire | **The ioctl is off the loop since 2026-09-03.** The loop only does `try_recv` (`display_hours.rs:219`) | The supervisor stays parked; the leaked thread is `S` in an ioctl | **Ruled OUT** — see §5 |
| **B23** | `kernel.rs:3185`, `thread::sleep` in the no-signalfd degraded path | nothing | **Yes — `DEGRADED_POLL_MS` is 200** | `S` | Out; `arm_signals` succeeded |
| **B24** | `kernel.rs:2826`, `read_dir` inside `share_tree`'s recursion | the card, and a tree of unbounded depth | **None** | `D` on a stalled card | Repository convergence, boot only |
| **B25** | `kernel.rs:2730`, `remove_dir_all`, reached from `supervise.rs:2389` — the scratch directory is **emptied before every spawn** — and from an uninstall | the card, and however much the application wrote into its scratch directory | **None** | `D` on a stalled card | Only on a start |

---

## 5. What the evidence rules out by itself

Three of the twenty-five are settled without any instrument.

**B22, the CEC ioctl, is out.** `Television::converge` returns before it can spawn
anything when the document names no display window (`display_hours.rs:129`), and
`rivet-01`'s document carries no `[display]` table — read off the board while
`ADR-056` was written (`ADR-056` §7.2). Even with a window, the thread is spawned
only when the lit/dark answer *changes* (`display_hours.rs:142`) and at most one
is in flight (`display_hours.rs:152`). The supervisor's own pass does `try_recv`
and nothing else.

**B2's allocator path is out, and B22 is why.** The safety argument for calling
`setenv` between `fork` and `exec` (`kernel.rs:4154`) is conditional on the parent
being single-threaded, and a search of the crate for `thread::spawn` returns
`display_hours.rs:182` and nothing else outside a test module. With no
`[display]` table that thread cannot exist, and the board
answered `Threads: 1`. **The latent defect is real and should be closed on its own
account** — a child deadlocked in `malloc` inside `pre_exec` stops the parent in
`spawn`'s status-pipe read for ever — but it is not what happened here. B2's other
path, an `execve` that stalls reading the program, is not ruled out by anything.

**B12 and B13, the two condition-loops, are out.** Both keep the supervisor
running and both produce journal records as fast as they read. The journal stopped
mid-stride; a supervisor spinning in either of these would have filled it.

Two things the evidence does **not** rule out, contrary to how they read at first:

* **`/state` is not fully exonerated.** That TempMon's SQLite store advanced for
  five hours says the filesystem accepted *its* writes. It does not say that a
  particular `fsync` (**B6**) or a particular inode's writeback (**B5**) did not
  stall. It makes the card-stall reading much weaker, which is what `ADR-056` §1
  concluded, and it does not close it.
* **`ADR-056` §2.5's argument does not cut the way it is usually read.**
  `khungtaskd` reports only uninterruptible tasks, so it would have said nothing
  about an `S`-state wedge — that is §2.5's point and it stands. But it also means
  that **the absence of a hung-task report is not evidence that no `D`-state wait
  occurred**, because the only thing copying the kernel ring buffer to disk is
  `kmsg.rs`'s ingest, which sits in the wedged supervisor's own poll set
  (`kernel.rs:3297`), and the board was power-cycled. A `D`-state stall in B5 or
  B6 would have produced a hung-task report into a buffer nobody could read.

---

## 6. The four questions `ADR-056` §3.8 asks

### 6.1 Is a bare `State` letter enough?

**No, and the audit gives a sharper answer than "no": it is necessary and not
sufficient, and it makes exactly one cut.**

Twelve of the thirteen unbounded sites land in interruptible sleep. `poll` (B1),
a pipe read inside `spawn` (B2) or `output()` (B3), a tty write (B4), a unix
socket read or write (B7, B8, B9), a unix `connect` (B10) — every one is `S`, and
every one is a different diagnosis.

The cut `State` does make is worth keeping: **`D` means the card, and nothing else
in this crate produces `D`.** Sites B5, B6, B17, B18, B19, B21, B24 and B25 are
the filesystem, and B6 — the two `fsync` calls in `durable::commit` — is the one
that is uninterruptible by construction rather than only when the medium stalls.
So the letter separates *the card* from *everything else* on sight, which is what
`ADR-056` §6 already claims for it. It cannot separate *parked and healthy* from
*stopped in a pipe*.

`Threads` keeps its value unchanged: it is a one-line test of `kernel.rs:4154`'s
safety precondition, and §5 shows it already killed a suspect.

### 6.2 Are `wchan` and `stack` load-bearing?

**Yes for the diagnosis, no for the detection — and that distinction is the whole
answer, because it is what decides whether §2.2's privilege question is forced.**

* **For detection** — *has the supervisor stopped?* — `wchan` adds nothing. The
  comparison `ADR-056` §6 names does the work: cumulative `utime`, `stime` and
  `voluntary_ctxt_switches` against `/proc/uptime`. Frozen counters against hours
  of board uptime is the fact nothing on that machine could state, and it needs no
  privilege beyond §3.4's leaf grant.
* **For diagnosis** — *where is it stopped?* — `wchan` is the difference between
  five distinct one-word answers: `do_sys_poll` (B1, and probably healthy),
  a pipe read (B2 or B3), `unix_stream_read_generic` or its write twin (B7–B9),
  `unix_stream_connect` (B10), and a tty write (B4). Nothing else the sandbox can
  reach distinguishes them. `fdinfo`, `syscall` and `stack` are all gated the same
  way `wchan` is, and the one non-privileged discriminator that exists — the
  identity of the far end — needs a directory grant over `/proc` that
  `rivet_core::confinement` refuses on the record (`ADR-056` §2.2).

**So the privilege question is forced only if the button must answer *where*.**
This audit's recommendation is to split the page rather than to widen
`rivet-fleetd`: build the detection half now, within `ADR-015` §5.1's constraint
and §3.4's two-leaf grant, and state on the page in as many words that it can say
*the supervisor has stopped* and cannot say *where*. That is not a fudge — it is
the honest reading of §6's own table, where four of the five useful rows are
counters.

If *where* is wanted anyway, the three routes are: give `rivet-fleetd` root or
`CAP_SYS_PTRACE` (breaks `ADR-015` §5.1 for every application, since the
mechanism is per-application and the precedent is not); build a second, root,
non-application reader with no control-plane dependency (a new component, a new
uid question, and it must itself be unwedgeable); or **remove the ambiguity
instead of observing it** — §8's repairs, most of which turn an `S` with no
diagnosis into a bounded wait with a journal record. The third gets cheaper the
longer it is left; the other two get dearer.

### 6.3 Which counters are the right liveness witness?

**`voluntary_ctxt_switches` is the right one on this machine, for a reason that
must be written down because it is a property of the configuration rather than of
the code — and `ADR-056` §3.5's one-second sampling window is too short to read
it.**

* **Why it is sound here.** `Supervisor::next_timeout_ms` returns `Some` whenever
  any application is in `State::Started`, because `ADR-045` §3's health cadence is
  folded into it (`supervise.rs:1727`) at `INTERVAL_MS` = 30 000
  (`rivet-core/src/health.rs:197`). `rivet-01` runs four applications. So a healthy
  supervisor parks and re-parks **at least twice a minute**, and `vol` advances.
* **Where it stops being sound.** On a machine with no application in
  `State::Started`, `next_timeout_ms` returns `None` (`supervise.rs:1701`),
  `soonest` returns `None` (`main.rs:1304`), and `libc::poll` is called with `-1`
  (`kernel.rs:3322`). A healthy idle supervisor then has frozen counters
  indefinitely, and the witness says nothing. The page must not present the
  measure as universal.
* **The correction §3.8 asked for.** `utime` and `stime` are in clock ticks of 10
  ms. A supervisor doing two wakeups a minute burns far less than one tick in a
  second, and `vol` will very often show **zero delta over a one-second window on a
  perfectly healthy machine**. A page that took two samples a second apart and
  reported the difference would read as *stopped* on an idle board. **The
  comparison that works is the absolute one** — cumulative counters against
  `/proc/uptime`, which §3.5 already lists and §6 already relies on. If a delta is
  wanted as well, the window has to be longer than the slowest guaranteed cadence,
  which on this machine is 30 s, and a page a person waits 30 s for is a different
  page. The recommendation is: **report the absolute counters and the board
  uptime, say what the ratio means, and drop the one-second delta** — or keep it
  and label a zero as uninformative.

### 6.4 Is `ShdPnd` reliable?

**It is a corroborator, not a witness, and its zero means nothing.**

`arm_signals` blocks `SIGTERM`, `SIGINT`, `SIGCHLD` and `SIGHUP`
(`kernel.rs:1636`) and reads them from a `signalfd` (`kernel.rs:3114`). A blocked
signal that has been generated and not read shows in `ShdPnd`, so `ShdPnd`
carrying `SIGCHLD` while the counters are frozen is a direct witness that the loop
is not draining its signal descriptor — which is what §3.5 claims and it is
correct.

The two limits:

* **Ordinary states in which it is set.** Every child exit generates `SIGCHLD`,
  and the window between generation and the next `poll` return is up to the
  sleep — 30 s on this machine. A page drawn inside that window on a healthy
  supervisor shows it set.
* **A zero is not evidence of health.** Nothing sends the supervisor a signal on a
  quiet machine. A supervisor wedged at 20:16 with no child exiting afterwards
  would show `ShdPnd` clear for the whole five hours. The one incident the
  instrument exists for is the case where this field would have said nothing.

So: show it, in one sentence, **paired with the counters**, and word it so a zero
does not read as reassurance.

---

## 7. Ranked hypotheses

Ranked by how plausibly each explains all three incidents — 2026-09-11 23:05
local, 2026-09-14 about 06:43 local, and 2026-09-16 15:16 local — not the last one
alone. A hypothesis that needs a rare trigger has to explain why the trigger fired
three times in five days.

### H1 — The supervisor is blocked writing its own journal to a console that stopped draining (B4)

**The claim.** `LibcKernel::emit` writes every record to fd 1 with a blocking
`write_all` and a `flush` (`kernel.rs:2670`). If fd 1 is `/dev/console` rather
than `/dev/null`, every record is a tty write; a console that stops consuming
stops the supervisor inside `emit`, which is inside `Journal::emit`
(`journal.rs:347`), which is inside whatever the loop was doing. **The journal
would stop mid-stride with no error, which is exactly the observed signature**,
and it explains the eleven-minute delay: `rivet-mcpd` writes a line per connection
into a capture pipe the supervisor has stopped draining, so it dies by
`ADR-056` §7.1's mechanism some minutes later, while quiet applications carry on
for hours.

**Why it is first.** It needs no unusual event at all — no start, no stop, no
install, no operator — which every other hypothesis here does. It fires on the
next record,
whenever that is, which is why it could fire three times in five days at
unrelated hours. It also matches "the journal stops and nothing else does".

**What makes it possible.** Nothing in `cmd/rivet-sysd` bounds this write. The
protection is entirely PID 1's: `splash::maybe_draw` calls `quiet_console`
(`cmd/rivet-initd/src/splash.rs:126`) which points fd 1 and fd 2 at `/dev/null`
(`cmd/rivet-initd/src/kernel.rs:764`) **before** `Pid1::run_forever` spawns the
supervisor with inherited stdio (`cmd/rivet-initd/src/main.rs:251`,
`cmd/rivet-initd/src/kernel.rs:1187`). That happens only on the `Ok(false)` arm —
not on a trial boot, not when `in_trial_boot` returns `Err`, and
`discard_own_output` can itself fail.

**What would confirm or kill it, and it is one line.** Read the supervisor's
`/proc/<pid>/fd/1` — or, without the button, read the boot's own record:
PID 1's `say` also writes to `/dev/kmsg` (`cmd/rivet-initd/src/kernel.rs:801`),
which the supervisor ingests, so `rivet get log latest all` filtered for
`console taken` carries `output_discarded` for that boot. **If it reads `true`,
H1 is dead and can be struck from the list.** If it reads `false`, or the line is
absent, H1 becomes the leading hypothesis and the repair is two lines.

**What this audit could not do.** That command needs the board's pin, which is
derived from a `*.seed` file this audit was forbidden to read. The reading is
cheap and it is the single highest-value observation on this list.

### H2 — The supervisor is blocked in `run_to_completion` (B3)

**The claim.** `Command::output()` (`kernel.rs:2906`) has no bound of any kind.
`splash::restore` runs `rivet-splash` through it on the stop edge of a
display-owning service when no arbiter holds the screen (`supervise.rs:4064`,
`splash.rs:122`), and `rivet-splash` opens a device node and draws. A draw that
does not return stops the supervisor for ever.

**Why it is second.** The mechanism is beyond doubt — it is the worst-bounded
call in the crate and the 2026-09-03 audit already named it — but the trigger is
not ordinary. It needs a display-owning service to stop at a moment when the
launcher is not running, three times in five days.

**What would confirm or kill it.** The journal's last records before each stop.
A `release` on a display-owning service emits nothing by itself, but the stop that
caused it does, and `splash::restore`'s own three outcomes each emit a record
(`splash.rs:123`, `splash.rs:133`, `splash.rs:142`) — **so the presence of a stop
record with no splash outcome after it is the signature.** Its absence kills H2 for
that incident. The button would show `wchan` in a pipe read, if `wchan` is ever
available.

### H3 — The supervisor is blocked writing an answer nobody is reading (B9)

**The claim.** `answer_query` (`kernel.rs:3530`) does a blocking `write_all` with
no send timeout onto a stream that was deliberately switched to blocking
(`kernel.rs:2299`). A client that sends `get status` and then stops reading —
killed, suspended, or itself wedged — leaves the supervisor in that write once the
answer exceeds the socket's send buffer.

**Why it is third.** It needs the answer to be larger than the send buffer, which
is a few hundred kilobytes; `Status::render` on this machine is very unlikely to
be. But it also needs only a client that stops reading, and `rivet-mcpd` is
exactly the process that queries and exactly the process that dies first under
`ADR-056` §7.1 — which makes an ordering worth checking rather than dismissing.

**What would confirm or kill it.** Measure `Status::render`'s length on
`rivet-01` and compare it with the socket's `SO_SNDBUF`. If the answer is orders
of magnitude smaller, H3 is dead as a first cause and remains as a latent defect.

### H4 — An `execve` or a start-path filesystem read stalled (B2, B17, B18, B25)

**The claim.** A start does five unbounded filesystem things before the child
runs: emptying the scratch tree (`supervise.rs:2389`), three reads in
`verify.rs` including a 16 MiB program hash, opening confinement directories
(`landlock.rs:561`), and the `execve` itself behind `spawn`'s status pipe.

**Why it is fourth.** It needs a start, and the evidence says nothing was
starting. **What would confirm or kill it:** the journal's last record before each
stop. A start emits records; if the last record is not about a start, this is out
for that incident.

### H5 — The hourly clock write's `fsync` stalled (B6)

**The claim.** `Clock::tick` runs at the bottom of every pass (`main.rs:1258`)
and writes once an hour (`clock.rs:117`, `clock.rs:544`), through
`durable::commit`, which calls `sync_file` and then `sync_dir`
(`clock.rs:173`, `clock.rs:181`). Both are `fsync(2)`, both uninterruptible.

**Why it is fifth and not lower.** The interval is monotonic from boot, so the
write lands at a fixed minute past every hour, and the journal's last record at
**20:16:46Z** is a time worth testing against the boot record. **Why it is not
higher:** `/state` accepted TempMon's writes for five hours afterwards, and a
supervisor in `D` would be a supervisor the button would identify instantly by
`State` alone.

**What would confirm or kill it.** The boot's start record carries its own
timestamp; if `20:16:46Z` minus the boot time is not close to a whole number of
hours, H5 is dead. That arithmetic needs the previous boot's journal, which is on
the card.

### What cannot be decided by reading at all

Stated plainly, because `.rulings/` R-195 exists to stop phantoms being chased:

* **Which of the five `S`-state sites the supervisor was actually in.** Every one
  looks identical from `/proc/<pid>/status`. Only `wchan` or `stack` separates
  them, and neither is reachable from an application today.
* **Whether fd 1 was `/dev/null` on those boots.** One line off the board.
* **Whether the card stalled for the supervisor's inode while serving TempMon's.**
  Not answerable from the tree, and probably not answerable after the fact at all.
* **What the last records before each stop were.** They are on the card, in the
  rotated series, and they decide H2, H4 and H5 between them. This audit read no
  board journal.

---

## 8. Repairs — described, not built, with the risk of each

`.rulings/` R-195 D1 forbids merging any of this before a witnessed incident. They
are written here so the grilling session D4 allows has something concrete to
argue with, and they are ordered by how much of the invariant each buys per line
changed.

### 8.1 Bound `run_to_completion`, or take it off the loop (B3)

**What.** Either give it a deadline — spawn, put both pipes in the existing poll
set, kill the child when the deadline passes — or make it asynchronous the way
`display_hours.rs:182` made the CEC send asynchronous, with the outcome collected
on a later pass.

**Risk.** The asynchronous form is the pattern the crate already has and the one
whose cost is understood; it also violates the single-thread precondition at
`kernel.rs:4154` if it is done with a thread, which is why the poll-set form is
better here even though it is more code. The deadline form must decide what to do
with a splash that did not finish — and *nothing* is an acceptable answer,
because `splash.rs:133` already handles a splash that refused to draw.

**Weight.** One site, thirteen lines of unbounded wait, no behaviour change on any
path that currently works.

### 8.2 Decide what the supervisor's stdout is, in the supervisor (B4)

**What.** The supervisor should not depend on PID 1 having redirected fd 1. Either
open its own console descriptor `O_NONBLOCK` and accept that records are dropped
when it would block — which is what a console is for — or drop the console write
entirely on a machine where the journal reached the card, keeping it only for the
pre-`/state` window `main.rs:360` describes.

**Risk.** This is the one repair that removes a diagnostic the project has
deliberately kept: `main.rs:360` argues that the console is what a maintainer with
an HDMI capture sees when `/state` is the thing that failed. A non-blocking
console keeps that and loses records under pressure; removing the write loses the
capture. **The trade is real and it is the maintainer's**, which is why this is
described rather than chosen.

**Weight.** Closes the only site that fires on every record with no trigger at all.

### 8.3 Make the request sockets' deadlines per-request, and give the answers a send timeout (B7, B8, B9)

**What.** `kernel.rs:2211` already says what the real fix is — park the accepted
stream in the poll set with a deadline, as the ingest socket does — and says it is
a restructuring rather than a line. Beside it, set `SO_SNDTIMEO` on the answering
stream, which *is* a line.

**Risk.** The poll-set form makes `drain_query` and `drain_kiosk` stateful in the
way `drain_ingest` is, and `answering`/`kiosk_answering` already carry half that
state. The send timeout risks truncating an answer to a slow but honest client;
the client is `cmd/rivet` or the launcher, both in this repository, so the failure
is visible rather than silent.

### 8.4 Fix `ask_health`'s connect, and the comment above it (B10)

**What.** Create the socket non-blocking and connect on it, which is what the
comment at `kernel.rs:3698` already claims the code does.

**Risk.** Near zero. A non-blocking unix `connect` to a listening socket with
backlog room returns immediately; with a full backlog it returns `EAGAIN`, which
`ask_health` already has a path for.

**THIS IS THE REPAIR. §11 is why.** ~~The value is not the wait it removes — the
numbers make that wait nearly unreachable — it is that a comment and its code stop
disagreeing about a safety property.~~ **That sentence was wrong and is struck
rather than edited, because being wrong about it is the finding.** The wait was
reached on 2026-09-16 and again on 2026-09-18, at 34 h 08 m and 34 h 09 m of
uptime — ninety seconds apart — and the second was captured in the act:
`wchan` `unix_wait_for_peer`, syscall `203`. **The value is the wait it removes.**

The estimate was reached by believing the comment above the code, which argues that
a wedged application leaves backlog room *because* it is not accepting. §11.4 has
why that is backwards. **What is still unknown is the fill rate** — 30-second asks
and a 128-deep backlog do not obviously produce 34 hours — and §11.6 says so
plainly. The repair does not wait on that answer: an unbounded blocking `connect`
reachable from an application's socket violates `.rulings/` R-195 D5 at any fill
rate.

### 8.5 Give the loop a way to say where it is, without perturbing it

**What.** The diagnosis problem of §6.2 exists because a wedged supervisor cannot
tell anyone where it stopped. A single-word "phase" written to a fixed-size
`/run` file before each unbounded call, overwritten in place with one `pwrite` to
a descriptor opened at start-up, would make `wchan` unnecessary: the button reads
a tmpfs file with no privilege at all.

**Risk, and it is the one that matters.** `.rulings/` R-195 D1 requires every
instrument to argue its own perturbation. This one writes, and `ADR-056` §5.1
disqualifies the button's own logging for exactly that reason. The argument in its
favour is that `/run` is a tmpfs with no device behind it and a `pwrite` to a
pre-opened descriptor at a fixed offset allocates nothing and cannot block on a
medium; the argument against is that it is still a write on the hot path, and that
a project which has twice been bitten by an instrument feeding its own failure
should be slow to add a third. **It is the most useful repair on this list and the
one most likely to be refused, which is why it is written down rather than
recommended.**

### 8.6 The structural answer: no unbounded wait on the loop, as a rule

**What.** State in `ADR-017` that the supervisor's loop may make no wait that is
not bounded by a deadline the supervisor owns, and that any operation which cannot
be so bounded is performed off the loop with its outcome collected on a later
pass. `drain_kmsg`'s cap, the ingest socket's deadline and the CEC thread are
three existing instances of the same rule; naming it is what stops the next
`run_to_completion` being added.

**Risk.** It is a rule, and `AGENTS.md` §3 is candid that a rule without a gate
erodes. The enforceable form is a test or a lint over `cmd/rivet-sysd` that
refuses `Command::output`, `Command::wait`, a blocking socket without a timeout,
and `fsync` on the loop's own path — which is real work, and is what makes the
rule worth writing down at all.

---

## 9. What this audit is honest about not being

* **Nothing here was run.** No test, no container suite, no board. Every claim is
  a reading of the tree at the commit this was written against, and every one
  carries its file and line so it can be checked.
* **No board journal was read.** Four of the five hypotheses are decided by the
  last records before each stop, and those records are on the card.
* **The `/proc` columns in §4 are derived, not measured.** They follow from what
  each call does; they were not read off a supervisor sitting in each state. The
  one measured row is B1, which `ADR-056` §2.1 read off the board as `S` in
  `do_sys_poll`.
* **It settles `ADR-056` §3.8 and nothing else about the button.** §3.4's grant,
  §3.7's resistance to becoming an API and §4's kernel-evidence routes are
  untouched by this reading.

---

## 10. What the stored journal said afterwards — three hypotheses dead, and the failure located

This section was written after §§1–9, against Fleet Monitor's off-machine copy of
the incident: 23,577 journal records from the wedged boot
`f09650b9-0895-40aa-9433-e7ce2fb2427c`, and every per-signal capture outcome
either side of the stop. **It is evidence, not reasoning**, and it changes the
ranking in §7.

### 10.1 H1 is dead — the supervisor's stdout was `/dev/null` for that whole boot

The wedged boot's own early records carry the decision:

> `2026-09-14T11:34:27.146Z … rivet-initd: the splash is being drawn, so the console stops here`
> `2026-09-14T11:34:27.146Z … rivet-initd: started /system/bin/rivet-splash as pid 143; console taken: fbcon=vtcon1 printk=lowered output=discarded`

`output=discarded` is `cmd/rivet-initd/src/kernel.rs`'s `discard_own_output()` —
file descriptors 1 and 2 pointed at `/dev/null` before the supervisor was
executed. **B4 cannot block on a console that is not there.** §7's top-ranked
hypothesis is ruled out by the machine's own record of its boot.

### 10.2 H5 is dead by arithmetic

The hourly clock write landed at `16:08:11`, `17:08:10`, `18:08:10`, `19:08:10`
and `20:08:10` on 2026-09-16. The supervisor stopped at **`20:16:46.568`** — eight
and a half minutes after one write completed and fifty-one minutes before the next
was due. **B6's `fsync` was not running when the loop stopped.**

### 10.3 H2 is dead — there was no stop edge

`crate::splash::restore` is reached from exactly one place, `supervise.rs:4077`,
on the release path of a service that owns the display, and only when no arbiter
holds the screen. Across the entire 34-hour boot:

* **no service stop, restart or respawn record exists at all** — the only splash
  records in the boot are pid 1's, at mono `2.875`;
* the last record of `warn` severity or worse is at `02:28:47`, **nearly eighteen
  hours before the stop**, and it is a weather fetch;
* `rivet-launcher` was the DRM master throughout, so even a lessee stopping would
  have taken the arbiter branch and never called `restore`.

§7 states the test plainly — *"the presence of a stop record with no splash outcome
after it is the signature. Its absence kills H2."* It is absent.

### 10.4 What did happen: the query split, and what it proves

The decisive evidence is not in the journal but in **which questions the machine
kept answering after it stopped writing one.** Fleet Monitor asked eight
different things on independent cadences:

| Query | After `20:16:46` | Latency |
| :-- | :-- | :-- |
| `get thermal` | **answered**, through `20:27:23` | 62 ms — normal |
| `get boot`, `get memory` | **answered**, through `20:24:52` | 654–85 ms — normal |
| `get log` | **answered** at `20:17:22`, returning the final 7 records | 678 ms — normal |
| `get status` | **timed out**, every minute from `20:17:02` | 20,014 ms — the client's bound |
| `get screenshot` | **timed out** | 20,008 ms — the client's bound |

The split is not random and it is not a network effect. `cmd/rivet/src/thermal.rs`
answers from `/sys/class/thermal` and `/sys/devices/system/cpu/cpu0/cpufreq` with,
in its own words, *"no `mount(2)`, no device tree and no pid 1"*; `boot`, `memory`
and the journal read are the same shape. **`get status` is the one that asks the
supervisor** — `supervisor_status()` over `ADR-032` §3.2's query socket, which is
B9's `answer_query` — and `get screenshot --holder` asks it too.

So:

1. **The connection was established and the query was delivered every time.**
   `rivet-mcpd` was healthy, forking `/system/bin/rivet` per connection as usual.
2. **Every question answerable without the supervisor was answered in tens of
   milliseconds, for eleven more minutes.**
3. **Every question requiring the supervisor got nothing, ever.**

**The failure is confined to `rivet-sysd`, and it is total rather than partial:**
journal ingest, health polling, service supervision and query answering all
stopped in the same instant. That is a single-threaded loop that entered a wait
and did not come out — `Threads: 1` is confirmed on the live board — and it
eliminates any explanation in which the supervisor is merely slow, starved, or
partly alive.

**Then, at `20:28:23`, the second-order failure.** Every probe began failing in
~42 ms with `An existing connection was forcibly closed by the remote host`.
`rivet-mcpd` had gone, and **the thing whose job is to restart it was already
wedged**. Nothing about the first failure caused the second except the absence of
a supervisor; this is the invariant — *the supervisor stops first* — stated as an
observation.

### 10.5 One asymmetry, recorded without an explanation

The supervisor drains `rivet-mcpd`'s stderr into the journal (`"stream":"stderr"`,
`"actor":"supervisor"`) and ingests application records over the journal socket
(`ingest_pid`). **The stderr drain stopped at `20:16:00.529`; socket ingest
continued to `20:16:46.568`** — a 46-second gap — and a `get status` was answered
normally in 82 ms at `20:16:02`, inside that gap.

This may be buffering and may be two different points in the loop. It is written
down because it is the only thing in 34 hours that looks like a *degradation*
rather than a wall, and because an instrument that samples only the end state
would never see it.

### 10.6 The privilege question, measured on the board rather than reasoned about

§6.2 said `wchan` and `stack` are load-bearing for diagnosis and that the
privilege question is forced if the button must answer *where*. Both halves are
now measured on `rivet-01`, healthy, 2026-09-16:

| File | Mode | Healthy content |
| :-- | :-- | :-- |
| `/proc/147/comm` | `0644` | `rivet-sysd` |
| `/proc/147/wchan` | `0444` | `do_sys_poll` |
| `/proc/147/syscall` | `0400` | `73 0x7fad5b2f90 …` — arm64 `ppoll` |
| `/proc/147/stack` | `0400` | `do_sys_poll` → `__arm64_sys_ppoll` → `el0_svc` |

**This kernel populates all three.** `stack` and `syscall` are not stubs, so the
diagnosis §6.2 wants genuinely exists on the machine — it is gated, not missing.
And the gate is real: **`rivet-fleetd` runs as uid `10494`** (`/proc/173/status`,
`PPid: 147`), so it can read `wchan` and `status` and cannot read `stack` or
`syscall`.

**But §6.2's three routes to *where* missed a fourth, and §10.4 is what exposes
it.** For the eleven minutes that matter, `rivet-mcpd` was alive and forking
`/system/bin/rivet` **as root** for every connection — and that process reads
`/sys` and `/proc` directly without asking the supervisor anything. A control-plane
query that reads `/proc/<sysd>/{wchan,syscall,stack,status,fdinfo}` in the spawned
process would have returned the complete diagnosis at `20:17:02`, and again every
minute until `20:28`. It needs no new component, no new uid, and no widening of any
application's confinement.

The two routes are **complementary rather than competing**, because their windows
differ and each covers the other's blind spot:

* the **`rivet-fleetd` button** is unprivileged and answers *has it stopped* — and
  it keeps working after `rivet-mcpd` dies, which is exactly when a human is
  standing in front of the board's web page with nothing else left;
* the **control-plane query** is root and answers *where is it stopped* — for as
  long as `rivet-mcpd` survives the supervisor, which on 2026-09-16 was eleven
  minutes and thirty-seven seconds.

### 10.7 What §7 now looks like

| | Status after this section |
| :-- | :-- |
| H1 — blocked console write (B4) | **dead** — §10.1, `output=discarded` |
| H2 — `run_to_completion` via `splash::restore` (B3) | **dead** — §10.3, no stop edge in the boot |
| H5 — hourly clock `fsync` (B6) | **dead** — §10.2, wrong minute |
| H3 — blocked writing an answer nobody reads (B9) | **live, and now ranked first** |
| H4 — `execve` or start-path filesystem read (B2, B17, B18, B25) | **live** |

H3 rises on more than elimination. B9 is the supervisor side of precisely the
query that stopped answering, and §10.4 shows the loop stopped whole — which is
what a blocking write on a single-threaded loop looks like. **It is not proven,**
and §10.5 is the loose thread: a block in `answer_query` explains a wall, not a
46-second degradation before it.

**Neither H3 nor H4 can be separated by any further reading of this incident.**
One `wchan` value from a wedged board separates them in a single word, and that is
what the instruments are for.

---

## 11. The incident of 2026-09-18, witnessed under instrument — B10 confirmed

**This is the incident `.rulings/` R-195 D1 and D4 held every repair for**, and it
was caught by the instruments built on 2026-09-16 rather than reconstructed
afterwards. It names the blocking site in one word, kills both surviving
hypotheses, and falsifies this document's own risk estimate for the site it
confirms.

### 11.1 What the capture holds

Fleet Monitor's `process` signal, one capture a minute, off the machine. The row
for the supervisor, verbatim, from `2026-09-18T14:12:43Z`:

```json
{"pid":146,"parent_pid":1,"name":"rivet-sysd","state":"S","threads":1,
 "kernel":false,"resident_bytes":4337664,
 "user_millis":25990,"system_millis":42390,
 "wchan":"unix_wait_for_peer",
 "syscall":"203 0x9 0x7fe107e740 0x24 0x0 0x0 0x0 0x7fe107e660 0x370bf8"}
```

**`203` is `connect` on `aarch64`.** `unix_wait_for_peer` is the function a task
sleeps in inside `unix_stream_connect` when the listening peer's backlog is full
and nothing is accepting. The first argument is the descriptor — **fd 9** — and
the third is `0x24`, a 36-byte `sockaddr_un`.

So the supervisor is not blocked writing, not blocked reading, and not blocked in
a filesystem. **It is blocked making a connection**, and it stayed in that one
syscall for the forty minutes the board was watched before it was power-cycled.

### 11.2 The transition, to the minute

| time (UTC) | what the recorder holds |
| :--- | :--- |
| `14:11:25` | last healthy row — `user_millis` 25980, `system_millis` 42380, `wchan` `do_sys_poll` |
| `14:12:23` | first `get status` miss, `elapsed_ms` 20019 — *"no answer within 20 s"* |
| **`14:12:43`** | **first frozen row — 25990 / 42390, `unix_wait_for_peer`, syscall 203** |
| `14:13`–`14:22` | ten more captures, **every counter identical**, same `wchan`, same syscall |
| ~`14:45` | probes begin failing in **43 ms** — *"forcibly closed by the remote host"*. `rivet-mcpd` is gone and nothing restarts it |
| ~`15:22` | probes are **20-second timeouts again** — something is listening on `:7777` once more, and every connection it forks blocks the moment it asks the supervisor. **Unexplained, and recorded rather than theorised** |
| `15:30` | `apply restart confirm` issued from the workstation: **hung with no output and was killed after five minutes.** The one command that recovers the machine is a request *to the supervisor* |

**Uptime at failure: 34 h 09 m.** The 2026-09-16 incident stopped at mono
`122917.944` — **34 h 08 m 38 s**. The two are **ninety seconds apart**, which is
what ended the question of whether this failure is time-driven.

### 11.3 H3 and H4 are dead, and B10 is the site

§7's two survivors both predicted a different `wchan`:

* **H3** — blocked writing an answer nobody reads (B9) — is a `write` on an
  established socket. It would park in `sk_stream_wait_memory` or the unix write
  path, and its syscall would be `64`, not `203`. **Dead.**
* **H4** — an `execve` or start-path filesystem read — parks in the block layer or
  a filesystem's own wait. **Dead.**

§6.2 listed the five one-word answers this field can give and named
`unix_stream_connect` among them as **B10**. That is the one the machine gave.

### 11.4 The comment, and why its reasoning is inverted

`cmd/rivet-sysd/src/kernel.rs`, in `ask_health`:

> **Non-blocking from the first syscall.** A unix stream `connect` to a listening
> socket completes immediately as long as the backlog has room — which it does for
> a wedged application, because a wedged application is not accepting and therefore
> not consuming it.

```rust
let stream = std::os::unix::net::UnixStream::connect(&path).map_err(…)?;
stream.set_nonblocking(true).map_err(…)?;
```

**`set_nonblocking` is called on the line after the connect**, so the connect runs
on a blocking socket and the first syscall is precisely the one that blocked.

The second half is worse than imprecise, because it is the argument the first half
rests on. *"A wedged application is not accepting and therefore not consuming the
backlog"* — **an application that is not accepting is the thing that fills the
backlog.** Every unaccepted connection stays queued in it. The comment offers
not-accepting as the reason there is room, when it is the reason there is none;
the case it names as safe is the case that wedges the supervisor.

This is `AGENTS.md`'s rule about a comment and its code disagreeing on a safety
property, and the cost of the disagreement was the whole machine, twice.

### 11.5 What this document got wrong about it

§8.4 rated the repair **"risk near zero"** and said, of the wait it removes:

> **The value is not the wait it removes — the numbers make that wait nearly
> unreachable** — it is that a comment and its code stop disagreeing about a
> safety property.

**That estimate is falsified.** The wait was reached twice in five days, at the
same uptime to within ninety seconds. §7 ranked B10 fourth of five and this
document argued the fix was worth making for tidiness; it was the defect all
along, and the reason it was ranked low is that the audit accepted the same
inverted reasoning the comment offers. **An audit that reads a comment's argument
instead of the code's behaviour inherits the comment's mistake**, which is the
lesson worth more than the ranking.

### 11.6 What is established, and what is not

**Established by measurement:**

1. The supervisor blocks in `connect(2)` to a unix socket, on a blocking
   descriptor, with no bound of any kind.
2. The socket is reached from `ask_health`, whose path is
   `rivet_core::health::channel_path(service)`.
3. `tempmon-rivet` is the only adopter of the `ADR-045` channel and the only
   service that ever reports `healthy` — 554 of 554 status captures on the day of
   the failure.
4. **The channel does misbehave on this board.** The supervisor's own journal
   carries six records on 2026-09-18 alone: *"this service did not answer the
   health question, so it is UNKNOWN rather than healthy (`ADR-045` §3). Its
   process is still there."*
5. The failure recurs at a near-constant uptime, which is the signature of a
   steady accumulation against a fixed limit.

**NOT established, and not to be guessed at:** the arithmetic from *occasional
missed answer* to *backlog full at 34 hours*. Health is asked every 30 s
(`rivet_core::health::INTERVAL_MS`), which is **4,080 asks** across 34 hours,
against a backlog Rust's `UnixListener::bind` sets to **128**. Neither one
unaccepted connection per ask (which would wedge it in about an hour) nor one per
journalled miss (six in fourteen hours) produces 128 in 34 hours. **Something
about the fill rate is not yet understood**, and the next reading should measure
the queue rather than reason about it.

**The repair does not depend on that.** The defect is an unbounded blocking
`connect` on the supervisor's loop, reachable from an application's socket —
`.rulings/` R-195 D5's invariant, violated exactly as written: *the supervisor
cannot be blocked by a service.* That is true at any fill rate.

### 11.7 What the instruments proved about themselves

* **The `process` signal did the whole job.** One capture a minute held the last
  healthy row, the first frozen row, and eleven identical ones after it — with the
  `wchan` and the syscall that name the site. Nobody was awake for any of it.
* **`wchan` and `syscall` were worth adding.** Frozen counters alone say *stopped*;
  they do not say *where*, and every candidate here presents as `state` `S`.
* **The stop was caught within twenty seconds** of the first missed `get status`,
  by two signals that disagree for a structural reason: `status` asks the
  supervisor and `process` does not.
* **Matching on `name` rather than pid was load-bearing.** The supervisor was pid
  147 before the 2026-09-16 update and 146 after.
