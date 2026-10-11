# ADR-056: Seeing a Wedged Supervisor

* **Status:** Proposed
* **Date:** 2026-09-16
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Scope:** instruments only. This document changes no behaviour, and that is
  the ruling it serves (`.rulings/` R-195 D1) rather than a limitation of the
  work. No code, no manifest and no kernel configuration is altered by it.

---

## 1. Context

A Rivet wedges, repeatedly — `.rulings/` R-195 is the record of the occasions.
The most recent was caught while it was happening, and what it showed is that
the machine cannot describe its own failure.

The shape of the failure, as observed:

* The journal stops mid-stride. Nothing follows it, no kernel error, no
  refusal.
* About eleven minutes later the control plane stops answering. Port 7777
  **times out** rather than refusing, which is what a bound listening socket
  whose owner has stopped accepting looks like.
* **Everything already running carries on for hours.** Aura draws and accepts
  a settings change through its portal; TempMon serves and keeps writing its
  SQLite store on `/state`; `rivet-proxyd`,
  `rivet-fleetd` and mDNS all answer.

That last point settles one thing and opens another. **`/state` was never the
problem** — applications wrote to the card for hours after the journal's last
record, so the card-stall reading taken from the second incident is withdrawn.
What is stuck is the **supervisor**.

`get process`, read off a board while this was written, shows the structure that
makes such a wedge quiet: **PID 1 is `rivet-initd`, `rivet-sysd` is its child,
and every service is a child of `rivet-sysd` in turn.** A supervisor that stops
is therefore a process asleep beneath a PID 1 that is itself asleep. Linux
panics when *PID 1* dies; it says nothing at all about a child of PID 1 that has
simply stopped making progress.

### 1.1 The invariant this serves, stated first

The maintainer's framing, and it is the reason the button is worth building
rather than a curiosity: **a supervisor that can be blocked by a service, a
device or a filesystem is a design defect.** "The supervisor stops first"
should be impossible, and every instrument in this document exists to
**witness a violation of that invariant**, never to make the invariant
unnecessary.

`ADR-045` §3 already has one instance of the same reasoning, reached the hard
way: the health poll had to be made asynchronous because the supervisor is
single-threaded, and a blocking ask would have stalled the whole machine on
precisely the application the poll exists to notice. That is this failure's
species, and it says that the class was understood once already.

### 1.2 What is a hypothesis and what is a reading

This document keeps the two apart on purpose, because the ruling it serves is
that no behaviour changes until the failure is reproduced **under instrument**.
§2 is what was read off a running board and off the tree. §7 is the two
suspects, held as hypotheses the instruments are meant to confirm or kill.

---

## 2. What the machine already offers, read rather than assumed

Everything in this section was read off `rivet-01` over the control plane, or
out of this repository's source. The method is named per row because a design
that guessed here would design for a machine nobody runs.

### 2.1 The supervisor, as `/proc` describes it

| Path | Mode and owner | What it carries | How this was established |
| :-- | :-- | :-- | :-- |
| `/proc/<sup>/status` | `0444` root | `State` letter, `Threads`, `SigBlk`/`SigPnd`/`ShdPnd`, `voluntary_ctxt_switches`, `nonvoluntary_ctxt_switches` | read off the board; the supervisor answered `State: S (sleeping)`, `Threads: 1` |
| `/proc/<sup>/stat` | `0444` root | state letter, `utime`, `stime`, `num_threads`, `starttime` | read off the board |
| `/proc/<sup>/wchan` | `0444` root | the symbol the task is parked in — it answered `do_sys_poll` | read off the board **as root**, through `rivet-mcpd` |
| `/proc/<sup>/stack` | `0400` root | the full kernel stack — it answered `do_sys_poll` → `__arm64_sys_ppoll` → `el0_svc` | read off the board **as root**; `CONFIG_STACKTRACE=y` in the running kernel |
| `/proc/<sup>/comm` | `0444` root | the program name, for confirming an identification | mode read off the board |

`CONFIG_KALLSYMS_ALL=y` and `CONFIG_STACKTRACE=y` are both set in the running
kernel's own configuration, which is why the last two rows answer anything at
all.

### 2.2 The two sandboxes are different, and that is the whole difficulty

| Process | uid | Capabilities | Confined by Landlock |
| :-- | :-- | :-- | :-- |
| `rivet-mcpd` | 0 | the full bounding set | no — it is an image service |
| `rivet-fleetd` | its own derived uid | `CapPrm`, `CapEff` and `CapBnd` all zero, `NoNewPrivs` set | yes — it is a deployed application |
| `rivet-proxyd` | its own derived uid | the same | yes |

Read off the board. `rivet-mcpd` runs as root, which is exactly why every
reading in §2.1 came back — and exactly why none of it is available to the
portal when it is wanted.

**`rivet_core::confinement` grants an application one file under `/proc` and no
directory**: `BOOT_ID_PATH`, the boot id the SDK stamps on every journal
record. Its own comment says why the grant is a leaf rather than a tree — *a
directory grant here would hand over every other process's command line* — and
`rivet-core`'s `nothing_outside_the_declaration_is_granted` names `/proc` among
the paths a policy must never carry.

So, stated plainly and in the form the ruling asked for:

* **What `rivet-fleetd` can read as it stands:** the boot id, and nothing else
  under `/proc`. Every row of §2.1 is refused by Landlock before any question
  of file mode arises.
