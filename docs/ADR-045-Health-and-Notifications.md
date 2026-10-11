# ADR-045: Health and Notifications — From an Application's Own Opinion to a Tray Icon

* **Status:** Accepted. **§3.1 was REVERSED on 2026-09-08** — the supervisor
  asks, and the application no longer tells. See the History.
  **The first hop is BUILT** (2026-09-09): the vocabulary, the fold, the
  reserved descriptor, the SDK's answering surface, the supervisor's poll, and
  `get status`. §11.1 steps 1 and 1a are proved by `tools/qemu/adr-045-proof.sh`
  on a booted aarch64 guest, and **on a Raspberry Pi the same night** — §11.3,
  where TempMon became the first real adopter and answered with a true sentence
  about a sensor that had gone quiet. **Hops two, three and four are not**, and
  §11.2 says what that leaves.
* **Date:** 2026-09-05, amended 2026-09-08, first hop built 2026-09-09
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Ruled:** `.rulings/` R-146, which declined the narrow commit-gate repair and
  ruled that the general mechanism is built instead, in `rivet-core` and the SDK
* **Promoted from:** `docs/design-candidates/Health-and-Notifications.md`, on the
  maintainer's word 2026-09-05 to promote it to an ADR the next session
  (`.rulings/` R-298)
* **Answers:** *"how does a Rivet application say it is unwell, and how does that
  reach a person who is not looking at it?"*

---

## 1. Context

Two things arrived at the same design from opposite ends on 2026-09-05.

**From below:** PID 1 committed a system update whose screen was blank
(`PLAN.md` §132.3). The commit gate checks four things — mounts, the document,
the supervisor, the control plane — and **none of them can see whether the
machine is doing its job.** The narrow repair on offer was to teach the gate
about the kiosk role.

**From above,** the maintainer declined the narrow repair and named the
destination. He was interested in the more general hook, and asked whether it
could be used to ensure that a critical server such as Trends is up and running.
Eventually Rivet should have a system that lets a member server report
notifications up to the fleet server, and one day he hopes to create a desktop
tray icon that raises Windows 11 notifications from those very notifications.
*(`.rulings/` R-146, R-299.)*

So **the commit gate is one caller of a general mechanism, not the reason for
it.** Trends is the better motivating case precisely because nothing about it is
visible from a panel: a machine with no screen at all can be failing at its only
job while every check the gate knows how to make passes.

### 1.1 Most of the shape already exists

| already built | where |
| :-- | :-- |
| a per-service notify socket | `ServiceName` is *"one name that is a cgroup, a notify socket and…"* |
| declared readiness | `ADR-017` §4.2, ruled 2026-08-22 — `none` is one of its values, and is *"honest rather than lax"* |
| a fleet server that polls its machines | `ADR-044`, built 2026-09-05 |
| a client-certificate identity for a fleet client | `R-129`, and the web portal uses it |
| a closed-vocabulary rule | `ADR-006` §6.4 |

**What is missing is that nothing consumes readiness after the start**, and
nothing aggregates it. This ADR is mostly about the consuming and the
aggregating; the transport under all four hops already exists.

---

## 2. The distinction the whole design rests on

**Health is STATE. A notification is an EVENT. They are not the same thing, and
conflating them is how this ends up unusable.**

A dashboard asks *"is Trends healthy right now?"* — state. A tray icon asks
*"did anything change that I should know about?"* — an event, derived from a
**transition** between states.

| | state | event |
| :-- | :-- | :-- |
| who produces it | the application, the machine | **the fleet server, and only it** |
| shape | a value that is always readable | a thing that happened, at a time |
| idempotent | yes — reading twice is free | no — telling you twice is a defect |
| who wants it | `get status`, the web portal | the tray, and one day mail or a phone |

**Every layer below the fleet server reports state and nothing else.** The fleet
server is the single place that notices transitions, damps flapping, and decides
what is worth saying. That is not an arbitrary choice of location: it is the only
layer that can see a machine which has stopped answering (§6), and the only layer
where *"do not tell him twice"* is even definable, because it is the only one
that knows what has already been said.

**If every application decides when to shout, the tray becomes noise and gets
turned off.** That is the failure this section exists to prevent, and it is the
reason §3.1 refuses applications a severity of their own.

---

## 3. The four hops

```
application  --state-->  supervisor  --state-->  fleet server  --events-->  tray
   (SDK)                  (machine)               (aggregate,               (Windows
                                                   transitions)              toast)
```

### 3.1 Application → supervisor — THE SUPERVISOR ASKS

