# ADR-017: Services and Supervision

* **Status:** Accepted — fully ruled 2026-08-22. §3.1 carries the split and
  its consequences. **§3.1.5 item 1 was amended 2026-08-23** to include
  `/state`; the argument is §3.1.7.
* **That was not the only growth §3.1.5's list has had.**
  **Item 2 was amended 2026-08-23 as well** — PID 1 writes the
  provisioning document to `/state/config`, `ADR-004` §6 step 2. §3.1.9 carries
  that argument, and it is deliberately **not** §3.1.7's: the journal reasoning
  that put the *mount* in PID 1 does not reach a write to a `/state` that is
  already mounted, and §3.1.9 says so before making its own case.
* **§3.1.8 added 2026-08-23** — the runtime `/etc` projection (`ADR-007` §3.5)
  is **the supervisor's**, run before it converges any service. §3.1.5's list is
  deliberately unchanged there, and §3.1.8 records why the `/state` precedent
  does not carry.
* **Two amendments, opposite answers, same day.** §3.1.8 keeps work *out* of
  PID 1 and §3.1.9 puts work *in*. That is the list being applied rather than
  eroded, and reading the two together is the fastest way to see what the rule
  actually is.
* **§4.3 added 2026-08-23 — the supervisor supervises something.** The first
  definition exists (`rivet-netd`), §3.2.1 records that Stage 1
  needed **no** ordering mechanism and none was added, and §3.5.1 rules that a
  spawn the kernel refuses permanently is a refusal rather than a retry.
  §4.1's *"`stage0_services()` is empty"* is struck there, and §3.1.9's
  *"nothing reads the stored document yet"* is struck in place.
* **§3.1.3.1 added 2026-08-23 — the first amendment a *board* asked for.** A
  Raspberry Pi 4 printed the "cannot read the service cgroup" warning once per
  service on a boot where cgroup2 was mounted and nothing was wrong. `ENOENT` on
  a service's cgroup was two facts reported as one; §3.1.3.1 separates them by
  reading the cgroup2 root's own `cgroup.controllers`, keeps the warning for the
  case that is genuinely dangerous, and rules that the benign case records
  **nothing** — with the machine-wide fact said once at start-up instead. Both
  branches were then reproduced under QEMU.
* **§3.1.11 added 2026-08-23 — the second amendment a *board* asked for**, and
  the same shape as §3.1.3.1: a diagnostic reporting two facts as one. Item 4's
  zombie audit swept the whole of `/proc`, so a `rivet-netd` that had exited
  and not yet been reaped **by the supervisor** made PID 1 announce
  `ZOMBIES REMAIN — the reap loop is wrong`. Two boots of one image printed
  opposite verdicts. The audit is now scoped to the processes item 4 is
  answerable for — parented to PID 1, or with a parent that cannot be read —
  and everything else is printed and named as someone else's. **Confirmed on the
  board the same evening**, on the first boot carrying the fix and by the same
  `rivet-netd`: the split printed `1 to someone else` where the old code would
  have declared the reap loop wrong.
* **§3.2.2 added 2026-08-24 — the service set may grow after the supervisor has
  started.** The maintainer's 2026-08-24 ruling needed *one
  new idea* in §3.2 and said so in the recommendation: a **service instance
  created at run time**, so that an install request written onto `/state` can be
  taken up on the next convergence pass rather than at the next boot. §6 is
  deliberately untouched — the request creates an instance rather than writing
  `enabled`, which was the trap the ruling named — and **§6.1** records that.
  §4.5.3's *"there is no way for an operator to start this binary with a path
  through the supervisor"* is struck in place, and so is §4.5.4's *"nothing
  constructs it"*. The whole reading half was watched under QEMU on `ADR-024`'s
  own kernel; **no install has run through it on a board.**
* **§6.3.2 BUILT 2026-08-29 — a service can be restarted without restarting the
  machine.** `rivet apply service-restart <name> confirm`, and it is the second
  operation ever to cross the supervisor's socket. The ruling's own text is
  unaltered; the amendment appended to §6.3.2 records four departures and
  additions — the two permitted acts are **one** subject, the subject is
  `service-restart` because `restart` already means *restart the machine*,
  **`REACHABILITY_SET` is reported rather than refused** (a restart converges;
  the set exists against a stop that persists), and the plan is the
  **supervisor's**, sent over `ADR-006` §6.4's `plan` verb. It also corrects a
  sentence it made false: a memory cap is taken **at the next boot**, not at the
  next start of a service. **Not one byte of it has run on a board**, and
  §6.3.2's closing subsection says what four things a board would settle.
* **§6.5 — one application can be stopped and held stopped**, by `apply
  service-stop` and released by `apply service-start`. The hold is bounded by
  the boot and never writes `enabled`; §6's *no manual stop* and §6.3.2's
  refusal of a stop are rewritten to match, and History keeps the old text.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

"Service manager" and "service supervision" appear throughout this record —
`ADR-005` §3 lists it as `rivet-initd`'s job, `ADR-015` gives applications a
`kind = "daemon"` — and **nothing defines what a service is.** That gap is the
subject of this ADR.

---

## 2. What is a service, and what is not

The distinction is implied everywhere and stated nowhere, so state it:

> **A service is supervised by Rivet and appears in the desired-state
> document. A mechanism is owned by the service that wraps it and is invisible
> to the supervisor.**

`wpa_supplicant` is the worked example. `ADR-004` §4 gives `rivet-netd`
ownership of its lifecycle — starting it, driving its control socket, restarting
it. It is therefore **not** a service: it is a managed child, it has no entry in
the document, and the supervisor does not know it exists. The same is true of
`udhcpc`.

The reason this must be a rule rather than an accident: if both the supervisor
and `netd` could restart `wpa_supplicant`, there would be two authorities over
one process, which is exactly what `ADR-011` forbids.

### 2.1 The inventory

| Service | Stage | Owns (as mechanisms) |
| :-- | :-- | :-- |
| `rivet-netd` | 1 | `wpa_supplicant`, DHCP client, addressing, routes, mDNS |
| `rivet-timed` | 1–2 | designed in `ADR-021`. A minimal SNTP crate, not `ntpd-rs` — and the clock is *restored from disk* by the supervisor before any of it runs |
| journal / logging | 3 | **not a service** — part of the supervisor. `ADR-022` §7 |
| `rivet-apid` | later | the REST adapter, when `ADR-006` §5.1's dashboard arrives |
| user applications | 4 | whatever each one wraps |

`rivet-initd` does not appear: it *is* the supervisor.

---

## 3. Supervision

### 3.1 Where the supervisor lives — ruled

`ADR-005` §3 puts service supervision in `rivet-initd`, which is PID 1. That
deserves re-examination, because PID 1 returning or crashing is a kernel panic
(`ADR-005` §2.2), and supervision is where most of the code and most of the
risk is.

| | PID 1 supervises | PID 1 spawns a supervisor |
| :-- | :-- | :-- |
| Precedent | systemd, Android `init` | `s6`, `runit`, `daemontools` |
| PID 1 size | large | minimal — mounts, reaping, one child |
| A supervisor bug | **kernel panic** | supervisor is restarted by PID 1 |
| Complexity | one process | two, plus a handoff |

**Ruled 2026-08-22: PID 1 stays minimal and spawns `rivet-sysd`.**
PID 1 does the non-negotiable work of `ADR-005` §2.2 — mount the
pseudo-filesystems, reap every orphan on the system, never exit — and supervises
exactly one child. Everything else, including all service supervision, lives in
a process that is allowed to fail.

The cost is a process boundary and a handoff. The consequences below are what
that handoff actually requires, and one of them is a benefit that was not the
reason for the decision.

#### 3.1.1 A supervisor crash does not interrupt services

This falls out of the split and is worth more than the kernel-panic argument
that motivated it.

When the supervisor dies, its children are **not** killed. They reparent to
PID 1, keep running, and keep serving. PID 1 restarts the supervisor, which
re-adopts them. A control-plane session stays up across a supervisor restart; so does a
web service.

Under the alternative — supervision inside PID 1 — a supervisor bug is a kernel
panic and *everything* stops.

#### 3.1.2 The supervisor observes rather than remembers

The obvious difficulty with re-adoption is knowing what is already running.
Tracking pids across a restart invites pid-reuse bugs, and if a service exits
while the supervisor is down, PID 1 reaps it and the exit status is lost.

Neither matters, because the supervisor does not need history. On start it
**reads the desired state, observes what is actually running, and converges** —
the same model as `apply` (`ADR-015` §7) and the same division as `ADR-007`
§3.3, where desired state is persisted and observed state is never cached.

A service that died while the supervisor was down is simply observed as not
running, and started. No exit status is required to reach the right answer.

#### 3.1.3 cgroups are the authority for what is running

"Observe what is running" needs a mechanism that does not depend on having
spawned the process. **Each service gets its own cgroup, and cgroup membership
is the authority for which processes belong to it.**

This needs no new machinery — `ADR-015` §5.1 already requires cgroups for
resource limits. It removes pid tracking, pid reuse and stale state files in one
move, and it is `ADR-011` again: the kernel is the authority, and its verdict is
read rather than cached.

#### 3.1.3.1 Amended 2026-08-23: an absent cgroup is two facts, and only one is a hazard

A Raspberry Pi 4 booted a Rivet card on 2026-08-23 and printed this **once per
service**, for `rivet-netd` and for the supervisor's own daemons:

```text
{"sev":"warn","svc":"rivet-netd","msg":"cannot read the service cgroup, so whether it is
already running is unknown; starting it anyway, which would double-start it after a
supervisor restart","errno":2,"detail":"No such file or directory (os error 2) (errno 2)"}
```

**Nothing was wrong with that machine.** The same boot's `/etc` projection
reported `kernel_says_read_only:true`, PID 1 had mounted cgroup2, and
`cgroup.controllers` read `cpuset cpu io memory pids` (§8.1 of `ADR-024`, item 4,
closed the same day). The service's cgroup was absent for the most ordinary
reason available: **the service had not started yet on that boot**, so nothing
had created `/sys/fs/cgroup/rivet/rivet-netd`. No
`started, but could not be placed in its cgroup` record followed, which is the
evidence that placement then succeeded and a *restarted* supervisor would have
observed it correctly.

So the machinery was right and the sentence was wrong, in three separate ways:

1. It said the cgroup *could not be read* when the truth was that it did not
   exist yet.
2. It asserted a consequence — *"would double-start it after a supervisor
   restart"* — that does not apply to the case that actually fires. A service
   which has never run cannot be double-started; §3.1.1's re-adoption model is
   precisely the thing that has nothing to adopt on a first boot.
3. It fired on **every first boot of every service, for ever**, which is how a
   warning stops being read on the boot where it means something.

##### The ruling

**`ENOENT` on a service's cgroup is no longer treated as one fact. The
supervisor asks a second question — does the cgroup2 hierarchy itself exist —
and the pair of answers decides what is said.**

| Hierarchy | This service's cgroup | Severity | What it means |
| :-- | :-- | :-- | :-- |
| readable | `ENOENT` | **nothing at all** | it has not run on this boot. Nothing to adopt (§3.1.1), no hazard. |
| readable | any other error | `warn` | something is there, or something refuses; this one service cannot be observed. |
| unreadable | anything | `warn` | there is no authority for what is running (§3.1.3). **This is the dangerous case, and it keeps the old severity and the old argument.** |

##### The discrimination: the cgroup2 root's own `cgroup.controllers`

`ENOENT` alone cannot separate the two, and that is the substance of this
amendment: `ENOENT` is *also* what every path under `/sys/fs/cgroup` returns when
cgroup2 was never mounted there. The kernel creates a `cgroup.controllers` file
in **every** directory of a cgroup2 filesystem, its root included, and in no
other filesystem — so one `read(2)` at
`rivet_core::mount::CGROUP2_MOUNT/cgroup.controllers` answers *is there a
cgroup2 hierarchy here*, independently of whether any service's directory is.

A cgroup **v1** hierarchy has no such file and therefore reads as unreachable.
That is the true answer for a supervisor that speaks only v2: `stage0_plan`
mounts `cgroup2`, `ADR-015` §5.1's limits are v2 limits, and nothing here knows
what to do with v1's controllers.

**Two alternatives were considered and rejected:**

| Probe | Why not |
| :-- | :-- |
| `/proc/self/mounts` | It can say *mounted* about a hierarchy no path reaches. `rivet_core::mount`'s note on `CGROUP2_MOUNT` records exactly that failure — a cgroup2 mounted before sysfs stays in the kernel's mount table while every lookup under it lands in sysfs instead. Reading the root's control file exercises the same lookup `observe` performs, which is the only probe whose answer is about the thing that matters. |
| `statfs(2)` and `CGROUP2_SUPER_MAGIC` | Correct, and no more informative. It would add a second mechanism and a second `unsafe` block to an adapter that is thin by policy, to learn what a file read already says — and `cmd/rivet-initd`'s `report_cgroup_evidence` already reads that file on every boot. One fact, one mechanism (`ADR-011`). |

##### The benign case records nothing, and that is the decision

Not a lowered severity, not softer words — **no record**. Three reasons, in the
order that decided it:

1. **The very next record says it already.** A service whose cgroup is absent is
   one that is about to be spawned, and `started` carries the pid. "It was not
   running" immediately before "it is running as pid N" is one fact written
   twice.
2. **An empty cgroup is already silent**, and it means the same thing to the
   decision: not running, start it. Recording one and not the other would make
   the console depend on *how* the supervisor learned something rather than on
   what it learned.
3. **It is the noise that broke the original.** A line emitted once per service
   on every boot of every machine for ever is the reason the dangerous case had
   nowhere to be heard.

##### What replaces it: one machine-wide line at start-up

The supervisor asks `cgroup_hierarchy()` once during start-up, before it
converges anything, and records the answer in its own voice — `info` when the
hierarchy is readable, `warn` prefixed `DEGRADED` when it is not. This is the
`§3.1.5`-shaped division applied to a record: *whether the authority exists* is a
fact about the machine and is said once; *whether this service may end up
double-started* is a fact about a service and is said per service.

It also closes a hole the old warning had. The old text was reachable only
through a `start`, so a machine with no cgroup2 and every service disabled — or
no services at all — booted in total silence about an authority that was not
there. It now says so either way.

**The per-service latch stays per-service.** Making it supervisor-wide would
name one service and leave the rest of a blind machine unreported, which is the
opposite of what the latch is for.

##### Observed under QEMU 2026-08-23 on `ADR-024`'s own kernel — not on a board

**The emulator reproduces the board's defect, which settles the diagnosis.** The
*pre-amendment* supervisor was rebuilt and booted on the identical initramfs, and
printed the board's two lines on a machine whose own console, thirty records
earlier, had read the controllers off the kernel:

```text
  /sys/fs/cgroup/cgroup.controllers: cpuset cpu io memory pids

{"sev":"warn","svc":"rivet-netd","msg":"cannot read the service cgroup, so whether it is
already running is unknown; starting it anyway, which would double-start it after a
supervisor restart","errno":2}
{"sev":"warn","svc":"rivet-timed","msg":"cannot read the service cgroup, so whether it is
already running is unknown; starting it anyway, which would double-start it after a
supervisor restart","errno":2}
```

A readable hierarchy and a warning that says it cannot be read, on the same
console, two minutes apart — which is the whole argument for this amendment in
five lines. Then both branches of the amended supervisor, in the same session.

The healthy one, from
`tools/qemu/run.sh <init> --supervisor <sup> --cli <cli> --netd <netd>`, where
PID 1 mounts cgroup2:

> **Every fenced console and journal capture in this file predates the
> 2026-08-24 rename** (`PLAN.md` §0.4.7): PID 1 was `rivet-init` and its
> console prefix `rivet-init:`, and the supervisor's `svc` was
> `rivet-supervisor`. They are `rivet-initd`, `rivet-initd:` and
> `rivet-sysd` now. **The captures are not rewritten** — they are what the
> machines said (`AGENTS.md` §5) — so a boot today prints the new strings and
> everything else about these observations is unchanged. Stated once, here,
> rather than at each block.

```text
rivet-init: reading the cgroup2 controllers the kernel actually offers
  /sys/fs/cgroup/cgroup.controllers: cpuset cpu io memory pids
{"sev":"info","svc":"rivet-supervisor","msg":"the cgroup2 hierarchy is readable, so what
is already running can be observed (ADR-017 §3.1.3)",
"hierarchy":"/sys/fs/cgroup/cgroup.controllers"}
```

and **zero** records about a service's cgroup on the whole boot, against two
services started.

The dangerous one, from `--supervisor-as-init`, where the supervisor is PID 1
and nothing has mounted anything:

```text
{"sev":"warn","svc":"rivet-supervisor","msg":"DEGRADED there is no readable cgroup2
hierarchy, so ADR-017 §3.1.3's authority for what is running does not exist on this
machine. Every service starts blind, and a supervisor restart starts a second copy of
anything still running","hierarchy":"/sys/fs/cgroup/cgroup.controllers","errno":2}

{"sev":"warn","svc":"rivet-netd","msg":"there is no readable cgroup2 hierarchy, so
whether this service is already running cannot be established; starting it anyway, which
starts a second copy if a previous supervisor left one running (ADR-017 §3.1.3)",
"errno":2,"hierarchy_errno":2}
```

with the same line for `rivet-timed` — both services, as the latch requires.

**What is still unproven.** The *dangerous* branch has now been observed on a
real aarch64 kernel under emulation, but never on a board, and it never will be
on a healthy one: PID 1 mounts cgroup2 before the supervisor exists, so reaching
it on hardware means an image whose PID 1 failed. The benign branch is the one a
board reproduces, and the board's own confirmation of the *silence* is
outstanding — `PLAN.md` §2.2.

#### 3.1.4 What if the supervisor will not start at all

`ADR-005` §5.4 removed the emergency shell, so there is no drop-to-a-prompt
recovery. That is not a gap, because the correct answer already exists elsewhere
in the design.

**PID 1 is what marks a boot successful.** In the A/B scheme (`ADR-008` §3) the
bootloader marks a slot *trying*; something in userspace must mark it *good*. If
the supervisor cannot be started after its retry budget, PID 1 never confirms
the boot, and the next boot falls back to the previous image automatically.

So the recovery path for a broken supervisor is the same as for any broken
image, and PID 1 acquires one more responsibility: **participating in boot
confirmation.** It is a small amount of code in the one process that must not
fail, and it is the reason the system can recover from a supervisor that will
not run.

#### 3.1.5 PID 1's complete responsibilities

Deliberately short, because everything on this list runs in a process whose
failure panics the kernel:

1. Mount the **boot-time filesystems**: the pseudo-filesystems of `ADR-005`
   §2.2, **and `/state`** — amended 2026-08-23, §3.1.7.
2. Read the provisioning file from the boot partition **and store it**
   (`ADR-004` §6 steps 1 and 2) — amended 2026-08-23, §3.1.9.
3. Spawn and restart `rivet-sysd`, with backoff.
4. Reap **every** orphan on the system, not only its own children — the
   *audit* of that was narrowed 2026-08-23, §3.1.11, because it was checking a
   wider claim than this item makes.
5. Confirm the boot once the system is healthy (§3.1.4).
6. Handle `SIGTERM` for shutdown and reboot — **and `SIGINT`, and `SIGUSR1` for
   a trial boot; amended 2026-08-23, §3.1.10.**
7. Never return.

Anything not on this list belongs in the supervisor.

**The list is still seven items on 2026-08-23 after two separate pushes on it**,
and both outcomes are recorded rather than only the one that changed something:
`/state` joined item 1 (§3.1.7), and the runtime `/etc` projection was argued
for and **kept out** (§3.1.8). A list that only ever grows is not being applied.

**Still seven items after a third push, later the same day** — and this one did
change something, so the sentence above is corrected rather than repeated. Item
6 gained a *signal*, not a job: §3.1.10 rules that a `tryboot` restart
(`ADR-018` §5.1) is a shutdown like the other two and takes the same path. The
count is unchanged, which is the point of stating it as a count.

#### 3.1.6 What a failed mount means, when PID 1 cannot exit — ruled 2026-08-22

Item 1 says mount them; it did not say what happens when one will not mount.
Writing Stage 0 forced the question, and the answer turns on a constraint item 7
already fixes: **PID 1 cannot exit, so "fatal" cannot mean exit.**

**It means: never confirm the boot.** §3.1.4 already made PID 1 the thing that
certifies a boot, and `ADR-018` §5 makes an unconfirmed boot fall back to the
previous image on the next reset. That is the project's existing answer to
"unrecoverable", and reusing it costs nothing and invents nothing.

**Essential means PID 1 is blind or broken without it** — not that the system is
useless without it. The supervisor is separately allowed to fail (§3.1), so its
preconditions are not PID 1's to adjudicate. By that rule the list is **`/proc`
alone**: PID 1 reads it for its own mount convergence, its own zombie audit, and
`ADR-018` §5.3's tryboot flag. It reads `/sys`, `/sys/fs/cgroup`, `/dev/pts`,
`/dev/shm` and `/run` nowhere.