* **What needs a policy change and nothing more:** `status`, `stat` and `comm`
  for one process. These are world-readable by mode; only the ruleset is in the
  way. §3.4 designs the narrowest grant that serves them.
* **What needs privileges an application must not have:** `wchan`, `stack` and
  `syscall`. The kernel gates all three on `ptrace_may_access` against the
  target's credentials, so a different-uid reader is refused whatever the mode
  says, and `stack` is `0400` root-owned besides. Granting them means giving
  `rivet-fleetd` root or `CAP_SYS_PTRACE`, and `ADR-015` §5.1 drops every
  capability from every application by construction.

**One honesty note about that last bullet.** That `wchan` reads `0` and `stack`
refuses for a non-root reader is the kernel's documented rule, taken from its
source; it was **not** measured as `rivet-fleetd`'s uid on this board, because
doing so would mean running something there, which this work may not do. The
root readings in §2.1 are measurements; this one is not.

### 2.3 The journal, and why the sandbox cannot see it

`rivet-core`'s `state` module names the active journal at
`/state/log/journal.jsonl`, with `LOG_FILE_MODE` at `0o600` and `LOG_DIR_MODE`
at `0o700`. The board agrees: `/state/log` is `rwx------`, root-owned, holding
a rotated series beside the active file.

An application therefore cannot `stat` the journal — refused twice over, by
Landlock and by the directory's own search permission. §3.6 is what the design
does about the fourth item on the ruling's list.

### 2.4 The kernel's own facilities, read out of the running configuration

`CONFIG_IKCONFIG` and `CONFIG_IKCONFIG_PROC` are both `y` in Rivet's fragment,
so the running kernel reports its own configuration and every row below was
read from `/proc/config.gz` on the board rather than from a defconfig.

| Symbol | Value on the running kernel | Consequence |
| :-- | :-- | :-- |
| `CONFIG_NETCONSOLE` | `m` | **unreachable.** `ADR-024` §4 rules everything built in; a Rivet ships no `/lib/modules` and loads no module |
| `CONFIG_NETCONSOLE_DYNAMIC` | unset | even built in, it would be a boot-parameter facility with no configfs reconfiguration |
| `CONFIG_NETPOLL` | `y` | the transport netconsole would use is present; the console driver over it is what is missing |
| `CONFIG_PSTORE`, `CONFIG_PSTORE_RAM`, `CONFIG_PSTORE_CONSOLE` | `y` | ramoops is **built in and available**, and needs only a memory region |
| `CONFIG_DETECT_HUNG_TASK` | `y` | `khungtaskd` runs; `/proc/sys/kernel/hung_task_timeout_secs` reads `120` |
| `CONFIG_BOOTPARAM_HUNG_TASK_PANIC` | unset | a hung task is reported and the machine carries on |
| `CONFIG_SOFTLOCKUP_DETECTOR`, `CONFIG_HARDLOCKUP_DETECTOR` | unset | no lockup warnings of any kind |
| `CONFIG_MAGIC_SYSRQ` | `y`, and `/proc/sys/kernel/sysrq` reads `502` | the debugging dumps are enabled, and `/proc/sysrq-trigger` is present |
| `CONFIG_BCM2835_WDT` | `y` | the hardware watchdog is compiled in. Out of scope here — `.rulings/` R-192 D1 |

`ramoops.mem_size` reads `0` on the board and `/sys/fs/pstore` is present,
empty and unmounted: the driver is compiled in and has been given no region to
use. The kernel command line carries a console on the serial port and on the
virtual terminal, and no persistent-store parameters.

### 2.5 The finding that reshapes §4: the kernel would have said nothing

**`khungtaskd` reports only tasks in uninterruptible sleep.** That is its whole
subject: it walks tasks whose state is `TASK_UNINTERRUPTIBLE` and leaves every
other state alone.

**Every state §7's hypotheses can produce is an interruptible one.** A
supervisor parked in `ppoll` is `S` — `rivet-sysd` read exactly that on a board
while this was written, in `do_sys_poll`. A service blocked writing to a full
pipe is `S`. A caller blocked in `connect(2)` against a full listen backlog is
`S`. The uninterruptible case is the *card* case, and §1 has already excluded
it.

So the presence of `hung_task_timeout_secs` on the machine is **not** evidence
that the kernel would have printed a blocked task's stack for this wedge, and a
design that treated it as such would be built on a premise nobody checked. On
every reading available, the kernel had nothing to print.

This does not make §4 worthless; it changes what §4 is for. Getting the kernel
ring buffer off the machine is **insurance against a different failure**, and
saying so is worth more than an instrument installed on a false premise.

---

## 3. The button

`.rulings/` R-195 D2 and D3. A page in `rivet-fleetd` that **a person opens**,
answering one question: is the supervisor alive, and where is it stuck.

### 3.1 It reads `/proc` directly, and never through the control plane

Every page the portal serves today reaches the machine over `ADR-035`'s control
plane — `cmd/rivet-fleetd/src/node.rs` opens a pinned TLS connection and
`main.rs` hands the answer back unchanged, with the comment that says so:
*the browser's JSON-RPC goes to the machine over ADR-035's control plane and
the machine's answer comes back unchanged*. That is correct for every operation
and fatal for this one, because `rivet-mcpd` is the first thing to go silent.