**The supervisor polls. The application answers.** It is handed a descriptor it
did not open, it answers `get status` on it, and the supervisor decides how
often to ask and how long to wait.

**It says how it IS. It does not say how bad that is.** Whether a degraded Trends
is worth waking somebody is fleet policy, not an application's opinion about its
own importance — see §4.3. An author who can set their own severity will set it
high, and every author will.

#### 3.1.1 Why asking, and not telling

Ruled by the maintainer 2026-09-08, reversing this section. His argument was that
the existing shape breaks if the service in question fails and is unable to raise
the alert status on its own behalf, whereas if the supervisor is unable to poll
enrolled applications for status, the system has failed anyhow and needs a
reboot. *(`.rulings/` R-300.)*

**The case it catches is WEDGED BUT ALIVE.** A crashed process is already visible
— the supervisor holds the pid and counts the restarts. A deadlocked one keeps
its pid and simply never speaks again, and under a telling design that is
indistinguishable from a healthy application with nothing new to say.

**And the deeper reason is this document's own principle.** §4.3 takes severity
away from application authors because an author who can set it will set it high.
But an application that chooses *when* to speak has been handed something of the
same kind through the side door: one that reports every ten minutes has set the
fleet's monitoring resolution, and one that reports at startup and then goes
quiet has declared itself permanently healthy. **Polling puts the cadence and the
timeout where §4.3 already puts the judgement — outside the application.**

**Silence stops being a thing that has to be interpreted.** There are three
answers and each is a fact: it answered, it answered badly, or it did not answer
within the supervisor's own timeout. A service with no successful answer on
record is `unknown` (§4.2), which is different from `healthy` and different
from `down`.

**The failure mode this accepts is the one the maintainer named:** if the
supervisor cannot poll, nothing is learned about anything. That is a machine
whose supervisor is gone, which is a reboot rather than a health question.

#### 3.1.2 The channel is a HANDED DESCRIPTOR, not a new idiom

**It arrives under a reserved `LISTEN_FDNAMES` entry**, bound by the supervisor
and handed over exactly as `tcp-8080` is (`SPEC-03` §3). The manifest does not
declare it; every application gets one.

**This is deliberate and it is about the size of the contract.** An application
is handed things through four mechanisms — the environment, file descriptors,
stdout, and the filesystem — plus `NOTIFY_SOCKET`, which is a fifth because it is
`sd_notify`'s protocol and compatibility with it is worth keeping (§3.1.3). Each
of the four does something the others structurally cannot: the environment
arrives before `main` with no syscall, a descriptor transfers a **capability** a
program could not create for itself, stdout costs a new language nothing, and the
filesystem holds what is large, durable and ownable.

**A sixth idiom is what a fair critic would object to, and it would be the count
of MECHANISMS rather than the count of things handed.** systemd, containers and
twelve-factor all hand an application about this much; none of them invents a new
delivery shape per capability. So this one reuses the mechanism that already
exists, and `SPEC-03`'s own test is the measure: a second-language implementation
of the contract took about two hundred lines, and a capability that costs that
file twenty more is affordable where one that costs it a hundred is not.

#### 3.1.3 Telling survives as an interrupt, not as the mechanism

`NOTIFY_SOCKET` is untouched and keeps `sd_notify`'s protocol, because
`ADR-017` §3.4 chose it so that a foreign daemon speaking readiness works here
unchanged. **Ruled 2026-09-08:** the platform does not tie its hands regarding
that free compatibility with existing software (`.rulings/` R-300). `ADR-004` §3
makes `wpa_supplicant` a permanent dependency, and it is the standing evidence
that this machine hosts programs it did not write.

**Its value here is prospective and should be stated as such.** No foreign daemon
on a card today uses it: `wpa_supplicant` is spawned by `rivet-netd` rather than
supervised, and `wpa_cli`, `git`, `git-http-backend` and `busybox` are not
daemons. The convention is a cheap option on the *next* one.

**An application MAY still send `HEALTH=` on that socket**, and its only meaning
is *ask me now* — a hint that lets a data-integrity failure at 03:00 be seen
before the next poll. **The poll is the truth.** A push that arrives is not a
health report; it is a request for one.

#### 3.1.4 One unaccepted question at most, and the kernel holds the line

**The answering socket is listened on with a backlog of zero**, which a unix
listener reads as room for exactly one connection nobody has accepted. The
supervisor sets it after the bind and before the spawn.

