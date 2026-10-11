# ADR-057: A Machine Hands an Application Its Own Facts

* **Status:** Accepted
* **Date:** 2026-09-19
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Ruled by:** `.rulings/` R-199 D4, D5 and D6.
* **Surface:** `rivet_core::facts`, `rivet_sdk::facts`, `rivet_sdk::supervisor`,
  `rivet_core::confinement`, `cmd/rivet-sysd`.

---

## 1. Context

An application is about to be asked to show a machine its own state, on its own
screen (`docs/design-candidates/Dashboard-Kiosk-2026-09-19.md`). Today it has no
way to learn any of it. `ADR-036` gives an application what it was handed — its
directories, its sockets, its settings, its secrets — and nothing about the
machine underneath.

The obvious route is the control plane. It is the wrong one, and the reason is
sharper than a policy: **`ADR-035`'s control plane goes quiet about eleven
minutes after the supervisor wedges**, which is the exact failure a screen would
exist to make visible. `ADR-056` §1 is the shape of that failure, witnessed four
times — the journal stops mid-stride, everything already running carries on for
hours, and from across the room the machine looks perfect. An instrument that
learned its facts over the control plane would go blind at the only moment it
mattered.

### 1.1 The design that was refused, and what the refusal was about

The first proposal was a **world-readable facts file** under `/run/rivet`,
rewritten by the supervisor on a cadence. The maintainer refused the shape
before the details (`.rulings/` R-199 D4), and the refusal holds up on
inspection:

* **A projection exists for a C library** that reads a fixed path and cannot be
  told otherwise. `ADR-039` already calls `/etc` a compatibility surface rather
  than a configuration one, and that is what a projection is for. A Rivet
  application is the opposite case — `SPEC-03` §2 is that it is handed what it
  needs.
* ***Freely readable* is a policy about who may know, not a transport.** It is
  an argument for a permissive socket, and nothing else.
* **A stale file is a WEAKER wedge signal than a bounded call's silence.** A
  file written by a wedged supervisor is byte-identical to one written by a
  healthy supervisor that has not ticked yet. Telling them apart needs a
  staleness threshold somebody has to choose and every reader has to agree on.
* **A path with a format and a cadence and no schema is MORE surface than a
  typed call**, of the kind that rots without anyone noticing.

---

## 2. Decision: a typed call, answered by the supervisor

**An application asks; the supervisor answers, out of memory it already holds.**
`rivet_sdk::facts::ask()` returns a typed fact set or a named reason there is
none. The domain owns both halves of the wire form in `rivet_core::facts`, with
a round trip asserted, so the renderer and the reader cannot disagree.

Two properties follow from it being a call, and they are the whole case:

* **It costs nothing when nobody asks.** A file is serialized on a cadence
  whether or not anything is watching. This is paid for at the asking, so a
  machine with no dashboard selected pays nothing at all — which matters on a
  single-threaded supervisor whose spare capacity is the subject of `ADR-056`.
* **Silence is the signal.** The call is bounded at two seconds, so a supervisor
  that has stopped produces one named outcome — `NotAnswered::Silent` — at once
  and unambiguously. A healthy supervisor answers inside one pass of its own
  loop, which is milliseconds, so there is no band of ordinary behaviour for
  silence to hide in.

### 2.1 The answer is the same view `get status` gives

The fact set is composed from the supervisor's own `status()`, the fold
`ADR-032`'s query socket already renders for an operator. There is one
derivation of what is running on the machine and this is a second *reader* of
it, never a second author — `ADR-011`. The field most likely to drift between
two implementations is the machine's health fold, whose ordering clause puts
`unknown` above `degraded` (`ADR-045` §4), and it is exactly the clause a second
implementation would get wrong.

---

## 3. Decision: the socket is a datagram, and it is open to every application

Every other socket the supervisor opens is a stream it accepts on:
`ADR-032` §3.2's query socket, `ADR-036` §5.4's kiosk socket, `ADR-022` §11's
journal ingest. Both shapes are right for what they do. **This one is a bound
`UnixDatagram` at `/run/rivet/facts`,** and the reason is the failure the
surface exists to report.