The button does not use that path. It opens files under `/proc` in the process
serving the page, formats what it read, and returns HTML. It makes no
connection, holds no client identity, and has nothing that can time out.

`ADR-043` §12 already says the portal adds no authority beyond what the
operator's certificate carries. This page keeps that true: it reads what any
process on the machine could read given the grant in §3.4, and it changes
nothing.

### 3.2 One question, and the refusals that keep it one

Per `.rulings/` R-195 D3, the page answers **is the supervisor alive, and where
is it stuck** — and refuses everything else. It lists no services, reports no
health, shows no configuration, offers no operation and confirms no plan.

`ADR-011` §3 is the reason, and it has a second edge here. A second answer to
"what is running" is the failure that ADR names; a page that grew into one
would also become exactly the backdoor R-195 D2 forbids. The two constraints
point the same way, which is why neither needs to be traded against the other.

### 3.3 Finding the supervisor without asking it

`rivet-fleetd` must identify the supervisor by itself, since the thing that
would normally tell it is the thing under suspicion.

**The answer is `getppid()`.** The supervisor spawns every service and every
application, so an application's parent **is** the supervisor — confirmed on a
board, where `rivet-fleetd`'s `PPid` is the pid `rivet-sysd` is running under.
There is no scan, no name lookup and no list.

The design records the parent pid **at startup** and reads it again **at every
render**, and the comparison is itself diagnostic:

| What is read | What the page says |
| :-- | :-- |
| the same pid at both, and `/proc/<pid>/comm` reads `rivet-sysd` | this is the supervisor; here is its state |
| the pid at render is `1` | the supervisor that started this application is gone and this process was reparented to `rivet-initd`. That is an answer, not a failure of the page |
| the two pids differ and neither is `1` | the supervisor was replaced while this application kept running — worth naming, because it means a restart nobody asked about |
| the parent at startup was already `1` | this process was not started by a supervisor, so the page cannot identify one, and it says exactly that rather than guessing |

The last row matters more than it looks: a page that fell back to scanning
`/proc` for a process named `rivet-sysd` would need a directory grant over
`/proc`, which is the grant `rivet_core::confinement` refuses on the record. An
identification that cannot be made is reported as such.

**A scan would also be wrong on its own terms.** Two processes can carry the
same `comm`, and the one this application's fate is tied to is its parent, not
whichever one a scan met first.

### 3.4 The grant this needs, and why it is a leaf

The narrowest policy that serves §3.5 is **two leaves**: a Landlock
`path_beneath` rule over the directory `/proc/<supervisor pid>` with read-file,
and one over the file `/proc/uptime`. Nothing more, and no directory above
either.

`/proc/uptime` is there because the supervisor's CPU counters are cumulative and
meaningless without an interval to put them against: 900 ms of CPU over five
minutes and 900 ms over five hours are different machines. It carries nothing
about any process, which is why it can be a file grant and not an argument.

It is buildable in exactly the way `cmd/rivet-sysd/src/landlock.rs` already
builds every other rule: the ruleset is constructed **in the parent, before
`fork`**, each rule needing a directory that is already open. The supervisor
knows its own pid and can open its own `/proc` directory, so the descriptor the
rule is attached to is the supervisor's own — one process's subtree, granted to
one application, with no other process's command line inside it.

Three things the design owes the maintainer rather than deciding:

1. **What selects the application that gets it.** A new manifest key is a
   change to what a manifest means, which is `AGENTS.md` §1.1's third row. A
   name-keyed exception in the supervisor is narrower and uglier. This document
   states the choice; it does not take it.
2. **What happens to the rule when the supervisor restarts.** The rule is
   attached to a descriptor for one pid's directory. A supervisor that exits
   takes its children with it, so in practice the application is restarted and
   re-confined — but that is a claim about the restart path, and it should be
   read out of `cmd/rivet-sysd` rather than assumed here.
3. **Whether the grant is read-file alone or read-file plus read-dir.** The
   files needed are named exactly, so read-file is enough, and a read-dir would
   let the page enumerate the supervisor's descriptors and threads. The
   narrower one is proposed.

### 3.5 What the page reads, and why one sample of it is worthless

The metrics, all from files that are world-readable by mode:

| From | Field | What it distinguishes |
| :-- | :-- | :-- |
| `status` | `State` | running, sleeping, uninterruptible, zombie |
| `status` | `Threads` | whether the supervisor is still the single-threaded program its own safety argument depends on (§7.2) |
| `status` | `voluntary_ctxt_switches` | how many times the loop has parked itself. A loop that is turning moves this constantly |
| `status` | `nonvoluntary_ctxt_switches` | preemption, which separates "starved" from "parked" |
| `status` | `ShdPnd` against `SigBlk` | a signal that is blocked and pending is one the `signalfd` has not been read for — a direct witness that the loop is not draining it |
| `stat` | `utime`, `stime` | cumulative CPU. Counters, not percentages — `rivet-core`'s `process` operation carries the same argument about the same two numbers |
| `stat` | `starttime` | the supervisor's own age, which says whether it has been restarted since the boot |
| `comm` | the name | confirms the identification of §3.3 |
| `/proc/uptime` | seconds since boot | the interval the cumulative counters are read against, and the number that makes "the supervisor has done nothing for five hours" sayable at all |

