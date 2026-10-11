# ADR-021: Time

* **Status:** Accepted — ruled 2026-08-22.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

No Raspberry Pi has a real-time clock. A board that has been powered off does
not know what time it is, and starts at the epoch or wherever the kernel's
build-time default puts it.

`ADR-017` §2.1 listed time synchronisation as a service and named `ntpd-rs`.
The maintainer asked what that dependency actually costs, which turned out to be
the second question — the first is **what Rivet needs time for**, and the
answer splits into two problems that want different solutions.

---

## 2. Two problems, and only one of them needs a protocol

**Problem one: the clock is wrong at boot.** Not slightly — it is 1970.
Certificate validation fails, signature freshness is meaningless, and log
ordering is nonsense. This happens on *every* cold boot and needs solving before
the network exists.

**Problem two: keeping the clock accurate once running.** This needs a network
protocol, and the requirement is far smaller than it first appears — seconds on
a home network, not the microsecond discipline a time server needs.

Conflating these is what makes "we need NTP" sound like the whole answer. It is
the smaller half.

---

## 3. Problem one: persist and restore

**The clock is written to `/state` periodically and on clean shutdown, and
restored at boot.** This is the `fake-hwclock` approach that Raspberry Pi OS
uses, it is about twenty lines, and it has no dependencies.

It does not give correct time. It gives *approximately* correct time
immediately, and — more importantly — a **floor**: the machine never believes it
is earlier than when it last ran.

### 3.1 The write interval is an SD-wear decision

Persisting the clock looks harmless and is a periodic write to flash
(`ADR-018` §7). Every minute is over half a million small writes a year, which
with small-file journaling amplification is not nothing. **Hourly, plus on clean
shutdown**, is the default.

The cost is that an unclean shutdown loses up to an hour, which is irrelevant —
the point of the mechanism is *approximately* right, and problem two corrects it.

### 3.2 Who restores it

**The supervisor, as its second action** — not PID 1.

`ADR-017` §3.1.5 keeps PID 1's responsibilities deliberately short because
everything on that list runs in the process whose failure panics the kernel, and
restoring a clock does not earn a
place there. The window in which timestamps are wrong is a few milliseconds of
early boot.

**Two corrections, both found while implementing this on 2026-08-23**, and this
section said *"first action"* and *"read a file and call `clock_settime`"* until
they were made.

**(a) "First action" cannot be taken literally, and second is the right
answer.** Before anything else the *journal* has to exist, or the restore has no
way to report its own failure — `ADR-017` §8 item 2. It therefore runs after the
journal is constructed and **before any record is stamped**, which is what makes
the supervisor's own first record carry a restored wall time rather than 1970.
`cmd/rivet-sysd/src/main.rs` documents that start-up order as a
contract, and the restore is step 2 of it.

**(b) "Read a file and call `clock_settime`" describes the wrong
mechanism.** §3 above rules a **floor** — *"the machine never believes it is
earlier than when it last ran"* — and this phrase restates it as an
*unconditional set*, which moves a **correct** clock backwards. They are
different mechanisms and §3 is the ruling. The implementation is
`max(stored, now)`: the clock moves at most once per boot and only ever forwards.

**This is not academic.** On a Pi the two are indistinguishable, because a
cold-booted Pi's clock is always the earlier of the two — there is no real-time
clock to disagree. `PLAN.md` §9.2 commits to an **x86 target**, and an x86
machine has one. The board that would have exposed this is the platform Rivet has not
built for yet, which is exactly the class of defect that survives to production.

### 3.3 RULED 2026-08-23: who writes it on clean shutdown, and when

§3 says the clock is written *"periodically and on clean shutdown"* and **never
says by whom**. §3.2 answered the restore and stopped there. The periodic half
needed no ruling and was built the same day; this is the other half.

> **The supervisor writes it, on its own `SIGTERM`, at the moment the shutdown
> is *requested*.**

**Two reasons, and they point the same way.**