**A stream socket's listen backlog is what wedged this supervisor.** The
2026-09-18 incident was caught under instrument parked in `unix_wait_for_peer` —
where a task sleeps when a listening peer's backlog is full and nothing is
accepting — and `.rulings/` R-195 D5's invariant is that a supervisor a service
can block is a design defect. An instrument built on the mechanism that causes
the failure would be exposed at both ends: the application's `connect` would
block with no bound, and the supervisor's `accept` would be one more queue in
the loop under suspicion.

A datagram has neither. The caller writes one line and waits with a deadline it
set; the supervisor reads what is queued and writes one answer, both
non-blocking, with no per-caller state kept between wakeups. **There is nothing
here a caller can hold the supervisor with**, and that is what decides the next
question.

### 3.1 `0666`, and why it is the ingest socket's argument rather than the query socket's

| Socket | Mode | Why |
| :-- | :-- | :-- |
| `ADR-032` §3.2's query | `0600` root | the operator's whole control surface — install, edit, service control |
| `ADR-036` §5.4's kiosk | chowned to one uid | `ADR-015` §5.3's declare-and-chown, **one consumer per endpoint** |
| `ADR-022` §11's ingest | `0666` | filing a journal record is something any process legitimately does |
| **the facts socket** | **`0666`** | **asking what machine this is, likewise** |

The kiosk socket's grant mechanism was considered and does not fit: it admits
**one** consumer, and a platform surface every future application inherits
cannot have one. The ingest socket's argument does fit — the answer carries no
secret, no credential and no other application's prose (§4.2), `/run/rivet` is
root-owned so nothing can leave a socket of its own beside this one, and the
runtime directory is already in every application's ruleset with read and write
and no create (`ADR-042`).

**What makes that safe is §3's shape rather than the mode.** A `0666` stream
socket would let any application connect and say nothing, holding the supervisor
for a read deadline. With no connection to make, the worst an application can do
is send datagrams, and the supervisor takes a bounded number per pass and
answers each with one `sendto`. The residual is that a flood costs the
supervisor some fixed work per wakeup, which is true of the ingest socket
already and is bounded the same way.

### 3.2 Every fact carries how old it is

Not decoration: it is what turns a screen into an instrument. *Last updated 4 s
ago* becoming *6 minutes ago* is a signal a person catches from a doorway.

**An age, never an instant.** The supervisor computes each age at the moment it
answers, so nothing absolute crosses, the two processes need no agreement about
a clock, and no skew can make an age negative — which matters on a board with no
battery-backed clock, where the wall clock is set some seconds into a boot, in
the middle of the window a dashboard is first drawing. The SDK adds its own
elapsed time since the answer arrived, so a tile's age keeps climbing with
nothing having to arrive to say so.

Three classes, and they age differently:

| Class | Fields | Age |
| :-- | :-- | :-- |
| Fixed for the life of the boot | hostname, version, slot | **none, deliberately** |
| As of the supervisor's last pass | phase, machine health, mode, the service list | one `view` age for all of them |
| As of the last health poll | each service's health word | its own age, per service |

**The boot-fixed facts carry no age because they cannot have one**, and stamping
them with a zero would invite a screen to draw *updated 0 s ago* beside
something that has been true since the machine came up.

**A service's health word carries its own age because the supervisor polls it
every thirty seconds** (`ADR-045` §3). It is the field a dashboard is most
likely to draw as live when it is half a minute old. The age is stamped from
the **answer** and not from the question — `HEALTH=` on the notify socket moves
the *next question* to now, so deriving an age from the poll schedule would
report a stale answer as fresh because a question was recent.

**The view age is what makes one state drawable that a timeout cannot reach**: a
supervisor that is still answering and has stopped turning. That answer arrives,
and says the view in it is five hours old.

---

## 4. Decision: what the call answers, and what it does not

### 4.1 The rule that decides it

**Nothing in the answer makes the supervisor go and look.** No file opened, no
directory walked, no service asked. Every field is either in the supervisor's
own memory already or was read once at boot and handed to it.

This is not economy. The socket is reachable by every application on the
machine, so a question that cost a `statvfs` or a `sysfs` read would be a way
for an unprivileged application to put filesystem work inside the supervisor's
loop — and *the supervisor stops first* is the design defect `ADR-056` §1 names
and `.rulings/` R-195 D5 rules. A surface built to report that failure must not
be a way to cause it.

