# Admin Surface — survey and gap analysis

* **Status:** Proposal, not accepted. Feeds `ADR-035` phase 4.
* **Date:** 2026-08-29
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Why this exists

`ADR-035` §9 scoped phase 4's read surface narrowly — *"the reads a debugging
session actually needed on 2026-08-28"*: `/proc/cmdline`, `/proc/self/mounts`,
`/sys/class/block/*/uevent`, cgroups, the raw journal.

The maintainer widened it on 2026-08-29: he expects the admin interface to fully
support reading system state (processes, memory, networking, storage, devices,
uptime), and to provide tools for managing the file system and for managing
applications and services (install, configure, manage, update, remove).
*(`.rulings/` R-317.)*

That is a substantially larger surface than §9's list, and it changes phase 4
from *"add the verbs that unblock dropping `nu`"* to *"the CLI is the
administrative interface."* This document surveys what exists, maps it against
that expectation, and names what neither of us has named yet.

**Nothing here is ruled.** It is a worksheet for the decisions that come next.

---

## 2. What exists today

Enumerated from `rivet-core/src/operation/` on 2026-08-29, not from memory.

**`rivet get <subject>` — 5 subjects**

| Subject | What it answers |
| :-- | :-- |
| `log` | the journal — records, boots, `returned`/`held` |
| `status` | services, as the supervisor sees them |
| `storage` | filesystems and space |
| `install` | the installer's own account of the last install |
| `schema` | the document's shape |

**`rivet apply <subject>` — 7 subjects**

| Subject | What it does |
| :-- | :-- |
| `edit` | change a file, `--expect`/`--replace` or `--append` |
| `install` | hand a signed `.rvt` to the installer |
| `uninstall` | remove an application |
| `reload` | re-read the document and converge |
| `stop` | stop a service; also power-off and restart |
| `trial-boot` | arm the other A/B slot; `trial-restart` |
| `repo-delete` | remove a git repository |

**`rivet explain <path>`** — what a projected `/etc` file is generated from.

---

## 3. The expectation, mapped

Legend: **built** · **partial** · **missing** · **excluded by a ruling**

### 3.1 Reading system state

| Wanted | Status | Note |
| :-- | :-- | :-- |
| storage | **built** | `get storage` |
| services | **built** | `get status` |
| journal | **built** | `get log` — but see §4 on the cursor |
| processes | **missing** | no subject. `ADR-017` §3.1.3 makes the supervisor's cgroup the authority for what is running, and nothing exposes it |
| memory | **missing** | total/free, and per-service from cgroups |
| networking | **missing** | interfaces, addresses, routes, DNS, link state. `rivet-netd` owns this and answers nothing |
| devices | **missing** | block devices exist inside `get storage`; nothing else does |
| uptime | **missing** | trivially available, genuinely needed |

### 3.2 Managing the file system

| Wanted | Status | Note |
| :-- | :-- | :-- |
| change a file | **built** | `apply edit` |
| **read a file** | **missing** | `edit.rs` names this against itself: *"an agent can change a file it cannot read back"* |
| list a directory | **missing** | |
| create / delete / rename / chmod | **excluded by a ruling** | see §5.1 — this needs a new ruling, not an implementation |

### 3.3 Managing applications

| Wanted | Status | Note |
| :-- | :-- | :-- |
| install | **built** | `apply install` |
| update | **built** | same verb, a newer `.rvt` |
| remove | **built** | `apply uninstall` |
| configure | **partial** | via `apply edit` on the document — no structured verb |
| **list what is installed** | **missing** | `/state/app/*/current` is the truth and nothing reports it |

### 3.4 Managing services

| Wanted | Status | Note |
| :-- | :-- | :-- |
| stop | **built** | `apply stop` |
| reload / converge | **built** | `apply reload` |
| list + state | **built** | `get status` |
| **start / restart one service** | **missing** | `stop` exists and has no counterpart |
| enable / disable | **partial** | it is a statement in the document (`ADR-015` §3.2.2), reached only through `apply edit` |

---

## 4. What neither of us named, and should be here

These came out of the last three days of actually debugging this machine.

| Proposed | Why it earns a place |
| :-- | :-- |
| **`get document`** | the read half of `apply edit`. `cargo rivet deploy --enable` refuses over TLS *today* for exactly this reason. Probably the single highest-value missing subject |
| **`get file <path>`** | §3.2's gap, stated plainly |
| **`get slot`** | which A/B slot is running, what is armed, whether a trial is pending. Reconstructed by hand from `/proc/cmdline` twice this week |
| **`get time`** | `ADR-021`: a Pi has no RTC, so the clock is wrong until something sets it and then it *jumps*. Every timestamp reading depends on knowing this and nothing reports it |
| **`get identity`** | which pinned keys may reach this machine — public halves only. `ADR-025` §5 makes revocation *"removing from a list"* and nothing can read the list |
| **`get boot`** | boot id, uptime, boots in order. The journal already computes `boots_in_order` |
| **`get temperature`** | a Pi throttles silently under load. Debugging without it is guessing |
| **`get repo`** | `apply repo-delete` exists; nothing lists them |
| **`get account`** | git accounts and their keys, public halves only |