**Every one of these is either a state or a counter, and a single reading of a
counter says nothing.** So the page takes **two samples about a second apart
within the one request**, and reports the differences as well as the values.
The whole question — is it turning — is a difference, and a page that showed
one snapshot would leave the reader to hold the first number in their head and
press refresh, which is the reading nobody takes correctly at three in the
morning.

That one-second cost is deliberate and is not configurable. §3.7 is why.

### 3.6 Journal freshness: the one item of D3's list the sandbox cannot serve

`.rulings/` R-195 D3 names *how fresh the journal is* as the fourth thing the
page should show. §2.3 is why the page cannot read it: the journal is `0600` in
a `0700` root-owned directory, and Landlock refuses the path besides.

Three ways out, and the cost of each:

| Route | What it costs |
| :-- | :-- |
| **Widen the mode** on `/state/log` or the file | hands every application's uid the whole journal, including every other application's records and the ingest socket's. Refused |
| **Hand a descriptor over**, as the listening-socket contract already does (`SPEC-03` §3) | the journal **rotates by rename**, and a board carries the numbered series beside the active file to prove it happens — so a descriptor held across a rotation points at the rotated inode, whose modification time stops advancing. The instrument would then report a stale journal at every rotation, which is the exact false positive it exists to avoid. A rotation-aware hand-over is possible and is a design of its own |
| **Do not read the journal** | answer freshness from the supervisor's own counters instead |

**The design takes the third**, and states the substitution rather than hiding
it. A supervisor whose CPU counters and voluntary context switches are
advancing is a supervisor that is running its loop, and a loop that is running
is one that writes records; a supervisor whose counters are frozen has stopped
writing them. The journal's modification time and the supervisor's counters are
two readings of one underlying fact, and the counters are the one the sandbox
can reach without being widened.

If the maintainer wants the file's own timestamp, the second route is the one
to build, and it has to be rotation-aware before it is worth anything.

### 3.7 How this resists becoming an API

The constraint in the maintainer's own words is that this must not become a
thing an agent reaches for instead of `rivet-mcpd`. Access control cannot
deliver that: anyone who can open this page holds the same operator certificate
the control plane takes. So the design makes the page **useless as a machine
interface** rather than merely discouraged.

* **HTML only.** One route, one content type, no content negotiation, no
  `Accept` handling, no JSON anywhere on the path.
* **Prose, not fields.** The numbers are rendered inside sentences with units
  and comparisons — *the supervisor parked itself 41 times in the second this
  page was drawn* — rather than as a labelled set. A machine reading it is
  parsing English that is free to change.
* **No stable contract, stated as a rule in this ADR.** Every name, number,
  phrase and ordering on this page may change in any release with no notice and
  no deprecation. Nothing is permitted to depend on it. There is no version in
  the route and no `/api/` in it either; it sits among the portal's human pages.
* **No inputs.** No query parameters, no path parameters, nothing to vary. The
  one-second sampling interval of §3.5 is fixed, which also means the page
  cannot be made cheap enough to poll.
* **One question.** §3.2. There is nothing here an agent could want that
  `rivet-mcpd` does not answer better, for as long as `rivet-mcpd` answers at
  all — and when it stops, an agent is not the thing that should be looking.
* **It writes nothing.** §5 explains that this is a perturbation rule first; it
  is also the reason the page cannot be given a rate limit that logs, and
  therefore why the prohibition is written into the record instead.

### 3.8 THE METRIC LIST IN §3.5 IS SETTLED, AND THE BUTTON ANSWERS ONLY HALF THE QUESTION

The audit this section was waiting for exists —
`docs/design-candidates/Supervisor-Blocking-Audit-2026-09-16.md` — and its §10
measured `rivet-01` rather than reasoning about it. The four questions §3.8 asked
are answered, and the answer splits the work in two.

**§3.5's list is confirmed for detection and is complete as it stands.** The audit
§6.3 finds that every unbounded wait in the supervisor parks without moving
`voluntary_ctxt_switches`, so frozen counters against `/proc/uptime` is the whole
detection signal: *has this supervisor stopped* is answerable by the button, with
no privilege beyond §3.4's two-leaf grant, and nothing else on the machine can
state it.

**The button cannot answer *where*, and the page must say so in as many words.**
`wchan`, `syscall` and `stack` are each one word or one frame away from
distinguishing five different diagnoses that all present as the same `S`, and they
are readable on this kernel — measured healthy on `rivet-01`, `/proc/147/wchan`
reads `do_sys_poll`, `/proc/147/syscall` reads arm64 `ppoll`, `/proc/147/stack`
unwinds through `__arm64_sys_ppoll`. **But `rivet-fleetd` runs as uid `10494`**,
and `stack` and `syscall` are mode `0400` root. Giving the web-facing daemon root
or `CAP_SYS_PTRACE` to read them would break `ADR-015` §5.1 for every application,
because the mechanism is per-application and the precedent is not survivable.

**So *where* is answered somewhere else, and the audit §10.6 says where.** The
2026-09-16 incident shows `rivet-mcpd` alive and forking `/system/bin/rivet` **as
root** for eleven minutes and thirty-seven seconds after the supervisor stopped —
and that process already reads `/proc` and `/sys` directly without asking the
supervisor anything. The diagnosis belongs in the control plane, as a query, on the
root side of a boundary that already exists.