**Why a bound is needed at all.** A unix `connect` creates the server's half of
the connection in the kernel immediately and queues it, with the request written
into it, until the application calls `accept`. When the supervisor gives up on a
question it closes its own half; **the queued half stays until it is accepted or
the listener closes.** An application that never accepts — one that does not use
the channel, or one that is wedged — would otherwise gain one orphaned socket and
its buffer of unreclaimable kernel memory every interval, until the largest
backlog the kernel allows was full. A machine with six such applications loses
memory steadily for more than a day and then refuses every question anyway.

**What the bound does to each kind of application:**

| Application | First question | Later questions |
| :-- | :-- | :-- |
| Answers | accepted and answered | find the queue empty and are accepted in turn — nothing changes |
| Never accepts | waits in the queue, unanswered | refused with `EAGAIN`, which is already *not answering* |
| Wedged, then recovers | accepted late, answered into a closed socket | the next interval finds room and is answered |

**Why the supervisor does not drain the queue itself instead.** Accepting and
discarding the stale connections on its own copy of the listener would remove
them, but the supervisor cannot do it without risking a wait, and it may not
wait on an application (§3.1.1 gives it the timeout because it must never be
held by the thing being asked):

* its copy and the application's share one open file description, so making the
  descriptor non-blocking makes it non-blocking for the application too — and
  the SDK's blocking answering loop ends at the first `accept` that fails;
* a non-blocking flag on `accept4` governs the socket returned, not the call;
* `poll` followed by `accept` races the application's own `accept`, and the
  loser of that race sleeps in `accept` until somebody else connects, which on
  this socket is nobody;
* the supervisor has no threads to absorb a wait.

The backlog removes the growth where it starts and involves no call that can
block. Only the supervisor connects to this socket and it holds one question per
service, so a backlog of one is the whole demand rather than a limit on it.

### 3.2 Supervisor → the machine's answer

The supervisor folds every service's state, together with what it already knows
(running, failed, restart count), into one machine-level answer readable through
`get status`. **The fold is normative and is written out in §4.4**, because *"the
worst of them"* is wrong and a rule that is not written down is a rule each
reader invents.

### 3.3 Machine → fleet server

The fleet server already polls its machines for `ADR-044`. This is that same pass
carrying one more question, which is worth preferring over a second mechanism.

**Push is tempting and is refused at this hop.** A pushing machine cannot report
its own death (§6), so the fleet server has to poll anyway; having both would
mean two paths to the same fact, which is what `ADR-011` is about.

### 3.4 Fleet server → tray

The only hop that carries events. The tray is an ordinary client of the fleet
server behind the same fleet-issued client certificate the web portal uses
(`R-129`) — **no new identity mechanism is needed**, which is the strongest
reason to keep the tray a fleet client rather than something that talks to
machines directly.

Long-poll or server-sent events over the existing TLS surface; **a tray that
polls every thirty seconds is acceptable**, is much simpler, and the choice can
be deferred without changing anything below it.

---

## 4. The vocabulary, and it is CLOSED

`ADR-006` §6.4 already rules closed vocabularies, and this is the case it was
written for. **If the states are not a small fixed set defined once in
`rivet-core`, there will be forty severity words within a year and the tray will
have to guess at all of them.**

**Closed means:** the set is a Rust enum in `rivet-core`, it is the only health
type anywhere in the system, no layer accepts a state as free text, and **adding
a fifth state is an amendment to this ADR** rather than a patch. That last clause
is the whole cost of closing it, and it is deliberate: a vocabulary that can be
extended by whoever needs a word today is not closed, it is merely tidy.

### 4.1 The four states

| state | means | who can say it |
| :-- | :-- | :-- |
| `healthy` | doing its job | application, machine |
| `degraded` | working, but something is wrong that a person should eventually know | application, machine |
| `down` | not doing its job | application, machine, **and the supervisor about a failed service** |
| `unknown` | has not said, and it is too early or too quiet to tell | the layer above, never claimed by the thing itself |

**Why exactly these four.** `degraded` is the one that earns its place: without
it, an application with a failing upstream must choose between lying and crying
wolf, and it will pick whichever it picked last time. `down` and `healthy` are
the two a supervisor could almost infer on its own and are cheap to keep.
`unknown` is not a fifth wheel but the honest answer to a question that gets
asked constantly and currently returns a guess.

**Why not a fifth.** Every additional state is one the tray must render, the
portal must colour, the fold must order, and a person must learn. `starting`
belongs to `ADR-017`'s readiness and would duplicate it; `maintenance` is an
operator's intent and belongs in the document beside `critical`; a numeric
severity is §9's second bullet wearing a hat.