1. **`rivet-sysd` is already the sole writer of `/state/clock`.** It
   holds the floor this boot established, the `Origin` reasoning, and
   `rivet-store`'s crash-safe write sequence. PID 1 writing the file would be
   a second writer of one file — which is what `ADR-011` exists to prevent — and
   would mean restating all three in the process whose failure panics the
   kernel.
2. **`ADR-017` §3.2's argument for keeping PID 1's job list short still
   stands**, and §3.1.10 has just grown it by a signal.

**Why it could not be settled earlier, which is the more useful half of this
section.** Until `ADR-017` §3.1.10 there was **no clean shutdown to hang it
on**. A Rivet board could be stopped only by removing power: there is no shell
and no `kill`, and none of the image's five binaries could signal pid 1. Every
shutdown this project had ever performed was unclean, so a ruling about clean
ones would have been a ruling about a state the system could not reach. The
question was left open in the code, in as many words, rather than guessed at.

**And a clean shutdown could not be *requested* until later the same day.**
§3.1.10 made the signal receivable; `ADR-006` §8.5's `rivet apply power-off |
restart | trial-restart` is what sends it. Recorded here because this section's
own argument depends on it: the clock write happens in the supervisor's `SIGTERM`
handler, that `SIGTERM` comes from pid 1's shutdown, and pid 1's shutdown now has
a caller that is not a power cut. **It has still never run** — no Rivet machine
has been stopped by anything, on a board or under the emulator, so
`/state/clock` has never been written by a shutdown.

#### At the request, not at the exit

The obvious placement is the supervisor's last act before returning. It is
wrong, and the reason is a **grace-period mismatch that the record did not
previously state anywhere**:

| Grace period | Value | What it bounds |
| :-- | --: | :-- |
| `cmd/rivet-initd`'s `STOP_GRACE_MS` | 5 s | how long PID 1 waits for the supervisor before `SIGKILL` |
| the supervisor's `stop_grace_ms` | 10 s | how long *it* waits for a service before `SIGKILL` |
| the supervisor's `kill_grace_ms` | 2 s | how long it then waits before abandoning it |

A service that ignores `SIGTERM` therefore keeps the supervisor busy for up to
12 s, and PID 1 kills the supervisor at 5 s. **A write placed at the supervisor's
exit is one a badly-behaved service can cancel.** Placed at the request, it
happens within milliseconds of the `SIGTERM` and does not depend on which grace
period wins.

The cost is that the stored value is stale by however long the shutdown takes —
at most those 12 s, against a mechanism whose stated granularity is an hour
(§3.1). That is not a trade worth thinking about twice.

**The mismatch itself is reported rather than silently tuned.** How long a
Rivet machine may take to stop is a decision, it is not this section's, and
`ADR-017` §3.1.10 does not make it either.

#### What it does not relax

Three states leave a boot with no floor to write from — an unreadable stored
clock, no readable wall clock, and a `clock_settime` the kernel refused — and in
every one of them writing would *lose* information rather than save it: a
machine sitting at 1970 that persisted its reading would throw away every hour
the card had recorded. A shutdown is not a reason to relax that, so the shutdown
write goes through the same guard as the hourly one and says on the console when
it declines.

#### What is unproven

Under QEMU the supervisor's `SIGTERM` path can be driven end to end. **On a
board it has never run**, because until 2026-08-23 nothing could request a
shutdown at all — see `ADR-017` §3.1.10. That the write survives a power cut in
the seconds after it, on the card Rivet runs on, is `ADR-026` §4's right-hand
column.

### 3.4 The operator can lower the floor, and nothing else can

The floor only rises, and that is the hazard as well as the property: a clock
stepped once to a wrong future — the year 2200 from time servers that lied —
is written to the card at the next hour, every boot after that restores it, and
the hourly write then refuses the corrected clock as behind the floor. Nothing
on the machine can tell a future it reached from one it was told about, so
nothing on the machine lowers the floor on its own. (`.rulings/` R-210 D5.)

**`rivet apply clock-floor-reset` is how an operator lowers it.** It follows
the house shape of every `apply`: without `confirm` it changes nothing and its
plan names the floor the supervisor holds and the running clock that would
replace it; with `confirm` the supervisor — the only writer of `/state/clock` —
writes the running clock there as the new floor through the same crash-safe
sequence as the hourly write, and takes it in memory only once it is on the
card, so a refused write leaves the old floor in both places. It resets the
floor **to the running clock** and to nothing else: the running clock is the
one reading the operator can see and correct first, and a reset to an
arbitrary value would be a second way to set the time. A boot that had stopped
the hourly write because its stored clock was unreadable takes it up again from
the reset, because an operator writing a new floor is the decision that boot
declined to make by itself. The request crosses the supervisor's query socket
as `plan clock-floor-reset` and `apply clock-floor-reset`, and the journal
records every confirmed reset, at `Warn` when it lowered the floor.

**The floor is never raised by a single unauthenticated reading, because no
single reading moves the clock.** The floor rises when the hourly write records
the running clock, so what keeps one reading off the floor is keeping it off the
clock. `rivet-timed` steps only when **two different servers agree to within its
noise floor**, the one second below which it corrects nothing (§4.2.1's guard).
(`.rulings/` R-247.)

* A reading inside the noise floor moves nothing and needs no second opinion.
* A reading outside it is checked against a second server: the next address in
  the configured list that is not the first one's and answers. Two servers are
  two different addresses; a list naming one address twice is one server.
* No second answer withholds the step, and so does a second reading one second
  or more from the first. Each is a `Warn` naming both readings, and the poll
  is tried again. **A first boot that reaches only one server keeps its time
  until a second answers**, polling at the unsynchronised retry interval rather
  than the steady one.
* Two readings that agree are replaced by their midpoint, so neither server
  chooses the number alone. The midpoint then meets the noise floor, the rate
  limit and the epoch check as a single reading used to, and a step record names
  both servers and both readings.

The tolerance is the existing noise floor rather than a new number: two
readings closer than it are indistinguishable by the measure that already
decides an offset is worth correcting, and two further apart cannot both be
right to within it. **Rivet does not slew** (§4.2.1), so there is no route by
which one reading could move the clock in small steps instead of one large one.
The four default servers are two providers' anycast addresses, so the list can
always name two; a list of one address would never step the clock, which is
the ruling working as intended. What this leaves open is two servers
that lie together; `apply clock-floor-reset` above is the way back from that.

---

## 4. Problem two: a minimal SNTP client

**Ruled 2026-08-22: a small SNTP crate** — `sntpc` or similar — evaluated under
`ADR-009`'s survey-first rule.

One UDP round-trip and a clock adjustment. No async runtime, no TLS, no crypto
stack. This is still *reuse* rather than writing our own: SNTP is a protocol
implementation, which is squarely rung 1 of `ADR-009` §3.

### 4.1 Why not `ntpd-rs`

`ntpd-rs` is a good project — Trifecta Tech, funded through Prossimo
specifically to put memory-safe implementations under internet infrastructure,
production-oriented and actively maintained. It is the right answer for a time
server. It is more than this requirement.

The expected cost is **`tokio` and `rustls`**, the latter arriving with NTS
support. `ADR-009` §3.3 names exactly this — a crate that pulls a large runtime
behind it — as one of the ways reuse quietly costs more than it saves.

**VERIFIED 2026-08-22, and the expectation was right.** `rustls23` and `tokio`
are both **non-optional** dependencies of the
`ntpd` crate; there is no NTS feature to switch off, and the default feature set
additionally pulls `aws-lc-rs` (C and assembly crypto) and `hickory-resolver`
(a DNS implementation). The full reading is in §8 item 2. Nothing in this
section changes.

`ntpd-rs` remains the upgrade path if drift, accuracy or authenticated time ever
become real requirements rather than anticipated ones.

`chrony` was also considered — it handles the no-RTC, intermittently-connected
case better than almost anything — and rejected as another C mechanism to
cross-build and wrap when a small Rust crate covers the requirement.

### 4.2 Step at boot, slew while running

Standard discipline, stated because getting it wrong is subtle:

* **At boot, step freely.** Nothing depends on continuity across a power cycle.
* **While running, slew small corrections** via `adjtime` rather than jumping.
* **Step a large correction**, and emit it as a visible event rather than
  silently moving the clock under running software.

#### 4.2.1 RULED 2026-08-23: always step. Rivet does not slew.

**This section named a discipline and never gave the number that makes it
implementable** — where "small" ends and "large" begins. Without it an
implementer either slews something that never converges or steps something that
should not have been stepped. Ruled by the maintainer rather than defaulted.

**The maintainer's ruling:** Rivet does no work that depends on a finely
disciplined clock, so stepping is enough and is the simple choice.
*(`.rulings/` R-271.)* That is the correct test, and the reasoning below is why it lands
where it does rather than being merely convenient.

**Why the usual advice does not apply here, and is arguably inverted.** The
standard case for slewing is to protect consumers of the wall clock from a
discontinuity. Rivet has **one**: the journal's `ts` field. Ordering is
`boot` + `mono` (`ADR-022` §3.2), and every timer in §5's table is monotonic —
a step scrambles displayed timestamps and changes nothing that decides anything.

Against that, slewing carries a cost this ADR does not currently admit:
**`adjtime` adjusts the rate of `CLOCK_MONOTONIC` itself**, which is the clock
Rivet's timers actually read, while `clock_settime` on the wall clock does not
touch `CLOCK_MONOTONIC` at all. **So on this system stepping is the *less*
invasive of the two operations.** §5 below says `CLOCK_MONOTONIC` "never jumps",
which is true and reads as though it were also immune to correction; it is not,
and that sentence should be narrowed — the effect is bounded by `adjtime`'s
~500 ppm cap and cannot matter at these magnitudes, but it is exactly the
distinction this ruling turns on. *(Documentation-derived, `clock_gettime(2)`;
not measured on a board.)*

**And the arithmetic decides the boot case regardless.** At ~500 ppm a slew
corrects 1 second in ~33 minutes and 1 minute in ~33 hours. A Pi 4 has no
real-time clock, so a cold boot is 56 years out and slewing it would take on the
order of 112,000 years. Stepping at boot was never a choice.

**What is required with it, because always-stepping without these is a defect:**

* **A noise floor.** Below some offset, do not correct at all.
* **A rate limit.** Do not correct more often than some interval.

Without both, a flapping or hostile time server has the board stepping back and
forth every poll and producing sawtooth timestamps. Neither number is ruled
here; they are `rivet-timed`'s to choose and to state.

**What would reopen this:** an application that genuinely needs wall-clock
continuity — an application such as Trends on the x86 target (`PLAN.md` §9.2)
is the plausible one. Adding a
threshold and a slew path later is an addition rather than a rewrite, and that
is precisely what makes deferring it safe rather than optimistic.

---

## 5. Everything measuring a duration uses the monotonic clock

This is the implementation rule that a time ADR exists to state, and it reaches
into decisions already made.

> **Wall-clock time is for timestamps. `CLOCK_MONOTONIC` is for durations.**

A correction — including the one at boot, which may be hours — will otherwise
fire timers early, hang them, or make them fire twice. The places this already
matters:

| Depends on a duration | From |
| :-- | :-- |
| `--confirm-within` auto-revert | `ADR-007` §4.1 |
| Restart backoff and the give-up threshold | `ADR-017` §3.5 |
| Service readiness timeouts | `ADR-017` §3.3 |
| Retry intervals in converging services | `ADR-017` §3.2 |

`--confirm-within` is the one that would hurt. A clock correction during a
confirmation window could revert a good change early, or leave a bad one applied
past its deadline — on a headless machine the operator may have just locked
themselves out of.

**One narrowing, 2026-08-23.** `CLOCK_MONOTONIC` does not *jump*, and it is easy
to read that as meaning a clock correction cannot reach it at all. It can:
`adjtime` slewing adjusts the monotonic clock's **rate**, and only
`CLOCK_MONOTONIC_RAW` is exempt. Stepping the wall clock with `clock_settime`
does not touch it at all. At `adjtime`'s ~500 ppm cap the effect is about 0.15 s
across a five-minute window and cannot matter to anything in the table above —
but it is the distinction §4.2.1 turns on, and it points the other way from the
usual advice: on this system **stepping is the less invasive operation.**
*(Documentation-derived, `clock_gettime(2)`; not measured on a board.)*

---

## 6. Authenticated time: not now, and the fleet answers it later

**Ruled 2026-08-22: plain SNTP for now.**

NTS has a bootstrap problem that argues against it as the *first* thing to run:
it authenticates over TLS, and TLS validates certificates against the clock it
is trying to set. Plain SNTP over UDP has no such circularity.

**The better answer arrives with the dashboard.** Once `ADR-019`'s enrollment
has given the device a fleet identity, it can take time from the fleet endpoint
over the already-authenticated channel — no new protocol, no new trust
relationship, and it reuses something already established.

What is not protected meanwhile, stated plainly: an on-path attacker can lie
about the time, and time is an input to certificate expiry checks. On a home
network with a fleet of two or three machines this is accepted rather than
mitigated.

---

## 7. `rivet-timed` is a service

It is a service under `ADR-017` §2 — supervised, in the desired-state document,
configurable (servers, poll interval, enabled).

It is **not** part of `rivet-netd`. It needs the network, but under
`ADR-017` §3.2's convergence model that is not a dependency to declare: it
retries until the network works, like everything else.

### 7.1 It runs under its own uid, keeping only `CAP_SYS_TIME`

`rivet-timed` parses replies from servers on the internet, so it runs as an
image service with a home (`ADR-017` §4.1): the uid
`app_identity("rivet-timed")` derives, the image-service Landlock ruleset, and
a home at `/state/app/rivet-timed`. Setting the wall clock is the one thing it
does that needs privilege, so it keeps **`CAP_SYS_TIME` and no other
capability** (`ADR-017` §3.9). (`.rulings/` R-210 D1.)

What it touches, read from its source rather than assumed:

| Need | What it takes |
| :-- | :-- |
| One UDP exchange per server | a socket bound to `0.0.0.0:0`, an ephemeral port — no `CAP_NET_BIND_SERVICE`. Landlock does not mediate UDP. |
| The server list | its argument vector, addresses only; a hostname is refused, so no resolver and no `/etc/resolv.conf` |
| The boot identifier | `/proc/sys/kernel/random/boot_id`, which the ruleset grants every confined program |
| Its records | stdout, captured by the supervisor (`ADR-017` §7) |
| The stored floor | nothing: the supervisor alone writes `/state/clock` (§3.3), reading the wall clock this service corrected |
| `clock_settime(CLOCK_REALTIME)` | `CAP_SYS_TIME`, kept across the uid change |

A step refused with `EPERM` is journalled at `error` naming the capability, and
the rate limit is not armed by a step that did not happen.

---

## 8. Open

1. **Which SNTP crate**, surveyed rather than assumed.
2. **`ntpd-rs`'s actual dependency tree — CHECKED 2026-08-22. The answer is
   unfavourable, and §4.1 stands as written.**

   Read from the `ntpd` crate's `Cargo.toml` on `main`:

   * **NTS is not feature-flagged off.** `rustls23` is a **non-optional**
     dependency of the `ntpd` crate. There is no `nts` feature to disable; the
     TLS stack compiles in regardless of feature selection.
   * **The default features are worse than assumed**, not better:
     `default = ["aws-lc", "rustcrypto", "pps", "srv"]`. `aws-lc` pulls
     `aws-lc-rs` — a C and assembly crypto library, which is precisely the
     cross-build cost `ADR-009` §3.3 and `ADR-003` exist to avoid. `srv` pulls
     `hickory-resolver`, an entire DNS implementation.
   * **`tokio` is non-optional too**, with `rt-multi-thread`, `net`, `fs`,
     `signal` and `macros` — the large runtime §4.1 predicted.
   * **The one favourable detail** is that `--no-default-features` with
     `rustcrypto` avoids the C crypto and keeps the build pure Rust. It does not
     remove `rustls` or `tokio`, so it does not change the conclusion.

   §4.1 predicted "`tokio` and `rustls`, the latter arriving with NTS support"
   and predicted it correctly. The recommendation of a minimal SNTP crate is
   **strengthened**, not revised, and item 1 above is now the only crate
   question left.

   **Method:** the published `ntpd/Cargo.toml` on `main`, read directly. **Not
   proven:** the resolved transitive tree and its build on
   `aarch64-unknown-linux-musl` — unnecessary, since the manifest already
   settles the question that was asked.
3. **Taking time from the fleet endpoint** (§6), when the dashboard exists.

3. **Where "small" ends and "large" begins in §4.2 — RULED 2026-08-23,
   §4.2.1: Rivet always steps and never slews, so the threshold does not
   exist.** What remains open, and belongs to `rivet-timed` rather than to
   this ADR, are the two numbers that ruling *requires*: the **noise floor**
   below which no correction happens, and the **rate limit** on how often one
   may. Always-stepping without both is a defect, not a simplification.
4. **What is enough to raise the clock floor — closed: two different servers
   agreeing to within the noise floor** (§3.4, `.rulings/` R-247). No single
   reading moves the clock, so none reaches the floor.

---

## 9. The timezone: the document owns it, and `/etc` projects it

* **Ruled 2026-09-02.** The maintainer's ruling, in this document's words: **the
  configuration document is the authority for this machine's timezone, and
  `/etc` may project it if the C library needs it.**

**Numbered after §8 deliberately.** `AGENTS.md` §2 rule 2 forbids renumbering,
and sections here are numbered in the order the decisions were taken — so a
ruling that arrives after an "Open" section sits below it rather than being
inserted where it reads more tidily.

### 9.1 What this closes, and what it was costing

Three separate places in the tree had recorded the absence and acted on none of
them:

| Where | What it said |
| :-- | :-- |
| `rivet_core::etc`'s authority table | *what timezone this machine keeps → the document (`ADR-021`) → no schema key yet*, since 2026-08-23 |
| `cmd/rivet-sysd`'s `inputs_from` | the same, as the third of three facts blocked on a key |
| `PLAN.md` §110.7 item 1 | the first of the three things blocking work on Aura's clock |

The cost was concrete rather than theoretical. `PLAN.md` §110.7 item 1 records a
renderer drawing a wall clock **in UTC and labelling it UTC**, because a clock
confidently an hour out is worse than one that admits what it is showing — and
there was nothing on the machine for it to ask.

### 9.2 The document, not `/etc`, and not an environment variable

`ADR-011`: one authority. Three candidates were available and two are rejected.

| Candidate | Verdict |
| :-- | :-- |
| a `timezone` key in the configuration document | **ruled** |
| `/etc/localtime` as a thing an operator edits | refused — `ADR-007` §3.6.2 makes every generated file a **view** and never an interface, and `/etc` is a read-only tmpfs |
| `TZ` in the environment | refused — see below |

**`TZ` deserves the paragraph, because it is genuinely the cheapest mechanism
and it is still wrong here.** musl reads `TZ` before it reads anything else, and
a `TZ` naming a zone needs no `/etc` file at all. What it needs instead is
*something to set it*, and on a Rivet that is four different somethings: the
supervisor for a service, `cmd/rivet-launcher` for a kiosk application, the login
path for an interactive session, and PID 1 for anything it starts directly. Four setters of
one variable is four answers to *what time is it here*, and the first one that
gets missed is invisible — a program with no `TZ` does not fail, it keeps UTC.

**`/etc/localtime` is the opposite of that: one file, three runtimes.** musl
reads it when `TZ` is unset (`ADR-008` §5.1.1); Go's `time.Local` loads
`/etc/localtime` when `$TZ` is unset; chrono's `Local` parses the same file.
Nothing has to be told, and nothing can be missed.

### 9.3 A zone NAME, not an offset and not a POSIX `TZ` string

The key takes an IANA identifier — `America/Chicago` — and the two alternatives
are rejected for one reason each.

* **An offset** (`-06:00`) is wrong twice a year in every jurisdiction that
  observes daylight saving, and there is no honest way for a document to carry
  one.
* **A POSIX `TZ` string** (`CST6CDT,M3.2.0/2,M11.1.0/2`) handles daylight
  saving and needs no database at all, which makes it genuinely attractive for
  an image that would rather ship nothing. It is refused on two grounds:
  **nobody knows their POSIX string** — an operator knows they are in Chicago,
  and asking them to transcribe a transition rule is asking them to
  hand-maintain data — and **it hard-codes rules that change**. The United
  States moved its daylight-saving boundaries in 2007 and has legislated about
  abolishing them since; a board carrying a literal rule keeps the old one for
  ever and is exactly one hour wrong for a few weeks a year, which is the class
  of error nobody notices until a calendar is involved.

The rules therefore belong to a **database on the image**, which an A/B update
replaces — the same lifetime and the same authority as the trust roots
(`rivet_core::etc::CA_BUNDLE_SOURCE`).

### 9.4 The blocker this ruling had to clear: the image shipped no zone data

**The key alone would have been inert, and silently so.** musl resolves a zone
by reading a tzfile; with none on disk it keeps UTC and says nothing. A
`timezone` key on an image with no database would have parsed, validated,
projected and changed the clock by not one second, which is precisely the state
`ADR-034` §5.3 forbids — a setting reported as applied when it is not.

So the image ships the database.

* **Where:** `/system/share/zoneinfo`, beside the system typeface, and
  **deliberately not `/usr/share/zoneinfo`.** `ADR-008` §3 rejects the FHS.
  musl does search `/usr/share/zoneinfo/` for a zone *by name*, and that costs
  nothing here because nothing on a Rivet asks by name: `TZ` is unset, so musl
  reads `/etc/localtime` and stops, and the database is read by the projection,
  which is Rust and knows where the image put it.
* **What:** every file that actually begins `TZif`, which is the check musl
  itself performs. A distribution's database carries `zone.tab`, `tzdata.zi`,
  `iso3166.tab` and a `SECURITY` file beside the zones, and none of them is a
  zone.
* **What is refused:** `posix/`, which is a byte-for-byte duplicate of the top
  level — verified pairwise by sha256 on tzdata 2026c, 598 files, zero
  differences — and `right/`, whose files count leap seconds. A machine using a
  `right/` zone reports a `time_t` about 27 seconds away from every other
  machine's, because POSIX time is defined not to count them; that is a wrong
  answer, not a size saving.
* **Where the bytes come from:** the build container, exactly as the CA bundle
  does and for a stronger version of the same reason. A tzfile is
  architecture-independent and big-endian by RFC 8536 whoever compiled it, so
  Buildroot's `zic` would produce the bytes already sitting in the container —
  and `BR2_PACKAGE_TZDATA` would be hours of Buildroot for them. The
  container's copy is also the fresher one: tzdata is republished several times
  a year, and a Buildroot package is pinned to whatever version that Buildroot
  release pinned.

### 9.5 When it takes effect, stated honestly

**`on-next-restart`**, and unlike `hostname` this is not a near miss.

The zone reaches `/etc/localtime`, which the supervisor projects once when it
starts and does not re-project on a `SIGHUP` reload (`ADR-032` §3.2). Even if it
did, **every reader of a timezone has already resolved one**: musl caches the
zone per process after the first `localtime()` call, Go resolves `time.Local`
once, and an application drawing a clock read it at start-up. Answering
`immediately` because a file changed would report a setting as applied to
programs that will never see it — `ADR-034` §5.3 again, from the other side.

### 9.6 What is refused where, and what deliberately is not

| Fault | Refused by | Why there |
| :-- | :-- | :-- |
| a value that cannot be a zone name at all | the parser, naming the value | `rivet-config` is the authority for what a valid document is; the operator is looking at the document when they are told |
| a name containing `..` or a leading `/` | the type, by construction | the name is used to read a file out of the image, so containment is a property rather than a nicety |
| a well-formed name for a zone **this image has no file for** | **the machine, at boot** | which zones exist is a property of the image, not of the document — the same reason `ADR-007` §7.3 does not refuse an interface name this machine does not have |
| a file that is present and is not a tzfile | the machine, before it is copied | musl would map it, check the magic, unmap it and keep UTC **in silence**; this is the one failure that succeeds at every step |

Both of the last two end with the machine keeping UTC **and saying so in the
journal**. That record is the whole difference between a documented decision and
a clock that is quietly an hour out.

### 9.7 What is NOT ruled here

* **`rivet-timed` is untouched.** Its servers, poll interval, noise floor and
  rate limit are §4 and §8's, and a timezone is not a synchronisation question:
  the wall clock is UTC internally either way, and this key changes only how it
  is rendered locally.
* **The journal stays RFC 3339 UTC** (`ADR-022` §3.2). A log whose timestamps
  moved with a document key would be a log two machines could not be compared
  across, and ordering is `boot` + `mono` regardless.
* **Nothing sets `TZ`.** Stated as a rule rather than an omission, because the
  day something does, `/etc/localtime` stops being read and this section's whole
  argument is void.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §3.2 — the restore was the supervisor's first action

**Corrected 2026-08-23**, while implementing it. The superseded phrase was
~~**The supervisor, as its first action**~~. The journal has to exist first or
the restore has no way to report its own failure, so it is step 2.

### In §3.2 — the mechanism was described as an unconditional set

**Corrected 2026-08-23**, and it is the more serious of the two. The superseded
phrase, used twice, was ~~"read a file and call `clock_settime`"~~. §3 rules a
**floor**; an unconditional set moves a *correct* clock backwards. The
implementation is `max(stored, now)`.

On a Pi the two are indistinguishable, because a cold-booted Pi's clock is
always the earlier of the two. The board that would have exposed this is the
x86 machine Rivet has not built for yet.

### In §4.1 — `ntpd-rs`'s dependencies were expected rather than read

**Verified 2026-08-22**, and the expectation was right. The superseded phrase
was ~~Expected, not verified~~.

### In §8 item 2 — the dependency tree was an open question

**Checked 2026-08-22.** The superseded question was:

~~`ntpd-rs`'s actual dependency tree~~

### In §8 item 3 — the slew threshold was an open question

**Ruled 2026-08-23, §4.2.1: Rivet always steps and never slews, so the threshold
does not exist.** The superseded question was:

~~Where "small" ends and "large" begins in §4.2~~

### In §3.4 and §8 item 4 — what keeps one reading off the floor was open

The floor rule was ruled before the number that makes it implementable, and
§3.4 said so: ~~Keeping a single reading from raising it needs a rule for when
readings are enough — how many must agree, from how many servers, within what
tolerance, or how long a stepped clock must run before the floor may follow
it — and none of those is ruled.~~ Three answers were offered: two servers must
agree, the floor follows only after an hour unstepped, or rely on the reset
verb alone. Two agreeing servers was chosen because it keeps a lie off the clock
rather than only off the floor, and needs no number but the noise floor
`rivet-timed` already had. (`.rulings/` R-247.)