### 4.2 The fact set

`Dashboard-Kiosk-2026-09-19.md` §4 lists eight tiles a dashboard wants. Four of
them are answered here, in whole or in part:

| Tile | Answered | Fields |
| :-- | :-- | :-- |
| Identity | in part | hostname, the running image's version, the A/B slot |
| Supervisor | **no** — §5 | — |
| Uptime | **no** — §4.3 | — |
| Services | yes | name, enabled, state, pid, restart count, readiness, health, health age |
| Storage | **no** — §4.3 | — |
| Thermal | **no** — §4.3 | — |
| Network | **no** — §4.3 | — |
| Recent trouble | **no** — §4.4 | — |

Beside them the answer carries what a machine-level tile wants and the design's
list did not separate out: the supervisor's `phase`, the folded machine health,
and the three-valued kiosk `mode`. `mode` is three-valued rather than a boolean
because `unknown` is the answer for a machine whose own document could not be
read, and a screen that drew that as `headless` would state something
confidently about a document nobody parsed.

### 4.3 What is left out because the supervisor would have to go and look

* **Uptime and the boot id.** These are the two facts the instrument needs
  **while the call is silent** — uptime is the denominator that turns the
  supervisor's frozen cumulative counters into *five hours of nothing*
  (`ADR-056` §3.4), and the boot id says whether the machine came back. A fact
  that has to survive the supervisor's silence cannot be answered by the thing
  that goes silent. Both are already readable by the application: the boot id is
  granted to every one of them, and `/proc/uptime` rides with the window in §5.
* **Storage.** Free space is a `statvfs` and the artifact count is a directory
  the installer owns. `get storage` over the control plane is where an operator
  asks.
* **Thermal.** A `/sys` read, answered today by the root-side `rivet` the
  control plane spawns, with no supervisor involved. §6.2 is the open item.
* **Network.** The interface, its address and its lease belong to `rivet-netd`,
  and the supervisor would have to ask it. Asking a service on a caller's behalf
  is precisely the blocking call `ADR-056` traced the 2026-09-18 wedge to.

### 4.4 What is left out because it is somebody else's content

**Recent journal records, at any severity.** This is the firmest exclusion and
it rests on an argument the record has already made once. `ADR-056` §3.6
considered widening the journal's mode so the portal could read it and refused:
the journal is `0600` in a `0700` root-owned directory precisely so that one
application's uid cannot read another application's records. Putting the last
few `warn` records into an answer any application may ask for would hand over
the same content through a different door — and then draw it on a screen in a
room with other people in it, which the dashboard design's own §2 rules out by
saying the screen carries no secret.

**A service's health caption, and the supervisor's `reason` sentence.** The same
argument one step smaller: both are prose one program wrote for an operator, and
a caption is an application author's free text. The *words* — `healthy`,
`degraded`, `down`, `unknown`, `idle`, `started`, `done`, `failed` — are closed
vocabularies and carry no content, so they cross. The sentences do not.

### 4.5 One fact that is missing for a reason nobody chose

**Whether this boot is committed.** `Dashboard-Kiosk-2026-09-19.md` §4's
identity tile wants it, and it is the one exclusion here that is an accident of
where the fact lives rather than a decision about what should cross: PID 1 owns
the A/B commit (`ADR-018`) and the supervisor does not hold it. `get slot` is
the answer today, over the control plane.

**What would close it is small and worth doing**: `rivet-initd` writing a marker
into `/run/rivet` at the moment it confirms a trial boot, which the supervisor
reads once and then knows — the same shape the slot already has. It is **not
taken here**, because a new file in the runtime namespace is a decision about
what PID 1 publishes and this document is about what the supervisor answers.

---

## 5. Decision: the supervisor does not answer for its own liveness

`.rulings/` R-199 D5. **An instrument may not depend on the thing it measures.**
The facts call is answered by the supervisor, so the supervisor's own state
cannot come from it — a wedged supervisor stops answering, and nothing in the
silence distinguishes *stopped* from *busy* from *nothing has changed*.

So `rivet_sdk::supervisor` reads it from `/proc`, in the application's own
process, under the narrow grant `ADR-056` §3.4 already designed: two leaves,
read-only, no directory above either.