### 4.2 `unknown` is never self-reported

A thing that can speak can say `healthy`, `degraded` or `down`. **Only its
observer can conclude that it has said nothing.** This is what makes `unknown`
load-bearing rather than decorative: it is produced by exactly one layer above
the silent thing, so the layer that reports it is always the layer that was
waiting, and there is never an argument about whose `unknown` it is.

### 4.3 `critical` is the OPERATOR's word, in the machine's document

Not in the application's manifest. **An application must not be able to declare
itself critical**, for §3.1's reason. The same binary is critical on the machine
that serves the house and unimportant on a bench, and only the document knows
which machine this is.

```toml
[services.trends]
enabled = true
critical = true
```

**Built 2026-09-09.** It is `rivet_core::service::OPERATOR_KEYS`'s third key,
optional, and it satisfies all three clauses of `rivet_config::services`'s rule
for the table: it changes no code, it is a policy the machine's owner is
entitled to set, and Rivet enforces it from outside the service — in §4.4's
fold. `rivet-config`'s schema publishes it as taking effect **immediately**,
which is unambiguous here where it is not for `enabled`: this key starts nothing
and stops nothing, so a `SIGHUP` reload has nothing to withhold.

**It defaults to false**, which is §7's protection against a gate that never
passes, and it sits beside `enabled` because it is the same kind of statement:
what this operator wants of this service on this machine.

**This refines R-146.3's wording, and the refinement is named rather than made
quietly.** That ruling says *"the maintainer's word is critical, and that is a
manifest key with a default of no."* The ruling's substance — a word called
`critical`, defaulting to no — is taken exactly; its incidental placement is not,
because a manifest travels with the application and this fact does not. The
maintainer approved the design carrying this argument on 2026-09-05, which is
what makes it a refinement and not a departure.

### 4.4 The fold, written down

The machine's state is the worst state among its **`critical` services only**,
ordered:

    down  >  unknown  >  degraded  >  healthy

**`unknown` ranks worse than `degraded`** because a critical service that cannot
answer is nearer to a dead one than to a live one that is telling you it is
unwell. A `degraded` service is participating; an `unknown` one has stopped.

**Non-critical services do not move the machine's answer at all.** A `degraded`
weather probe must not make a machine `degraded` when everything that matters is
fine — that is what makes the machine-level answer worth reading. Their states
remain individually visible in `get status`, the portal and the fleet server;
they simply do not roll up.

**Two consequences, stated rather than discovered:**

* **A machine whose only broken services are non-critical answers `healthy`.**
  That is deliberate. The remedy available to the operator is to mark what
  matters `critical`, and the per-service states are never hidden.
* **A machine with no critical services at all answers `healthy` once the
  supervisor has converged.** An operator who has told the machine that nothing
  matters gets the honest answer that nothing it was told to watch is wrong.

---

## 5. The caption: a state with a subject

This is the newest part of the design and had the least argument behind it when
it was promoted. It gets that argument here.

### 5.1 Why four states are not enough on their own

Four states are the right vocabulary for *"is TempMon healthy"*. They are not
enough for *"the battery is getting low"*, because **that sentence is useless
without a sensor's name in it** — and TempMon with one quiet sensor out of four
is not the same condition as TempMon that cannot open its database, though both
are `degraded`.

A person woken by a tray toast that says only `degraded` has learned that they
must go and look, which is most of the cost of not having been told at all.

### 5.2 What a caption is, normatively

**A short human sentence carried alongside the state.** The state is what the
tray filters, sorts and colours; the caption is what a person actually reads.

* **Optional.** A state with no caption is complete and common; `healthy` rarely
  needs one.
* **One per report**, replacing the previous one. It is part of the state, not a
  log — a caption that accumulates is §9's third bullet arriving by the back
  door.
* **Capped at 120 bytes of UTF-8.** Long enough for *"no broadcast from
  GVH5075_2222 in 34 minutes"*, short enough for a toast and for one line of
  `get status`.
* **Over-length is truncated on a character boundary and reported, never
  refused.** `ADR-038` §4.4's containment reaches here too: **a caption must
  never be the reason a service stops.** The SDK truncates at the boundary it
  owns so that the supervisor's cap is a second line of defence rather than the
  first thing an author meets.

### 5.3 What nothing downstream may do with it

**Treat it as untrusted display text.** An application writes it; the supervisor
and the fleet server carry it verbatim without parsing it; **nothing downstream
may branch on its contents.** The moment a tray matches on the words,
applications have an accidental protocol nobody designed and cannot change.