---

## 5. The two questions that are not implementation

### 5.1 File management collides with a ruling, and needs a new one

Managing the file system is not missing by oversight. `ADR-006` §8.9 and
`rivet-core/src/operation/edit.rs` **excluded it on purpose**:

> **No delete-file, no rename, no chmod.** Each is a different act with a
> different plan; none of them is an edit.

and

> this must grow **refusals** rather than expressiveness … every extra verb here
> is a step towards being `sed`, and `sed` is the tool this exists to avoid.

So the request is in direct tension with a standing decision. It cannot be
implemented without ruling on the tension. **The shape that would honour both:**
new *subjects* under `apply` — `apply remove-file`, `apply rename` — each with
its own plan-before-apply and its own refusals, rather than new *flags* on
`edit`. That is exactly what §8.9 said ("each is a different act with a
different plan"), so it may be less a reversal than a reading of it.

**Not decided. Maintainer's.**

### 5.2 Reading is not the same risk as writing, and the surface should say so

Every subject in §4 is a **read**. Reads are additive, safe to grow, and each
one closes a place where the answer today is *"log in and look."* The write
verbs in §5.1 are a different conversation.

**Proposal: land every read first**, then rule on §5.1 separately. That also
matches `ADR-035` §9's precondition — what gates dropping `nu` is being able to
*see* the machine, not being able to change it, and `nu` was never how anything
was changed.

---

## 6. Suggested sequencing

1. **`get document`, `get file`, `get slot`, `get boot`/uptime** — the four that
   unblock work already in flight (`--enable` over TLS, `edit`'s asymmetry, A/B
   reasoning, `ADR-021`'s clock caveat).
2. **`get process`, `get memory`, `get network`, `get device`** — the maintainer's
   named list. These are what replace `open --raw` on `/proc` and `/sys`.
3. **`get identity`, `get repo`, `get account`, `get time`, `get temperature`** —
   the rest of §4.
4. **`apply start`/`restart`** — the missing counterpart to `stop`.
5. **Rule §5.1**, then the file-management verbs if ruled.
6. Only then is §9's precondition met and `nu` can go.

**Every one of these is independent of TLS and of `mcpd`** — `ADR-035` §9's
observation that the read surface *"needs no TLS, no `mcpd`, and no new
subsystem"* holds for the whole of this document. It can be built and tested
without touching the board, and each subject is a separate, committable unit.

---

## 7. Hardware and environmental telemetry

Raised 2026-08-29 after a review by Gemini. `get temperature` was already in §4;
thinking it through properly, **temperature is the least useful part of
"thermals" on a Pi**, and the framing needs correcting before anything is built.

**Verified 2026-08-29: there is no `thermal`, `hwmon` or throttle code anywhere
in this repository.** Nothing here is partly done.

### 7.1 "Thermals" on a Pi is four facts, and heat is the weakest of them

| Fact | Where | Why it matters more than the temperature |
| :-- | :-- | :-- |
| SoC temperature | `/sys/class/thermal/thermal_zone0/temp`, millidegrees | The number everyone asks for. On its own it tells you little: 70 °C is fine, and a throttled machine may read cool by the time you look |
| **Current frequency vs maximum** | `/sys/devices/system/cpu/cpu0/cpufreq/{scaling_cur_freq,cpuinfo_max_freq}` | A capped frequency **is** the throttle, observed rather than inferred |
| **Under-voltage** | firmware throttle flags | **The Pi's actual killer.** A marginal PSU or a cheap cable causes under-volt → throttle → and it is a leading cause of SD corruption. Far more often the answer to *"why is this board weird"* than heat is |
| **"…has occurred since boot"** | the sticky half of the same flags | **The one that cannot be recovered any other way.** A board that is fine now but throttled at 03:00 looks identical to one that never did. Everything else here is a spot reading |

That last row is the reason this is worth building carefully. It is also exactly
this project's house style: report **the observed fact**, not the inference —
`ADR-021`'s treatment of the clock is the same move.

**Unproven and needs research before it is specified:** the throttle flags come
from the Broadcom firmware, and `vcgencmd` — the usual way to read them — is a
Broadcom userland tool this image does not carry and should not. Recent kernels
expose some of it through the firmware node in sysfs; otherwise it is an `ioctl`
on `/dev/vcio`. **Which of those is available on the pinned kernel is a
measurement, not an assumption**, and the answer decides whether this is cheap
or needs a small adapter.

### 7.2 It must not be Pi-shaped, and that is a real constraint

Rivet's long-term target includes replacing the x86 machine, which has no
`vcgencmd`, no Broadcom mailbox and no tryboot. A subject modelled on the Pi's
throttle bitmask would be an operation shaped by one board.

**Proposed shape:** `get thermal` answers *zones*, *frequency against maximum*,
and a *throttle state* with a `since_boot` counterpart. The Pi's firmware
bitmask and x86's `coretemp` under `/sys/class/hwmon` are both **adapters** to
that, which is `ADR-011` applied to hardware: one fact, one owner, and the
board-specific spelling stays in the adapter.

### 7.3 Storage health, and the number this must refuse to invent

Gemini asked for *"wear-levelling metrics and remaining endurance"*. That splits:

* **eMMC** (JEDEC 5.0+) genuinely reports it — `life_time` and `pre_eol_info`
  under `/sys/class/mmc_host/…`. Real, and worth reading where present.
* **A consumer SD card generally does not.** A subject that reported "remaining
  endurance" for an SD card would be **inventing a number**, which is the exact
  failure this codebase refuses everywhere else.

What is available, and is the better early warning anyway:

* **The ext4 superblock error count**, which *persists across boots*. A card
  beginning to fail shows here before anything else notices.
* **Bytes written**, from `/sys/block/mmcblk0/stat`. `ADR-018` §7 already treats
  SD wear as a design constraint — the journal-on-disk decisions were all made
  for it — so measuring actual write volume **closes the loop on decisions that
  were taken blind**.

**Position: report writes and errors; refuse to report endurance that the device
does not know.**

### 7.4 What neither review named

| Proposed | Why |
| :-- | :-- |
| **reset reason / was the last shutdown clean** | **The A/B gap.** Nothing today distinguishes *"the trial slot booted and was not committed"* from *"the trial slot hung and something reset it."* On a headless board those need different responses |
| **watchdog state** | armed or not. On a machine in another room this is the difference between self-recovery and a trip to the room |
| **kernel taint** (`/proc/sys/kernel/tainted`) | one integer, and a tainted kernel changes what every other reading is worth |
| **pressure stall** (`/proc/pressure/{cpu,io,memory}`) | *"is this box struggling"* answered directly. Free-memory is a poor proxy and always has been |
| **per-interface counters** | errors, drops, link speed. `get network` in §3.1 should carry these rather than only addresses |
| **entropy available** | matters at first boot, when a machine is doing its most security-relevant work |

### 7.5 Gemini's list, mapped against what exists

| Asked for | Status |
| :-- | :-- |
| service state (running/failed/backoff) | **built** — `get status` |
| config dry-run before applying | **largely built** — plan-before-apply is the house rule, and `get schema` says what is accepted |
| update/self-update state | **partial** — `get install` reports the installer's own account |
| structured Rust panics as JSON | **not built.** Verified: stderr is captured into the journal as *text*, and no panic payload, backtrace or exit code is parsed into fields |
| binary integrity — running hash vs target | **unverified.** A loose grep matched three files and that is not evidence either way; it needs a real check before anyone claims it |
| thermal / throttling | **nothing at all** |
| storage wear | **nothing** — `get storage` reports space, not health |
| endpoint reachability, Bluetooth stack health | **out of scope for Rivet core.** These are properties of an *application's* dependencies. Rivet should make them expressible by an application rather than know about weather APIs or sensor MACs itself |

### 7.6 MEASURED on the board, 2026-08-29 — what the Pi actually exposes

§7.1 left the throttle flags as *"a measurement, not an assumption"*. Measured
on the Pi 4 at `10.0.0.2`, running the pinned kernel:

| What | Where | Result |
| :-- | :-- | :-- |
| SoC temperature | `/sys/class/thermal/thermal_zone0/temp` | **present** — `53556`, i.e. 53.5 °C |
| the same, via hwmon | `/sys/class/hwmon/hwmon0` | **present**, named `cpu_thermal`, carrying only `temp1_input` |
| current vs maximum frequency | `/sys/devices/system/cpu/cpu0/cpufreq/` | **present and complete** — `scaling_cur_freq` 1500000 against `cpuinfo_max_freq` 1500000, so this board is unthrottled at full 1.5 GHz |
| under-voltage alarm | an `rpi_volt` hwmon device | **ABSENT.** The only hwmon is `cpu_thermal`; this kernel has no voltage driver, so §7.1's *"the Pi's actual killer"* is **not** readable from sysfs |
| `get_throttled` as a file | anywhere under `/sys/devices/platform/soc` | **absent** |
| the firmware mailbox | `/dev/vcio` | **PRESENT** — along with `vcio_gencmd` and five siblings |

**Conclusion, and it splits the work cleanly in two:**

* **Temperature and frequency are free** — plain file reads, no privilege, no
  new dependency. They are also the two that port to x86 unchanged through
  `hwmon` and `cpufreq`, which is exactly the shape §7.2 asked for.
* **The throttle flags and under-voltage need an `ioctl` on `/dev/vcio`.** They
  are reachable — `vcgencmd` is not required, and carrying a Broadcom userland
  tool was never going to happen — but it is a Pi-specific adapter of perhaps
  fifty lines, and it is the only way to get the **sticky since-boot bits**
  §7.1 identified as the most valuable fact of the four.

**Proposed split:** build `get thermal` now from temperature and frequency,
which is cheap and portable; add the mailbox adapter as a second commit whose
absence degrades the subject rather than blocking it — a board whose flags
cannot be read reports the two facts it has, and says the others were not
measured rather than reporting them as zero. That is the same rule
`get memory` follows for PSI.