**cgroup2 is the sharpest case this rule has met, and it stays non-essential.**
Added to the plan 2026-08-22, it is the one mount whose absence has a *silent
and severe* consequence: §3.1.3 makes a service's cgroup the authority for what
is running, so without it the supervisor cannot tell a running service from a
stopped one and starts a second copy of something already serving. That is worse
than most things on the essential list.

It is nonetheless not PID 1's blindness — it is the supervisor's, and §3.1 puts
the supervisor's preconditions outside PID 1's jurisdiction deliberately. So the
rule holds, and what changed instead is that **PID 1 now says on the console what
a missing cgroup2 costs**, naming the double start, rather than reporting a
neutral degraded mount. The constant is pinned by a test asserting it is exactly
`["/proc"]`, so promoting anything becomes a decision somebody takes on purpose.

If that trade ever looks wrong, the argument to revisit is here rather than in
the code.

| Verdict | Condition | Behaviour |
| :-- | :-- | :-- |
| Healthy | all six mounted — **seven since 2026-08-23, §3.1.7** | confirm normally |
| Degraded | a non-essential mount failed | say so loudly, carry on, **the boot may still be confirmed** |
| Unconfirmable | `/proc` failed | never confirm; the A/B scheme reverts on the next boot |

In all three, PID 1 attempts **every** spec — a failure on `/dev/pts` must not
cost `/run` — reaps for ever, and never returns. A degraded machine that reaps
and can be reached beats one that rolls back.

**The essential set is correct for Stage 0 and must be re-examined at Stage 1.**
It is derived from what PID 1 itself reads, which is the right test only while
nothing else load-bearing depends on the other four. It will not stay that way:
the day something load-bearing needs a device PID 1 does not itself read, that
device's absence means the machine is unreachable — and under the table above
the boot is confirmed anyway, which is precisely the outcome A/B rollback exists
to prevent. **Reachability, not just PID 1's own sight, becomes the criterion**,
and this section must be revisited then rather than inherited. It is recorded
now because the failure mode is silent: nothing goes wrong at Stage 0, and the
rule quietly stops being right later.

**Amended hours later, 2026-08-22: the re-examination is already due.**
Implementing §3.1.5 item 2 gave PID 1 a dependency
on `/run` — it mounts the boot partition at `/run/boot-partition` to read the
provisioning file, and §3.1.5 item 5 writes `autoboot.txt` through the same
path. So the "derived from what PID 1 itself reads" test now argues for `/run`
being essential, one section above where the table says it is not.

**The behaviour is nevertheless already correct, by mechanism rather than by the
table.** If the `/run` tmpfs fails to mount, its mount point is the read-only
erofs `/system` root, `mkdir_p` fails with `EROFS`, and a trial boot is not
confirmed. Nothing in item 5 is keyed to the essential set — it takes
`BootHealth::may_confirm_boot()` as an opaque input — so widening the set is a
one-constant change whenever it is made.

That is a reprieve, not a correction. The prediction in the paragraph above was
right about the mechanism and wrong about the schedule, which is the more useful
half to have learned: **a rule derived from "what PID 1 reads today" goes stale
the moment PID 1 does anything new**, and that is an argument for re-deriving it
at every ADR pass rather than at a named stage.

**Re-derived 2026-08-23, and the set is unchanged.** `/state` joined the mount
phase (§3.1.7) and PID 1 still reads nothing on it, so `ESSENTIAL_TO_PID1` is
still `["/proc"]` and the constant's pinning test still asserts exactly that.
What is new is that `/state`'s exclusion now rests on `ADR-018` §8 — a rollback
cannot repair it — rather than on the read-nothing test alone, which makes it
the first entry whose non-essential status would survive PID 1 starting to read
it.

**Not yet built:** `BootHealth` is computed and nothing consumes it, because
boot confirmation (§3.1.5 item 5, `ADR-018` §5) is unwritten. Today an
`Unconfirmable` boot prints its warning and keeps reaping; **the rollback half
does not happen.** The decision is made and encoded; the mechanism is not.
Whoever writes the confirmation step should take the verdict as input rather
than re-deriving the question.

#### 3.1.7 Amended 2026-08-23: item 1 grows by one filesystem, and `/state` is it

`ADR-018` §4 makes `/state` the only continuously-written area on the card and
the only writable filesystem in the whole layout. `ADR-008` §3 puts machine
configuration, keys and persistent data on it. **Nothing mounted it.** The mount
point shipped in the image and stayed an empty directory, and that gap was
recorded and then closed the same day.

**This section adds one filesystem to §3.1.5 item 1 and nothing else.** It is
written as an amendment rather than a line quietly added to the list, because
§3.1 is emphatic that PID 1 does the non-negotiable work and *everything else*
lives in a process that is allowed to fail. Growing that list needs an argument.

##### The case for the supervisor, which is the one §3.1 makes

Two clauses of the existing record point at the supervisor and they are not
weak. §3.1: PID 1 "does the non-negotiable work of `ADR-005` §2.2 — mount the
pseudo-filesystems, reap every orphan on the system, never exit". `/state` is
not a pseudo-filesystem; it is a block device with failure modes a `tmpfs`
cannot have. And §3.1.6's essential-set rule — *essential means PID 1 is blind
or broken without it* — puts `/state` outside PID 1's jurisdiction, since PID 1
reads nothing there.

##### Why it is PID 1 anyway

**The journal is the deciding argument, and it is this ADR's own.** §7 puts the
journal's storage under `/state`, and §8 item 2 already answered the general
form of the question when it ruled the journal part of the supervisor rather
than a service:

> A journal that were itself a service could fail to start, and the record of
> *why* would have nowhere to go — the one failure a logging system must not
> have.

A supervisor that mounted `/state` would have exactly that defect one layer
down. The failure to mount `/state` is the single event *least* able to be
journalled, because journalling it is what has just failed. Whatever mounts
`/state` must sit below the journal, and PID 1 is what is below it.

Two supporting arguments, neither sufficient alone:

* **The cost to the process that must not fail is a table row, not machinery.**
  PID 1 already converges mounts by reading the kernel's mount table, already
  treats a refusal as a verdict rather than an error, and already mounts a real
  block device — §3.1.5 item 2 mounts the FAT boot partition. In the supervisor
  the same job costs restart-idempotency logic *in the process that restarts*.
* **§3.1.6 asks for exactly this re-derivation.** Its closing amendment: "a rule
  derived from *what PID 1 reads today* goes stale the moment PID 1 does
  anything new, and that is an argument for re-deriving it at every ADR pass
  rather than at a named stage."

##### `/state` is non-essential, and that is a ruling rather than a default

Under §3.1.6's table a failed `/state` is **`Degraded`**: say so loudly, carry
on, **and confirm the boot anyway**.

That does not follow from "PID 1 does not read it". It follows from `ADR-018`
§8: *A/B rollback does not help here, because `/state` is deliberately never
touched by an update.* An `Unconfirmable` verdict would roll the image back on
the next reset, the previous image would find the same damaged `/state`, and
the machine would alternate between two healthy images for ever without
addressing the fault. **Rolling back a fault the rollback cannot fix is worse
than carrying on**, and a machine that is reachable is one somebody can repair.

The console therefore does the work instead. `consequence_of` — the same
mechanism §3.1.6 introduced for cgroup2 — names what a missing `/state` costs:
no journal (§7), no `/state/config` (`ADR-008` §3.1), no home directories and
therefore no per-account credential files (`ADR-008` §7.3), and no system extensions
(`ADR-018` §6).

##### What is **not** on the list

**PID 1 does not create anything inside `/state`.** `/state/config`,
`/state/home` and `/state/ext` are created by whatever writes into them, because
`mkdir` is idempotent and a directory list in PID 1 is the creep this section
exists to resist. `cmd/rivet-image`'s `main.rs` carried a comment saying PID 1
"creates its tree as root on first boot"; that was a plan, it is not what was
built, and the comment is corrected rather than the code expanded.

**Built and tested 2026-08-23; not run on a board.** The refusal path was
observed under QEMU, which has no `/dev/mmcblk0` and therefore exercises exactly
the `Degraded` branch. The *success* path — an `ext4` on `p5` actually
mounting — is in `ADR-026` §4's right-hand column and is untouched.

#### 3.1.8 Amended 2026-08-23: `/etc` is the supervisor's, and §3.1.5 does **not** grow

`ADR-007` §3.5 gives `/etc` exactly one generator — the runtime projection, on a
tmpfs — and `ADR-008` §5.3.1 says which files it renders. It did not say **what
process runs it**, which is the same omission §3.1.7 had just closed for
`/state`, hours earlier and in the opposite direction.

**Ruled: `cmd/rivet-sysd`, before it converges any service.** This
section exists because §3.1.7 is a live precedent for growing PID 1's job list
and the same reasoning does *not* reach here — and because a line quietly *not*
added is as easy to miss later as a line quietly added.

##### The case for PID 1, which is real

Two clauses point at it. `/etc/shells` is the difference
between a board that can be logged into and one that cannot; and §3.1 puts the
supervisor in the category of things that are **allowed to fail**. A supervisor
that fell over before writing `/etc` would leave a headless machine that nobody
can reach — which is exactly the class of outcome PID 1 exists to keep off the
table.

##### Why it is the supervisor anyway

**The failure that argument describes cannot occur, and the reason is an
ordering rather than a hope.** Everything that reads `/etc` is a *service*
(§2.1), so it is started by the supervisor. A supervisor that dies before
rendering `/etc` also dies before starting anything that reads it; and §3.1.1's
re-adoption case runs the same way round — a service that is *already* up
survived a supervisor that had already rendered `/etc`, because rendering
happens before the first `converge`. There is no ordering in which a process
reads `/etc` on a machine where the projection has not run.

**And §3.1.7's deciding argument does not generalise, as §3.1.7 itself says.**
That section turned on one property: the failure to mount `/state` "is the
single event *least* able to be journalled, because journalling it is what has
just failed", so whatever mounts `/state` must sit below the journal. `/etc` has
no such property. The journal is this process's own (§8 item 2), it is running
before the projection is called, and every step of the projection is recorded by
it — including its refusals, which is a thing PID 1 could only have printed to a
console.

Two supporting arguments, neither sufficient alone:

* **§3.1.6's own rule already answers it.** *Essential means PID 1 is blind or
  broken without it.* PID 1 calls nothing that reads `/etc` — no `getpwnam`, no
  name resolution, and `Kernel::spawn` takes a path. It is not blind without it,
  and §3.1 puts a component's preconditions in the component.
* **The projection is a convergence over desired state, which is this
  process's job by §3.1.2**: "on start it reads the desired state, observes what
  is actually running, and converges". PID 1's item 2 reads the *provisioning
  file* and hands it on; it does not read desired state and has no document.

##### What this costs, stated rather than buried

**A supervisor that cannot start leaves a machine with no `/etc`, and that is
accepted.** §3.1.4 already covers it: after the retry budget PID 1 never
confirms the boot, and the next reset falls back to the previous image. The
machine is unreachable either way in that state, because the thing that would
have answered a control-plane connection is the process that will not start.

##### Two things this section does not do

* **It does not add to §3.1.5.** PID 1's list is unchanged at seven items. This
  is recorded because the *absence* of a change is a decision here.
* **It does not make `/etc` a mount in `stage0_plan`.** `rivet-core`'s
  `etc_is_not_in_pid_1s_mount_plan` fails if anybody adds it, and
  `cmd/rivet-image`'s
  `nothing_is_mounted_over_etc_so_these_files_are_the_live_ones` — written for
  the withdrawn build-time renderer — still passes for the same reason.

##### Built and tested 2026-08-23; not run on a board

Observed under QEMU: the tmpfs mounted, all five files written and read back
byte-for-byte, and `/proc/self/mounts` showing
`tmpfs /etc tmpfs ro,nosuid,nodev,noexec,relatime,size=1024k,mode=755`. What
that does **not** settle is `ADR-026` §4's right-hand column, and one thing
nearer to hand: the emulator's root is an initramfs, so the `/etc` mount point
is created at runtime there, whereas on a board it comes out of the read-only
`erofs` image (`ADR-018` §3) and the `mkdir` is a different branch.

**One finding worth keeping**, because it is the reason the projection reads
`/proc/self/mounts` back rather than trusting its own `mount(2)`: with the
read-only remount's `MS_RDONLY` removed as an injection test, `mount(2)`
returned **zero** and the kernel's table still said `rw`. A success return is
not evidence that a remount took.
#### 3.1.9 Amended 2026-08-23: item 2 gains `ADR-004` §6's second step

§3.1.5 item 2 said "read the provisioning file from the boot partition". It now
says read it **and store it**. `ADR-004` §6 is a three-step sequence and PID 1
performed only the first; the second — *"the result written to the persistent
configuration"* — had no implementation anywhere, because `ADR-014` §2's
`rivet-store` did not exist. It does now, and this section is the argument for
the write being PID 1's rather than the supervisor's.

**This is the second growth of §3.1.5 in one day and the bar has not been
lowered.** §3.1.7 is emphatic that growing the list needs an argument; so does
this, and it is a *different* argument. Reusing §3.1.7's would be the easy
mistake and it does not hold — see immediately below.

##### The journal argument does not carry this one

§3.1.7's deciding argument was that whatever mounts `/state` must sit **below**
the journal, because the failure to mount `/state` is the single event least
able to be journalled — journalling it is what has just failed (§8 item 2).

**That reasoning is about the mount and stops there.** By the time a document is
written to `/state/config`, `/state` is mounted and §7's journal has somewhere
to go. A failed write is perfectly journallable, so a supervisor could own it and
report its own failure honestly. The argument that put the mount in PID 1 gives
this no answer at all, and it is recorded here so that nobody borrows it.

##### What decides it: steps 1 and 2 cannot be separated

The provisioning document exists only inside the process that read it. Moving
step 2 above PID 1 means one of exactly two things:

| Alternative | What it costs |
| :-- | :-- |
| **A second process mounts the boot partition and parses the file again** | A second reader of the one file on the machine that holds a plaintext wireless passphrase (`ADR-004` §6), and a second thing that must agree about when step 3 removes it. `ADR-020` §3 puts *secrets leaking into logs and errors* among the failures that will actually occur, and it is the one this repository can most cheaply prevent |
| **The document crosses a process boundary** | Through `argv`, where it is world-readable in `/proc`; or through a pipe or an inherited descriptor, which is machinery that does not exist and would be invented for one file. The obvious channel — a file both processes read — **is `/state/config`**, which is the thing being seeded |

**And the cost to the process that must not fail is four syscalls.**
`crate::confirm` already performs this exact shape of work on the boot partition
— write a temporary file, `sync`, rename it over the target, `sync` again — so
`Kernel` gained one method (`remove_file`) and nothing else. Every decision lives
in `rivet-store`, above a filesystem port, where the failure paths that matter
are driven by tests rather than by a card.

##### The counter-precedent, stated rather than hidden

`ADR-018` §10.3 refused PID 1 a first-boot job three sections of the record ago,
in as many words: *"It must not be PID 1's job. `ADR-017` §3.1.5 is a complete
and deliberately short list, and this is not on it."*

**That refusal stands and this does not weaken it.** Growing `/state` is a GPT
rewrite and an `ext4` resize — a mechanism `ADR-018` §10.3 records as not yet
surveyed — and it depends on nothing PID 1 already holds, so it could equally be
done above PID 1 and should be. The list resists **machinery**, and it resists
work that has no reason to be here. This is neither: it is the second half of a
step PID 1 already performs the first half of, and it cannot be moved without
creating a second reader of a secret.

##### A seeding fault never makes the boot unconfirmable

Same verdict as §3.1.7's mount, and the same reason (`ADR-018` §8): **a rollback
cannot repair a `/state` that an update never touches.** Withholding the
confirmation would revert to the previous image, which would find the same full
or unwritable card, and the machine would alternate between two healthy images
for ever.

Note the asymmetry with the *read*, which `crate::provision` already had, and
which **does** withhold the confirmation for a document it could not parse. That
fault is on the boot partition, which an update replaces — so there a rollback is
a remedy. The rule is `ADR-018` §5's and not a rule about configuration: a boot
is withheld when the rollback can fix what is wrong.

##### What is still **not** on the list

* **PID 1 does not create anything inside `/state` that it does not write.**
  §3.1.7's sentence stands unchanged and is what this amendment is measured
  against: `/state/config` is created here because this is the thing that writes
  into it, and `/state/home` and `/state/ext` are still not PID 1's.
* **`ADR-004` §6 step 3 — removing the credential from the provisioning file —
  is not implemented and is not merely unwritten: it has nothing to remove.**
  `ADR-007` §7.3 keeps wireless credentials out of the configuration document
  entirely, so the schema declares no key a passphrase could be read under and a
  `rivet.toml` carrying one is refused at parse time as unknown. Step 3
  becomes implementable when `ADR-020`'s encrypted store and a `Secret`-typed
  schema key exist. Not yet ruled.
* **The stored document is read and applied — closed 2026-08-23**, and it is
  listed here because it was open when this list was written.

  **§4.3 is the change.** `rivet-sysd` reads
  `/state/config/machine.toml` through `rivet-store` before it converges
  anything, and it starts `cmd/rivet-netd` with that exact path as its
  `--document`. The value is taken from `ConfigStore::machine_path()` rather
  than written out again, so the reader and the writer cannot drift.

  **Not on hardware, and not off a real `/state` even under QEMU** — the
  emulator has no `/dev/mmcblk0`, so the mount degrades and the file was staged
  in the initramfs at the path the store writes. What was exercised is the
  supervisor reading it and `netd` acting on it; that an `ext4` on `p5` serves
  the read is `ADR-026` §4's right-hand column.

**Built and tested 2026-08-23; not run on a board, and not run under QEMU
either.** The emulator has no boot partition (`ADR-026` §4), so it produces no
provisioning document and takes the "nothing was offered" branch — which
exercises none of this. Every path above is driven through `Kernel` and through
`rivet-store`'s filesystem port by tests. That an `ext4` on `p5` accepts these
writes, and that the card honours the cache flush, are `ADR-026` §4's right-hand
column and are untouched.

---

#### 3.1.10 Amended 2026-08-23: item 6 gains a third signal, and it is `SIGUSR1`

§3.1.5 item 6 said *"handle `SIGTERM` for shutdown and reboot"*. It now covers
**three** signals, and the list is still seven items because no job was added —
what changed is which requests reach the shutdown PID 1 already performs.

| Signal | Means, on a Rivet machine | Reaches the kernel as |
| :-- | :-- | :-- |
| `SIGTERM` | shut down and stay off | `RB_POWER_OFF` |
| `SIGINT` | shut down and come back (the kernel sends it for Ctrl-Alt-Delete) | `RB_AUTOBOOT` |
| **`SIGUSR1`** | **shut down and come back once, in the slot a trial boot armed** | `LINUX_REBOOT_CMD_RESTART2` with `"0 tryboot"` — `ADR-018` §5.1 |

`SIGINT` was never in item 6's text and has been handled since Stage 0, because
the kernel delivers it to pid 1 for Ctrl-Alt-Delete whether or not an ADR says
so. It is written down here rather than left as a discrepancy between the record
and the code.

##### The problem this closes

`ADR-018` §5.3 item 2 — that a `[tryboot]` section in `autoboot.txt` selects a
different `boot_partition` — is **the oldest open item in the project**, and it
had stopped being blocked on implementation. `rivet apply trial-boot` writes
the override; `RebootHow::TrialRestart` makes the syscall. What was missing was
that **nothing on a Rivet board could ask for a restart at all**: there is no
shell and no `kill`, and none of the image's binaries could signal pid 1. A
board could be stopped only by removing power, and a power cycle does not carry
the one-shot flag.

Three shapes were put to the maintainer. **Option 2 is taken.**

##### Rejected: the CLI calls `reboot(2)` itself

The smallest change, and it takes item 6's responsibility away from the process
the record gives it to. `rivet` already has a syscall seam, so it *could* —
and the machine would then go down with services running, `/state` unflushed and
the supervisor mid-write, at the request of whatever is holding a login shell.
Item 6 exists because stopping a machine is a sequence, not a syscall.

##### Rejected: a request file PID 1 polls

Also option 2 in shape, and worse in every detail: it needs a writable path
before `/state` is mounted, an interval that trades latency against wakeups, and
an agreement about who removes the file and when. A signal is a mechanism the
kernel already provides, that pid 1 already reads through `signalfd(2)`, and
that costs one entry in a mask.

##### Rejected: `rivet` asks the supervisor, and the supervisor asks PID 1

The shape a fleet API eventually needs, and much the largest — it is the
CLI-to-daemon channel `ADR-006` §5 deferred. It also makes stopping the machine
depend on a supervisor that §3.1 explicitly permits to be dead. Deferred, not
refused: when that channel exists it can send this signal.

##### Why `SIGUSR1` specifically

`SIGTERM` and `SIGINT` are taken. `SIGHUP` conventionally means *reload
configuration*, and a system that overloaded it would eventually meet something
that sends `SIGHUP` meaning the conventional thing — at which point the board
reboots into an untested slot. `SIGUSR1` has no convention to violate, which is
what it is for. `SIGUSR2` is deliberately left unassigned.

**It is a new contract and it is written down as one.** `SIGUSR1` to pid 1 means
a trial boot and means nothing else, ever, and nothing in the image may send it
for any other reason. `cmd/rivet-initd/src/kernel.rs`'s `Sig::User1` carries
that sentence at the definition.