**The two are complementary, and the reason is their windows:**

| | Answers | Privilege | Works when |
| :-- | :-- | :-- | :-- |
| The button, §3 | *has it stopped* | uid `10494`, two leaves | `rivet-fleetd` is up — including after `rivet-mcpd` dies |
| A control-plane query | *where is it stopped* | root, via the spawned `rivet` | `rivet-mcpd` is up — which outlived the supervisor by 11m37s |

Each covers the other's blind spot. The button is what a human has left when the
control plane is gone; the query is what an operator has while it is not.

**What this section no longer claims.** §3.5 is not provisional and should be
built. What is still undesigned is the control-plane half — its verb, its answer
shape, and whether it reads only the supervisor's pid or any pid — and that is a
separate design, not a deferred part of this one.


---

## 4. Getting the kernel's own evidence off the machine

The kernel's ring buffer dies with the reboot, and the only thing copying it to
disk is the wedged supervisor: `cmd/rivet-sysd/src/kmsg.rs` is `ADR-022` §4.1's
ingest, the descriptor sits in the supervisor's own `poll` set, and a supervisor
that has stopped turning has stopped draining it.

§2.5 already says what this is worth for **this** failure: very likely nothing,
because the kernel has nothing to say about interruptible sleep. The routes
below are compared anyway, because a machine that wedges a fourth time for a
different reason should not lose the evidence twice.

### 4.1 netconsole: this kernel builds it as a module

`CONFIG_NETCONSOLE=m` in the running kernel's own configuration. `ADR-024` §4
rules `=y` throughout and never `=m`, a Rivet carries no `/lib/modules` and
loads nothing, so **netconsole is compiled and unreachable** — the same pattern
`tools/buildroot/linux/rivet.config` records four times over for wireless,
graphics, the HDMI I2C bus and Bluetooth, each one inherited from the Pi
defconfig and each one silently absent on a board.

Making it available means adding the symbol to `ADR-024` §3's requirements
table and the fragment, rebuilding the kernel, building a card image and
shipping a system update. It then needs a destination address and the
destination's MAC, because netpoll does not resolve one.

**And it perturbs the thing it measures.** netpoll transmits from inside the
printk path, with interrupts disabled, polling the NIC for completion. Every
kernel message on the machine then takes a different and longer path, and the
transmit itself can block on a driver — on a board whose USB controller already
dies on every boot, adding a synchronous network transmit to the kernel's
logging path is not a neutral observation.

**Rejected**, on cost and on perturbation together.

### 4.2 ramoops: built in, unconfigured, and the recommended route

`CONFIG_PSTORE`, `CONFIG_PSTORE_RAM` and `CONFIG_PSTORE_CONSOLE` are all `y`
already. `/sys/fs/pstore` is present on the board, empty and unmounted, and
`ramoops.mem_size` reads `0`: the driver is compiled in and nothing has given it
a region to use.

What it needs is a **reserved memory region** — a `reserved-memory` node in the
device tree, or `ramoops.mem_address=` and `ramoops.mem_size=` on the kernel
command line, which `cmd/rivet-image`'s boot module owns (`ADR-002` §4). No
kernel rebuild. With `PSTORE_CONSOLE`, every kernel message is copied into that
region as it is printed, and the next boot finds it under `/sys/fs/pstore`.

**What it costs:** one memory region, and a `memcpy` per printk line. No I/O,
no new process, no scheduler interaction, no change to message ordering or
content.

**The limitation, stated because it decides how useful this is.** Ramoops lives
in DRAM. It survives a **warm reset**; it does not survive a power cut. The
incidents so far have ended with the board being power-cycled by hand, because
with the control plane dead there is no way to ask it to reboot — which means
ramoops pairs with the hardware watchdog (`.rulings/` R-192 D1, its own ADR and
explicitly not this one) and is worth much less without it.

**Recommended anyway**, because it is the cheapest thing on this list by a wide
margin, it changes nothing about how the machine runs, and the one case it
catches — a kernel that did say something — is the case where losing the
evidence hurts most.

### 4.3 A second reader of `/dev/kmsg`, which needs no kernel change at all

`/dev/kmsg` is `0644` root-owned on the board, so an unprivileged uid can read
it; the kernel supports many independent readers, each with its own position.

A small service whose only job is to open `/dev/kmsg` and append what it reads
to a file of its own on `/state` would carry the kernel's messages past the
supervisor's wedge, because once started it depends on the supervisor's loop for
nothing.

**What it costs**, and it is more than it looks: a new service, a new uid, a
second writer to the one writable partition, a second copy of every kernel
message the journal already has, and a retention policy of its own. It is also
a component that has to be right about back-pressure, since a service that
blocks writing is the failure this whole document is about, wearing a new face.

**Not recommended as the first move.** It is the right answer if ramoops turns
out to be unusable — if the region cannot be reserved on this firmware, or if
the watchdog work is deferred far enough that a warm reset is never available.

### 4.4 Who catches it on the workstation, ranked by cost

The question only arises for netconsole, which is rejected: ramoops is read on
the machine after the reboot, and a second `/dev/kmsg` reader writes to the
machine's own card. Ranked anyway, because the ranking is what makes the
rejection concrete.

