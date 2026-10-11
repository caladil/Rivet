# ADR-039: Scheduling and Time-Driven State

* **Status:** **Accepted, 2026-09-02**, with §11 items 1, 2 and 4 ruled the same
  day and item 3 dissolved by §8.4's correction. Ruled by the maintainer after arguing
  the opposite case and finding the evidence against it. **§3.1's converged
  state is BUILT end to end and NOT PROVEN ON A BOARD; §3.2's scheduler is not
  built at all.** §12 says what runs, what is tested, and by what method.
* **Date:** 2026-09-02
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Answers:** `ADR-030` §6 item 3 (*whether a backup is taken on a timer*),
  `ADR-028` §4.3 (*"automating it is a scheduling decision rather than a
  security one"*), `PLAN.md` §110.7 item 2 and `SPEC-04` §11 (2) — the half of
  Aura's homeless verbs that is not `ADR-038`'s.
* **Depends on:** `ADR-021` §5 (durations are monotonic) and `ADR-021` §9 (the
  document owns the timezone, built 2026-09-02). **The second is a hard
  prerequisite and it landed the same day this was ruled** — a schedule in
  local time was not expressible on this machine a week ago.

---

## 1. Context

**This ADR was opened to argue that Rivet should *not* grow a scheduler**, on
the ordinary grounds that an abstraction invented before its second user is a
liability. The evidence went the other way, and it is worth recording that it
did, because the conclusion is only as good as the count.

### 1.1 The second user arrived a long time ago

Six places in this tree already need behaviour driven by the clock. **Two are
built, each with its own private timer**, and the rest are ruled or open and
waiting on exactly this decision:

| Need | State today |
| :-- | :-- |
| **persisting the wall clock** | **BUILT ad-hoc** — `WRITE_INTERVAL_MS` and a private `next_timeout_ms` in `cmd/rivet-sysd/src/clock.rs` |
| **the SNTP poll** | **BUILT ad-hoc** — `cmd/rivet-timed`, its own interval, its own config key |
| **backups** | **OPEN** — `ADR-030` §6 item 3: *"whether a backup is taken on a timer, on change, or only when asked"* |
| **update automation** | **RULED, unbuilt** — `ADR-028` §4.3 settles the security question and says what is left: *"automating it is a scheduling decision rather than a security one"* |
| **journal and payload retention** | `ADR-022` §8, `ADR-029` §13 — *"retention is configuration, a field in the desired-state document"* |
| **display hours** | this document, §8 |

A seventh is outside the tree and is the reason the question was asked: Aura's
`internal/scheduler` does brightness by time of day, screensaver changes and
display on/off **in Go, in an application**, because the platform offers
nothing. `PLAN.md` §112.2 scoped it out of the port with nowhere for it to go.

**So the abstraction is late, not premature.** Building the display case
narrowly would have written the fifth private timer and left the sixth to write
the sixth.

### 1.2 The case as the maintainer put it

A client wants every one of the twenty information displays in a building to
switch off by itself at 7pm, to save power, and to come back on at 7am.
*(`.rulings/` R-295.)*

It is worth keeping because it decides §8. The person asking is **facilities**,
not the author of whatever happens to be drawing. If the schedule belongs to the
application then it is lost when the application is swapped, absent when the
application has crashed and the supervisor has not yet restarted it, and absent
again whenever the machine has fallen back to the launcher — which is a normal
state, not a fault. Twenty displays and three application vendors is three
implementations, three sets of bugs, and no single place an operator can ask
whether the building is dark.

### 1.3 Why it looked more complex than it is

The objection to a scheduler was *"an abstract scheduling system with hooks into
practically every subsystem"* — and that would be complex, and it is not what
any of the six users need. Two decisions collapse it: §2 separates the two
things being called a schedule, and §4 closes the action set so that nothing
ever registers a hook.

---

## 2. Two different things are called "a schedule"

**Conflating them is what makes this look like a large piece of work.** They
have different semantics, different failure modes, and only one of them is hard.

| | **(a) Time-varying desired state** | **(b) Scheduled actions** |
| :-- | :-- | :-- |
| Example | *the display should be off between 19:00 and 07:00* | *take a backup at 02:00* |
| Shape | a **predicate over the clock** | an **event that fires** |
| Evaluated | every convergence pass | once per occurrence |
| Missed occurrence | **not a concept** — boot at 21:00, evaluate, converge dark; nothing was missed | a real question with a real answer required |
| Overlap | impossible | possible, and must be ruled |
| A clock step | **harmless** — no fire time was ever stored | **the central hazard** (§5) |
| Idempotent | yes, by construction | no |

**(a) is nearly free on this machine**, because Rivet is already a
desired-state system: the document says what should be true and things converge
on it. A time-varying value is a value whose input happens to include the clock.
It needs no scheduler at all.

**(b) is the one with semantics**, and it is where both existing ad-hoc timers
live — although both dodge the hard part, because they are intervals on the
*monotonic* clock rather than times on the wall clock. Every unbuilt user in
§1.1 wants the wall clock.

---

## 3. Decision: two mechanisms, and they are not the same mechanism

### 3.1 Converged state is a document value, not a job

A time-varying desired value is expressed in the machine document and evaluated
by whatever already converges that subsystem. There is no registration, no
callback and no scheduler involvement.

```toml
[display]
# Local time, HH:MM, in the zone `ADR-021` §9 gives this machine.
# Both or neither: a machine that is told when to switch off and never when to
# switch on is a machine somebody has to visit.
off = "19:00"
on  = "07:00"
```

**Both keys or neither, and the schema refuses one alone.** This is `ADR-038`
§4.4's rule in a different place — a bad setting must not refuse the document —
so the refusal is at *composition* time where a human can see it, and a document
that somehow arrives with one key falls back to *no schedule* and journals that
loudly, rather than switching a building's screens off with no way to switch
them back on.

### 3.2 Scheduled actions are one service with a table

A single supervised service (`ADR-017` §2) holds the table, computes what is due,
and performs it. One implementation of the local-time arithmetic, the clock-step
rule (§5) and the catch-up policy (§7) — not six.

It is a service and not part of PID 1 for `ADR-021` §7's reason, which applies
unchanged: *"the argument for keeping PID 1's list short cuts the other way."*

---

## 4. Decision: the action set is CLOSED, and nothing registers a hook

**This is the decision that prevents the design the maintainer objected to**, and
it is worth stating as a prohibition rather than as a default.

The actions a schedule may name are a **closed set, owned by the operating
system** and spelled in one enumeration:

* take a backup (`ADR-030`)
* check for an update (`ADR-028` §4.3)
* apply retention to the journal and to stored payloads (`ADR-022` §8,
  `ADR-029` §13)
* persist the wall clock (`ADR-021` §3 — folding in the existing private timer)
* poll SNTP (`ADR-021` §4 — likewise)

**There is no mechanism by which an application registers a job**, and adding
one is a decision this ADR rules against rather than an omission. An application
that needs something to happen on a cadence **is a service and has its own
loop** — which it can already do, at no cost, with no platform involvement.

Two reasons, and the second is the one that matters:

1. **Precedent.** Every control surface in this project is a closed set:
   `ADR-006` §6.4's verb set, `rivet get`'s eighteen named subjects,
   `Device::ALL`'s two device classes. An open registry would be the first.
2. **A registry is a capability.** A third-party addon that can schedule an
   action on the machine is a third-party addon that can schedule the machine's
   actions. The first one to ask for *"power off the building's televisions at
   an hour of my choosing"* is a conversation not worth having, and the cheapest
   way never to have it is for the surface not to exist.

---

## 5. Decision: the clock step is the requirement this exists to get right

**This is the strongest technical argument for one implementation, and it is not
obvious.**

`ADR-021` §4.2 rules **step at boot, slew while running**. A Rivet starts at
1970, restores a floor from `rivet_store::clock`, and then *jumps* to real time
when SNTP first answers. So on every cold boot there is a window in which the
wall clock is wrong by years, followed by a discontinuity.

A schedule that pre-computes a fire time inside that window gets one of two
wrong answers, and both are silent:

* computed at 1970 and compared forward — **everything is overdue**, and every
  action in the table fires at once, seconds after boot
* computed at 1970 and stored as an absolute instant — **nothing ever fires
  again**, because the instant is fifty-six years in the past and the next one
  is fifty-six years away

Four private implementations will each get this wrong differently, and each bug
is invisible until three in the morning on somebody's appliance. **Doing it once
is the return on this ADR.**

The rules that follow, and they are testable:

1. **Durations are monotonic** — `ADR-021` §5, unchanged and restated because a
   scheduler is the most tempting place to break it.
2. **Wall-clock schedules are re-evaluated, never pre-computed.** *"What should
   be true now"* and *"what is due now"* are both questions asked against the
   current clock each pass. Nothing stores an absolute future instant.
3. **Nothing is due until the clock is trustworthy.** A schedule in local time
   is meaningless before the first successful step, so the table is inert until
   then and says so once in the journal. This is a real behaviour, not a
   nicety: it is what stops a cold boot from taking a backup it does not owe.
4. **A step is an event the table sees**, and the pass after it re-evaluates
   from scratch. That is free given rule 2, which is the argument for rule 2.

---

## 6. Decision: local time, because 7pm means 7pm in the building

Schedules are written in the machine's local time, from `ADR-021` §9's
`timezone` key, and **not** in UTC and not as an offset.

The reasoning is §1.2's: a building operator says *"off at seven"* and means the
clock on the wall. A UTC schedule drifts an hour against that twice a year and
is wrong for half the year in most of the world — and the operator who wrote it
is not the person who will be standing in the dark building.

This is only expressible because `ADR-021` §9 landed on 2026-09-02. **Before
that there was no zone on the machine to ask**, and a local-time schedule could
not have been implemented honestly — which is why this ADR could not have been
written a week earlier.

**Daylight saving is therefore in scope and is `musl`'s problem, not ours.** The
same `localtime_r(3)` that `cmd/rivet-sysd`'s `Kernel::local_time` and
`aura-renderer`'s clock already use reads the tzfile the supervisor projects. A
transition day has a missing hour and a repeated hour; §7's catch-up rule is
what keeps those from being special cases.

---

## 7. Decision: missed occurrences

**For (a) there is nothing to rule.** A predicate over the clock has no
occurrences to miss. A machine powered off at 19:00 and booted at 21:00
evaluates, finds itself inside the off window, and converges dark.

**For (b): at most one catch-up, and only inside a stated grace.** When the
machine was down at the moment an action was due:

* it runs **once** on the next pass inside the grace window, not once per
  occurrence missed — a machine off for a week owes one backup, not seven
* outside the grace it is **skipped and journalled**, because an update check
  that fires nine hours late is not the schedule the operator wrote and doing it
  silently is worse than not doing it

The alternative — replaying every missed occurrence — is what makes a cold boot
after a long shutdown the busiest minute in the machine's life, which is the
minute it can least afford it.

---

## 8. Decision: display hours are the machine's, and the lease decides the transport

The display case is **(a)**, converged state, and it belongs to the machine for
§1.2's reasons. But *who performs the switch* is decided by a constraint that is
easy to miss and settles the whole argument:

### 8.1 A lease is exclusive, so the machine cannot blank a leased screen

A kiosk application is granted a DRM lease over **the CRTC and its connector**
(`PLAN.md` §106.1, `rivet-sdk/src/screen.rs`). DPMS is a connector property.
While the lease is held the lessee owns those objects and **the lessor cannot
modify them** — that is what a lease is.

So *"the OS blanks the panel"* is not available while a kiosk application is
drawing, which is precisely when a building's screens need switching off. This
is the same fact `Screen::capture` records for screenshots, arriving from the
other side.

### 8.2 CEC is outside the lease, and that is why it is the machine's transport

HDMI-CEC is `/dev/cecN` — a different subsystem, a sideband to **the
television**, which is another device on the far end of a cable rather than this
appliance's output. It is not in the lease, not in `Device::ALL`, and not
affected by who holds the display.

**That independence is the reason to prefer it**, quite apart from it being the
only thing that actually saves the client's power: a blanked panel commonly
keeps its backlight lit, and a television in standby is what the electricity
meter notices.

### 8.3 So: the policy is the machine's, the evaluation is the platform's, the action is the display-holder's

| | Owner |
| :-- | :-- |
| **the schedule** | the machine document (§3.1). One key, templatable across twenty machines from `ADR-034`'s register |
| **evaluating it** | **`rivet-sdk`**, so there is one implementation and no application carries this logic — the answer to §1.2's *three vendors, three sets of bugs* |
| **acting on it while a kiosk holds the lease** | the lessee, on the connector it owns, prompted by the SDK |
| **acting on it otherwise** | the launcher, which holds the display when nothing is leasing it |
| **switching the television off** | the machine, over CEC, independently of all of the above |

**No application authors this policy and no application can override it.** What
an application does is *ask the platform whether it should be drawing*, which is
one call, and Rivet's own launcher answers it the same way.

### 8.4 CEC is unbuilt, and it is USERSPACE-ONLY work — corrected

**This section said Rivet had no CEC at all and needed a kernel change. It was
corrected the same day, 2026-09-02, on the maintainer's question of why CEC could
not be used and whether it had been compiled for Rivet yet (`.rulings/` R-296) —
and the answer is that the kernel half is already done.**

| | |
| :-- | :-- |
| `CONFIG_CEC_CORE` | **`=y`** in the built kernel |
| `CONFIG_MEDIA_CEC_SUPPORT` | **`=y`** |
| `CONFIG_DRM_VC4_HDMI_CEC` | **`=y`** |
| `/dev/cec0`, `/dev/cec1` | **present on `rivet-01` right now** |

**What was actually verified, and what was wrongly concluded from it.** The two
probes were of this *repository*: no Rust source, and no line in
`tools/buildroot/linux/rivet.config`. Both true. The conclusion — *the kernel
needs changing* — was never checked against the **built artifact**, and it is
wrong: CEC arrives through the Raspberry Pi base `defconfig`, which is exactly
what §5 of `ADR-024` warns about in the other direction (*"the base defconfig
brings things along"*). `AGENTS.md` §7's rule is about claims of absence, and
this was one: the source was searched and the machine was not asked.

**So the remaining work is userspace only**, and it is smaller than the struck
sentence implied. Aura shells out to `cec-client` (`internal/cec/cec.go`), which
cannot port — there is no shell in a service's world and no such binary in the
image, and `ADR-001` would not admit `libcec` to get one. But **none of that is
needed**: the kernel's own CEC API is `/dev/cecN` plus a handful of ioctls
(`CEC_ADAP_G_CAPS`, `CEC_S_MODE`, `CEC_TRANSMIT`), and *Standby* and *Image View
On* are one-byte opcodes. That is `libc` and no dependency at all — the same
posture `rivet_sdk::screen` already takes toward `libdrm`.

**It is still new work, and it is still the machine's under whichever owner
wins**: an application-owned display schedule would need the same client, and
would additionally need a device class in `Device::ALL` that does not exist.

### 8.5 It is published to `/run/rivet`, and the first build got that wrong

**The first build put the window in `/etc/display-hours`, as a sixth conditional
file of the `/etc` projection. The maintainer asked why it needed one, and the
answer is that it does not.** The file moved the same day; this section exists
because the rule it broke had never been written down as a rule.

> **`/etc` on a Rivet is a compatibility surface, not a configuration surface.**
> Every file in it is there because a C runtime demands that exact path: musl's
> `localtime_r` reads `/etc/localtime` and nowhere else, its resolver reads
> `/etc/resolv.conf`, `getpwnam` reads `/etc/passwd`, and a login checks
> `/etc/shells`. That is why `ADR-007` §3.6.2 calls a file there a *view and
> never an interface*, and why §3.5 remounts the whole thing read-only.

No C library looks for a display schedule. Its only reader is
`rivet_sdk::schedule` — Rivet's own code, inside somebody else's program — so
the window belongs in **Rivet's own runtime namespace**, beside the journal
socket, the kiosk socket, the query socket and `reached`.

**The test, stated so the next key does not have to rediscover it:** *name the C
runtime that will not look anywhere else.* If there is not one, it does not go
in `/etc`. `cmd/rivet-sysd/src/etc.rs`'s tripwire now carries that sentence, so
the question is asked of every key the schema grows.

**Why a file rather than an environment variable**, since `ADR-036`'s first
article is that an application is *handed* what it needs and `RIVET_DISPLAY` is
handed as a variable: **a variable is fixed at `exec` and a file is not.** §11
item 5 records that the window currently takes effect on the next restart, and
closing that gap means a reader noticing a change without being restarted — which
a variable forecloses and a file does not.

---

## 9. What is NOT in scope

* **Cron expressions.** Every user in §1.1 wants *a time of day* or *an
  interval*. `0 */3 * * 1-5` is a language, and adding one costs a parser, a
  validator, an error surface and a documentation burden for a generality
  nothing has asked for. A time and an interval; more when something needs more.
* **Third-party job registration** — §4 rules against it rather than deferring
  it.
* **Sub-second scheduling.** `ADR-021` §2's accuracy target is *"seconds on a
  home network"*, and a scheduler cannot be more precise than the clock beneath
  it.
* **Calendars, holidays and exceptions.** *"Except on public holidays"* needs a
  data source the machine does not have and cannot keep current offline.
* **A user interface.** `ADR-038` §2 says the same about settings and `ADR-038`
  §10.5 leaves the fleet page open; this joins that question rather than
  answering it separately.

---

## 10. Consequences

* **The two private timers become the first two rows of the table.**
  `cmd/rivet-sysd/src/clock.rs`'s hourly write and `cmd/rivet-timed`'s poll are
  the closed action set's first members, and folding them in is how the
  mechanism earns its place rather than sitting beside what it replaces.
* **`rivet get schedule` becomes a subject** — `ADR-032`'s question, and one no
  machine can currently answer about itself. *"What will this machine do, and
  when"* is exactly what `ADR-012`'s agent-as-operator needs and what six
  private timers make unanswerable in principle.
* **`ADR-030` and `ADR-028` §4.3 unblock.** Both were waiting on this and are
  named in §1.1.
* **A new document region**, with `ADR-038` §4.2's cost paid a second time: the
  document grows a section whose validity is checked in more than one place.
* **The display half of Aura's homeless verbs finds a home**, and it is not the
  one `PLAN.md` §110.7 item 2 expected — `brightness`, `set_screensaver` and
  `display_sleep` are `ADR-038`'s settings, while *when the screen is on* is
  this document's and is the machine's.

---

## 11. Open

**Items 1, 2 and 4 were RULED by the maintainer on 2026-09-02** and are struck
below with the ruling beside them. Item 3 was answered by asking the machine
rather than by a ruling — see §8.4. Item 5 stands.

1. **Where the grace window's length is set**, and whether it is per action.
   §7 rules the policy and not the number. A backup and an update check plausibly
   want different answers, which argues per action; one number argues for one
   fewer thing to get wrong.

   **RULED 2026-09-02: ONE NUMBER.** Split it when a second action actually
   disagrees, and not before — which is §1.1's own discipline applied in the
   direction it was wrong in this morning.
2. **Whether a schedule may name an application** — *"restart the kiosk at
   04:00"*. It is not third-party registration (§4) because the *machine* would
   be authoring it about a service it already supervises, so §4's prohibition
   does not obviously reach it. Left open rather than assumed either way.

   **RULED 2026-09-02: NO — for now.** Nothing needs it, and it is the shape §4
   exists to keep out even when the author is the machine. The deferral is the
   maintainer's own qualifier and is kept: this is a door left closed rather
   than one nailed shut.
3. **What `display.off` means with no CEC and no lease holder —
   largely dissolved 2026-09-02.** The maintainer asked why CEC could not be
   used and whether it had been attempted; it had not, and the kernel already
   supports it — §8.4. So *"with no CEC"* is a temporary state of the userspace
   rather than a property of the machine, and the question shrinks to what the
   schedule does in the window before that client is written.

   The answer stands as it was drafted: **blank the panel and journal that the
   television was not asked**, so the partialness is visible rather than
   inferred.
4. **Whether `rivet-timed`'s existing `poll interval` key survives the fold
   in §10 — RULED 2026-09-02:
   it becomes a ROW and the key is REMOVED.** Two ways to say one thing is
   `ADR-011`'s problem and this ADR would have created the second one.
5. **`[display]` publishes `on-next-restart` and the predicate is continuous.**
   Found while building §3.1, and it is a delivery gap rather than a design one:
   `Hours::is_on` is re-evaluated every pass, but the window reaches a reader
   through a file the supervisor writes when it converges, and
   `aura-renderer` reads that file once at start-up rather than every frame
   (`rivet_sdk::schedule::screen_is_lit`'s own note says why — thirty opens a
   second on a compositing Pi).

   So a document that changes the hours at noon does nothing until the
   supervisor restarts, and the schema says so. Answering `immediately` because
   the predicate is continuous would be the applied-when-it-is-not state
   `ADR-034` §5.3 exists to prevent.

   **Closing it needs two things and neither alone is enough**: the projection
   re-run on reload, and every reader noticing. The second is the harder half
   and is the same shape as `timezone`'s, where §9's own note records that musl
   caches the zone per process anyway.

---

## 12. What is built, and what has run

**§3.1 — the converged display window — is built end to end on 2026-09-02, the
day this was ruled.** §3.2's scheduler is specified here and nothing implements
it; the six users in §1.1 are all still waiting on it, and the two ad-hoc timers
still have their private loops.

| | State | Proven by |
| :-- | :-- | :-- |
| the predicate (`rivet_core::display`) | **built** | 7 tests. `is_on` is asserted at **every one of the 1440 minutes** of the day, for six windows including both orderings and both midnight boundaries — every minute belongs to exactly one state, no gap and no overlap. `minutes_until_change` is checked to land on the transition and never to return zero |
| the document key (`[display]`, `rivet-config`) | **built** | the `[control-plane]` model: the table is optional, and a table that exists carries both keys, so *both or neither* is expressed in the schema rather than enforced beside it. Schema fixtures, the round-trip corpus, and the crate's own guards — `there_is_a_fixture_for_every_property_the_schema_publishes` and `the_parser_agrees_with_every_published_requirement` both had to be satisfied rather than widened |
| the published file (`/run/rivet/display-hours`) | **built** | `cmd/rivet-sysd/src/runtime.rs`, written once per convergence beside the `/etc` projection and **not into it** — see §8.5. It answers the `the_document_contributes_nothing_and_a_new_key_must_decide_whether_it_does` tripwire with **no**, and appears in `generated::GENERATED_PATHS` so `rivet explain` covers it |
| the reader (`rivet_sdk::schedule`) | **built** | 5 tests, plus **4 agreement tests** — `ADR-036` §8.3's price for re-declaring rather than importing. The SDK and `rivet_core` are asserted to agree on the path, on the `HH:MM` parse for fifteen inputs, and on the predicate **at every minute of the day for six windows**; and the file `rivet_core::etc` renders is parsed back by the SDK, which is the round trip that would otherwise only happen on a booted board |
| `aura-renderer` acting on it | **built** | 0.4.0. Re-evaluates on the minute tick, blanks the canvas, blits once and then draws nothing — the animated background stops, which is the power the schedule exists to save. Cards still arrive and still expire, so the store is correct the moment the screen is due back |
| **any of it on a board** | **NOT PROVEN** | see below |
| §3.2's scheduler | **not built** | nothing in the tree schedules an action. `ADR-030` §6 item 3 and `ADR-028` §4.3 are still blocked on it |
| **the CEC transport** (`rivet-adapter-cec`) | **built and tested; NO TELEVISION HAS OBEYED IT** | 2026-09-02. `/dev/cecN` and six ioctls, `libc` and nothing else — no `libcec` and no `cec-client`. **The uapi numbers were PRINTED, not derived**: a C program compiled against the board's own `linux/cec.h` emitted every constant, `sizeof` and `offsetof`, and 12 tests assert the Rust matches — including that the size baked into each ioctl request equals the struct passed with it, which is the disagreement that answers `ENOTTY` and reads as *unsupported device* |
| **the machine acting on the window** (`display_hours`) | **built and tested; NOT on a board** | the supervisor evaluates the window every convergence pass and tells the television **only when the answer changes** — a `Standby` per pass would be a set nobody can switch on by hand until morning. A machine booting inside the dark window still sends, because `lit` starts as *unknown* rather than *on*. An unreadable clock sends nothing: `ADR-021` §4.2 steps the wall clock at boot, and a schedule evaluated then would act on 1970 |
| CEC *(superseded by the two rows above)* | — | §8.4. The KERNEL half is already there: `CONFIG_CEC_CORE=y`, `CONFIG_DRM_VC4_HDMI_CEC=y`, and `/dev/cec0` present on `rivet-01`. What is missing is a userspace client — `/dev/cecN` and a few ioctls, `libc` and nothing else. The first version of this row said a kernel change was needed; it had searched the repository and not asked the machine |

**4402 tests pass across the workspace**, and the changed crates cross-compile
clean for `aarch64-unknown-linux-musl`.

### 12.1 What is NOT proven, stated plainly

**None of this has been on a board.** `rivet-01` runs an image built before any
of it existed, and the following are consequences rather than speculation:

* **No screen has gone dark on a schedule.** The blanking path is asserted by
  construction and by unit test; nothing has watched a panel at 19:00.
* **The projection has not been written by a real supervisor**, only by
  `etc_projection` under test. The renderer half and the reader half are proven
  to agree with each other, which is not the same as either having run in place.
* **`takes_effect` is published as `on-next-restart` and that is the honest
  answer, not a limitation nobody noticed** — see §11 item 5. The predicate is
  continuous, but the window reaches a reader through a projection made when the
  supervisor starts, and `aura-renderer` reads it once at start-up for the
  reason its own comment gives.

What would close it: an image carrying these crates, a document with a
`[display]` table, and somebody watching the wall at the minute the window
closes.

**And for CEC specifically, the last step is a system image rather than an
application.** `display_hours` lives in `cmd/rivet-sysd`, so it reaches a board
through a signed system update and not through `rivet apply install` — which
means `ADR-027` §10's sequence: refuse a dirty tree, bump the patch, commit that
alone, build. The bump is a release decision and is the maintainer's.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §8.4 — Rivet was said to have no CEC at all

**Struck the same day it was written, 2026-09-02**, on the maintainer's
question of why CEC could not be used and whether it had been compiled for Rivet
yet (`.rulings/` R-296). The superseded text read:

~~Verified 2026-09-02, two ways — no tracked source, and no kernel config or
package in `deploy/`, `tools/` or `ADR-024`. **Rivet has no CEC at all.**~~

~~It is `/dev/cecN` ioctls in Rust plus a kernel configuration change.~~

The kernel half was already done: `CONFIG_CEC_CORE=y`, `CONFIG_MEDIA_CEC_SUPPORT=y`,
`CONFIG_DRM_VC4_HDMI_CEC=y`. **It had searched the repository and not asked the
machine**, which is the whole lesson.

### In §11 items 3 and 4 — two open questions

**Item 3 largely dissolved and item 4 was ruled, both 2026-09-02.** The
superseded titles read:

~~**What `display.off` means with no CEC and no lease holder.**~~

~~**Whether `rivet-timed`'s existing `poll interval` key survives** the fold
in §10, or becomes a row in the table and is removed.~~

### In §12 — the CEC row

**Superseded by the two rows above it in that table.** The superseded row label
was ~~CEC~~.