* `/proc/<supervisor pid>` with read and **not** list, so the files it names can
  be opened and the supervisor's descriptors and threads cannot be enumerated.
* `/proc/uptime`, which carries nothing about any process and is what the
  cumulative counters are read against.

**The pairing is what makes the screen worth having.** When the supervisor
wedges, the facts call goes quiet and the `/proc` read keeps working — so a
dashboard reports *the supervisor stopped* rather than merely going stale.

### 5.1 The grant stays a name in the platform

`rivet_core::confinement::SUPERVISOR_WINDOWS` was one name and is now a list.
**The shape of the grant did not change with it**, and neither did the reason it
is a list of names rather than a manifest key: a capability an author can ask
for is one that will be asked for, and `ADR-056` §3.7 exists to stop this
becoming something anything else reaches for. A second entry widens nobody's
window — each named program gets exactly those two leaves and every other
application on the machine still gets neither.

It is ugly on purpose. A list edited by hand is a list somebody has to think
about.

### 5.2 What the `/proc` read can say, and what it cannot

**It can say whether the supervisor has stopped**, which is the fact the
2026-09-18 incident turned on and which nothing else on the machine can state.
The state letter, the thread count, the two context-switch counters and the
cumulative CPU all read; two samples and `/proc/uptime` answer *has this
supervisor done anything in the last five hours*.

**It cannot reliably say where, and the dashboard design promised that it
could.** `Dashboard-Kiosk-2026-09-19.md` §3 describes a screen saying *the
supervisor stopped, and it is parked in `unix_wait_for_peer`*. The second half
is not deliverable to an application today:

| File | Mode | What an application gets |
| :-- | :-- | :-- |
| `status`, `stat`, `comm` | `0444` | the whole reading |
| `wchan` | `0444` | the single character `0` — the kernel prints the symbol only for a reader it will let `ptrace`, and `0` is **also** what a task that is not parked reads |
| `syscall` | `0400` root | refused by the file's own mode, before `ptrace_may_access` is reached |

`ADR-056` §2.2 predicted exactly this from the kernel's documented rule and said
plainly that it had **not** been measured as an application's uid. **It is
measured now** — §7.1, on `rivet-06f5c`, as the two readings of one file that
differ only in who is reading it.

So `wchan` is reported as `Reading::Indeterminate` and never as a symbol or as
an error: one character, two answers, and no way at this end to tell them apart.
A probe that cannot tell its own answer from an error is the failure `AGENTS.md`
§7 names, and the type is how this avoids it.

**`ADR-056` §3.8 already reached the same place from the other side** and named
the route: *where* is answered by the control plane's own root-side reader,
which already carries `wchan` and `syscall` on every `get process` row and
already outlived the supervisor by eleven minutes and thirty-seven seconds on
2026-09-16. That route is unavailable to a screen during a wedge, and the two
facts a screen can still state — *it stopped*, and *it stopped this long ago* —
are worth having without it. §6.2 is what remains open.

---

## 6. What is NOT decided

### 6.1 An application reaching the control plane is not forbidden

`.rulings/` R-199 D6 **retired** the prohibition the dashboard design was
originally written around. It said an application may not reach the control
plane. Nothing enforces that: `ADR-035` authenticates a pinned key, and an
application holding one is an operator.

So the record says what is true instead. **This call is the *supported* way for
an application to learn machine facts**, and an operator who installs a
credential into an application has made a deliberate choice the platform does
not pretend to prevent. Until §6.2 exists, that choice is root-equivalent on
that machine.

### 6.2 The `monitor` identity is what would make that choice safe

A pinned key the control plane accepts for `get` and refuses for `apply`. It is
named in this project's record beside operator and agent — `ADR-052` refers to a
fourth named identity beside those three — and it is **implemented nowhere**.

It is the missing piece under three separate open ends, which is why it is
written here rather than left for the next reader to rediscover:

1. an application that wants a fact this call does not answer (§4.3, §4.5) has
   no safe credential to ask with;
2. a screen that wants *where the supervisor is parked* (§5.2) needs the
   control plane's root-side reader, and reaching it needs a credential;
3. a credential installed into an application today carries `apply` as well as
   `get`.

**It is not decided here.** It amends the identity model and belongs in its own
ADR, not in a document about what a supervisor answers.

