# The dashboard: a machine's own state, on its own screen

* **Status:** Design candidate. Nothing built, nothing decided.
* **Date:** 2026-09-19
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Why, and the week that argued for it

Between 2026-09-11 and 2026-09-18 a board wedged four times. Each time the
symptom was the same: everything already running carried on — the screen drew,
an application served, a database was written — while the supervisor sat in one
syscall and the control plane went quiet eleven minutes later. **From across
the room the machine looked perfect.**

What eventually caught it was a monitor on a workstation, capturing `get
process` every minute. That was the right instrument and it stays. But the
thing that would have made the fourth incident obvious the moment somebody
walked past the board is a screen that says what the machine thinks of itself.

This is that screen. It is worth building because the failure it addresses has
happened four times, and because the board already has a display, a launcher
that selects between applications, and a renderer that proves an application
can own one.

## 2. What it is, and what it is not

**A kiosk application**, its own artifact, selected by `rivet-launcher` exactly
as `aura-renderer` is. Not a background (§ of the background design): a
background is chosen underneath a clock, and this replaces the clock.

**It is read-only and it changes nothing.** No buttons, no restart, no service
control, no settings. The moment it can act, it is a second way to operate the
machine, and `ADR-011` exists to stop a second answer to one question.

**It carries no secret and no credential**, so it is safe on a screen in a room
where other people are.

## 3. Where the facts come from — ruled 2026-09-19 (`.rulings/` R-199)

**Two sources, split on one principle: an instrument may not depend on the
thing it measures.**

**The machine's facts — hostname, version, slot, whether the boot is committed,
the supervisor's phase, the machine's folded health and every service's state —
are handed over by a typed SDK call**, answered by the supervisor and specified
in `ADR-057`. `SPEC-03`
§2 is the rule it follows: an application is handed what it needs and does not
go and take one. It is typed, it is versioned, and it costs nothing at all when
nobody has selected this application.

**The rule that decided what is in it: nothing in the answer makes the
supervisor go and look.** No file opened, no directory walked, no service
asked. The socket is reachable by every application, so a question costing a
`statvfs` would let any unprivileged program put I/O into the supervisor's own
loop — which is the defect the screen exists to witness. **Storage, thermal and
the network address are therefore NOT in it**, and neither are journal records,
because handing one application another's records is a disclosure `ADR-056`
already refused through a different door.

**A fact fixed for the life of the boot is not an exception to that rule.**
`slot`, `version` and `committed` are read once at startup and held in memory,
so the answering path opens nothing to produce them. The test the rule applies
is what a *question* costs, not where a fact originally came from.

**Storage and thermal reach the screen anyway — the dashboard reads them
itself** (§4.1, `.rulings/` R-203 D2). The rule above is about the supervisor's
loop, and an application's own `statvfs` is in the application's own loop.

**The supervisor's own liveness is read from `/proc` directly**, in this
process, under a narrow grant of the shape `SUPERVISOR_WINDOW` already carries.
This is not a second answer to a question the supervisor already answers: it is
the one question the supervisor **cannot** be trusted to answer about itself,
for the same reason R-195 D2 kept the supervisor probe off the control plane.
**What that read can actually contain is narrower than this design first
assumed** — §3.1, measured rather than predicted.

**Uptime and the boot id ride with it**, not with the call, and the reason is
the whole design in one line: they are the facts the instrument needs *while
the call is silent*. Uptime is the denominator that turns frozen counters into
*five hours of nothing*, and a fact that must survive the silence cannot be
answered by the thing that goes silent.

The pairing is what makes the screen useful during the failure it was built
for: **the facts call goes quiet and the `/proc` read keeps working**, so the
display says *the supervisor stopped* rather than merely going stale.

### 3.1 What that `/proc` read can and cannot say — measured 2026-09-19

This section promised a screen reading *parked in `unix_wait_for_peer`*.
**An application cannot deliver that**, and `ADR-056` §2.2 predicted it without
ever measuring it. `examples/probes/facts-probe` measured it on `rivet-06f5c`:
at an instant when the supervisor was demonstrably asleep (`state S`), the
application read `wchan` as `0` and was refused `syscall` with *Permission
denied*, while `get process` read **the same two files of the same pid** as
root and answered `do_sys_poll` and `73`. One process, two readers, one file;
the difference is the uid, because both fields want `PTRACE_MODE_READ` on a
root-owned process.

**What survives is still an instrument**, and it is the half that matters:
the state letter, the thread count, the voluntary context switches, the CPU
ticks, and the uptime beside them. **Frozen counters against an advancing
uptime is the wedge** — *five hours, and nothing has moved* — which is exactly
what a person needs from a doorway. What is lost is the diagnosis, and the
diagnosis was never the screen's job: Fleet Monitor's `get process` capture
runs as root, already records `wchan` every minute, and is where the fourth
incident was actually named.

**So the supervisor tile says whether it is turning, and never why it stopped.**
A screen that guessed at the reason would be worse than one that did not try.

**Every tile carries how old its fact is** regardless, because that is what
turns a screen into an instrument. *Last updated 4 s ago* becoming *6 minutes
ago* is a signal a person catches from a doorway.

### 3.2 What was rejected, and why it is not worth revisiting

**A world-readable facts file** under `/run/rivet`, rewritten by the supervisor
— rejected on the shape rather than on the details. A projection exists for a C
library that reads a fixed path and cannot be told otherwise; `ADR-039` already
calls `/etc` a compatibility surface and not a configuration one. Every
advantage claimed for it dissolved on inspection: *freely readable* is a policy
about who may know rather than a transport; a stale file is a **weaker** wedge
signal than a bounded call's silence, because a file written by a wedged
supervisor is byte-identical to one written by a healthy supervisor that has
not ticked yet; and a path with a format and a cadence and no schema is **more**
surface than a typed call, of the kind that rots without anyone noticing.