##### The hazard that makes this more than a match arm

**`SIGUSR1`'s default disposition is to terminate the process.** A dispatch arm
for a signal that is not in the blocked set would be a handler that reads as
working and kills pid 1 the first time it is used — which is
`Kernel panic - not syncing: Attempted to kill init!`. It is blocked because
`HANDLED_SIGNALS` is both the mask and the dispatch set, and the two cannot
drift apart without a compile error or a failing test.

Linux additionally discards a signal sent to pid 1 whose disposition is
`SIG_DFL`. **That is a backstop and is deliberately not relied on**: it does not
apply to this binary under QEMU's `init=` harness when it is not pid 1, it does
not apply in any test, and a mechanism whose safety depends on being pid 1
cannot be exercised anywhere it is safe to exercise.

##### The trial boot takes the ordinary shutdown path

It stops the supervisor, waits for it, syncs the filesystems, and only then asks
the kernel — exactly as a power-off does. A trial boot is the boot **most likely
to be rolled back**, so a card left inconsistent by it is the hardest failure in
the system to diagnose. `ADR-021` §3.3 depends on this ordering: the supervisor
is stopped *first*, which is what gives it a chance to write the clock.

##### What is unproven

`ADR-026` §4 puts `tryboot` in the column QEMU does not emulate at all.
`SIGTERM`'s and `SIGINT`'s shutdowns *can* be driven under the
emulator; `SIGUSR1`'s reaches the syscall and stops there. §5.3 item 2 is now one
**experiment** away rather than one decision away.

**`reboot(2)` with `LINUX_REBOOT_CMD_RESTART2` has executed, and the correction
is smaller than it looks — 2026-08-23.** It ran with `"0 tryboot"`
**once**, in a throwaway PID namespace on a development host. What that
establishes is that the call is *well-formed and accepted by a Linux kernel* —
the argument reaches the kernel, the command constant is right, and nothing
`EINVAL`s. **It establishes nothing whatever about the firmware**, which is the
half `ADR-018` §5.1 depends on. Do not let it be read as the mechanism working.

#### 3.1.10.1 Amended 2026-08-23: the sending half exists

§3.1.10 closed with *"the `rivet apply` subjects that would send these signals …
are a separate change in `cmd/rivet`; what exists today is that the signals are
**receivable**."* That change landed the same day and this section records it, so
that the record does not have to be assembled from two files.

**Three subjects under the existing `apply` verb**, because `ADR-006` §6.4's verb
set is closed and `reboot` is not in it:

```
rivet apply power-off confirm        → SIGTERM  → RB_POWER_OFF
rivet apply restart confirm          → SIGINT   → RB_AUTOBOOT
rivet apply trial-restart confirm    → SIGUSR1  → LINUX_REBOOT_CMD_RESTART2, "0 tryboot"
```

The operation is `rivet_core::operation::stop`, in the core where `ADR-006` §3
requires it; the signal is `cmd/rivet`'s `shutdown` module over a fifth
`unsafe` block in its `syscall` wall. `ADR-006` §8.5 carries the administrative
shape and what one catalogue entry with four subjects costs.

##### The interlock, which is the part that had to be got right

**`kill(1, SIGTERM)` on a development host signals that host's own init.**
`cmd/rivet` is built for the board and is also runnable, by an agent or by the
maintainer, in a checkout under WSL or on a Linux workstation. So:

> **Nothing is signalled unless `/proc/1/comm` reads exactly `rivet-initd`, and
> an unreadable `/proc/1/comm` refuses.**

Three properties, each deliberate:

1. **It fails closed.** `AGENTS.md` §7 — an error and a true negative are
   indistinguishable — so the direction taken when the reading is missing is
   *refuse*.
2. **It runs on the plan path as well as the confirmed one.** An operator on a
   workstation is refused *before* being shown a shutdown they were never going
   to get, so the refusal is not mistaken for the confirmation being wrong.
3. **`comm` decides and `/proc/1/cmdline` only corroborates.** `comm` is the
   kernel's own name for the process; `cmdline` is argv, which is a claim, and is
   empty for a kernel thread. Both are quoted in the refusal so an operator can
   tell a workstation from a broken board. Making `cmdline` part of the gate
   would add a way to refuse a real board without adding a way to catch a
   development host, since every development host fails the `comm` check already
   — `systemd`, `init`, `launchd`, WSL's `/init`.

`AGENTS.md` §9 asks for the target to be **named** rather than the surroundings
observed. There is exactly one pid 1 and its number is fixed, so naming is not
available and this is the strongest form of the same idea that is: the target
must identify itself. The pid in the syscall is a literal `1` and the signal is a
three-member enum, so nothing in that crate can ask the kernel to send pid 1 an
arbitrary signal — the other half of the agreement `HANDLED_SIGNALS` makes here.

##### `trial-restart` does not check that anything was armed, and that is a ruling

The obvious interlock, considered and **not** implemented, for a reason specific
to this hardware rather than for cost: **there is no unarmed state.**
`cmd/rivet-image` writes a `[tryboot]` section naming the non-default slot into
every card it builds, so `[tryboot]` names a slot from the moment a card exists
and `rivet apply trial-boot` only ever *moves* it.

An interlock reading *"was this armed by an operator"* would therefore need a
second record — a flag written at arm time — which is the second authority
`ADR-011` forbids and which would also be **wrong**: the firmware honours
`[tryboot]` whatever Rivet believes about who wrote it, so a lost flag would
refuse a restart that was going to happen and a stale one would permit a restart
that was not intended. `AGENTS.md` §5's *derive, never store* points the same way.

What is done instead is weaker and honest: the `trial-restart` result says in
capitals that it does not check what is armed, and names the command that reads
it — `rivet apply trial-boot <a|b>` without the confirmation word, which is a
plan and changes nothing.

**If the maintainer wants the check**, the shape that would not duplicate
anything is recorded rather than left implicit:
`rivet_adapter_image::BootSlots` gains `armed_slot`, reading the same
`[tryboot]` line `arm_trial_boot` writes, and `Machine` gains a domain method
over it. It is not built because it would put a FAT mount on the shutdown path,
and a machine that cannot be stopped because its card will not mount is a worse
failure than the surprise it prevents.

##### What was actually observed, and by what method

Both halves of the interlock were driven with the **real binary** against a
**real `/proc`**, because a unit test that supplies its own bytes proves only
that the comparison works.

* **The refusal**, on the development host itself. `/proc/1/comm` reads
  `systemd`, `/proc/1/cmdline` reads `/sbin/init`, and all three subjects refuse
  with `E_UNSAFE` and exit 1 — on the plan path *and* on the confirmed path. The
  message quotes both readings. The host was still running afterwards, which is
  the assertion that matters.
* **The accept path**, in a **throwaway PID namespace** — the same technique that
  first executed `reboot(2)`'s tryboot argument. `unshare -rpf --mount-proc` with
  a copy of `bash` renamed `rivet-initd` as pid 1, so `/proc/1/comm` reads what
  the interlock requires and being signalled costs nothing. All three subjects
  were permitted, exited 0, and the **correct signal arrived**: `SIGTERM` for
  `power-off`, `SIGINT` for `restart`, `SIGUSR1` for `trial-restart`, each caught
  by a trap in that namespace's pid 1.

That settles *this binary sends the right signal to the process it identified*.

##### What is still unproven after this change

**Everything about a Rivet machine.** The signals above reached a renamed
`bash`, not `rivet-initd`; **no Rivet machine has ever been stopped by
anything**, on a board or under the emulator. Consequently:

* PID 1's shutdown sequence — stop the supervisor, wait, sync, `reboot(2)` — has
  never been driven by a request from outside itself.
* `ADR-021` §3.3's `/state/clock` write has never happened.
* The interlock **permitting a real board** is unobserved. A false refusal there
  would look like a safety feature and would block the experiment.

**THE EXPERIMENT RAN — 2026-08-23, late. Everything in the list above is now
observed**, and the list is kept rather than deleted because what it was afraid
of is the useful part:

* **A Rivet machine has been stopped by a request from outside itself**, four
  times in one session — `SIGUSR1` for the trial restart, `SIGINT` for the
  ordinary one — each taking §3.1.10's path: stop the supervisor, wait, sync,
  `reboot(2)`.
* **`/state/clock` is written and read.** The boot after each restart restored
  its floor from the card *before the network existed* — `"was":"1970-01-01…",
  "moved_forward_ms":1787543462183` — and `rivet-timed` then stepped it onto
  true time. `ADR-021` §3 and §4 on a board with no RTC.
* **The interlock permits a real board.** `rivet apply trial-restart` without
  `confirm` returned `E_CONFIRMATION_REQUIRED`, not `E_UNSAFE`. The feared false
  refusal did not happen.

`ADR-018` §5.3.3 carries the rollback this unblocked, which is the result that
mattered. **One thing in this section is now wrong and is the CLI's to fix**: the
`next` text of both stop subjects promises the client will see a dropped
connection rather than an exit status, and in all four runs the answer arrived
and the process exited 0 — PID 1's shutdown is slower than the transport is at
delivering it. The console is still the authority for what happens *after* the
answer; it is simply not true that the answer is lost.

---

#### 3.1.11 Amended 2026-08-23: item 4's audit is about item 4's zombies

**Item 4 is *"reap every orphan on the system, not only its own children"*, and
the audit that checks it was asking a wider question than item 4 makes.** It
swept `/proc` for **any** process in state `Z` and called a non-empty result
`ZOMBIES REMAIN — the reap loop is wrong`.

**The board disproved it in one session.** Two boots of the *same image* printed
opposite verdicts:

```
rivet-init: ZOMBIES REMAIN — the reap loop is wrong.
    pid 103 (rivet-netd) state Z
```
```
rivet-init: no zombies remain, per /proc. Stage 0 reap criterion met.
```

`rivet-netd` is the **supervisor's** child (§4.3): it converges an interface,
exits 0, and stays exited. Between that exit and the supervisor's `waitpid` it is
a zombie, and it is not PID 1's — a process may only reap its own children and
the orphans the kernel reparents onto it. The audit runs the instant the
self-test's own children are reaped, which is **microseconds after the supervisor
starts `netd`**, so whether it catches that window is a coin toss. The reap loop
was never wrong; the criterion was.

**The ruling: the audit is scoped to the same processes item 4 is.** A zombie
whose parent is PID 1 decides the verdict. A zombie with any other parent is
printed — a supervisor that stopped reaping is worth seeing — and named as not
PID 1's:

```
rivet-init: self-test complete; auditing /proc for zombies
  /proc lists 52 process(es); 0 in state Z parented to pid 1, 0 to someone else
rivet-init: no zombies of pid 1 remain, per /proc. Stage 0 reap criterion met.
```

**A zombie whose parent cannot be read counts against the reap loop**, not for
it. Unknown ownership is not an acquittal, and folding it into "someone else's"
would be the one direction of this change that could hide a real defect.

**Why this is a correctness fix and not a cosmetic one.** A diagnostic that
declares a defect on roughly half of all boots is worse than one that says
nothing: it teaches the reader to skip the line, and the line exists precisely
for the boot where it is true. `ADR-005` §2.1 makes reaping one of the four jobs
that justify writing PID 1 at all, so the sentence asserting it works has to mean
something.

**The parent comes from `/proc/<pid>/stat`'s fourth field**, read after the last
`)` for the same reason the state letter is (`comm` may contain parentheses and
spaces). It is the kernel's answer, not an inference from Rivet's own records
of what it spawned — `AGENTS.md` §5's *the kernel is the authority for system
state*.

**What is proven, and by what method.** The split is covered by unit tests in
both directions — a supervisor-owned zombie must not fail the criterion, a
PID-1-owned one must — and each was watched failing with the fix broken. The new
console line was then read off a **live kernel** under QEMU on an aarch64 guest,
where the audit found 52 processes and zero zombies of either kind.

**The "someone else's" branch printed on a real system within the hour.** This
section had said it could not be demanded of a single boot, since producing one
means a board with `rivet-netd` exiting in that window — the same
non-deterministic race that caused the defect. It printed on the **first boot of
the first card carrying this fix**, unprompted:

```
rivet-init: self-test complete; auditing /proc for zombies
  /proc/98 is gone — direct child fully reaped
  /proc/99 is gone — orphan-maker fully reaped
  /proc lists 95 process(es); 0 in state Z parented to pid 1, 1 to someone else
    not pid 1's to reap: pid 102 (rivet-netd) state Z, parent 101
rivet-init: no zombies of pid 1 remain, per /proc. Stage 0 reap criterion met.
```

pid 101 is the supervisor, which is exactly the parentage this section predicted.
**The window is measurable on the same console**: the audit printed at
`mono 3.53`, and the supervisor's record of the exit —
`"svc":"rivet-netd","msg":"exited","status":"exit 0","uptime_ms":223` — lands
at `mono 3.613`. The audit fell inside an ~80 ms gap.

**Under the code this replaced, that boot would have printed
`ZOMBIES REMAIN — the reap loop is wrong`.** So the defect is not merely
reproduced but *demonstrated fixed on the hardware that produced it*, and the
"cannot be demanded of a single boot" caveat was pessimism rather than caution —
the race is close to a coin toss, which is precisely why the old line cried wolf
so often. `evidence/2026-08-23-ab-rollback-withheld-commit.mkv` carries it.

**The other branch printed too, on all three later boots** —
`0 in state Z parented to pid 1, 0 to someone else` — so both arms of the split
have now been read off a board.

---

#### 3.1.12 Amended 2026-08-24: item 1 grows by `devtmpfs`, and the board proved the branch that matters

**The ruling existed; this entry did not.** The maintainer's 2026-08-24 ruling —
*"PID 1 mounts `devtmpfs`, non-essential"* — names **this section** as what
carries it. `15f1acb` implemented
it. What was missing was the ADR text, and §3.1.7's precedent is why that
matters: growing item 1 by `/state` cost a full argument, and §3.1.8 argued
`/etc` and **kept it out**. A ruling recorded only in the status file has not
been documented (`AGENTS.md` §7). Written after the implementation rather than
backdated, so the record shows the order it happened in.

**The argument, made properly.** `/dev` is a pseudo-filesystem of `ADR-005`
§2.2, so item 1 already covers it in principle; what changed is that the plan
now *names* it instead of taking it as given. §3.1.6's essential-set rule
settles the question: PID 1 mounts `/dev/pts` and `/dev/shm` **inside** `/dev`,
so a PID 1 that assumes `/dev` is a mount and is wrong fails two mounts with no
way to say why. Naming it converts an assumption into a check, in the one
process that cannot exit to report a broken assumption.

**Why it could not be rehearsed, and what a board did on 2026-08-24.** The
kernel mounts `devtmpfs` itself under `CONFIG_DEVTMPFS_MOUNT`, so on real
hardware PID 1 always arrives *after* the mount it is being asked to perform.
That is the branch — and `ADR-026` §4 is why no emulator run could reach it:
`tools/qemu/run.sh` boots an initramfs, where `CONFIG_DEVTMPFS_MOUNT` does not
apply and `/dev` is an empty directory. The first card carrying the change
printed, on its first boot and on the reboot after it:

```
[    2.905307] devtmpfs: mounted
[    2.917089] Run /system/bin/rivet-init as init process

rivet-init: performing the stage 0 mount plan
  mounted   proc      proc      on /proc          [noexec,nosuid,nodev]
  already   devtmpfs  devtmpfs  on /dev           (converged, not an error)
  mounted   sysfs     sysfs     on /sys           [noexec,nosuid,nodev]
  mounted   cgroup2   cgroup2   on /sys/fs/cgroup [noexec,nosuid,nodev]
  mounted   devpts    devpts    on /dev/pts       [noexec,nosuid,gid=5,mode=620]
  mounted   tmpfs     tmpfs     on /dev/shm       [noexec,nosuid,nodev]
  mounted   tmpfs     tmpfs     on /run           [nosuid,nodev,mode=755]
  mounted   tmpfs     tmpfs     on /tmp           [noexec,nosuid,nodev,size=64m,mode=1777]
rivet-init: mount phase healthy; the boot may be confirmed
```

`already … (converged, not an error)` is §3.1.6's convergence rule meeting the
only case that exists on hardware. Capture in `evidence/`,
`2026-08-24-first-provisioned-card-devtmpfs.mkv`.

**The plan is eight entries, and `/state` is a ninth mounted after it.** Stated
because `PLAN.md` §0.0.7 said seven while writing the hand-off, and a count that
appears in a hand-off is worth being right.

**Item 1's list grows; the item count does not.** Still seven items, which is the
fourth push on this list to be recorded and the second to change something.

### 3.2 Ordering: socket activation and convergence

**Ruled 2026-08-22.** Rivet does **not** use a declared dependency graph.

* Rivet pre-creates every listening socket — which `ADR-015` §5.2 already
  requires, so that applications never hold `CAP_NET_BIND_SERVICE`.
* All services start together.
* A service that needs something not yet ready **retries**.

The consequences are what make this the right choice rather than merely the
simpler one:

* **A dependency is implicit in connecting to a socket**, rather than being a
  declaration that can be stale or wrong. Ordering declarations are notoriously
  hard to keep correct and their failures are hard to diagnose.
* **It is the same reconciliation model as `apply`** (`ADR-015` §7). The system
  converges rather than being sequenced, which is one idea rather than two.
* It is what `launchd` does and what systemd itself prefers.

**The obligation this places on service authors:** a Rivet service must be
written to retry rather than to assume its dependencies are already up. That is
a real constraint, it is stated here so it is not discovered later, and it is
good practice regardless.

#### 3.2.1 Applied 2026-08-23: Stage 1 needed no ordering, and none was added

Writing the first two definitions (§4.3) is the first time this section has met
real services, and it met the two orderings that look most obviously necessary
in the whole design:

1. **`/etc` must exist before anything that reads it starts**, or
   `getusershell()` reads nothing and every login is rejected before
   authentication — measured on hardware.
2. **The network must be up before a listener is reachable.**

Neither needed a mechanism, and recording *why* is the point of this section:
the absence of a change is a decision, and the next person to meet these two
will otherwise reach for a dependency graph.

**The first is already structural, and §3.1.8 is where it was settled.** The
projection runs in the supervisor *before the first `converge`*, so there is no
ordering in which a service reads `/etc` on a machine where it has not been
rendered. That is not an ordering *between services* at all — it is a phase of
the supervisor's own start-up, one line above the first convergence in
`cmd/rivet-sysd`'s `run`.

**The second is not an ordering, and calling it one is the mistake.** A listener
that binds a wildcard address succeeds on a machine with no addresses at all;
the address appearing later makes it reachable without anything
having been sequenced. The dependency is on a *state of the world*, which
arrives when it arrives — exactly the case §3.2 was ruled for. A declared
`After=network` would buy nothing and would be wrong the moment a second
interface came up later.

**What was considered and rejected: a fixed Stage 1 order.** The tempting
middle path is to keep the convergence model and simply state that the
definition list is ordered, `rivet-netd` first. It is rejected because it
would be a **guarantee nobody can honour**: `converge` starts every service in
one pass with no waiting between them, so `netd` being spawned first says
nothing about it having *finished* — and on this ADR's own model it must not,
because `netd` runs for tens of milliseconds and a listener runs for ever. A
documented order that does not order anything is worse than no order, because
the next reader will build on it.

**The list in `stage0_services`'s successor is nonetheless in a deliberate
order and it is incidental**, in the sense §3.2 requires: `converge` iterates
it, so `rivet-netd` is spawned microseconds before the rest on each pass.
Nothing may depend on that. It is written down here so that a future change
which reorders the list is not mistaken for a behavioural change, and so that
one which *starts* depending on the order is recognised as reopening this
section.

**A third shape looks different from both and is not.** *This service's key
material must exist before it starts* is a genuine happens-before rather than a
state of the world, and the tempting answer is a service that runs first to
create it. This section is why it cannot be, because there is no "first". The
answer the convergence model already gives is that **a service whose material is
not there yet fails and is retried** — `crate::service::Limits`'s first backoff
exists for exactly that case, and its own documentation calls a transient
precondition the *normal* case at boot rather than an error. Three orderings
met, no mechanism added for any of them.

#### 3.2.2 Amended 2026-08-24: a service instance created at run time

**The ruling is the maintainer's, taken in conversation on 2026-08-24 evening,
and it names this section as what carries it.** It put three
shapes of install channel on the table and he took **option A** — the CLI writes
a request naming a payload into a file under `/state`, and each of the
supervisor's convergence passes checks for one. The recommendation said, in the
same sentence, what it would cost:

> It needs **one new idea** in `ADR-017` §3.2 — a service instance created at
> run time — and that is the amendment.

So the amendment is ruled with the option, because A cannot exist without it.
This is that text, written after the implementation rather than backdated, on
§3.1.12's precedent.

##### What was true before, and it was checked rather than assumed

`cmd/rivet-sysd`'s `main::run` built its `Supervisor` from §4.1's
definitions ∪ discovered applications ∪ the stored document, **once**, before the
event loop, and nothing re-read any of the three afterwards. §4.5.3 recorded it
as one of two facts that narrowed the channel question, and it is the one that
made this an ADR amendment rather than an implementation detail: *a request
written anywhere the supervisor could see it would not be noticed until the next
boot.*