| Option | Cost | Verdict |
| :-- | :-- | :-- |
| **A small listener script** on the workstation, writing UDP datagrams to a dated file | an afternoon, and it has to be running at the moment the board wedges — which means it is a service on the workstation, which means it is the thing Fleet Monitor already is | viable, and duplicates something that exists |
| **A general tool** (`netcat`, a syslog receiver) | nothing to write, nothing to maintain, and nobody remembers to start it | the worst of the three: it works and it will not be running |
| **Fleet Monitor gains it as a signal** | its own repository, its own specification, and a new *shape* rather than a new signal | the right home, and the most work |

Fleet Monitor is the right home and the reason is worth stating, because it is
also a limitation. **Every signal it has goes through the control plane.** Its
signals specification lists `status`, `log`, `thermal`, `screenshot`, `memory`,
`boot`, `storage` and `slot`, and every one of them is a `get` over `ADR-035`.
So Fleet Monitor
cannot gain a *supervisor* signal that works during this failure: the same
silence that hides everything else would hide that too. What it could gain is a
**listener** — something that receives rather than asks — and that is a
different shape from every signal it has, in a program whose whole scheduling
model is a cadence and a timeout.

Fleet Monitor's `reach/` condition already did its job on this incident, and its
`log-stalled/` condition already draws the distinction *alive but its journal
stopped* against *gone*. The gap is not detection. The gap is evidence.

**That work belongs to Fleet Monitor's own repository and its own specification,
and nothing in this ADR rules it.**

---

## 5. Perturbation, argued per instrument

`.rulings/` R-195 D1 requires each instrument to argue what it **changes** about
how the machine behaves, not merely what it shows. An instrument that alters the
failure path is disqualified.

| Instrument | What it changes about how the machine behaves | Verdict |
| :-- | :-- | :-- |
| **The button's reads** (§3) | one application's ruleset gains read on one process's `/proc` directory. Reading those files walks the supervisor's task structures; it does not signal it, does not wake it, and takes no lock it holds across a sleep. Nothing is added to the supervisor's loop | **admissible** |
| **The button's own output** | see below — this is the one that is easy to get wrong | **admissible only with the exemption below** |
| **netconsole** (§4.1) | every kernel message takes a synchronous polled network transmit from inside printk, with interrupts disabled, on a board whose USB controller already fails at boot. Plus a kernel rebuild and a system update | **disqualified** — it alters the path it is meant to observe |
| **ramoops** (§4.2) | one reserved DRAM region, and a `memcpy` per printk line. No I/O, no process, no ordering change | **admissible** |
| **A second `/dev/kmsg` reader** (§4.3) | a new process, a new uid and a second writer to the one writable partition; the supervisor's loop is untouched, but the card is shared | **admissible, and the most expensive** |
| **Polling anything through `rivet-mcpd`** | every control-plane connection writes records into the board's own journal — Fleet Monitor's own specification says so in as many words, where it records 7–25 journal records a minute from this board at a one-minute cadence. An instrument on that path feeds the queue it is measuring | **disqualified**, and dead at the moment it is wanted besides |

### 5.1 The exemption the button needs, and why it is not a detail

`cmd/rivet-fleetd/src/main.rs` writes a line to stdout for every request it
serves. **An application's stdout is a supervisor capture pipe** —
`cmd/rivet-sysd/src/kernel.rs` sets `Stdio::piped()` on both streams for every
service, and the supervisor drains them in its own loop.

So a diagnostic page that logged the way every other route logs would push bytes
into **the exact queue the leading hypothesis says filled**. The instrument
would be feeding the failure.

**The rule, therefore:** the button's route writes nothing to stdout, nothing to
stderr, and files no journal record — not on success, not on refusal, not on a
read that failed. Its own errors are rendered on the page, for the person who is
looking at it.

This has a cost worth naming: a page that leaves no trace cannot be shown to
have been polled. §3.7's defence against that is design, not detection, and this
is the reason it has to be.

---

## 6. What each instrument would have shown at 20:16Z

This is the test of whether any of it is worth building.

| Instrument | What it would have said |
| :-- | :-- |
| **The button — counters** | that between the journal's last record and the moment the page was opened, the supervisor's `utime`, `stime` and `voluntary_ctxt_switches` had **not moved**, while the board's own uptime had advanced by hours. That single comparison is the fact nothing on that machine could state, and it separates *the supervisor stopped* from *the supervisor is busy* from *the card stalled* — the last of which cost the second incident its diagnosis |
| **The button — `State`** | `S`, almost certainly. Useful as a negative: it rules out uninterruptible sleep, and with it the card, on the spot |
| **The button — `Threads`** | `1` or more than `1`. On this board it would have read `1`, because the document carries no `[display]` table (§7.2), and that reading would have killed one of the two suspects in a single line |
| **The button — `ShdPnd`** | whether a blocked `SIGCHLD` was sitting pending — that is, whether the `signalfd` had stopped being read. A direct witness that the loop had stopped, as opposed to having nothing to do |
| **The button — `wchan`/`stack`** | the syscall it was parked in, which is the difference between *waiting in `poll` with nothing to wake it* and *blocked in a pipe read inside `Command::spawn`* (§7.1). **This is the reading the page cannot take today**, and §3.8 is the work that decides whether that is acceptable |
| **netconsole** | nothing, for this incident. §2.5: no task was in the state the kernel reports on |
| **ramoops** | nothing, for this incident, and for the same reason — and the board was power-cycled, so even a message would have been lost. Its value here is having asked the question once and got a definite empty answer |
| **A second `/dev/kmsg` reader** | the same empty answer, durably, on the card |
| **Fleet Monitor** | what it did say: `reach/` raised when the control plane stopped answering. A control-plane-based supervisor signal would have been silent with everything else |