**And the prohibition that framed the original version of this section is
withdrawn.** It said an application may not reach the control plane. Nothing
enforces that — the control plane authenticates a pinned key, and an
application holding one is an operator. What is true instead: this call is the
supported way to learn machine facts, and an operator who installs a credential
into an application has made a deliberate choice the platform does not pretend
to prevent. **The `monitor` identity is what would make that choice safe** — a
pinned key accepted for `get` and refused for `apply`. It is named in this
project's record beside operator and agent, and it is implemented nowhere.

## 4. What it shows

Seven tiles, at 1920x1080, in the order a person scans:

1. **Identity** — hostname, the machine's version, slot and whether the boot is
   committed. The four facts that answer *what is this and what is it running*.
   The commit flag is in the call by `.rulings/` R-203 D3, because a tile that
   showed a slot letter and stayed silent about commitment would omit the one
   thing that matters after an update: whether this machine is one power cut
   from rolling back.
2. **Supervisor** — its state letter, thread count, voluntary context switches
   and CPU ticks, read twice a few seconds apart, against the uptime beside
   them. The tile answers **is it turning**, and §3.1 is why it cannot
   answer *where it is parked*: those two fields need `PTRACE_MODE_READ` on a
   root-owned process and an application is refused them. Counters that have
   not moved in five hours of uptime is the whole story of the last week, and
   it needs no diagnosis to be legible.
3. **Uptime**, with the boot id abbreviated. Uptime is the field that made the
   failure look time-driven — two incidents ninety seconds apart at 34 hours.
4. **Services** — each one's name, whether it is started, its restart count and
   its health. Red is a service that failed; amber is one that did not answer
   the health question.
5. **Storage** — free space on `/state`. **Read by the dashboard itself**, not
   answered by the call: §4.1.
6. **Thermal** — the SoC temperature and whether it is throttling, from
   `/sys/class/thermal`. Also the dashboard's own read.
7. **Recent trouble** — the last few *events the dashboard itself witnessed*:
   a service that changed state, a fact that stopped arriving, a call that
   timed out, each with how long ago. **Not journal records**, which §3 keeps
   out of the call; this tile is the dashboard's own memory of what it watched
   change, which costs the supervisor nothing.

Each tile carries its own freshness. A tile whose fact is older than a few
seconds says so; a tile whose fact has stopped arriving says that instead.

### 4.1 Storage and thermal are the dashboard's own reads — ruled 2026-09-19 (`.rulings/` R-203 D2)

**The dashboard reads them itself**, rather than a second answering process
reading them or the screen going without. **This does not weaken §3's rule, and
it was checked against it rather than traded away.** That rule forbids the
**supervisor** opening a file in the answering path, because its socket is open
to every application and a question costing a `statvfs` would let any
unprivileged program put disk I/O into the loop this screen exists to witness.
A kiosk application doing its own `statvfs` puts that I/O in **its own** loop,
where blocking for a millisecond costs a frame and nothing else. The grant is
the shape `SUPERVISOR_WINDOW` already carries.

**What that buys.** A full card and a throttling SoC are two ordinary failures
a person would otherwise have had to ask the control plane about. `get storage`
and `get thermal` answer both to an operator already; what was missing was a
route to a screen on the machine they describe.

**The network address stays off**, and the reason is not squeamishness about
one more read: nothing unprivileged can read the DHCP lease, so the tile could
name an interface and never say whether it holds one — which is the misleading
half rather than the useful half.

**The journal stays out** for the reason §3 gives, unchanged: handing one
application another's records is a disclosure `ADR-056` already refused through
a different door.

**The property these two tiles must keep, and it is worth a test:** a read that
blocks must not stall the frame loop. A tile whose read has not returned says
so through its own freshness, exactly as a tile whose facts call has gone
silent does.

## 5. What it looks like

The same typeface and the same restraint as Aura, because it is the same wall
and the same person: Geist Mono for every number, one accent, a dark ground.
Large where it matters — the supervisor's state and the uptime are readable
from across a room, the service list is readable from a metre.

No animation except the freshness counters. A dashboard that moves is a
dashboard you stop noticing, and the entire point is that a change catches an
eye that was not looking.

## 6. Order

1. **The fact set**, as a specification: exactly which questions the SDK
   answers, and what each answer's shape is. It is a new platform surface and
   it is the part that deserves an ADR rather than a commit.
2. **The supervisor's side**, answering them over a socket it already opens.
3. **The dashboard**, drawing with the SDK's existing draw and text calls —
   the renderer is the worked example, including the part about being a DRM
   lessee rather than master.
4. **On the board**, with the HDMI capture as the evidence.
5. **Then the hard one**: make a wedge visible. Arrange for the supervisor to
   stop answering — the harness can do this without breaking anything — and
   photograph the screen going stale. **Until that has been seen, this is a
   dashboard rather than an instrument.**

## 7. What is the maintainer's

* **How much a tile may say about failure.** *Three services unhealthy* is a
  fact; *this machine needs attention* is a judgement, and a screen that makes
  judgements is a screen that will be wrong in front of guests.
**Settled 2026-09-19, and kept here because a reader arriving at this section
should not have to wonder whether they are still open** (`.rulings/` R-203):

* **How storage and thermal reach a screen** — the dashboard reads them itself,
  §4.1. The network address does not reach one at all.
* **Whether the dashboard replaces Aura** — it swaps on the development board
  only (D6). The launcher selects one kiosk application at a time, and proving
  this screen catches a wedge means occupying one for a while; rivet-01 keeps
  Aura, because that screen is a wall in a house rather than a test surface.