##### And discovery itself grows a dimension — `ADR-031`, 2026-08-24

**"Discovered applications" means `/state/app` today and will not tomorrow.**
`ADR-031` rules that an application and its data live together on a **volume**,
that `/state` is the built-in volume, and that a data device is another. So the
set this sentence calls *discovered applications* becomes a union across every
attached volume rather than a listing of one directory.

Two consequences this section must not lose, both `ADR-031`'s and both about
what the supervisor says when it finds nothing:

* **`ENOENT` on a volume's `app` directory currently reports *"no applications
  are installed on this machine"* at `Info`.** That is correct while there is
  one volume and becomes a **lie** the moment a document declares a second. The
  two cases have to be told apart by *whether the volume attached*, not by
  whether a directory exists.
* **Nothing may be created beneath a failed mount point.** Otherwise a machine
  whose data volume did not attach quietly starts a second, empty copy of an
  application's state **on the system device** — which is `ADR-031` §3.2's
  failure arriving through the back door.

`ADR-031` §8 carries the full cost to this section; §11 item 2 recommends that
**this** process is the one that attaches volumes, before it converges anything,
on §4.3's precedent. Neither is built.

##### The amendment

**A supervisor's service set may grow after it has started.** One method —
`Supervisor::adopt` — takes a `ServiceDef` and puts it in the set at `Idle`; the
next `converge` then decides what to do with it by exactly the rules every other
service is subject to. It has one caller, `crate::deploy_request`, and the
narrowness is the design rather than an accident of there being only one thing
to build.

Three properties, and each is a refusal rather than a promise:

| | |
| :-- | :-- |
| **It is not a `start`** | §6 has no manual start and still does not. Adoption adds a *definition*; `decide` is what starts anything, so a run-time instance is subject to `enabled`, to its precondition, to the cgroup observation of §3.1.2, and to §3.5's restart policy exactly as a compiled-in one is |
| **It refuses a duplicate** | a second `ServiceDef` under one name would be two services sharing one cgroup (§3.1.3) and one notify socket (§3.4), which is what `Supervisor::new` already refuses at boot. An instance in `Done` or `Failed` is **replaced** — a second install is a second thing, not a restart of the first — and anything else is refused. The one that matters is a running install: replacing its definition mid-write would leave the supervisor holding a path that is not the one the running process was given, and the next exit would be attributed to the wrong payload |
| **It does not touch `enabled`** | see below; this is the trap the ruling named out loud |

##### The trap, and why it decides the shape

§6 makes `enabled` **the operator's entire control surface**, and the cheapest
implementation of option A would have been to put `rivet-deploy` in
§4.1's compiled-in list with `enabled: false` and have a request flip the flag.
It is wrong, and the reason is §6's own argument pointed at itself:

> **`rivet status` can never disagree with the document.**

A request that wrote `enabled` would be a *second* writer on the one key the
desired-state document owns. The machine would then have a service running that
the document says nothing about, or — worse — one the document says is disabled,
and the two answers would be produced by two mechanisms neither of which knows
about the other. That is the runtime-override tier `ADR-007` §3.4.1 removed,
arriving through a side door.

**So a request creates an instance that no document names and no document can
name.** Nothing in `ADR-007`'s schema refers to it, `document::apply_desired_state`
runs once at boot and cannot have seen it, and the instance's own `enabled: true`
is read from the definition and from nowhere else. §6 is untouched, which is the
whole of why this is an amendment to §3.2 and not to §6.

**The cost is stated rather than hidden:** an operator cannot disable installs by
setting a key, because there is no service to name until they ask for one. The
control they have is not writing the request — which is the same control they
have over every other `apply` verb — and `§6.1` records that this was considered.

##### What was rejected

**A control socket the supervisor listens on** — the rejected alternative. It
is the general answer, it unlocks §6's `restart`, `rivet status` and everything
a future network API wants, and it is a wire format, a versioning story and an
authorisation story — `ADR-028` §4 makes authority an authenticated principal's,
and this would be the first thing in Rivet that had to carry one. **Bigger than
the problem today, and the right thing when the surface is wanted rather than the
install.** The run-time instance this amendment adds is one B would need anyway,
so nothing here is thrown away by taking B later.

**The CLI spawning the installer detached** (option C) — `fork`, `setsid`,
return. Refused, and worth writing down because it is the obvious shortcut: it
loses supervision, loses the cgroup, and loses `ADR-022` §4's output capture,
which happens only for children the *supervisor* spawned. `rivet get install`
would then have nothing to read, which is half of the maintainer's 2026-08-24 install-request ruling.

##### This document already rejected a request file once, and the two are not the same

**§3.1.10's *"Rejected: a request file PID 1 polls"*.** A reader who finds that
section after this one will reasonably ask why the shape refused for the trial
boot is accepted for an install, so the three objections it made are answered
here rather than left to be rediscovered:

| §3.1.10's objection | Against a trial boot | Against an install request |
| :-- | :-- | :-- |
| *"needs a writable path before `/state` is mounted"* | fatal — pid 1 is what mounts `/state`, so a file there does not exist when pid 1 would have to read it | **does not apply.** The supervisor is spawned by pid 1 *after* `/state` is mounted (§3.1.7), and `ADR-022` §7's journal already depends on that |
| *"an interval that trades latency against wakeups"* | fatal — pid 1 has no loop of its own to hang this on, so it would need one | **does not apply.** The supervisor already has a convergence pass, already runs it on every event, and this is one `stat(2)` at the top of one that was going to happen anyway. No timer, no new wakeup, and a machine with nothing pending still blocks on its descriptor |
| *"an agreement about who removes the file and when"* | real, and unanswered there | **real, and answered.** The consumer removes it, before it starts anything, and the ordering is argued above rather than assumed |

**And the alternative that decided §3.1.10 is not available here.** A signal is a
mechanism the kernel already provides — but a signal carries no payload, and an
install is *of something*. `SIGUSR1` could say *reboot into the trial*; nothing
in the signal vocabulary can say *install this file*. That is the whole
difference, and it is why the two sections reach opposite answers about the same
shape without either being wrong.

**Re-reading the definitions, the applications and the document on every pass.**
The tempting generalisation of this amendment is to make the whole service set
live rather than adding one method. It is refused for §3.2.1's reason in a new
place: it would make every convergence pass do three filesystem reads on a
machine where nothing has changed, and it would make *when* an operator's
document takes effect a property of the supervisor's wakeup schedule rather than
of `apply`. One method that one caller uses is smaller and says what it means.

##### Where the request lives, and it is attacker-reachable state

`/state` is writable and survives reboots, so the request is the first thing in
Rivet that is **both** persistent and able to make the supervisor execute
something. Four things follow, and `cmd/rivet-sysd`'s `deploy_request`
module is where each is argued at the code:

1. **The size is read before the contents.** A request is capped at one
   `PATH_MAX`; a `stat(2)` reporting more is a refusal and the file is never
   opened. §3.1 says this process must not fail, and allocating a card-sized
   `String` inside it is how it would.
2. **The request is removed *before* anything is started.** The ruling says
   *"removing the request when it is started"*; the order within that is chosen,
   because the other one reinstalls on every boot for ever — `/state` is where
   the request lives precisely so it survives a reboot. The cost of the safe
   direction is a lost request in a window of microseconds, which the operator
   can see and repair by asking again.
3. **A request that cannot be understood is removed too**, or every subsequent
   pass re-reads and re-refuses it, which is §3.5's card-destroying loop with a
   `stat(2)` in place of a `fork(2)`. A request that could not be *examined* is
   the exception: a card that will not `stat` will not unlink either, so that
   one is reported once and left.
4. **The path is not the payload.** A request names a file; everything about the
   file is still refused by the installer under `ADR-029` §11's single-verifier
   rule. Writing a request buys the ability to name a file, not to install one.
   It defends nothing against somebody who can already write `/state` as uid 0 —
   nothing at this layer could, and that is a machine whose `/state/app` they can
   write too (`ADR-015` §7). The boundary held is the unprivileged one:
   `cmd/rivet-image` creates `/state` with `root_owner=0:0`, so an
   application's own uid (`ADR-015` §5.1) cannot create a file in it.

##### Observed 2026-08-24 under QEMU on `ADR-024`'s own kernel — **not on a board**

`tools/qemu/run.sh` gained `--deploy-request` and `--deploy` so this could be
watched rather than inferred. A machine booted with a request naming a payload
that does not exist, and the console carried the whole reading half in one
correlation chain:

```
supervisor  an install was requested and the installer is now a service of this
            supervisor. NOTHING IS INSTALLED YET …
            "payload":"/state/nonexistent-payload.rvt","svc_name":"rivet-deploy"
rivet-deploy  started  "pid":60,"program":"/system/bin/rivet-deploy","attempt":1
rivet-deploy  exited   "status":"exit 3","uptime_ms":75,"restart":"never"
rivet-deploy  the install did not complete: cannot read this machine's trust
            store at /system/trust … "outcome":"refused-machine"
```