The pattern across that table is the argument for the priority in §9: **the
evidence for this failure is entirely in userspace `/proc`, and the kernel has
none of it.**

---

## 7. The two suspects, held as hypotheses

Neither is established. Both are written here so that the instruments can be
judged against something falsifiable.

### 7.1 The pipe-fill hypothesis

**Partly borne out by the stored journal, 2026-09-16 — see the audit §10.4 and
§10.5.** The supervisor did stop draining `rivet-mcpd`'s stderr, at `20:16:00.529`,
**forty-six seconds before it stopped ingesting over the journal socket** and
sixteen seconds before it answered a `get status` normally in 82 ms. `rivet-mcpd`
then kept serving for eleven minutes and thirty-seven seconds and went away at
`20:28:23`. The order this section predicts is the order observed; **what is not
explained is the gap** — a supervisor blocked on one wait does not drain one queue
and not the other, so either the drains are at different points in the loop or the
stop was not a single event.

**The claim:** the supervisor reads every service's output; when it stops
reading, each service's capture pipe fills and that service blocks on its next
line. Services therefore die in order of **how much they log** — `rivet-mcpd`
first, because it writes a line per connection and Fleet Monitor connects
several times a minute, and a quiet application hours later.

What the tree says in its own favour. The supervisor gives every service piped
stdout and stderr, and its poll loop drains every open pipe **before it blocks**,
for a stated reason: *a pipe that filled while the supervisor was busy has a
blocked writer on the other end, and a blocked `rivet-mcpd` is a machine nobody
can reach*. The mechanism is understood; what is unproven is that it is what
happened.

There is a second queue with the same owner. `rivet-core`'s journal names the
ingest socket at `/run/rivet/journal` with a bounded number of connections in
flight, and the supervisor drains it on every wakeup *for the pipes' reason
exactly*. An application filing records through the SDK blocks in `connect(2)`
once that backlog fills. So there are two independent ways for a supervisor that
has stopped reading to stop its children, and the observed order of death would
be a mixture of both.

**What the instrument does to it.** If the hypothesis holds, the button shows a
supervisor whose counters are frozen and whose services are alive-but-blocked;
the ordering prediction is then testable against the journal's own last records
and Fleet Monitor's capture line. If the counters are frozen and the services
are *not* blocked on output, the hypothesis is wrong and the instrument said so.

**One thing the timeout does not prove.** A port that times out rather than
refusing is consistent with a blocked owner, and equally consistent with a
listen backlog that has filled while the owner is alive and slow. It narrows the
field; it does not pick a member of it.

### 7.2 The violated single-thread invariant

`cmd/rivet-sysd/src/kernel.rs` justifies calling `setenv` between `fork` and
`exec` — which allocates, and is therefore not async-signal-safe — on a stated
fact about the process:

> `fork` in a **single-threaded** parent gives the child a heap no other thread
> can have been holding, so `malloc` there cannot deadlock. `rivet-sysd` has no
> `thread::spawn` anywhere in it — it is a `signalfd` and a poll loop. **If that
> stops being true this stops being safe.**

`cmd/rivet-sysd/src/display_hours.rs` spawns a detached thread, and its own
comment concedes the thread may never finish:

> The thread is detached and its result is picked up by `collect` on a later
> pass; if it never finishes, this machine leaks one thread that is blocked in
> an ioctl, and does everything else.

**Both were checked against the tree rather than taken from the comments.** A
search of `cmd/rivet-sysd/src` for `thread::spawn` finds the call in
`display_hours.rs` and, otherwise, only `kernel.rs`'s own assertion that the
crate contains none. The assertion and its counter-example turn up in the same
search result.

**What the violated invariant does imply.** The safety argument for the `setenv`
is conditional and its condition can be false. If the supervisor forks while
another thread holds an allocator lock, the child can deadlock in `malloc`
inside `pre_exec` — and because `Command::spawn` blocks the parent on a
close-on-exec status pipe until the child either execs or reports an error, a
child deadlocked there **stops the supervisor**, in a pipe read, indefinitely.
That is a complete and plausible mechanism for a wedge, and it should be closed
on its own account whatever caused this incident.

**What it does not imply.** It is not a finding about this board:

* The window is narrow. The deadlock needs the `fork` to land while the other
  thread is *inside* the allocator, not merely while it exists.
* The thread cannot be created on this machine. `Television::converge` returns
  before it can spawn anything when the document names no display window, and
  **`rivet-01`'s document carries no `[display]` table** — read off the board.
* And that board's supervisor answered `Threads: 1` when this was written, which
  is the reading the button would have shown.

So this is a **latent defect with a real mechanism and no evidence linking it to
the observed failure**, and the honest statement is exactly that. The instrument
is what makes the two distinguishable: `Threads` greater than `1` on the button
is the tell, and it costs one line.

---

## 8. What is deliberately NOT designed here

Stated so the absences are not read as oversights.