**It is rendered as text and never as markup.** The web portal and the tray both
display a string an application chose, so a caption is an injection surface
wherever it is put into a document. Escaping it is the renderer's obligation, at
every renderer, and that is cheaper than a sanitiser in the middle that every
renderer would then trust.

**It is not a second severity vocabulary and must not become one.** §4's whole
argument holds — this is a caption, not a class.

---

## 6. The alert a machine cannot send

**The most important notification is the one that never arrives.** A board that
has crashed, lost power or fallen off the network reports nothing, and that is
precisely the alert worth having.

So the fleet server's model is **absence-aware**: a machine that has not answered
within the interval it was expected to is **`unknown`, which is a state, not a
hole in the data**. It is never omitted from a listing, because a row that
disappears is indistinguishable from a machine nobody ever enrolled.

That requirement shapes the protocol, because the fleet server needs an
**expectation** of when it should next hear from each machine, and something has
to establish it.

**This is the single strongest argument for polling** (§3.3): a poller already
knows when it last succeeded, so absence is free. A push-only design has to
invent a heartbeat and then a timeout on the heartbeat, which is a poller wearing
a disguise.

`ADR-044` §7's brick detection is the same observation from the fleet-as-code
side — **a machine that answered before and is silent after** — and the two are
one mechanism, to be built as one rather than as a health absence and an update
absence that happen to agree.

*(The design candidate this was promoted from also cited `ADR-034` §6.2 here, for
*"two measured disasters"*. That section is about `apply document` refusing to
erase what the register does not know, and no such pair is in it; the citation
is dropped rather than carried forward, because a normative document earns
nothing from a reference a reader cannot follow.)*

---

## 7. The commit gate, which is where this started

`PLAN.md` §132.3's problem becomes one sentence with no kiosk in it:

> **Do not commit a boot on which a service the document marks `critical` never
> became healthy.**

A blank screen is covered as a consequence — the kiosk's `auto-run` application
is `critical` on a machine whose purpose is the panel — and so is a Trends that
never came up on a machine with no screen at all.

**The gate's test is *became healthy*, not *is healthy*.** A critical service
that reported `healthy` and later went `degraded` has passed; one that only ever
said `degraded` never became healthy and fails. This is what keeps a transient
upstream failure from rolling back a good image while still refusing a boot on
which the thing never worked at all.

**The trap, named so it is not walked into:** *a gate that requires six things to
be true is a gate that never passes.* Three clauses hold it open — `critical`
**defaults to false** (§4.3); the gate's bound is **finite**, so a boot waiting
for a service that will never report **fails rather than hangs**; and
`ADR-017`'s existing readiness timeouts are the precedent for what that bound
looks like.

---

## 8. Thresholds live in the application, about itself

The first two real consumers were named by the maintainer as soon as TempMon's
port landed, and they are the best test this design has had, because neither was
invented to fit it:

* **TempMon reports `degraded` when it has heard no broadcast in thirty
  minutes.**
* **TempMon reports `degraded` when a sensor's battery reading is low**, once the
  meaning of that field is settled.

**The first agrees with the maintainer's ruling on TempMon ticket `1b3957`, and
that is why it is well-shaped.** TempMon is not diagnosing the sensor — no
receiver can say a sensor is out of range or that a battery died; it can say only
*"I heard a broadcast from X at T"* and *"X's last payload reported battery N"*.
What TempMon reports is **its own ability to do its job**: it is supposed to be
recording, and it has recorded nothing for thirty minutes. A service is entitled
to that opinion about itself, and `degraded` is precisely the word for it.

**Thirty minutes is a threshold, and §9's first bullet forbids thresholds. The
rule survives intact**, because the threshold is not at the fleet level: the
fleet server still has none. TempMon has one, **about itself**, which is the only
place that can know what silence means for a Govee sensor on a 55 ms broadcast
cycle. The fleet server cannot know that and must never be taught it.

**It is an `ADR-038` setting rather than a constant**, so it can be changed on a
running board without a rebuild — and, since 2026-09-05, without a reboot either
(`PLAN.md` §134).

### 8.1 An application's own fact, and the question it must NOT answer

The question, as the maintainer framed it with one of his own applications
(`.rulings/` R-300): how would a service raise a status saying that nothing has
been recorded in over seven days, and is that a problem? Put with Reliquary,
which stores what a phone uploads to it, the shape is identical.

**The first half is the application's to say and the second half is not**, and
the question is worth keeping because it contains the whole distinction in one
breath.