**What that settles**, and it is the reading half only: the file was found,
parsed, consumed and removed on the *first* convergence pass; an instance existed
that did not exist when the supervisor started; `decide` spawned it under a
policy of `never`; and the terminal record carrying `ADR-006` §4.1's outcome
token reached the journal in the installer's own name, which is what `rivet get
install` reads. The request record appears **once** across the whole boot, which
is the removal working.

**What it does not settle.** The installer refused because the emulator's
initramfs carries no trust store — a fact about the harness (`ADR-026` §4), not
about the channel — so **no payload has been installed through this route on any
machine.** Nothing has run on a Raspberry Pi: `ADR-026` §4's right-hand column
owns `/state` being an ext4 on real media, a request surviving a real reboot, and
a 640 MiB write. The writing half — `rivet apply install <digest> confirm` —
has run only against a filesystem in a map. **"It works in QEMU" is not "it
works."**

### 3.3 Readiness is distinct from started

A service reports readiness separately from having been started, so that
`rivet status` can distinguish `running` from `running, ready`. Reporting a
service as up when it is not yet serving would be the same dishonesty
`AGENTS.md` §6 rejects — "wired up" is not "works".

Supplying this is the SDK's job (`ADR-036`, `ADR-013` §6, `ADR-015` §9).

### 3.4 Adopt existing conventions, not just existing code

`ADR-009` says reuse rather than reinvent. That applies to *conventions* as well
as implementations, and here it costs nothing:

* **Socket passing uses systemd's `LISTEN_FDS` convention** — `LISTEN_FDS` and
  `LISTEN_PID` in the environment, descriptors from 3 upward. The supervisor
  binds a TCP or UDP socket on `127.0.0.1` for an application whose manifest
  declares a `proxy-path` and on every address otherwise, and a port declared
  `:direct` on every address (`ADR-046` §7.1, `.rulings/` R-188); each start
  journals the addresses bound.
* **Readiness uses the `sd_notify` convention** over `NOTIFY_SOCKET`.

Rivet does not run systemd and never will. Adopting its wire conventions means
existing Rust crates work unmodified and that software already written for
socket activation runs on Rivet without changes. Inventing equivalent
protocols would cost the same effort and buy nothing.

### 3.5 Restart policy

`always`, `on-failure`, or `never`, with **exponential backoff and a give-up
threshold.** After the threshold the supervisor stops trying, marks the service
failed, and reports it.

**The threshold counts consecutive failures, not a lifetime's.** A run that
stayed up for the success window (`Limits::success_window_ms`) is a success,
and the exit that ends it starts the count again. Without that, a service that
fails once a month would one day be marked failed on a machine where it works.

A service that restart-loops indefinitely on a Pi writes to an SD card until the
card dies, so giving up and reporting is not merely tidier — it is the
difference between a broken service and a broken machine.

### 3.5.1 Amended 2026-08-23: a program the image cannot run is refused, not retried

§3.5's three policies and §3.2's retry model both assume the failure is
**temporal** — a precondition that is not ready *yet*. The first definitions
(§4.3) produced a failure that is not: **a service is defined and the program it
names is not in the image**, which is what a definition and a build disagreeing
looks like from the supervisor's side.

**Ruled: when the *spawn* fails with an errno that says the program cannot be
executed, the service is marked failed immediately, with no backoff and no
retry.** Four errnos, and each is the kernel's own verdict rather than an
inference (`AGENTS.md` §5): `ENOENT`, `ENOTDIR`, `EACCES`, `ENOEXEC`.

**The argument is `ADR-018` §3.** `/system` is read-only `erofs`, so nothing can
appear there while the machine is running and the fifth attempt fails exactly as
the first did. The backoff would spend four sleeps and five journal lines to
reach the answer the kernel gave in the first microsecond, and — the part that
matters on a serial console — it would bury the one line saying what the machine
has lost under four that do not.

**`EPERM` is deliberately outside the set.** It is what a seccomp filter or a
hardened kernel returns, and a policy can change; treating it as permanent would
give up on a service a later moment would have started. The rule is *the kernel
says this cannot be executed*, not *the start failed*.

**It is not a new state.** It is `Failed`, reached by a different route, so
everything §3.5 already says applies: the supervisor stops trying, the machine
keeps working, and `enabled = false`, apply, `enabled = true`, apply is still
the way back (§6 — there is no manual start either).

**And the record now names what was lost.** A definition carries a
`consequence` string, which is `ADR-017` §3.1.6's `consequence_of` mechanism —
introduced there for a missing cgroup2 and reused in §3.1.7 for a missing
`/state` — applied to a service. *"`rivet-mcpd` could not be started"* and
*"there is no control plane, so a headless board cannot be reached at all, by
anything"* are the same fact and only one of them tells a reader whether to
care. It is a
**definition** key by §4.1's three clauses: a fact the artifact's author
asserts, not a policy the owner sets, and nothing outside the process could
enforce it. It never reaches `ADR-007`'s schema.

**Observed under QEMU 2026-08-23, not on a board.** A definition whose program
the image did not carry produced **one** `error` record naming the service, the
program path, `errno` 2, the reason no later attempt could succeed
(*"`ADR-018` §3 makes `/system` read-only"*) and the definition's own
`consequence` string — with **zero** `will retry after backoff` records on the
same boot. The transcript itself is in git; what matters here is the shape and
the count.

**Extended 2026-08-23 to the step before the spawn.** §4.3.1 gives a definition
an optional *precondition*, and a precondition that fails reaches `Failed` by
this same route and for this same argument — every way one can fail is a fact
about the filesystem rather than about the moment. The console line is
`REFUSED a precondition of this service could not be met…`, and it carries the
same `consequence` field for the same reason.

### 3.5.2 An application missing its declared material is held, not retried as `Failed`

`ADR-055` §3 lets a manifest declare per-machine material an application needs
before it can work. A missing file is not §3.5.1's kind of failure —
`/state/app/<name>/data` is writable, and an operator running `cargo rivet
secrets` or `rivet apply install-secret` makes the precondition true without
touching this machine's image or its supervisor. Retrying it is therefore not
§3.5.1's institutional futility, and refusing it permanently would be the
opposite mistake: holding a service `failed` for a precondition a person is
about to satisfy.

**So it is a distinct state, `waiting-for-material`, and not `Failed` reached
by a different route.** It is checked before every spawn attempt, by asking
the application's data directory whether every file its manifest's declared
material names is there. While anything is missing the service does not
spawn; its journal names what is missing and both commands that install it;
and it is checked again on a poll rather than a push, because nothing on this
machine wakes the supervisor when a file lands in that directory. It never
counts toward this section's give-up threshold — a service that was never
spawned has not crashed, so there is nothing to forgive and nothing to give up
on.

`get status` reports the hold and the same sentence its journal record
carries as the service's `reason` — one fact, one wording, `AGENTS.md` §5.

### 3.5.3 A kiosk client whose compositor stops answering exits and is restarted

A kiosk application draws through the compositor (`ADR-058`), and after every
frame it waits for the compositor's answer to a round trip. **That wait is
bounded by the start-up patience, ten seconds; past it the SDK journals what it
was waiting for and closes the surface, the application exits, and this
section's restart policy recovers it** (`.rulings/` R-210, D18). An unbounded
wait would leave a process alive, holding its surface and its memory, doing
nothing and reporting nothing, which the supervisor cannot tell from a working
one; an exit is a state it already knows how to act on, with backoff and a
give-up threshold. A display that is unplugged never reaches the deadline,
because the compositor answers the round trip with or without one (`ADR-058`
§5.5).

### 3.6 The supervisor's loop does no unbounded work

`rivet-sysd` is one thread and one `poll(2)` loop. Anything that loop waits on
is time in which it reaps nothing, converges nothing, reads no signal and
answers no socket — and a supervisor that does not read `SIGTERM` is one pid 1
eventually kills. **So work whose duration someone else controls leaves the
loop, through a short-lived forked helper, or runs under a bound.** The CEC
worker (`ADR-039` §8) was the first; this section is the rule it was an
instance of. (`.rulings/` R-210 D12.)

| Work | What bounds it |
| :-- | :-- |
| Reading a service's pipe, an ingest connection, a notify socket, `/dev/kmsg` | **64 KiB per source per pass** (`.rulings/` R-244) |
| Emptying an application's scratch directory | a rename in the loop; the deletion in a forked helper |
| Uninstalling an application, evicting a superseded library build, destroying a repository | a rename in the loop; the deletion in a forked helper (§3.6.4) |
| Restoring the splash on a display owner's stop edge | **a 2 s deadline**, then `SIGKILL` |
| A CEC transaction | a forked helper (`ADR-039` §8) |

#### 3.6.1 Each pass reads at most 64 KiB from any one source

A drain that reads until the source would block never returns while its writer
refills the source faster than the reads empty it. With a budget the pass
ends, the rest of the loop runs, and the next pass takes the next 64 KiB.
**64 KiB is one Linux pipe's default capacity**, so a pipe that filled while
the loop was busy still empties in a single pass and its blocked writer is
released at once; only a writer that refills faster than the loop turns is
held to the budget. The constant is `DRAIN_BUDGET_BYTES`, named once in
`cmd/rivet-sysd`'s kernel, and every drain reads through the one function that
applies it. Every drain's read buffer divides it, so a whole-record read — a
datagram, a `/dev/kmsg` entry — is never cut by it.

The other reads in the loop were already bounded below the budget by their own
protocol: a health answer, a kiosk or query request, a facts datagram, the
signal descriptor.

**A pass that found work before it polled still polls**, with no timeout,
before it returns. The pipes, `/dev/kmsg` and the ingest socket are drained
unconditionally at the top of every wait, because a pipe that filled while the
loop was busy has a blocked writer behind it; returning straight away with what
they gave would let a writer that never stops keep every signal and every other
service's notify socket unread for as long as it wrote. Those sources are not
drained a second time in that pass.

#### 3.6.2 Scratch is renamed aside in the loop and deleted in a helper

`ADR-015` §3.2B's scratch directory is emptied at a spawn and after an exit.
Deleting a tree costs time in proportion to what the application left in it; a
rename costs one syscall and frees the name at once. So the loop renames
`tmp` to `tmp.reclaim-<monotonic ms>-<count>` beside it, makes a fresh `tmp`,
and hands the application's directory and the `tmp.reclaim-` prefix to a
forked helper that deletes **every** entry carrying it — so one a power cut
left undeleted is reclaimed by the next. The helper follows no symbolic link,
closes every descriptor it inherited, and reports through its exit status; the
supervisor reaps it beside the services and journals a failure once, as a
warning. A directory that is not there forks nothing.

#### 3.6.3 The splash runs under a two-second deadline

`ADR-015` §6.2's restore runs `rivet-splash` on a display owner's stop edge.
Past **2 s** it is killed, the call answers `ETIMEDOUT`, the screen keeps what
it drew, and the journal says so. A splash that has exited is answered with
what it said even if something it left behind holds its streams, and one the
kernel has not finished yet is reaped later rather than waited for.

The value rests on three facts:

* **The splash's work is one paint.** A run that finds no framebuffer exits in
  under 20 ms on a Pi 4. The paint has not been timed apart from the rest; it is
  one write of a few megabytes, so 2 s is tens of times over any run that is not
  stuck.
* **Its address watch is cut short on purpose.** With the endpoint line enabled
  and no address yet, `rivet-splash` stays up to ninety seconds watching for one
  — measured on a board as about six seconds at an ordinary DHCP boot, all of it
  watching. In the loop that is up to ninety seconds of no supervision. The
  picture is already drawn, so what a stop edge on a machine with no address
  loses is the endpoint line arriving later.
* **It sits inside `rivet_core::shutdown::SUPERVISOR_MARGIN_MS`** (3 s), and a
  compile-time assertion keeps it there: a hung splash on a stop edge during a
  shutdown cannot on its own carry the supervisor past pid 1's grace.

#### 3.6.4 A removal answers at the rename, and a helper deletes

Every other directory the supervisor removes goes the way scratch does: an
uninstall's versions (and, with `purge`, the whole application directory, data
included, in one rename), the retirement of an application the image now
provides (`ADR-060` §3), a library build retention evicts (`SPEC-02` §12), and a
repository an operator destroys (`ADR-025`). Each target is **renamed aside
inside the directory that held it**, the operator is answered, and a forked
helper deletes it afterwards. (`.rulings/` R-245.)

**What "removed" means is the rename.** It is atomic, so from the moment it
returns the target's name is free and nothing on the machine can see the
target: discovery, retention, the repository convergence and git's route in
`rivet-mcpd` all pass an aside entry over without a word. The space comes back
shortly after the answer rather than before it, and the helper's report says
when — at `Info`, since removals are rare and that is the moment the space is
free.

**The aside name is the scratch name with an empty stem**: `.reclaim-<monotonic
ms>-<count>`, where scratch is `tmp.reclaim-<monotonic ms>-<count>`. The
leading `.` is what keeps it from ever colliding with a live name: no
application name, library reference or repository name may begin with one, and
an installed version may not carry the `.reclaim-` mark at all, so a rename can
never land on something live and a helper can never take one. Dropping the stem
also means one prefix covers every removal in a directory, and that no removed
name, however long, can push its aside name past the filesystem's limit on one
entry. The clock starts again at every boot and the count at every start, so a
name a power cut left behind can be the one a new removal wants; a rename that
finds its aside name taken moves to the next count.

**The port has no call that deletes a tree.** The loop renames; only the helper,
after `fork`, deletes. The helper follows no symbolic link and deletes every
entry carrying its prefix, as scratch's does.

**Nothing a power cut leaves set aside outlives the next start.** Before it
retires or discovers any application, the supervisor looks once in each
directory a removal renames inside — the application root, each application's
own directory, the library root and the repository root — and forks one helper
for each that still holds an aside. A directory it cannot list is passed over;
its readers report that in their own words.

### 3.7 The mDNS responder is restarted when its name or address changes

`rivet-mdnsd` reads its inputs at its start and then answers: the machine's
name from the kernel, and its one address from the projection the supervisor
writes it (§3.7.1) — a static address as the document wrote it, or a DHCP
interface's address from that interface's lease. Nothing else would ever tell
it they had moved, so **the supervisor restarts it when they do.**
(`.rulings/` R-210 D7.)

Both changes reach the supervisor as a reload. A renamed machine arrives with
the document, by `apply reload`; a renewed lease arrives with
`rivet-dhcp-hook`'s nudge, which it sends with every lease it writes. So the
supervisor records the responder's inputs at boot, before the first
convergence, and compares them at every reload. On a change:

| The responder is | It is |
| :-- | :-- |
| running, and its name changed, or it had an address and the addresses changed | restarted by the next convergence, the change journalled under its name with the before and after |
| running with no address yet, and a first lease has arrived | left alone: it is waiting for exactly that lease, and reads it from its projection, which the same reload rewrites |
| `Failed` or backing off | started again, because what it gave up on may be what changed |
| disabled by the document, or held stopped by an operator | left alone: this follows the document and the operator, and never overrides either |

**How `rivet-netd` applies a renewed lease to the interface is a separate
decision and is not taken here.** This section is only about the announcement
following whatever address the lease names.

### 3.7.1 The responder runs confined, and is handed a projection of what it announces

`rivet-mdnsd` parses datagrams from any device on the link, so it runs as an
image service with a home (§4.1): the uid `app_identity("rivet-mdnsd")`
derives, `confinement::image_service_policy`, and a home at
`/state/app/rivet-mdnsd`. Nothing it does needs root — UDP 5353 is above 1024
and joining a multicast group takes no capability — so it opens its own socket
and is handed none. (`.rulings/` R-210 D1, R-252.)

Confined, it may read neither the machine document nor the DHCP leases, whose
directory is root's at mode `0700`. The supervisor therefore writes it a
projection of exactly what it announces from, which is the same reading the
supervisor already makes to decide a restart (§3.7):

| | |
| :-- | :-- |
| **Path** | `/state/app/rivet-mdnsd/mdns` (`rivet_core::mdns::path`), root-owned, mode `0644`, in the root-owned home directory, so the responder can neither plant a link there nor rewrite what it announces. The ruleset grants it read and nothing else. |
| **Writer and when** | `cmd/rivet-sysd`'s `deliver_mdns_projection`, wherever the name service's projection is delivered (`ADR-047` §6) and from the same list: at boot before the first convergence, at every reload, and after a finished install. Only while the service is enabled. Every lease `rivet-dhcp-hook` writes causes a reload, so the file follows each lease. |
| **Contents** | One line per fact, then a terminator, below. |

```text
# (a generated header, every line starting with #)
hostname=<the name the supervisor settled, absent when there is none>
address=<interface> <IPv4 address>      one per interface with an address
leaseless=<interface>                   one per DHCP interface with no lease yet
end
```

**It is not a configuration document**, unlike the name service's projection.
A leased address is what the machine *is*, and the document holds only what it
*should be* (`ADR-007` §3.3), so it has no field a lease could go in. The
format is `rivet_core::settings`' kind instead: `key=value`, one fact a line,
with a writer and a strict reader side by side in `rivet_core::mdns`. A line
the format does not hold is refused, never skipped.

**`end` is load-bearing.** The supervisor rewrites the file in place while the
responder may be reading it. Without a terminator, a file cut after the first
of two addresses reads as a valid machine with one, and the responder would
announce an address it should have refused as ambiguous. With it, a short file
is incomplete and never a smaller answer.

**How the responder starts:**

1. **Its socket first, on its main thread** —
   `rivet_adapter_net::mdns::open`: one UDP socket bound to `0.0.0.0:5353` with
   `SO_REUSEADDR`, joined to `224.0.0.251`. A failure exits non-zero with a
   record naming the step (socket, `SO_REUSEADDR`, bind, join) and the errno,
   so the restart policy and `get status` show it. **One failure is a moment
   and is tried again, for up to 30 seconds:** a join refused with `ENODEV`,
   which is what the kernel answers while no interface is up and routed. A
   responder started early in a boot meets exactly that, and without the
   retry it spent its whole restart budget in the five seconds before its
   DHCP lease arrived and was marked failed. `SO_REUSEADDR` lets the
   port be shared with the mDNS client, which binds it too; `SO_REUSEPORT` is
   not set, because between sockets of one uid it load-balances unicast
   datagrams onto sockets that may never read them.
2. **The name, by `uname(2)`**, which needs no path, and **the projection**.
   The two names must agree; a projection naming another machine is refused.
3. **Waiting up to 30 seconds**, re-reading the projection, through the two
   moments that are not states: a file caught mid-rewrite, and a DHCP
   interface with no lease yet. A first lease therefore reaches a responder
   already waiting for it without a restart, as the table above requires.
   Everything else is refused at once.
4. **Answering on the same thread.** A receive or send that fails exits
   non-zero naming which.

**The responder is written over `simple-dns`, not `simple-mdns`.**
`simple-mdns` opens its sockets on a thread of its own, reports a failure only
through the `log` facade, and takes no socket handed in, so a responder built
on it could sit `started` while answering nothing, with no way to learn of it
short of a new logging dependency — which still could not see the thread die.
`simple-dns` is the wire format `simple-mdns` is itself built on and was its
named fallback; answering one `A` record for one name is a short loop over it.
The answer keeps `simple-mdns`'s behaviour — the query's id echoed, unicast to
the sender for a question with the QU bit, the group otherwise — and sets the
authoritative-answer bit RFC 6762 §18.4 requires.

### 3.8 The OOM killer chooses the supervisor never, and the control plane last

When a machine runs out of memory the kernel kills a process, and which one is
weighted by each process's `oom_score_adj`. (`.rulings/` R-210 D4.)

| Process | `oom_score_adj` | Why |
| :-- | :-- | :-- |
| `rivet-sysd` | **−1000**, never chosen | it restarts everything else; losing it loses every restart with it |
| `rivet-mcpd` | **−900**, chosen after everything but the supervisor | it is the way in, and a machine that ran out of memory should still be one an operator can reach. Not −1000, so a control plane that had itself grown without bound remains something the machine can survive |
| every other service | **0**, the kernel's default | |

The supervisor sets its own value before it spawns anything, and says so in
the journal; a refusal is a warning and the boot continues. **Every child is
given its value between `fork` and `exec`, the reset to `0` included**: a
child inherits its parent's value, so without the reset every service would be
as immune as the supervisor. A spawn whose child cannot be given its value is
refused, except on a machine with no `/proc`, where there is no value to
inherit.

This is the other half of `ADR-015` §3.2.1's default memory limit: the limit
keeps one application from exhausting the machine, and this decides what
survives if the machine is exhausted anyway.

### 3.9 An image service keeps exactly the capabilities its definition names

An image service's definition may name the capabilities it keeps, and it then
holds those and nothing else. There are two shapes, and the difference is the
uid:

* **A homed service** (§4.1) runs under its own uid, and `setuid` to a non-zero
  uid discards every capability. Most need none; one whose work needs a single
  piece of root names it and keeps it across the uid change. (`.rulings/`
  R-210 D1.)
* **A uid-0 service** stays root, so it keeps root's ownership of root's files
  — the stored document, a wireless key, the lease directory — and loses every
  other capability: its bounding set is reduced to the list and `NoNewPrivs`
  is set. (`.rulings/` R-253.)

| Service | uid | Keeps | For |
| :-- | :-- | :-- | :-- |
| `rivet-timed` | its own | `CAP_SYS_TIME` | `clock_settime` on the wall clock (`ADR-021` §7.1) |
| `rivet-bluetoothd` | its own | `CAP_NET_ADMIN`, `CAP_SETPCAP`, until its controller is bound | binding `HCI_CHANNEL_USER`, then emptying its own bounding set; it holds nothing while it scans (`ADR-041` §6.6) |
| `rivet-netd` | 0 | `CAP_NET_ADMIN`, `CAP_NET_RAW`, `CAP_NET_BIND_SERVICE` | netlink writes of links, addresses and routes |
| `rivet-dhcp-<interface>` (§3.10) | 0 | the same three | `udhcpc`'s packet socket and port 68; the hook it execs writes the root-owned lease directory |
| `rivet-wpa-<interface>` (§3.10) | 0 | the same three | `wpa_supplicant` driving `nl80211` and reading EAPOL frames |
| every other uid-0 image service | 0 | everything | |
| every other homed service | its own | nothing | |

**The mechanism** is `Program::capabilities` in `cmd/rivet-sysd`, empty by
default. Between `fork` and `exec`, for a non-empty list on a homed service:

1. `PR_SET_NO_NEW_PRIVS`, then the bounding set reduced to the list rather than
   emptied;
2. the Landlock ruleset;
3. `PR_SET_KEEPCAPS`, so the permitted set survives the `setuid` that follows;
4. the groups, gid and uid drop (`ADR-015` §5.1.1);
5. `capset`: permitted, effective and inheritable set to exactly the list;
6. each raised as an **ambient** capability, which is what `execve` carries into
   a program file that has no file capabilities;
7. the sets read back with `capget` and `PR_CAP_AMBIENT_IS_SET`, and the spawn
   refused if the kernel holds anything else.

After `execve` the program holds the list in its permitted, effective and
ambient sets, its bounding set is the list, and `NoNewPrivs` is set, so it can
never gain another. **Ambient, not file capabilities**: the programs are files
on the read-only system slot with none, and `PR_SET_NO_NEW_PRIVS` would refuse
them if they had any.

**A program may give up what it was given, and one does.** The list is a
ceiling, not a duty to hold it. `rivet-bluetoothd` needs `CAP_NET_ADMIN` only
for the bind that takes its controller, so it is started keeping that and
`CAP_SETPCAP`, the capability that lets a process drop from its own bounding
set; once the bind succeeds it empties its bounding, ambient, permitted,
effective and inheritable sets itself and reads them back. It exits when the
controller goes away, and `on-failure` starts it again holding the list. A run
that lasted §3.5's success window starts the give-up count again, so a controller
unplugged and plugged back in is restarted however often it happens.
(`.rulings/` R-253.)

**For a uid-0 service** the same list is applied without the uid change:

1. `PR_SET_NO_NEW_PRIVS`, then the bounding set reduced to the list;
2. `capset`: permitted, effective and inheritable set to exactly the list, and
   read back with `capget`.

The kernel recomputes a root program's sets from its bounding set at every
`execve`, so the reduction is what holds: the program and every child it
starts — a DHCP client's hook included — hold the list at most, and
`NoNewPrivs` stops a setuid file raising anything past it. `RLIMIT_NPROC` is
not set, because the kernel does not apply it to uid 0. `rivet-netd` journals
its own `Uid`, `CapEff`, `CapBnd` and `NoNewPrivs` as its first line, which is
the durable record of what a program that exits within seconds held.

**What is refused.** A list on an application, with or without its own uid, by
`ServiceDef::validate`: an application holds no piece of root. A ruleset on a
uid-0 definition is refused separately (§4.1), so the uid-0 shape is never
also confined. A capability the supervisor does not itself hold fails the
spawn with `EPERM`, rather than starting a program that cannot do its work.

### 3.10 The network's long-lived mechanisms are services derived from the document

The supervisor runs one `udhcpc` per interface the document addresses by DHCP,
named `rivet-dhcp-<interface>`, and one `wpa_supplicant` per interface it gives
an `ssid`, named `rivet-wpa-<interface>`. (`.rulings/` R-253.) `ADR-004` §8 is
the network's side of the decision; this is the supervisor's.

**Derived, like an image service's `enabled`.** They are built at boot from the
stored document's `[network]` table, from programs on `/system`, as image
services: `restart = always`, `ready = none`, uid 0 reduced to the network's
three capabilities (§3.9). They are not applications and have no manifest. A
`[network]` table changed on a running machine reaches them at the next boot,
as it reaches `rivet-netd`. They keep the machine reachable exactly as
`rivet-netd` does, so a reload does not stop one, an operator cannot hold one
stopped, and an application under either name prefix is refused at the install
door.

**Why not `rivet-netd`'s children: the sweep.** When a service exits, or is
stopped, the supervisor asks its cgroup whether anything is left; whatever is
— a helper it forked that outlived it — is sent `SIGTERM`, the service rests in
`Draining` until the cgroup is empty, and `SIGKILL` follows at the stop
deadline. That is what stops a program's helper running on under the devices
and sockets granted to a service that has gone. `rivet-netd` runs once, so a
DHCP client or supplicant it started was in its cgroup when it exited and was
signalled with it: the lease was never renewed. A service of its own has its
own cgroup, its output captured for its whole life, and a restart when it dies.

**`rivet-netd` is run again when what it applies changes.** It is the one
program that writes an interface's address (`ADR-011`). Each lease the client's
hook records arrives as a reload; the reload compares each DHCP interface's
lease file with what was last recorded, and asks the next convergence to run
`rivet-netd` again when one is new, different or gone — whatever state it is
in, as a restart would. The hook nudges on a removal as on a write, so a lease
withdrawn by `nak` or `leasefail` reaches the interface too. A supplicant started after `rivet-netd` has already run gets it
run again too, because the network a supplicant joins is told to it over its
control socket and lives only in its memory.

---

## 4. Where service definitions live: nowhere hand-editable

There is no `/etc/systemd/system` equivalent and there are **no unit files**. A
service definition originates in exactly one of two places:

* **System services** are defined by Rivet itself, in the image. The operator
  configures them — enabled, settings, limits — and does not define them.
* **Application services** come from the application's own manifest
  (`ADR-015` §3.2). Installing the application is what makes the service exist.

**The operator never writes a unit file.** This is `ADR-007` applied to
services: the document is the authority, and there is no second place a service
can be described.

#### Discovery is across volumes — `ADR-031` §8, 2026-08-25

The second bullet is unchanged in substance and changes in every detail of how
it is performed. *Installing the application makes the service exist* stayed
true; **where the supervisor looks for one did not.** It was `/state/app`; it is
now every attached volume's `<volume>/app`, and `ADR-031` §8 names five costs
rather than leaving them to be discovered:

1. `rivet_core::service::APP_ROOT` is a function of a volume root —
   **built 2026-08-25**, and `service::BUILT_IN_VOLUME` is `/state`, so a
   machine with no declared volume behaves byte-for-byte as it did.
2. **`ENOENT` no longer means "nothing installed"** — `ADR-031` §6.1. An
   application root missing because a disk did not turn up is a different fact
   from one missing because nothing was ever installed, and reporting the second
   when the first happened is the trap. **Not handled**: `crate::applications::discover`
   still reads `ENOENT` as *nothing installed*, which is safe only while the one
   volume is the one the machine is running off.
3. **A failure is per volume rather than one failure.** §3.1's existing argument
   generalises without change: an unreadable volume removes that volume's
   applications and leaves everything else running, loudly.
4. **The `/etc` projection walks more than one directory** — built as far as
   carrying the volume on each account (`rivet_core::etc::ApplicationAccount`),
   because `passwd`'s home field is the application's data directory.
5. **Every directory from `/` down to the binary must be searchable by the
   application's uid**, which now includes the mount point and the volume's root
   — neither made by the installer. `ADR-015` §5.1.1's stated nightmare is a
   volume arriving mode `0700` root-owned and every application on it failing
   `EACCES` after the uid drop, at a moment nobody is watching. **Whoever mounts
   checks this**, and nothing mounts yet.

Three of the five are unbuilt because `ADR-031` §11 items 1, 2 and 8 are
unruled — whether a running machine can gain a volume, which process attaches
one, and where it is mounted.

### 4.1 What a definition contains, and where it physically lives — ruled 2026-08-22

§4 said *where a definition originates* and never said **what one is**. Building
the supervisor found the hole: `stage0_services()` was empty and `ServiceDef`
was a Rust struct nothing parsed, because there was nothing in the record to
parse.

**`stage0_services()` stopped being empty on 2026-08-23** and is renamed
`definitions::system_services()` — the old name would have gone stale on the day
it stopped describing Stage 0, and did. §4.3 records what it holds.
**The rest of §4.1 is unchanged and was implemented as written.**

**The rule, and it is a consequence of `ADR-015` §4.1 rather than a new idea:**

> **A service definition ships inside the signed artifact that provides the
> service. It is never a loose file.**

* **Application services** — `[package.metadata.rivet]` in the application's
  own manifest (`ADR-015` §3.2), which §4.1 already brought **inside the
  signature**. That was ruled so an application could not rewrite its own
  permission request; what it executes and how it is restarted belong in the
  same protected space for the same reason. Note there is deliberately **no
  `exec` field** — the artifact is a Cargo package, so the binary *is* the
  crate's binary.
* **System services** — **compiled into `rivet-sysd`**, shipped with the
  image and covered by its signature. Not a data file under `/system`: that
  filesystem is read-only erofs (`ADR-018` §3), so changing a service file means
  rebuilding the image regardless. A file would therefore buy nothing a constant
  does not, while costing a second schema and a second parser — which is exactly
  what `ADR-011` exists to prevent.

**Where a definition lives does not decide what privilege it runs with.** A
system service is ordinarily uid 0 with no confinement, because each does
root's work. One that does not — `rivet-proxyd` (`ADR-060`), `rivet-dnsd`
(`ADR-047`), `rivet-displayd` (`ADR-058` §5.6), `rivet-mdnsd` (§3.7.1),
`rivet-timed` (`ADR-021` §7.1), `rivet-bluetoothd` (`ADR-041` §6.6) — is
compiled in exactly as the others are and still runs under its own uid, its own
Landlock ruleset and a home under `/state/app`, the shape an application has.
One whose work needs a piece of root keeps that capability alone (§3.9);
`rivet-timed` keeps `CAP_SYS_TIME`.
`rivet_core::service::IMAGE_SERVICE_HOMES` names every such service and the
settings keys it understands, which is the one declaration its missing manifest
would otherwise have made.

#### The split that makes this work

The question looked hard because definitions appeared to need to be
operator-facing configuration. They do not. Two different facts, two
authorities:

| Question | Authority |
| :-- | :-- |
| *What is `rivet-mcpd`?* — program, restart policy, how readiness is known | the **definition**: the image, or the signed manifest |
| *Should `rivet-mcpd` run? On which address? With what limits?* | the **configuration document** — `ADR-007` desired state, confirmed by `apply` |

The definition is not operator-editable and never was; §4's "the operator
configures them — enabled, settings, limits — and does not define them" already
said so, without saying where the definition then lived. The supervisor sees
*definitions ∪ desired state* and converges, which is the shape §3.2 already
gives it.

#### Consequences

* **`ADR-015` §3.2's manifest gains two keys**, `restart` and `ready`, and
  nothing else. `kind`, `listen`, `network`, `state` and `memory` are already
  there.
* **`ADR-007`'s schema gains a services table** carrying enable/disable and
  per-service settings — the operator's whole surface. **Built 2026-08-22** in
  `rivet-config`: `enabled` (required) and `memory`. **`listen` was there too
  until 2026-08-24 — it is the signed manifest's; see §4.4.**

  Building it turned the boundary from prose into **a rule with three clauses,
  all of which must hold** before a key may live in the document rather than the
  definition:

  1. Changing it cannot change which code runs (`ADR-015` §4.1 — otherwise the
     signature guarantees nothing).
  2. It is a policy the machine's owner is entitled to set, not a fact the
     artifact's author asserted.
  3. Rivet enforces it from *outside* the service.

  **Clause 3 is the one that decides the arguable cases**, and it is checkable
  rather than tasteful: *if the setting is something the service must be told,
  it is a definition; if it is something Rivet does around the service, it is
  configuration.* A memory cap passes — Rivet writes it to a cgroup and the
  kernel enforces it. `exec` fails all three.

  **A port is the case that shows clause 3 is not sufficient on its own —
  corrected 2026-08-24 (§4.4).** A port *does* pass clause 3, because Rivet
  really does open the socket, and this section used it as clause 3's worked
  example until then. It fails **clause 2**: *which* sockets a program is served
  on is a fact its author asserted about the program, not a policy of the
  machine's owner. All three clauses must hold, and `listen` holds two.

  `exec`, `restart` and `ready` are refused **as definition keys with an
  explanation**, not as unknown keys: an operator arriving with systemd habits
  has a wrong model, and "unknown key" leaves it wrong.
* **`ADR-016` §5's "extensions provide no services" is unaffected** and is now
  better supported: an extension is not a signed artifact that *provides a
  service*, so it has nowhere to put a definition even if someone wanted to.

### 4.2 How readiness is determined — ruled 2026-08-22

§3.3 requires readiness to be declared rather than inferred and did not say how
a service that cannot declare it is handled. A third-party mechanism will never
call `sd_notify`. **Three values, a closed vocabulary per `ADR-006` §6.4:**

| Value | Means | For |
| :-- | :-- | :-- |
| `none` | started counts as ready | a service with no readiness signal — most C mechanisms |
| `notify` | the service sends `READY=1` over `NOTIFY_SOCKET` | anything built against the SDK (`ADR-036`), and any systemd-aware daemon — the convention is not Rivet's to break (`ADR-013` §6.4) |
| `listening` | the supervisor's pre-created socket accepted a connection | a socket-activated service (§3.2) that cannot notify |

`none` is honest rather than lax: it says *this service has no readiness signal*
instead of pretending a spawn is a readiness. `listening` is what keeps such a
service from being ready-on-spawn once socket activation exists.

### 4.3 The first definition — built 2026-08-23

§4.1 ruled what a definition is and where it lives; this is what was put there,
and the one decision writing it forced.

| | `rivet-netd` |
| :-- | :-- |
| Program | `/system/bin/rivet-netd` |
| Arguments | `--document /state/config/machine.toml` |
| `restart` | **`on-failure`** |
| `ready` | `none` — it exits |
| `enabled` | `true`, overridden by the document |
| uid | 0, bounding set `CAP_NET_ADMIN`, `CAP_NET_RAW`, `CAP_NET_BIND_SERVICE` (§3.9) |
| run again | when a DHCP lease changes, and when a supplicant starts after it has run (§3.10) |

`ADR-017` §2.1's inventory is complete at Stage 1 with it: the journal is part
of the supervisor (§8 item 2), `rivet-timed` is `ADR-021`'s and unwritten, and
`rivet-apid` and applications are later stages.

#### The decision: **run-once needs no new vocabulary**

`cmd/rivet-netd` converges every interface the document names and **exits**.
Its own module documentation is blunt about not being a daemon. So the question
§3.5's three policies had never been asked is: *how does a definition say "runs
once, exits 0, stays exited" as distinct from "stays up"?*

**It already can, and adding a fourth value or a `kind` key would have been the
mistake.** `on-failure` plus the supervisor's `Done` state is exactly that
shape:

| `rivet-netd` exits | `wants_restart` | Resulting state | Next convergence pass |
| :-- | :-- | :-- | :-- |
| **0** — every interface configured | false | `Done` | **nothing** |
| 1 — an interface refused | true | `Backoff` | started again |
| 2 — no usable document | true | `Backoff` | started again |
| 3 — no netlink socket | true | `Backoff` | started again |

`Done` is distinct from `Idle` precisely for this: convergence has no memory, so
an `Idle` enabled service is started on the very next pass, which is right for
one that has never run and wrong for one that has finished.

**`never` is the wrong reading of "runs once"**, and this is the trap. It would
leave a machine whose interface was not yet present at boot permanently
unconfigured — and §3.2 is explicit that a service needing something not yet
ready **retries**. The obligation §3.2 places on service authors is met here by
the *policy* rather than by a loop inside `netd`, which is the better place for
it: the backoff, the give-up threshold and the SD-card protection of §3.5 all
come for free, and a retry loop inside `netd` would have none of them.

**A `kind = "daemon" | "oneshot"` field was considered and rejected.**
`ADR-015` §3.2 already has `kind` for *applications*, so it is not a new idea —
but for a system service it would be a second encoding of a fact `restart`
already carries completely, which is `ADR-011` in the small. Every behaviour a
`kind` could select is already selected by the policy, and the two could
disagree.

**What it costs, stated rather than buried.** A document asking for
`method = "dhcp"` makes `netd` exit 1 *deterministically* — `ADR-004` §8's
client is not built — so it is started five times over about four seconds and
then marked failed. That is the honest outcome and it is bounded, but it is five
starts rather than one for a fault the first start already diagnosed. Making the
supervisor understand *which* non-zero exit is permanent would mean the
supervisor knowing `netd`'s exit-code table, which is a coupling this ADR should
not take on. §3.5.1's rule is about the **spawn** failing, which is the kernel's
verdict and needs no knowledge of the program.

#### The operator's half is read now, and two of its three keys are refused

§4.1 says "the supervisor sees definitions ∪ desired state and converges", and
until 2026-08-23 nothing read the desired state at all — `ADR-007` §7.6.2 and
§3.1.9 both said so. The supervisor now loads `/state/config/machine.toml`
through `rivet-store` before it builds anything, and:

* **`enabled`** is applied. §6 makes it the entire control surface.
* **`listen`** is **refused with an explanation**, because §3.2's pre-created
  socket and §3.4's `LISTEN_FDS` handoff are not built.
* **`memory`** is **refused with an explanation**, because `ADR-024` §8.1
  records the cgroup memory controller as absent on the board for a cause
  misdiagnosed twice. A cap reported and not enforced is worse than no cap.

Both refusals are said out loud rather than dropped: `ADR-007` §3.6 — the system
never accepts an input it will discard.

**A document that cannot be read does not stop the supervisor.** The definitions
run with their own `enabled`, loudly. That is `ADR-018` §8's reasoning again —
a rollback cannot repair a `/state` an update never touches, so escalating would
brick the machine rather than fix it — and its cost is stated where it happens:
an operator who set `enabled = false` and whose document then became unreadable
gets that service started.

#### What this settles, and what it does not

**Observed under QEMU 2026-08-23** with `ADR-024`'s own kernel and a virtio NIC:
the supervisor read the stored document, projected `/etc`, started
`rivet-netd` as pid 60 with `--document /state/config/machine.toml`, and
`netd` brought `eth0` up, assigned its address, installed the default route and
read the link back — *configured, carrier present* — then exited 0 and was
**not** started again for the remaining 45 seconds.

**Not settled, and it is most of it** (`AGENTS.md` §6):

1. **Nothing here has run on a board.** `ADR-026` §4's right-hand column is
   untouched, and `bcmgenet` in particular: this was `virtio_net`.
2. **The document under QEMU was read off an initramfs**, not off an `ext4`
   `/state` — the emulator has no `/dev/mmcblk0`, so PID 1's mount degraded and
   the file was simply there. That an `ext4` on `p5` mounts and serves this read
   is `ADR-026` §4's column.
3. **Standard output is inherited, not captured.** §7 says the supervisor
   captures a service's streams and `ADR-022` §3 wraps them; it does not.
   `netd`'s lines above reach the console unstructured, interleaved with the
   journal's JSONL. Closed 2026-08-24, `ADR-022` §9.2: both of §4's capture
   paths are built.

---

### 4.4 Amended 2026-08-24: the split has a direction, and §4's second origin is built

§4.1 ruled *where* a definition lives and §4.3 built the first two. Both are
about **system** services, which is half of §4's own sentence: *"application
services come from the application's own manifest (`ADR-015` §3.2). Installing
the application is what makes the service exist."* That half could not be built,
and this section is why it could not and what unblocked it.

#### The conflict, which was in the record rather than in the code

`ADR-007` §7.2 declared `listen` and `memory` in the **operator's document**.
`ADR-015` §3.2 declared the same two keys in the **signed manifest**. Both were
accepted on 2026-08-22, neither cited the other on those keys, and `ADR-011`
says exactly one thing is authoritative for any fact. So *"where does an
application service's `listen` come from"* had two answers, and any code that
answered it would have picked a side of an unresolved decision — which
`AGENTS.md` §1 puts out of an agent's reach.

#### The ruling, by the maintainer, 2026-08-24

> **The manifest owns what the application *is*** — restart policy, readiness
> contract, the sockets it listens on. **The document owns what the operator
> *limits*** — the memory cap. **Where both name `memory`, the lower wins.**

**The direction is the test**, and it is checkable: `ADR-015` §4.1 protects an
application's behaviour from being rewritten outside its signature, and a cap
runs the other way — the machine's owner protecting the machine **from** the
application. Restated as a rule for the next arguable key: *a key is the
operator's only if the worst they can do with it is give the application less
than it asked for.*

That corrects §4.1's clause 3, which this ADR called "the one that decides the
arguable cases" and which gave the wrong answer for a port. The correction is
struck into §4.1 rather than replacing it.

#### What a definition now holds, and where each field comes from

| Field | System service | Application service |
| :-- | :-- | :-- |
| name | this binary's constant | the directory under `rivet_core::service::APP_ROOT` |
| program | this binary's constant | **derived** from the name — `ADR-015` §3.2 has no `exec` field |
| `restart`, `ready` | this binary | the manifest |
| `listen` | **none** — a system service's address is its own to read today, from the document rather than from the supervisor (`ADR-017` §4.1) | the manifest |
| memory *request* | none — there is no manifest to ask with | the manifest |
| memory *cap* | the document | the document |
| `enabled` | this binary, overridden by the document | `true`, overridden by the document |
| `consequence` | this binary | **empty** — `ADR-015` §3.2's manifest has no such key, and inventing *"the application is not running"* would add nothing to the service name already on the record (§3.1.6) |
| `precondition` | this binary | **none, and never** — a precondition runs a program (§4.3.1), so a manifest that could declare one would be an `exec` field by another route |

**`enabled` starting `true` for an application is a judgement**, not a reading:
§4 says installing an application is what makes its service exist, and `ADR-015`
§5.4 shows the grant happening at install. It is flagged in `PLAN.md` §0.4.3 and
it carries the cost this ADR already accepts for system services — an operator
who set `enabled = false` and whose document then became unreadable gets the
service started.

#### The cap is written to a cgroup, and §3.5's refusal-shaped honesty applies

`ADR-007` §7.2's `memory` was **refused with an explanation** until now, on the
grounds that `ADR-024` §8.1 found the controller absent on the board and *"a cap
reported and not enforced is worse than no cap"*. That reasoning was right and
its conclusion was too strong: §8.2 then found the *same kernel* offering the
controller under an emulator, so whether a cap can be enforced is a fact about
the running machine rather than about this build.

So the supervisor writes `memory.max` immediately after the spawn that created
the cgroup, and reports what the kernel said — `capped` with the number and
which side of the split it came from, or `NOT ENFORCED` at `Severity::Error`
naming the errno. Neither branch guesses.

**The window this leaves is real**: between `execve` and that write the service
is uncapped. Closing it needs the cgroup to exist before the fork and the child
placed into it by `clone3`'s `CLONE_INTO_CGROUP`, which is a change to how this
supervisor *spawns*. `PLAN.md` §0.4.3 carries it.

#### Two things that got worse, said rather than buried

1. **A collision between two `listen` claims is reported, not refused.** One
   document held every claim, so `rivet-config` could refuse one at parse time
   with no knowledge of the machine. The claims are in separate signed manifests
   now and only the supervisor holds them all at once — and refusing there would
   mean choosing which of two signed applications does not boot, on a headless
   machine, when neither has done anything wrong. Nothing binds these sockets
   today anyway (§3.2's pre-created socket is unbuilt), so the collision is a
   fact about two manifests rather than a failure to start.
2. **`ADR-015` §4.2's launch-time verification is now reachable and still
   absent.** The supervisor executes a binary out of `/state`, which is
   writable, having verified nothing. It says so at `Severity::Warn` on every
   discovery, in `ADR-015` §6.1's terms, and that record is the only thing
   standing between this and a machine that runs whatever was written there.

#### Observed under QEMU 2026-08-24 on `ADR-024`'s own kernel — not on a board

An application `demo-app` whose manifest asked for `restart = "always"`,
`listen = ["tcp:8080"]` and `memory = "128M"`, and a document that said
`enabled = true` and `memory = "32M"`:

```text
{"sev":"info","svc":"rivet-supervisor","msg":"an installed application defines a service;
what it IS comes from its signed manifest and only `enabled` and a memory cap are the
operator's (ADR-017 §4.1, ruled 2026-08-24)","svc_name":"demo-app",
"program":"/state/app/demo-app/current/bin/demo-app","restart":"always","memory_request":"128M"}