### 6.3 A second question on this socket

There is one question and the wire form carries a protocol number, so a second
question is an addition rather than a break. What is **not** decided is whether
one should exist: the narrowness is a property worth keeping, and the day
something wants a second question is the day to argue about it with a concrete
case in hand.

---

## 7. What is proven, and by what method

**Unit-tested, on the build host and in the container:** the wire form's round
trip between the supervisor's renderer and the SDK's reader, field for field,
including that an absent health age arrives absent rather than as zero; an
answer from an unknown protocol refused rather than parsed hopefully; a
truncated answer refused rather than read as a smaller machine; the fact set
bounded on a service boundary with what it dropped counted; the facts answer and
`get status` agreeing field for field; the ages measured from the answer rather
than the question; the `/proc` parsers against fixture files and against a real
Linux `/proc`; and **the silence, over a real pair of sockets** — a supervisor
that binds the socket and never reads it produces `Silent` inside the deadline,
which is the reading the whole surface exists to produce.

**Proved on `rivet-06f5c`**, and §7.1 is what a probe application read off the
board.

### 7.1 The first article, on the board — 2026-09-19

`examples/probes/facts-probe`, an ordinary confined application with its own
uid, on `0.1.92`, slot `a`, committed. Its whole report is in the machine's
journal under `facts-probe:`.

**The call is answered, and the answer is this machine.** `ANSWERED in 30.6 ms,
protocol 1`, then `host=rivet-06f5c version=0.1.92 slot=a`,
`phase=converging health=healthy mode=kiosk`, and fifteen services with
`0 did not fit`. Nothing about that path had ever run on a board: the socket is
`0666`, the answer comes back to a socket the application bound under
`RIVET_APP_TMP`, and both ends are inside a Landlock ruleset.

**The ages are real numbers about a real machine.** Fourteen of the fifteen
services report `never answered`, which is the honest reading for services that
speak no health; `tempmon-rivet`, which does, reports
`health=healthy (12.3 s ago)`. That figure is the supervisor's own measurement
plus the probe's elapsed time since the answer arrived, which is the feature
rather than a field copied off the wire.

**The window is granted and the counters read.** `comm rivet-sysd`,
`threads 1`, `voluntary 569`, `nonvoluntary 521`, `cpu ticks 562`,
`uptime ms 467150`, and two samples 1.2 s apart that answer *turning*. That is
the detection signal §5.2 claims, taken by an application rather than by the
portal.

**And the two rows that were a prediction are now a measurement.** At an
instant when the supervisor was parked — the probe's own third sample says
`state S` — it read `wchan` as `0` and was refused `syscall` outright with
`Permission denied`. At the same time, `get process` over the control plane
read the **same two files of the same pid 146** as root and answered
`"wchan":"do_sys_poll"` and `"syscall":"73 …"`, which is `ppoll` on `aarch64`.
One process, one file, two readers, and the difference is the uid.

**The pairing, demonstrated.** A datagram socket the probe binds and never
reads is a supervisor that is alive, holds its socket and has stopped turning.
Asking it returned `SILENT after 2.01 s` against a two-second bound, **and a
thread reading `/proc` throughout that call answered 9 of 9 reads**, the last of
them `state S`. Beside it in the same run, the machine's real socket answered in
30.6 ms. That is the contrast the screen rests on: a supervisor that is turning
answers at once, and one that is not answers not at all, while the `/proc` read
carries on either way.

**What this did NOT do is stop the real supervisor**, and the probe's own report
says so in as many words. There is no verb on this platform that signals a
process — `apply`'s subjects change a machine's state and `get process` only
reads one — so the silence
is **staged rather than caused**: what is reproduced is the wedge's shape, not
the wedge. `docs/PLAN-Addon-Feature-Set-2026-09-19.md` T11 is where a real one
gets arranged, and until it has been seen this is an instrument that has been
shown its own mechanism rather than the failure.

**Nothing was disturbed.** Aura's reading-source addon opened on the new image
— the controller journalled `addon: opened … (4 field(s), 1 secret(s))` — and
HDMI captures either side of the update show the card lit throughout: a
reading before, one after the reboot, and one after the probe ran.

---

## History

Nothing yet. Created 2026-09-19 with the surface it decides.