Reliquary can say *"nothing has arrived from the phone in seven days"* — a fact
about its own records, which nothing else on the machine can see. **It cannot
say whether that is a problem**, because the answer depends on whether the
phone's owner is travelling, has replaced the phone, or has switched its uploads
off. §4.3 already places that judgement with the operator, in the machine's
document; this example is why.

    report(Degraded, "nothing uploaded in 7 days")

**Seven days is a threshold and therefore a setting** (§8's rule), for a reason
sharper than configurability: hardcoded, the *application* has quietly made the
policy decision this document spends §2 and §4.3 taking away from it.

**And it is the first case that needs an application to WAKE UP.** TempMon is
already handling radio traffic when it notices silence; Reliquary is an idle
daemon holding its port and nothing happens to it at all. It must assess on a
cadence.
**That cadence is the application's own loop and not a platform facility** — `ADR-039` §4
closes the scheduler's action set and states it plainly: *"an application that
needs something to happen on a cadence is a service and has its own loop, which
it can already do, at no cost, with no platform involvement."*

**Under §3.1's polling, the tick and the answer come apart**, which is better:
the application's loop keeps its own assessment current, and the supervisor's poll reads
whatever the last assessment concluded. An application that is wedged stops
updating that conclusion **and** stops answering the poll, and both are visible.

---

## 9. What this does NOT do, stated so it is not oversold

* **No alerting rules, thresholds or schedules at the fleet level.** The tray
  gets transitions; what a person does about them is a later question. §8 is the
  one place a threshold is allowed and it is inside an application.
* **No metrics.** This is health, not telemetry. A number that goes up and down
  is a different design and must not be smuggled in through `degraded`.
* **No history.** The journal already records what happened; this reports what
  *is*. The fleet server keeps only enough of the past to notice a transition.
* **No cross-machine logic.** *"Three machines degraded at once"* is a fleet
  policy and needs the four hops working first.
* **No tray.** The tray is the fourth consumer of a mechanism whose first three —
  the commit gate, `get status` and the web portal — are wanted anyway, and none
  of them needs it to exist.

---

## 10. Consequences

* **`rivet-core` gains one small pure module** — the vocabulary, the fold, and
  what `critical` means. No I/O, testable with no machine, and it is the file
  every other layer imports rather than restates. **Built.**
* **The SDK gains an ANSWERING surface**, not a reporting one — a handler on the
  handed descriptor (§3.1.2), plus the `report` call demoted to §3.1.3's *ask me
  now* interrupt. An application that answers nothing is `unknown`, and `unknown`
  on a non-critical service moves nothing (§4.4). **Built** —
  `rivet_sdk::health::Answering`, about eighty lines.
* **The supervisor gains a poll and owns its cadence and timeout** (§3.1.1). It
  binds one more socket per service, which it already does once for
  `NOTIFY_SOCKET`, and hands the descriptor over with the listening ones.
  **Built**, and the ask is **asynchronous**: the question goes out non-blocking
  and its answer arrives at a later wakeup, because a blocking ask would put the
  convergence pass behind whichever application is slowest — and the case this
  exists to catch is one that never answers at all.
* **`SPEC-03` gains a reserved `LISTEN_FDNAMES` name** and the rule that every
  application is handed it whether or not its manifest asks. **No new delivery
  mechanism is added** — that is the constraint §3.1.2 exists to hold. **Built**
  — `SPEC-03` §3.10, and a manifest declaring `unix:rivet-health` is refused at
  packaging time so the name cannot be claimed twice.
* **Not the image's own services.** §3.1.2 says *every application* and the
  build takes that literally: `SPEC-03` is the contract with an application, and
  `rivet-netd`, `rivet-timed` and `rivet-mcpd` neither hold a channel nor are
  asked. They run as root and the supervisor already holds their pid and cgroup.
  **What it costs is stated in §11.2** and it is a foot-gun rather than a
  defect.
* **The desired-state document gains one key**, `critical`, under `[services.*]`.
  **Built** — see §4.3.
* **The commit gate stops being special.** It becomes a caller of §4.4's fold
  with §7's *became healthy* test, and the kiosk carve-out that `PLAN.md` §132.3
  nearly acquired is not written.
* **The fleet server's poll gains a question, not a mechanism** (§3.3), and its
  absence handling merges with `ADR-044` §7's brick detection (§6).
* **A closed vocabulary is a cost as well as a protection.** Adding a fifth state
  later means amending this ADR and touching every renderer. That is the price of
  a tray that does not have to guess, and §4 pays it deliberately.

---

## 11. What would prove it, and what is unproven

### 11.1 The order to prove it in

1. **The supervisor polls one application, it answers `degraded`, and `rivet get
   status` says so.** The whole first hop, and it needs no fleet server.
   **DONE 2026-09-09.** The article is `examples/probes/health-probe`, in this
   repository. Reliquary (§8.1) is the best *consumer* and is the next step, but
   proving a mechanism by changing four repositories at once is how a mechanism
   ships that only its author can run.
1a. **The same application, wedged, is reported as not answering rather than as
   healthy.** This is §3.1.1's whole argument and it is the test the telling
   design could not pass. `SIGSTOP` on the process is enough to stage it.
   **DONE 2026-09-09** — the probe stops itself, and the assertion checks that
   its state, pid and restart count are all unchanged, because that is what
   makes it a wedge rather than a crash.
2. **A `critical` service that never becomes healthy stops a boot committing** —
   §7, and it closes `PLAN.md` §132.3 for real rather than by special case.
3. **The fleet server reports a machine it cannot reach as `unknown` rather than
   omitting it** — §6, the hop most likely to be got wrong, and testable by
   powering one board off.
4. **A transition produces exactly one event, and a flapping service produces
   fewer than one per flap.** The tray's whole usability is this test.
5. **TempMon says which sensor** — §5's caption arriving at a consumer intact,
   which is the first test that a state and a subject travel together.

### 11.2 What is built, and what is still unproven

**The first hop is built and both of its steps were run**, on an aarch64 guest
under QEMU, on 2026-09-09 — `tools/qemu/adr-045-proof.sh`. One boot, two
applications from one binary, differing only in the name they were installed
under:

| | |
| :-- | :-- |
| step 1 | the supervisor polled, the probe answered `degraded no reading in 5s`, and `rivet get status` over the control plane said so |
| step 1a | the same program, `SIGSTOP`ped after answering once, was reported `unknown` while its state stayed `started`, its pid stayed held and its restart count stayed `0` |
| §4.4, first half | a `degraded` **non-critical** service left the machine `healthy` |
| §4.4, second half | a `critical` service that stopped answering took the machine to `unknown` |

The pieces: `rivet_core::health` (the vocabulary, the fold, the channel's
constants), `rivet_sdk::health::Answering`, `Kernel::open_health` and
`ask_health` in `cmd/rivet-sysd`, `critical` under `[services.*]`, and the
`machine` line and three service columns in `get status`.
`examples/probes/health-probe` is the article.

**Still unproven, and each names what would close it:**

* ~~Nothing has run on a Raspberry Pi.~~ **Closed the same night, 2026-09-09.**
  A card build carrying this supervisor was installed on `rivet-01`, trial-booted
  into slot `a` and committed. `get status` over the control plane answered
  `machine healthy` with every service `unknown` and every restart count `0`, on
  a board that was serving a television, a kiosk launcher and three applications
  at the time. §11.3 is the account.
* **The 120-byte cap in §5.2 is still a judgement.** A `get status` line has now
  been rendered with a caption in it, so the *frame* holds. No toast has been,
  because §9's last bullet leaves the tray to a later design.
* **`unknown` ranking worse than `degraded` (§4.4) has never been read by a
  person under pressure**, which is the only test that matters for an ordering.
* **§7's *became healthy* test is unbuilt.** The commit gate is unchanged and
  still cannot see whether the machine is doing its job, which is §1's whole
  starting point; step 2 is what closes it.
* **Hops three and four are unbuilt.** The fleet server asks its machines
  nothing about health and there is no tray, so §6's absence handling has no
  implementation to be wrong.
* ~~No real adopter answers yet.~~ **TempMon answers, from 2026-09-09.** On
  `rivet-01` it reports `degraded` with the caption *"no broadcast from 1 of 2
  sensor(s): GVH5075_xxxx"* (the sensor's identifier masked) — a true sentence about a sensor that had actually
  gone quiet, and §5's worked example arriving as a measurement rather than as
  an illustration. **Reliquary and Trends are still `unknown`**, which
  is the honest state and not a failure.

  What that adoption cost, because it is the number an author wants: two halves,
  about a hundred lines. A task that re-reads the store every ten seconds and
  leaves an opinion behind, and a thread that hands the latest one over. **The
  split is not optional** — §3.1's two-second patience means the answering side
  must never touch a database, and the first draft that did would have reported
  a healthy application wedged, intermittently.
* **An image service cannot answer, by construction.** §3.1.2 says *every
  application*, and the build takes that literally: `rivet-netd`, `rivet-timed`
  and `rivet-mcpd` are handed no channel and are never asked, so each is
  permanently `unknown`. An operator who writes `critical = true` against one of
  them gets a machine that is permanently `unknown` — which is honest and is
  also a foot-gun this ADR has not ruled on.

### 11.3 What a Raspberry Pi did — 2026-09-09

The board is `rivet-01`, which serves a television, a kiosk launcher and three
applications, and holds the maintainer's own data. It was updated in
place: a card build carrying this supervisor, installed into the inactive slot,
armed with `apply trial-boot a confirm`, restarted with `apply trial-restart
confirm`, and committed by PID 1 once a pinned caller had reached it.

**What it answered, over `ADR-035`'s control plane:**

* every one of thirteen services back up, `restarts: 0`;
* `machine healthy` — the fold over an empty set of `critical` services, which
  is the correct answer and not a vacuous one;
* every service `unknown`, because at that moment none of them adopted the
  channel;
* and then, after TempMon was rebuilt against `Answering::handed()` and
  restarted, `tempmon-rivet degraded` with a caption naming the sensor.

**What that adds to the emulator's account** is the part `ADR-026` §4 says the
emulator cannot give: SD timing, real firmware, a `tryboot` one-shot, and a
supervisor polling while a renderer holds a DRM lease and draws every frame. The
television was photographed afterwards and was drawing normally.

**What it still does not prove:** a board under load with twenty applications,
which is where §8's thirty-second interval and two-second patience stop being
judgements. That has not been run anywhere.

---

## History

### 2026-09-09 — the first hop was built, and one thing in this document was wrong

**§11.2 said *"nothing in this ADR is built"* and simultaneously claimed
`rivet-core`'s FOLD existed.** It did not: `rivet-core/src/health.rs` carried the
vocabulary, the wire form and the caption cap, and its own header said the fold
*"and the commit gate are later steps and are deliberately not here — a fold
with no caller would be code nobody has run."* The status block at the top of
this file repeated the claim. Both are corrected above; the fold is now built
and has a caller.

**What the build settled that the design could not.** The ask has to be
asynchronous. A blocking `connect`-write-read with a timeout is the obvious
reading of §3.1.1 and it puts a single-threaded supervisor behind the slowest
application on the machine — on exactly the machines this mechanism is for.
§10's third bullet now says so.

### 2026-09-08 — the first hop was TELLING, and is now ASKING

**§3.1 was reversed by the maintainer**, who arrived at it by asking how an
idle service would ever raise anything (§8.1) and then answering his own
question. The superseded text:

> ### 3.1 Application → supervisor
>
> The application states **its own opinion of itself**, over the notify socket it
> already has. `rivet-sdk` gives it one call.
>
> **Silence is a state, not an absence.** A service that has never reported is
> `unknown`, which is different from `healthy` and different from `down`. The
> supervisor already knows whether the *process* is running; the application's
> report is about whether the process is *working*, which is the thing no
> supervisor can see from outside.

**Why it changed, and it is not the reason first given.** The maintainer's own
argument was that a telling design fails when the service is too unwell to tell.
That is true but narrower than it sounds — a dead process is already visible to
the supervisor, and the case it really catches is **wedged but alive**.

**The argument that decided it is this document turning on itself.** §4.3 takes
severity away from application authors because *"an author who can set their own
severity will set it high, and every author will"*. An application that chooses
WHEN to speak has been handed the same kind of power by another route: reporting
every ten minutes sets the fleet's monitoring resolution, and reporting once at
startup declares permanent health. Polling puts the cadence and the timeout
outside the application, where §4.3 already puts the judgement. **The ADR was
inconsistent with itself and nobody had noticed.**

**What was NOT changed, and why the mechanism is not new.** `NOTIFY_SOCKET`
keeps `sd_notify`'s protocol untouched — ruled the same day, on the grounds that
the compatibility is a cheap option on foreign software this machine may host
but permanently might (`ADR-004` §3's supplicant). The answering channel is
therefore a handed descriptor under a reserved `LISTEN_FDNAMES` name rather than
a sixth way of handing something over. §3.1.2 argues that at length because the
defensible number is the count of MECHANISMS, not the count of things handed.

**The cost of the reversal was approximately nothing, and that was deliberate.**
It was made while the vocabulary and the SDK call existed, the receiver did not,
and **no adopter called `report`** — verified across `examples/` and every
adopter's repository. One shipped caller would have made this a coordinated
release across several repositories.