{"sev":"warn","msg":"NOT ENFORCED ADR-015 §4.2 requires an application's signature to be
verified at every launch and this build verifies nothing…","svc_name":"demo-app"}

{"sev":"warn","msg":"NOT ENFORCED the manifest asks for listening sockets and this supervisor
cannot supply them…","svc_name":"demo-app","listen":"tcp:8080"}

{"sev":"info","svc":"demo-app","msg":"started","pid":66,
"program":"/state/app/demo-app/current/bin/demo-app","attempt":1}

{"sev":"info","svc":"demo-app","msg":"capped","memory":"32M","memory_bytes":33554432,
"memory_source":"document-lower"}
```

and read back **from the running machine**, not from the console that
wrote it: `/sys/fs/cgroup/rivet/demo-app/cgroup.procs` is `66`, its
`memory.max` is `33554432`, and an uncapped service's — nobody capped it — is
`max`.

**What is unproven, and it is most of the interesting part.** Nothing has run on
a board, where `ADR-024` §8.1 says the cap may be refused. Nothing was
**installed**: `ADR-029` §12 item 4's installer does not exist, so the harness
put an unsigned manifest and an unverified binary in place, which is a different
act reaching the same paths. And the cap was **read back rather than exercised**
— no process has been refused memory by it, so what is established is that the
kernel accepted the number, not that it enforces it.

---

### 4.5 The installer's definition — 2026-08-24, and it is the first that is not in the list

Ruled by the maintainer in conversation, 2026-08-24. **The question was
whether the installer is one-shot or long-running, and the answer is one-shot:
nothing needs it resident, and a crashed one-shot is a failed install rather
than a broken machine.** *(`.rulings/` R-036.)*

`cmd/rivet-sysd`'s `definitions::deploy(payload)` is that definition,
and `cmd/rivet-image` puts the binary at
`rivet_core::deploy::PROGRAM_PATH`. **Nothing calls the function**, and the
reason is a ruling rather than an omission — §4.5.3.

#### 4.5.1 It takes a parameter, and that is why it is not in `system_services`

An install is *of something*. `ADR-029` §9.2 step 1 begins with a payload,
`cmd/rivet-deploy` takes its path on `argv`, and there is no default and no
directory it searches — `ADR-011` §3's distinction, the same one that makes
`rivet-netd --document <path>` an argument rather than an address.

§4.1's list is a compile-time constant, read once, before the `Supervisor` is
built. It has nowhere to get a payload from. So this definition is a
**function** rather than an entry, and a test asserts it is absent from the list
rather than a reader having to notice: a `deploy` that appeared there would be
started on every boot with no payload, exit 2, and under any policy but `never`
say so five times.

#### 4.5.2 `never`, and the asymmetry with `rivet-netd` is the argument

Two run-once services, opposite ends of §3.5's vocabulary. §4.3 ruled that
*"run-once needs no new vocabulary"* and `on-failure` plus `Done` is `netd`'s
shape; this one is `never`, and each of the three failure exits says why:

| Exit | Means | Retrying it |
| :-- | :-- | :-- |
| 1 | the payload is refused | re-reads and re-verifies the same bytes to reach the same verdict. §3.5.1's argument exactly: the failure is a fact about the file, not about the moment |
| 2 | `argv` was not usable | identical every time |
| 3 | the machine, the card or the kernel refused | **the only one that might work later** — and it may have written part of a 640 MiB slot, so an automatic second attempt starts writing again over a partial one, unasked |

**The deciding difference is what each service is for.** `netd` converges *the
machine's own desired state* — something it should be doing anyway, so retrying
is the supervisor doing its job. An install is an **operator-initiated act on a
payload the operator named**, and repeating somebody's request without being
asked is not convergence.

The cost is stated rather than hidden: a card that was momentarily busy is not
retried and the operator has to ask again. `rivet get install` is how they
find out which of the three it was (`ADR-006` §8.8.5).

#### 4.5.3 The channel — RULED 2026-08-24 evening; see §3.2.2 — and two facts that narrowed it

The maintainer ruled the channel — a request file on
`/state` — and §3.2.2 carries the amendment it needed. The section is kept whole
rather than rewritten, because the two facts below are what the ruling was taken
on and the first of them is still true.

**There is a way for an operator to start this binary through the supervisor**,
which `ADR-029` §14.4 named as a gap in the same paragraph as the ruling that
created the binary:
`rivet apply install <digest> confirm` writes the request and returns before a
byte is written. Two things were checked rather than
assumed on 2026-08-24, and both narrow the question rather than restating it:

**The operator names an artifact and no longer names a path** — `ADR-052` §7.2,
2026-09-12. The installer still takes a path on its argument vector; what
composes that path is this module's resolution of a digest in the machine's own
artifact store, not anything the operator typed.

1. **There is no CLI-to-supervisor socket.** §3.4's `NOTIFY_SOCKET` is
   `sd_notify`'s convention — created by the supervisor **per service**, handed
   to that service in its environment, carrying `READY=1` and `STATUS=` in the
   *service → supervisor* direction. It is the wrong direction and the wrong
   scope, and it is the only `AF_UNIX` socket in this repository.
2. **This supervisor's service set is fixed when it starts.** `main::run` builds
   its `Supervisor` from §4.1's definitions ∪ discovered applications ∪ the
   stored document, once, before the event loop; nothing re-reads any of the
   three afterwards. **So a request written anywhere the supervisor could see it
   would not be noticed until the next boot.**

**Point 1 is still true and is the reason option A was taken:** there is no
control socket, and A exists so that none is needed.

**Point 2 stopped being true on 2026-08-24 evening**, and §3.2.2 is what changed
it. It was correctly identified here as the thing that made this an amendment
rather than an implementation detail: whatever the channel turned out to be, it
needed the supervisor to start something it did not know about when it started,
and §3.2's convergence model had no such notion. It has one now, in exactly one
method with exactly one caller. `ADR-029` §14 carries the options and the
maintainer's 2026-08-24 ruling. `cmd/rivet/src/deploy.rs` is now the **writing half** of the channel; it
used to carry the refusal an operator saw, naming both facts, and that refusal
is struck in place there for the same reason it went from here.

#### 4.5.4 What has run, and what still has not — `AGENTS.md` §6

**This definition had never been handed to a `Supervisor` — half of that stopped
being true on 2026-08-24 evening.** `cmd/rivet-sysd`'s `deploy_request`
module constructs it from a request file, and under QEMU on `ADR-024`'s own
kernel the supervisor spawned it, watched it exit 3, and captured its terminal
record carrying `ADR-006` §4.1's `outcome` token — §3.2.2 quotes the console.

What is asserted by tests is unchanged: that it
validates, that its path is `rivet-core`'s rather than a second spelling, that
its argument is the bare positional `cmd/rivet-deploy`'s parser reads, and
that no failure exit provokes a restart. None of those is *"an install ran"*.

**And no install has run.** The emulator's initramfs carries no trust store, so
the one execution this definition has ever had refused before reading a payload;
nothing has been installed through this route on any machine, and nothing at all
has happened on a Raspberry Pi.

---

## 5. Extensions provide no services

**Ruled 2026-08-22.** A system extension (`ADR-016`) may not carry a
long-running service. The three-way line:

| | Runs continuously? | Sandboxed? |
| :-- | :-- | :-- |
| **System service** | yes | no — it is Rivet |
| **Extension** | **no — tooling you invoke** | n/a |
| **Application** | yes | **yes** |

Extensions carry editors, compilers and debuggers: things invoked and exited.
Anything that runs continuously is an **application**, so that it gets a UID, a
sandbox and resource limits. Anything that must run unsandboxed for ever is a
**system service** and belongs in the base image — at which point Rivet itself
is being extended, not added to.

The reason to hold the line: extensions are unsandboxed by design (`ADR-016`
§2). Permitting daemons in them would mean signed-but-unconstrained processes
running for ever, and the only remaining distinction from an application would
be "no sandbox" — which is the wrong property to make optional. A database
should be an *application* precisely so that it is confined.

---

## 6. The control surface

**`enabled` is configuration.** Set it false, `apply`, the service stops. It is
a field in the desired-state document like any other, previewable and
attributed.

**`restart` is an operation, not a configuration change.** This is not a
violation of the desired-state model: restarting does not change what the
machine *should* be. It discards current state and lets the supervisor converge
to the *same* document.

**A held stop on one application is an operation too, and it is bounded by
the boot.** `rivet apply service-stop <name> confirm` stops an installed
application and keeps it stopped; `rivet apply service-start <name> confirm`
releases it. The hold lives in the supervisor's memory and is never written,
so a reboot — or a restart of the supervisor — returns the machine to the
document. To stop a service for good, set `enabled = false` and apply. §6.5
carries the rules. *(`.rulings/` R-183.)*

**What the document still guarantees.** The runtime-override tier `ADR-007`
§3.4.1 removed stays removed: no operation writes `enabled`, so the document
remains the only statement of what a machine *should* run, and every boot
converges to it. `get status` reports the document's `enabled` and, for a held
application, gives the hold as the reason it is idle — so a status that
disagrees with the document says why, in the answer itself. A stop the
supervisor immediately undoes would be theatre, which is why a convergence
pass never clears a hold.

### 6.1 Amended 2026-08-24: a request is not a second control surface

**This section is unchanged, and saying so is the point.** §3.2.2 added a service
instance created at run time, which is the first thing in Rivet that puts a
service in the supervisor's set without the document having named it — so the
obvious reading is that `enabled` has stopped being *the entire* control surface.
It has not, and the distinction is worth stating precisely because the
implementation that would have broken it was the cheaper one.

**What a request does is create an instance. What it does not do is write
`enabled`.** The tempting shape — compile `rivet-deploy` into §4.1's list with
`enabled: false`, and have `rivet apply install` flip the flag — was rejected
in §3.2.2, and the reason is this section's own sentence: a request that wrote
`enabled` would be a second writer on the one key `ADR-007`'s document owns, and
`rivet status` would disagree with the document about a service the operator
never mentioned. That is `ADR-007` §3.4.1's runtime-override tier arriving
through a side door.

**So the two are different kinds of thing, and the line is the one this section
already draws:**

| | Is | Reached by |
| :-- | :-- | :-- |
| `enabled` | **configuration** — what the machine *should* be | the document, `apply` |
| `restart` | **an operation** — discard state, converge to the same document | `rivet apply service-restart <name> confirm` (§6.3.2) |
| a held stop | **an operation** — stop one application and keep it stopped until released or the next boot | `rivet apply service-stop` / `service-start <name> confirm` (§6.5) |
| an install | **an operation** — a one-shot act on a payload the operator named | `rivet apply install <digest> confirm`, `ADR-006` §7's plan-then-apply |

An install is `restart`'s kind, not `enabled`'s: it does not change what the
machine should be. Repeating somebody's install request without being asked would
not be convergence, which is §4.5.2's argument for `never` seen from this side.

**The cost, stated rather than hidden.** An operator cannot disable installs by
setting a key, because there is no service to name until they ask for one. What
they have instead is the control they have over every other `apply` verb: not
typing it. **A second control was considered and refused** — a document key that
gated the request — because it would be exactly the second authority this section
exists to prevent, and because a machine that silently discarded install requests
according to a key nobody was looking at is worse than one that does what it is
asked.

### 6.2 Amended 2026-08-25: what a definition's starting value may say

§6 makes `enabled` configuration and gives it to the document. §6.1 established
that a run-time instance does not write it. This adds the third case, which is
the one that had been answered in code rather than here.

**A definition carries a starting `enabled`, and the document replaces it
wherever the document names the service.** So a service the document does *not*
name runs, or does not, according to a value chosen by whoever wrote the
definition. That is a real control surface and it was never described as one.

**The rule, ruled by the maintainer: a starting value of `true` is only ever
legitimate as a *bootstrap* argument — meaning the service is one without which
the machine cannot be reached in order to configure it. Everything else starts
`false`.** *(`.rulings/` R-037.)*

Applied to what exists:

| Service | Starts | Because |
| :-- | :-- | :-- |
| **`rivet-mcpd`** | **enabled** — *2026-08-29* | `ADR-023` §4.1 — a freshly flashed headless board must be reachable, and you cannot reach it to enable the thing you reach it with. See §6.2.1 |
| `rivet-netd` | **enabled** | the same argument one layer down: a machine that does not configure its interface cannot be reached at all |
| `rivet-timed` | **enabled** | one layer down again: a machine whose clock is wrong cannot complete the TLS handshake the control plane is, so it cannot be reached either (`ADR-021` §4) |
| `rivet-mdnsd` | disabled | a board reached by address does not need its name resolved; this is convenience, not bootstrap |
| **any application** | **disabled** | no bootstrap problem exists. The operator is already on the machine, or they could not have installed it. `ADR-015` §3.2.2 |

**Why the test is *bootstrap* and not *usefulness*.** Every service is useful or
it would not be in the image; that criterion would enable everything. The
bootstrap criterion is narrow, checkable, and self-limiting — it names exactly
the services whose absence would leave nobody able to change the answer. A
fourth service claiming it should start enabled has to make that argument and
lose it, which is the point. `timed_is_enabled_exactly_when_the_image_installs_it`
in `cmd/rivet-sysd/src/definitions.rs` asserts the enabled set, and its own
comment says why it is the only place a *card's* behaviour is visible: a board
runs the document, so a wrong starting value can sit here for days while every
board in service behaves correctly.

**This does not weaken §6.** `enabled` remains the operator's entire control
surface, and a starting value is what the machine does when the operator has not
used it. What changes is that silence now means *no* for everything except the
**three** services that make an operator possible at all.

#### 6.2.1 `rivet-mcpd` passes the bootstrap test — and passed it three phases ago

**Amended 2026-08-29.** `cmd/rivet-sysd/src/definitions.rs` shipped `rivet-mcpd`
with `enabled: false`, and said so correctly for phase 1: the listener was
loopback, so a machine without the control plane had lost no way in, and §6.2's
test returned *not bootstrap*. **That definition also wrote down when the answer
would change** — *"it flips to `true` in phase 2"*, `ADR-035`.

**Phases 2 and 3 both completed, were proven on hardware, and the value never
moved.** It was corrected on 2026-08-29.

**What the omission actually did, stated precisely, because the obvious reading
is wrong.** No board was ever unreachable. A board runs the *document*, and the
document has enabled `rivet-mcpd` since phase 2 — which is why deploys, installs
and trial-restarts all worked over TLS. **A freshly flashed card is the only
thing that runs these defaults**, and every card flashed since phase 2 was
flashed by someone who then reached it through a control plane the provisioning
document had already turned on. So the defect was real, latent, and invisible to
every test that used a board.

**It also made the phased-removal plan's own justification true by accident.**
That sequencing argument was *the old recovery path stays until the last phase*;
while this line stood, the old path genuinely was the only way onto a fresh card
— but only because the image defaults said so, not because the control plane
could not do the job.

**The general finding, which is `PLAN.md` §94.7's:** a value whose trigger for
being revisited is *somebody happening to remember* does not get revisited. This
one carried its own trigger in a doc comment and still did not fire. `AGENTS.md`
§7 asks for claims that can go stale to be testable; the fix that came with the
correction is a test — `the_reachability_set_is_the_control_plane_and_the_interface`
— because **nothing had pinned the reachability set at all.**

### 6.3 Amended 2026-08-29: control surfaces, ruled daemon by daemon

**Moved here from `PLAN.md` §95 on 2026-08-29, unaltered.** Ruled by the
maintainer, Joel Lindell, the same day. Only the heading numbers changed; every
line of the body below is the text as it was ruled. **Bare `§N` references in it
are `PLAN.md` sections** and resolve through that file's pointer table — `§93.3`
is the scope correction that raised the question, and `§94.7` is the finding that
this section's *"recorded as the next control surface"* answers.

**Why here and not in `ADR-032`.** The transport half is that ADR's — §6.3.1
finishes the sentence `ADR-032` §3.2 already wrote about the query socket — but
the ruling is one ruling, and splitting it across two files would give it two
homes and eventually two answers, which is `ADR-011`. `ADR-032` §8.1 carries the
pointer. What §6.3 amends here is §6 itself, and §6.5 amends §6.3.2 in turn:
a stop held on one application is permitted, bounded by the boot.


§93.3 tabled *how* service control reaches the supervisor and left it to the
maintainer. **He ruled it, then widened the question the way he widened §93.2:
rather than adding a command, should the project implement a proper control
surface for `sysd` and make service control the first tenant of it — and does
`initd` want any control surface extended to the admin MCP at all?**
*(`.rulings/` R-038.)*

Ruled below, daemon by daemon. **The general shape is: one control surface, on
the supervisor, carrying the operation grammar the socket was always designed
for — and nothing else in the system grows one.**

#### 6.3.1 RULED: sysd gets a real control surface, not a second command

`ADR-032` §3.2's socket carries `REQUEST = "get status\n"` — **a whole
operation, deliberately**, and that module already said why:

> **It is a whole operation and not the bare subject**, even though `status` is
> the only thing that crosses today. A request of `status` alone would be a
> second command language with one word in it … a request of `get status` is a
> Rivet command, and **the day a second one is ruled across it needs no new
> grammar.**

**That day is today.** So this is not new architecture; it is finishing the one
already written down. The socket becomes the supervisor's control surface,
carrying Rivet operations, and service control is its first tenant rather than a
bolt-on.

The widening still has to be *argued* rather than assumed — `ADR-032` §3.4 is a
whole section on why widening the **ingest** socket was the tempting trap, and
the analogous case must be made here rather than skipped. What differs: the
ingest socket's `0666` is a ruled consequence of any process legitimately filing
a record, and none of that reasoning applies to this one, which is `0600`.

#### 6.3.2 RULED: the authority rule is CONVERGE ONLY, and it dissolves the tension

The hard question was never transport. `ADR-011` makes the document the single
source of authority and `ADR-015` §3.2.2 makes `enabled` **a statement in the
document** — so `apply start` on a service the document disables would
manufacture machine state that contradicts the record. That is systemd's
`start`-versus-`enable` split arriving in a system that is deliberately
declarative.

**Ruled: the supervisor's control surface may only ask it to CONVERGE, never to
diverge.**

* **Permitted** — restart a service, start an *enabled* service that is idle or
  failed. Both are requests to re-converge toward what the document already
  says.
* **Refused** — starting a service the document disables. The refusal names
  `apply edit` and `apply reload` as the way to change what the machine should
  be running, because changing that is a change to the document by definition.
* **The one divergence permitted is §6.5's held stop** of an installed
  application, which the next boot undoes. It is a separate subject with its
  own refusals, and it never writes `enabled`.

This dissolves the tension for everything that *starts*: no command can start
what the document disables, so the classic failure — a service started by hand
that silently vanishes at the next reboot — cannot be constructed. A held stop
diverges in the safe direction only: what it leaves behind at a reboot is the
document, and `get status` names the hold as the reason while it lasts. The
refusals are also trivially writable, which is the tell that the boundary is in
the right place.

---

##### BUILT 2026-08-29 — `rivet apply service-restart <name> confirm`

**Everything above this line is the ruling as it stands**, including §6.5's
amendment; the text as first ruled is in History. What follows is what building
it changed about it, in `ADR-032` §7's shape: the
departures first, because they are what a reader has to know, and the mechanism
after.

`rivet_core::operation::service_restart` is the subject;
`cmd/rivet-sysd`'s `Supervisor::restart_service` is the authority;
`cmd/rivet/src/status.rs` carries it over `ADR-032` §3.2's socket, which is now
what §6.3.1 said it would become. **3836 tests pass and 3799 did before it**;
every refusal below was broken on purpose and watched to fail (`AGENTS.md` §4).

###### Departure 1: the two permitted things are ONE subject

§6.3.2 permits *restart a service* and *start an enabled service that is idle or
failed* as two bullets. **They are one subject, `service-restart`.**

The argument is §6.3.2's own next sentence — *"Both are requests to re-converge
toward what the document already says"* — and a question a separate
`service-start` could not answer well: **what does starting an already-running
service do?** Either *nothing*, which is a command that reports success for
having done nothing, or *restart it*, which is `service-restart` under a second
name. One subject means one refusal path and one guard, which is the property
§6.3.2 values.

**§6.5's `service-start` is not that second subject.** It releases a held stop
and nothing else — a start of a service that is not held is refused and names
`service-restart` — so the question above still has one answer.

**It is cheap to overturn**, and that is stated rather than implied: a second
subject is one match arm and one `subject_table` entry (`ADR-006` §8.4's price,
quoted and paid ten times now).

###### Departure 2: `restart` was already taken, so the subject is `service-restart`

`apply restart` restarts the **machine**. A per-service restart spelled
`apply restart <name>` would mean that a truncated or mistyped command reboots a
board, so the two share no prefix and
`the_two_restarts_cannot_be_confused` asserts it — including that
`apply service-restart` with no service is an *argument error* rather than a
reboot. Setting the subject to `restart-service` was tried during the build and
that test failed on the shared prefix, which is the whole reason it is a test.

###### Addition 1: `REACHABILITY_SET` is REPORTED, never refused — and this is the decision to check

`ADR-032` §3.2.1's reachability set holds `rivet-mcpd` and `rivet-netd`, and a
`SIGHUP` reload may not stop them. **This surface consults it for one purpose
only: to decide what to warn about. It never refuses on it.**

**Why the set does not reach a restart.** The set exists because a stop that
*persists* removes the means of undoing itself — a board that honours
`enabled = false` on the control plane at 3am is up, healthy and gone, with no
next reset underneath it. A restart is not that act. The document and the
definition are unchanged, so the supervisor starts the service again on its own
next pass, by the rules every service is always subject to. There is nothing to
undo, which is exactly §6.3.2's *converge, never diverge*.

**And refusing it would be incoherent with what this project already permits.**
`apply restart confirm` reboots the whole machine, which stops `rivet-mcpd`
along with everything else; `ADR-006`'s `stop` subject documents that the answer
*"RACES the shutdown whichever channel carried it"* and ships. A surface that
refused the small act while permitting the large one that contains it would
protect nothing, and would leave anything `rivet-mcpd` reads only at start-up —
its identity — needing a **whole-machine restart** to change, closed by the very
command that cannot be used on it. (Its peers file is no longer such a thing:
`ADR-063` §3.1 reloads it live.)

**What is honestly owed to the caller, and is therefore in the PLAN before the
confirmation:** on an ordinary board the process carrying the request *is*
`rivet-mcpd`. It pumps bytes between the TLS connection and a `rivet mcp` child;
it does not hand the connection over. So when it is signalled, the pump dies and
the answer has nowhere to go.

**The ordering is arranged to give the answer its chance, and it is still a
race.** `Supervisor::restart_service` signals **nothing** — it sets a flag and
returns — and `cmd/rivet-sysd`'s loop writes the socket answer *before* the
convergence at the bottom of the pass, which is where the `SIGTERM` happens. The
reply is therefore written first and then has to travel through the process being
stopped. **This build does not win or lose that race deterministically and does
not claim to.** The plan says so in those words: *a dropped connection is an
ORDINARY OUTCOME here, not a failure*, and `get status` is the authority for what
happened.

**A mechanism to widen the race was considered and rejected** — deferring a
reachability-set member's stop by one convergence pass. It removes nothing; it
makes a failure rarer, and a failure that is rare is one nobody debugs
(§3.1.11's argument, in its other direction).

###### Addition 2: two refusals the ruling did not name

§6.3.2 names one refusal. Three exist, and the other two are not widenings of the
authority rule but consequences of it:

| | |
| :-- | :-- |
| **the document disables it** | §6.3.2's own, and the sentence names `apply edit` and `apply reload` |
| **there is no such service** | there is nothing to converge toward. It carries the list of what there is, because the next move is to type one of them |
| **the machine is shutting down** | every service is coming down and none is going back up, so a restart taken now would be a stop with no start after it — the one thing this surface may never produce |

A fourth refusal belongs to §6.5: **a held service is not restarted** (`held`),
because a restart would start what the operator stopped and release the hold as
a side effect.
`the_socket_answers_its_listed_requests_and_nothing_near_them` walks the request
grammar and holds it to exactly the listed requests.

###### Addition 3: the PLAN is the supervisor's, over `ADR-006` §6.4's `plan` verb

`ADR-006` §6.5 requires a dry run. The obvious implementation — the CLI builds a
plan from `get status` — would have been the CLI's *guess* at a running machine
and a second place that decides what a restart does.

**So the interlock is the plan.** The socket carries `plan service-restart
<name>` as well as `apply service-restart <name>`; the supervisor runs the
identical guard on both paths and answers what it *would* do, in the state it is
actually holding the service in. Two consequences worth having:

* **the refusals happen before the confirmation is ever typed** — §6.3.2's
  *"the refusals are also trivially writable"* cashed out at the one moment an
  operator can still choose differently;
* the plan and the result are the same type, so what was confirmed and what
  happened are comparable rather than two descriptions.

`plan` is in §6.4's closed verb set and this is the first use of it anywhere.
**Nothing exposes `rivet plan service-restart` on the CLI**, because §6.5 makes
the dry run a property of the framework rather than a word an operator types.

###### Addition 4: a sentence this change made false, corrected where it stood

`ADR-032` §3.2.1 withholds memory caps from a reload, and the **code** that
carries that ruling to an operator said each one *"applies the next time its
service starts"* — in `rivet-core`'s `reload::withheld` and in
`cmd/rivet-sysd`'s `document::apply_on`, twice, in slightly different words.
**That was true only because nothing could start a service without a reboot.**
The ADR itself says *"at the next boot"* and was right. A reload does not touch
the definition's cap
at all — `apply_cap` runs on `Occasion::Boot` only — and a spawn writes what the
definition holds, so `apply service-restart` after a reload would write the
**old** cap while every sentence said the new one was on. That is `ADR-034`
§5.3's hazard produced by prose rather than by code.

Both statements of it now say **at the next boot, not at the next start of this
service**, and name `apply restart confirm` as the way to take a new cap now.

###### The mechanism, in one paragraph each

* **`decide` gained a fourth input.** A restart cannot be expressed as a function
  of `(phase, def, state, now)` — a `Started` service the document enables is
  precisely where `decide` answers `Nothing`. Keeping it out would have meant a
  second place deciding what happens to a service, so `Restart::Asked` is a
  parameter and `decide` is still *the whole* of that decision.
* **It is never `enabled = false`.** An uninstall borrows that (`Managed::
  pending_uninstall`); a restart may not, because `enabled` is what `get status`
  reports and a service reading `enabled: false` for even one pass would be the
  machine holding a second notion of what it should be running — the property
  §6.3.2 exists to make unconstructable.
* **A stopped service is `Start`ed, not cleared to `Idle`.** `Cancel` was the
  obvious spelling and is a hang: `next_timeout_ms` returns nothing for an idle
  service, so a supervisor with nothing else pending would sleep indefinitely
  with the restart taken and not performed. Two tests catch it.
* **The attempt counter is forgiven**, or a restart of a `failed` service is one
  retry back into the same give-up latch. The counter already means *consecutive
  failures since something went right* — `Limits::success_window_ms` forgives it
  for that reason — and an operator asking for a restart is the other thing that
  goes right. The old value is journalled rather than lost.
* **`Event::Queried` gained the request**, which the variant's own documentation
  predicted: *"The day a second operation is ruled across, this variant gains
  it."* It carries a **parsed** request, so no handler is ever given a line to
  parse.

###### What is NOT proven, and exactly what would settle it

**No board has run one byte of this.** Everything above was established by tests
on a build host and in the container. In particular:

1. **That an answer ever comes back at all**, over a real socket on a real
   supervisor. The exchange is exercised over a real `AF_UNIX` socket in
   `cmd/rivet`'s tests, with the supervisor's own parser on the far end — but
   never against `rivet-sysd` itself.
2. **The `rivet-mcpd` race, which is the interesting one.** Whether the reply
   wins is unknown and this section deliberately does not guess. What a board
   would settle: `rivet apply service-restart rivet-mcpd confirm` over the
   control plane, then `rivet get status` — the second command is the one that
   matters, and the first may simply hang up.
3. **That a restarted application runs the NEW bytes**, which is the finding
   that motivated the work (`PLAN.md` §102.3 item 4): deploy a new version, then
   `apply service-restart <app> confirm`, then check the pid moved and
   `get install` agrees.
4. **That `rivet-netd` survives being restarted on a wireless board.** §6.3.4
   names this as the case netd's own control surface is for, and re-running
   interface convergence has never been done on hardware.

#### 6.3.3 RULED: initd exposes reads through the journal, and grows no listener

**pid 1 is the process that must not die**, and every parser added to it is a
panic that takes the machine with it. Its existing control surface — `SIGTERM`,
`SIGINT`, `SIGUSR1`, `ADR-017` §3.1.10's three rows — is impoverished *as a
safety property*, not by omission.

**Ruled: no new listener in pid 1.** What initd knows, it already writes to the
journal — boot confirmation, what it read from the boot partition, why it did or
did not confirm — and `get` subjects read it there. `get slot`, being built as
this is written, reconstructs trial and armed state from the kernel cmdline and
the boot medium without asking pid 1 anything at all.

**The one live counter-argument is recorded rather than dismissed:** `ADR-035`
§8A.1 proposes MCP over the serial console precisely for a machine whose
supervisor and network are both gone, and observes that pid 1 already holds
`/dev/console`. §8A.1 does **not** settle whether pid 1 or `rivet-mcpd` hosts
that, and this ruling does not settle it either. It rules only that ordinary
administration does not reach pid 1; break-glass is §8A.1's question and stays
open there.

#### 6.3.4 `rivet-netd` is the gap, and it is the same gap `get process` closed

Raised by the maintainer asking which other daemons deserve one. **netd does,
and for the argument this project has already accepted once.**

`get process` exists alongside `get status` because they answer different
questions — the kernel's answer and the supervisor's belief — and *the
interesting cases are where they disagree*. netd is that split again:
`get network`, being built now, reads `/proc/net` and `/sys/class/net`, which is
the kernel's reality. **netd holds what the kernel cannot tell you**: what the
DHCP lease actually said, whether the supplicant associated or is still
retrying, what it attempted to configure and failed to. Survey §3.1 already
recorded the read half — *"`rivet-netd` owns this and answers nothing"*.

The control half earns its place on a **headless wireless board**: renew the
lease, re-associate, bounce an interface. §6.3.2's converge-only rule applies
unchanged and fits netd even more naturally than sysd, since every one of those
is *"try again to reach what the document declares."*

**Not built. Recorded as the next control surface after sysd's**, so it is a
tabled ruling with a name rather than something rediscovered later — §94.7.

#### 6.3.5 RULED: `rivet-timed` gets no control surface, and the reason is a good one

**The maintainer's instinct: `timed` is so simple that he could not think of
anything anybody would want to control on it.** *(`.rulings/` R-039.)*

**Correct, and for a stronger reason than obviousness.** The one scenario that
would have justified controlling it is a deadlock: a Pi has no RTC (`ADR-021`),
so its clock is wrong until something sets it; a wrong clock fails certificate
validity; a failed TLS handshake means the control plane cannot be reached; and
there is nothing behind it. That is a board bricked by a clock.

**It cannot happen, because it was already designed out.** `rivet-tls` never
reads a validity window at all, and the test asserting it is named for the
claim: `a_long_expired_certificate_is_accepted_because_expiry_is_not_a_trust_condition`,
closing its window in 1980. `ADR-035` §7.1 permits it and `ADR-025` §2 rules the
reasoning — *"certificate validity windows are a poor fit"* for a machine with no
trustworthy clock.

So timed may be arbitrarily wrong and nothing locks anyone out. **Worth keeping
as a counter-example to §94:** this project has thought this exact class of
problem through before, carefully, and written the reasoning down. §94's finding
is that tabled decisions go unrevisited — not that decisions are made badly.

**Its READ still earns a place.** Survey §4's `get time`: whether the clock has
been set, by what, and whether it jumped, because the meaning of every other
timestamp on the machine depends on the answer.

#### 6.3.6 The rest: reads only

| Daemon | Ruling |
| :-- | :-- |
| the git routes in `rivet-mcpd` | reads only — survey §4's `get repo`; `apply repo-delete` already exists |
| `rivet-mcpd` | reads only — `get identity` for the pinned keys, `get account`. Controlling the control plane through itself is circular |
| `rivet-mdnsd` | nothing of its own; the advertised name follows the device label and is covered by `get device` |
| `rivet-smbd` | disabled on the board; shares belong to `ADR-031`'s volume question, not here |



### 6.4 An orphaned service declaration

`apply uninstall <name>` removes the application's tree under `/state/app` and
touches `/state/config/machine.toml` nowhere, so a `[services.<name>]` table
written for that application survives the verb exactly as the operator left it.
Where that table says `enabled = true`, what remains is an **orphaned
declaration**: a desired state asking the supervisor for a program the machine
does not have.

Neither this ADR nor `ADR-007` said what such a table means. This section says.

#### 6.4.1 The document is a declaration of intent, not a mirror of the machine

**Nothing rewrites the document because an artifact went away**, and that is a
position rather than an omission.

`ADR-007` §3.6.3 makes the document an input artifact authored anywhere — a
workstation, a git repository, a fleet layer applying one document across
several machines. `rivet-config` therefore accepts a `[services.<name>]` table
naming something this machine has no definition for, and argues at length that
the refusal belongs one layer up where the information is. §6 above makes
`enabled` the operator's whole control surface. The two say the same thing about
this case: **the table records what was asked for, and what is installed is a
different fact.**

A verb that edited the document because a directory disappeared would make the
machine a second author of its own desired state. That is `ADR-011`'s single
authority broken, and it is `ADR-007` §3.4.1's runtime-override tier arriving
through a side door — the same move §6.1 refused when a run-time instance was
tempted to write `enabled`.

**The cost is the orphan, and it is paid rather than denied.** A declaration
nothing can satisfy yields no service, no failure and no diagnosis of its own.
The two mechanisms below are what stop that being silent.

#### 6.4.2 The verb enumerates what it keeps, and it keeps two things

`apply uninstall`'s answer names what survives it rather than implying it, and
names the command that removes each. Two things survive:

| Kept | Removed by |
| :-- | :-- |
| `/state/app/<name>/data` — the application's private data (`ADR-015` §3.2's `state = "private"`) | the `purge` word on this same verb |
| the `[services.<name>]` table in the document | `apply edit` on `/state/config/machine.toml`, then `apply reload` |

**`purge` reaches the first and not the second**, so the enumeration is present
whether or not the word was typed. A `keeps` field that vanished under `purge`
would be promising that a purge keeps nothing, which was never true of the
document — the same shape of defect as naming one of two kept things, one step
further along.

#### 6.4.3 The supervisor reports an orphan when it applies the document

A declared service with no definition is visible at every point the supervisor
applies the document — at boot, at an install edge, and on a reload — because
that is where the document and the definitions are held together. This is the
obligation `rivet-config` names when it accepts an unknown service name: the
diagnosis happens where the information is.

**Two cases share that shape and only one of them is a divergence:**

| The table says | What it is | What is recorded |
| :-- | :-- | :-- |
| `enabled = false` | configuration written ahead of an install, which is the ordinary case for a document authored anywhere | kept, named, and nothing is started for it |
| `enabled = true` | an **orphaned declaration** — a desired state nothing on this machine can satisfy | `ORPHANED DECLARATION`, naming the document and the edit that clears it |

Telling them apart is the whole of the mechanism. A record that says only
*nothing is started for it* is true of both and describes the harmless one, so
the machine reported the benign half of a real divergence and an operator
learned to read past the line.

**This is the half that catches drift arriving some other way**, which is why it
is not merely a second copy of §6.4.2's sentence. A hand-edited document, an
install that failed after its table was written, a `/state` restored onto a
machine with a different application set — none of those passes through
`apply uninstall`, and all of them end in the same state.

#### 6.4.4 Whether the verb should strip the table is the maintainer's

**Unruled, deliberately**, and both arguments are written down so that the
ruling does not have to reconstruct them. The question is whether plain
`uninstall` removes the `[services.<name>]` table, whether only `purge` does, or
whether neither does.

**For stripping it, on plain `uninstall`.** `deploy` is the upgrade path:
replacing an application is a second install over the first, never an uninstall
followed by a reinstall. So an uninstall is a statement that this machine is
finished with the application rather than a step on the way to anything, and
under that reading the table has no reader left — leaving it leaves a
declaration whose only remaining effect is a warning on every boot. Confining
the strip to `purge` separates it from the argument that would justify it:
`purge` is about **data**, ruled so because removing a program is not consent to
discard what it wrote, and a table in the desired-state document is not data.

**Against, and for leaving it exactly where it is.** An artifact going away is
not the operator changing their mind. A fleet document that enables an
application across several machines is authored once and applied to all of them,
and a verb that rewrote it on one machine would make that machine disagree with
its own source at the moment somebody was removing something for an unrelated
reason. `ADR-006` §6.5's plan is also what an operator confirms, and an
uninstall that removed a tree *and* rewrote the desired state would be two
different kinds of act under one confirmation — §6.1's line between an operation
and a configuration change, crossed by the verb that draws it.

**What holds whichever way it is ruled:** §6.4.2's enumeration is right about
whatever the verb leaves behind, and §6.4.3's report is needed regardless,
because an orphan can arrive without any uninstall having run.

### 6.5 A stop held on one application

**`rivet apply service-stop <name> confirm` stops one installed application and
holds it stopped. `rivet apply service-start <name> confirm` releases the
hold.** It is the pause the other subjects cannot give: `apply uninstall` stops
the service and then removes `/state/app/<name>`, and `apply power-off`,
`restart` and `trial-restart` act on the whole machine (`ADR-006` §8.5).
*(`.rulings/` R-183.)*

#### 6.5.1 The names

`service-stop` and `service-start` are siblings of `service-restart`
(§6.3.2's Departure 2). The `service-` prefix is the vocabulary's mark for *one
service*, and it keeps the pair apart from the machine subjects: a bare `stop`
would sit beside `power-off` and `restart`, which stop the whole machine. No
subject is a prefix of another, and `rivet-core`'s
`the_two_subjects_cannot_be_confused_with_each_other_or_with_the_machine`
asserts it.

#### 6.5.2 What the hold is

| | |
| :-- | :-- |
| **Where it lives** | the supervisor's memory — `Managed::asked` in `cmd/rivet-sysd`. Never on disk, never in the document |
| **What it does** | a running application is signalled and stopped by §3.5's ordinary stop, with its deadline and `SIGKILL`. While held, `decide` never answers `Start` from any state: an exit is not a crash to retry, a backoff is cancelled rather than waited out, and a reload leaves the hold in place |
| **What releases it** | `service-start`; an uninstall, which removes the service from the set; and the supervisor process ending — so a **reboot**, or a restart of the supervisor by PID 1 |
| **What it never touches** | `enabled`, which stays the document's; and `/state/app/<name>` — a stop never reaches the removal an uninstall performs |
| **How it is reported** | `get status` shows the application `idle`, `enabled: true`, with the hold as its `reason` |

**Why a reboot releases it.** An operation changes what a machine is doing and
never what it should be (§6). Persisting the hold would make it configuration
kept outside the document — the runtime-override tier `ADR-007` §3.4.1 removed —
and a machine rebooted with nobody watching would come up disagreeing with its
own record. Bounding the hold by the boot keeps the document the whole answer to
what runs after a reset.

**Why a pass never clears it.** A restart is a request taken up once. A hold is
a standing instruction, and a convergence pass that cleared it would start the
application one pass after it was stopped — a stop the supervisor immediately
undoes.

#### 6.5.3 What is refused

Every refusal happens on the plan path too, because the plan is the
supervisor's own answer over `ADR-006` §6.4's `plan` verb, exactly as §6.3.2's
Addition 3 arranges for a restart.

| Refusal | Wire code | Why |
| :-- | :-- | :-- |
| a name the supervisor does not know | `unknown-service` | nothing to act on; carries the list |
| the machine is shutting down | `shutting-down` | every service is already coming down |
| **`rivet-mcpd` or `rivet-netd`** | `carries-the-way-in` | stopping the reachability set (`ADR-032` §3.2.1) cuts off the operator who asked, with the hold still in place and nothing left to release it but a power cycle. The refusal names `service-restart`, which bounces the service and brings it back on the same pass |
| any other service from the image | `not-an-application` | this operation pauses applications; what an image service does is the document's |
| a stop of a service nothing enables | `not-enabled` | nothing is running to stop — including a display application that does not hold the screen |
| a stop of a held service | `already-stopped` | a success that did nothing is not reported |
| a start of a service that is not held | `not-stopped` | `service-start` releases a hold and nothing else; the refusal names `service-restart` for an idle or failed service |
| a restart of a held service | `held` (on `service-restart`) | a restart would start what the operator stopped |

**Authorization is the apply surface's.** The two subjects are entries in the
`apply` catalogue, reached through `rivet_apply` over `ADR-035`'s control plane
by whoever it admits, and confirmed with `confirm` like every other subject.
Nothing about them is more reachable than `apply uninstall`, which does more.

**The machine-level proof is `tools/qemu/adr-017-hold-proof.sh`**: a booted
guest, an installed application, and every step through `cargo rivet run` — the
refusal for `rivet-mcpd`, the plan, the stop, `idle` with no pid after longer
than the restart backoff, `enabled: true` and the hold as the reason,
`/state/app/<app>` intact, `service-restart` refused while held, and a new pid
after `service-start`. It does not reboot the guest, and it is not a board
(`ADR-026` §4). What a board adds: the same sequence on hardware, and a stop
followed by `rivet apply restart confirm` with the application running after the
boot.

---

## 7. Logging

**A Rivet service does not open a log file.** It writes structured records, or
to standard output and error, and the supervisor captures them.

**Designed 2026-08-22 in `ADR-022`.** Records are structured and the on-disk
form is line-delimited JSON; text emitted by C mechanisms is captured and
wrapped rather than being allowed to remain a line.

That follows from `ADR-007` §3.6 — a service writing its own log file would be
a second place system state lives, in a format Rivet does not own. It also
means log rotation, retention and limits are solved once by the supervisor
rather than badly by each service.

Storage is under `/state`, with a size cap. An unbounded log on an SD card is
the same hazard as the restart loop in §3.5.

---

## 8. Open

1. **§3.1 — PID 1 spawns a supervisor rather than being one. RULED the same
   day, 2026-08-22.** §3.1 above carries the ruling and §3.1.1–§3.1.5
   the consequences; `PLAN.md` §4.15 records it too.

   **This item was stale for the rest of that day and is corrected here as a
   defect.** It went on saying a ruling was needed "before `rivet-initd` is
   written, which is Stage 0" while §3.1 — twelve lines up, in the same file —
   already recorded the answer. `rivet-initd` was in fact written against the
   ruling and boots. `AGENTS.md` §7 names exactly this failure: a ruling was
   written into the section that argued it, and the sibling sentence that
   announced the question was left alone. Anyone reading §8 first would have
   concluded Stage 0 was blocked.
2. **Whether the journal is a service or part of the supervisor.**
   **Answered 2026-08-22 by `ADR-022` §7: part of the supervisor.** A journal
   that were itself a service could fail to start, and the record of *why* would
   have nowhere to go — the one failure a logging system must not have.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

Most of what follows outlived its subject by hours rather than weeks. Six of
these were struck on the day they were written, and two of them twice.

### In §3.9 — a capability list on uid 0 was refused

**Superseded 2026-10-04** (`.rulings/` R-253), when `rivet-netd` was reduced to
the network's three capabilities while staying uid 0. The superseded rule was:

~~A list on a definition without its own uid (uid 0 already holds everything,
so the list would describe a restriction applied nowhere)~~ was refused.

That was true while the list only meant *what is kept across the uid change*.
It now also means *the bounding set of a program that keeps the uid*, which is
a restriction the kernel applies at every `execve`. `rivet-netd` reads
root-owned files and writes netlink, and its own uid is a later step than the
DHCP client's own owner, so keeping root's uid and losing the rest of root is
the step taken first.

### In §3.7 — the responder read the document and the lease itself

**Superseded 2026-10-04** (`.rulings/` R-252), when `rivet-mdnsd` was confined
under its own uid (R-210 D1). A confined program is granted neither the
document nor the root-only lease directory, so the reading moved into the
supervisor and the responder is handed its result (§3.7.1). The superseded
sentence was:

~~`rivet-mdnsd` reads its inputs once, at its start, and then parks: the
machine's name from the kernel, and its one address from the document — a
static address as written, or a DHCP interface's address from that
interface's lease.~~

### Status block — §3.1.5's list had grown only once

**Stopped being true later on 2026-08-23.** The superseded clauses were:

~~, and it is the only growth this list has had~~

~~"the only growth this list has had"~~

### In §3.1.6 — the `/dev/pts` re-examination was not due

**Superseded 2026-08-23** by an amendment made hours later, when implementing
§3.1.5 item 2 gave PID 1 a dependency on `/run`. The superseded sentence
re-checked the `/dev/pts` trigger when the first service definition landed and
concluded the re-examination was not due; hours later it was.

The reasoning it introduced survives in the body as a worked example of a trigger
that fires quietly.

### In §3.1.9 — nothing read the stored document

**Closed later on 2026-08-23 — §4.3.** The superseded bullet read:

~~**Nothing reads the stored document yet.** `cmd/rivet-netd` is told a
`--document` path by whoever starts it, and `rivet-sysd` does not start
it.~~

### In §3.1.10 — `reboot(2)` had never executed

**Corrected 2026-08-23**, when it ran once in a throwaway PID namespace on a
development host. The superseded clause read:

~~`reboot(2)` with `LINUX_REBOOT_CMD_RESTART2` has still never executed
anywhere~~

It establishes nothing whatever about the firmware, which is the half `ADR-018`
§5.1 depends on.

### In §3.1.10 — the shutdown experiment was still one board away

**The experiment ran on 2026-08-23, late.** The superseded sentences read:

~~§5.3 item 2 is unchanged: one experiment, on a board, and no test suite shortens
it. `PLAN.md` §0.0.4 is that experiment, written step by step.~~

### In §3.3 — the "someone else's" zombie branch had never printed

**Wrong within the hour**, on the first boot of the first card carrying the fix.
The superseded paragraph read:

~~**The "someone else's" branch has never printed on a real system**: producing
one means a board with `rivet-netd` exiting in that window, which is the same
non-deterministic race that caused the defect, so it cannot be demanded of a
single boot. What a board settles is that the wolf-crying stops.~~

### In §4.1 — `stage0_services()` was empty

**Changed 2026-08-23**, and the function is now `definitions::system_services()`.
The superseded clause was ~~`stage0_services()` is empty~~. It is the hole the
section was written to close, and a reader who found it still asserting emptiness
three sections above the definitions would conclude the ruling was never
implemented.

### In §4.1 — `listen` was an operator key, and was clause 3's worked example

**Both changed 2026-08-24 — §4.4.** The superseded entry in the document's key
list was ~~`listen`~~, and the superseded example read:

~~A port passes — Rivet opens the socket and passes the descriptor, and
`ADR-015` §5.2 already ruled the application "need not know the port".~~

A port *does* pass clause 3 and fails **clause 2**. Calling clause 3 "the one
that decides" while it gave the wrong answer is worth keeping in front of
whoever reads this rule next.

### In §4.5.3 — the channel was unsettled, and there was no way to start the installer

**Ruled 2026-08-24 evening**, the maintainer's 2026-08-24 ruling. The superseded heading
was ~~What this does NOT settle — the channel~~, and the superseded claim read:

~~`ADR-029` §14.4 named the gap in the same paragraph as the ruling that created
the binary, and it is unchanged: **there is no way for an operator to start this
binary with a path through the supervisor.**~~

Also superseded, in the same section:

~~`cmd/rivet/src/deploy.rs` is where the refusal an operator actually
sees is written, and it names both facts~~

### In §4.5.4 — nothing had run

**Half of it stopped being true on 2026-08-24 evening.** The superseded heading
was ~~What has run~~ and the superseded paragraph read:

~~**Nothing.** This definition has never been handed to a `Supervisor`, on a
board or under QEMU, because nothing constructs it.~~

### In §6.2 — two services were enabled by default

**Changed when a third joined them.** The superseded number was ~~two~~; date
not recorded in the file.

### In §6.3.4 — the converge-only rule lived in `PLAN.md`

**Changed 2026-08-29**, when `PLAN.md` was split. The superseded pointer was
~~`PLAN.md` §95.2~~; the rule is §6.3.2.

### In §6 and §6.3.2 — there was no manual stop

**Changed 2026-09-14 by the maintainer's ruling that a stop-one-service and
start-one-service operation be built** (`.rulings/` R-183 D6). The design for
restoring application data had found that nothing paused one application
without uninstalling it, and he chose to build the pause rather than leave it.
The superseded paragraphs read:

~~**There is no manual `stop`.** Ruled 2026-08-22. A stop that persists
contradicts the document; a stop the supervisor immediately undoes is theatre.
To stop a service, set `enabled = false` and apply.~~

~~This is the same reasoning that removed the runtime-override tier in `ADR-007`
§3.4.1, and it buys the same property: **`rivet status` can never disagree
with the document.** The cost is friction for a quick debugging stop, and
`restart` covers most of that case.~~

and, in §6.3.2:

~~* **Refused** — starting a service the document disables, and stopping one it
  enables.~~

~~No command can produce state contradicting the document, so `get status`
never has to carry a second notion of correct~~

**What survived into the body, and why the new shape does not undo it.** Both
halves of the 2026-08-22 argument still bind. *A stop that persists contradicts
the document* is answered by bounding the hold by the boot and never writing
`enabled`, so nothing persists; *a stop the supervisor undoes is theatre* is
answered by a convergence pass that never clears a hold. What was given up is
the absolute form of *status can never disagree with the document*: for the
length of a hold it does, and the status names the hold as the reason.

### In §8 item 1 — whether PID 1 supervises was open

**Ruled the same day, 2026-08-22: it spawns.** The superseded question was:

~~§3.1 — does PID 1 supervise, or spawn a supervisor?~~

**This item was stale for the rest of that day**, going on saying a ruling was
needed "before `rivet-initd` is written, which is Stage 0" while §3.1 — twelve
lines up, in the same file — already recorded the answer. `AGENTS.md` §7 names
exactly this failure: a ruling written into the section that argued it, and the
sibling sentence announcing the question left alone.