* **The non-blocking-logging repair.** `.rulings/` R-195 D1 allows it to be
  written and left on a branch, unmerged, and forbids merging it until the
  failure has been reproduced under instrument. The reason is in the ruling: a
  correction merged now, followed by three or four quiet days, leaves nobody
  able to say whether the fault was fixed or merely absent.
* **The watchdog.** Queued by `.rulings/` R-192 D1 as its own ADR — the
  alive-enough criterion, the trial-boot interaction, reboot-loop prevention,
  and telling the next boot that a watchdog caused it. §4.2 depends on it for
  its warm reset and does not design it.
* **Any fix at all**, to either suspect in §7.
* **The audit of the supervisor's blocking calls.** §3.8 says what it would
  settle and why §3.5 waits on it. It is the next job, not this one.
* **Whether the supervisor should stop being single-threaded**, or whether the
  `setenv` should become a hand-written `fork`/`execve` as its own comment
  proposes. Both are repairs.
* **A Fleet Monitor signal.** §4.4 says why the shape is a listener rather than
  a signal; the decision is that repository's.
* **The invariant of §1.1 itself** — making it impossible for a service, a
  device or a filesystem to block the supervisor. That is the deeper target and
  it is a repair, not an instrument. This document builds the witness; it leaves
  the invariant to whatever the blocking-call audit turns into.
* **Making `/proc` readable to applications in general.** §3.4's grant is one
  process's directory for one application, and widening it is a policy decision
  about what an application may do, which `AGENTS.md` §1.1 puts in the third
  row.

---

## 9. Priority, and what would prove any of it

**Build the button first**, because §6's table says the evidence for the failure
that actually happened is entirely in `/proc` and none of it is in the kernel.
Build it after the blocking-call audit of §3.8, because that audit decides
whether the privilege question of §2.2 is forced.

**Add the ramoops region second**, as cheap insurance against a different
failure, and read it as insurance rather than as a diagnosis of this one.

**Neither is proved by being written.** What would prove the button is a wedge,
caught, with a person opening the page and finding either a supervisor whose
counters have stopped or one whose counters are moving — and both answers are
worth having, because the second one would mean every hypothesis in §7 is wrong
and the search starts again somewhere else.

What would prove the ramoops region is a boot that finds something under
`/sys/fs/pstore`. Until then it holds a reserved region and an empty directory,
and this document says so rather than counting it as coverage.

---

## History

### 2026-09-16 — §3.8 stops being provisional

§3.8 originally held §3.5's metric list open, naming four questions that only an
audit of `rivet-sysd`'s blocking calls could settle, and instructing that nothing
in §3.5 be built as final before that audit existed. That audit now exists
(`docs/design-candidates/Supervisor-Blocking-Audit-2026-09-16.md`) and its §10
tested the hypotheses against Fleet Monitor's stored copy of the 2026-09-16
incident. Three of the four questions resolved in §3.5's favour; the second —
*whether `wchan` or `stack` are load-bearing* — resolved **against** the button,
and that is why the section was rewritten rather than merely marked settled.

The argument that survives, and now sits in the body: `wchan`, `syscall` and
`stack` distinguish diagnoses that are otherwise one indistinguishable `S`, and
`rivet-fleetd` cannot read two of the three. The original text called that a forced
choice between widening `rivet-fleetd` and finding a third route. The audit found a
fourth — the control plane's own root-side reader, which already exists and already
outlives the supervisor — so the choice was never between widening and giving up.

The superseded text follows verbatim.

> ### 3.8 THE METRIC LIST IN §3.5 IS PROVISIONAL
>
> This is the part of the button's design that is **not settled**, and it is
> marked so that a later reader does not take §3.5 for a contract.
>
> **What refines it: an audit of every blocking call in `rivet-sysd`** — which
> waits are bounded, which are not, and what happens when a bound is reached.
> That audit is the thing that can say which states the button must be able to
> tell apart. Today's reading cannot: a supervisor blocked writing to a full
> pipe, blocked in a CEC ioctl, and blocked in an `fsync` are three different
> `wchan` values, three different diagnoses, and one indistinguishable `S`.
>
> What the audit would settle, specifically:
>
> 1. **Whether a bare `State` letter is enough.** If every unbounded wait in the
>    supervisor lands in the same interruptible sleep, then `S` plus frozen
>    counters is the whole signal and §3.5 is complete as it stands.
> 2. **Whether `wchan` or `stack` are load-bearing.** If the audit finds several
>    distinct unbounded waits that matter differently, then the page needs the
>    symbol, and §2.2's privilege question is **forced** rather than avoidable —
>    the design would then have to choose between widening `rivet-fleetd` beyond
>    what `ADR-015` §5.1 allows any application and finding a third route, and
>    neither is decided here.
> 3. **Which counters are the right liveness witnesses.** `voluntary_ctxt_switches`
>    is proposed because a `poll` loop moves it every pass; an audit that finds
>    waits which park without a voluntary switch would demote it.
> 4. **Whether a pending blocked signal is reliable enough to show.** `ShdPnd` is
>    proposed on the strength of the supervisor's `signalfd` design; the audit is
>    what says whether there are ordinary states in which it is set.
>
> **Nothing in §3.5 should be built as final before that audit exists.** The list
> is a starting position derived from what is readable, not from what is
> diagnostic.
