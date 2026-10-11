# ADR-022: Logging and the Journal

* **Status:** Accepted — ruled 2026-08-22. Closes `ADR-017` §8 item 2.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`ADR-017` §7 sketched logging in four sentences — services do not open log files,
the supervisor captures their output, storage is capped — and left the format,
the record shape, and the relationship to Ignition undecided.

The maintainer asked whether a modern operating system still does text logs.
It does not, and has not for some time.

| | What it actually does |
| :-- | :-- |
| Classic syslog | text lines with a severity. Human-readable, machine-hostile |
| **systemd-journald** | structured key-value records, indexed and queryable; text is a rendering |
| Windows ETW | structured events with *registered schemas* — further than Unix ever went |
| Android logcat | binary ring buffer with typed fields |
| OpenTelemetry | typed events with attributes, correlated by trace and span identifiers |

---

## 2. This is the same decision, for the third time

> **A log entry is a structured record. The text an operator reads is a view of
> it.**

That is `ADR-006` §4 — structured output primary, human text rendered from it —
applied to a third surface. `/etc` was the second (`ADR-007` §3.6). The design
is not acquiring a new idea here; it is applying one it already has, which is
the strongest available evidence that the idea was right.

---

## 3. Format: line-delimited JSON

**Ruled 2026-08-22.** One complete, self-describing record per line.

### 3.1 Structure is right; opacity is journald's mistake

journald's binary format is the most-complained-about thing about it, for
precisely the reason `ADR-006` §1.2 gave about the Windows Registry: **when
your tooling is broken is exactly when you need to read the logs**, and a binary
journal requires working tooling to diagnose why the tooling is not working.

JSONL is readable with `cat` and `ripgrep` when everything else is on fire. It
is more verbose than a packed encoding, and that is an acceptable trade for a
machine producing a few megabytes a day.

**The on-disk encoding sits behind a typed interface**, so a compact binary
format (CBOR, MessagePack) can replace it later without anything above noticing.
This is the same reasoning that chose `ext4` over `f2fs` (`ADR-018` §7) and
static over shared linking (`ADR-014` §4.2): take the debuggable option during
bring-up, revisit when a measured number says otherwise.

### 3.2 The record

```json
{"ts":"2026-08-22T14:02:11.412Z","boot":"0d3f…","mono":184.221,
 "sev":"warn","svc":"rivet-netd","corr":"a91c…","actor":"agent:claude",
 "msg":"association failed","ssid_len":11,"reason":"4-way handshake timeout"}
```

Fixed fields are few — timestamp, boot identifier, monotonic offset, severity,
service, correlation, actor, message — and arbitrary typed fields follow.

**`boot` and `mono` are not redundant with `ts`, and this matters here more than
on most systems.** A Pi has no RTC (`ADR-021`), so wall-clock time is wrong at
early boot and jumps when corrected. Wall-clock ordering is therefore
unreliable across exactly the window most worth reading. A boot identifier plus
a monotonic offset makes ordering **always** recoverable, whatever the wall
clock did.

### 3.3 Secrets cannot be logged

`ADR-020` §4's `Secret` type redacts in `Debug` and `Display`, so a secret
rendered into a log field emits `<secret>`. That is not a rule about logging
discipline — it is a property of the type, and it is the reason there is no rule
to remember.

---

## 4. Two capture paths

Easy to forget the second one, and half the system depends on it.

* **Structured, natively.** Rivet's own services and applications emit records
  through the platform crate (§6). No serialisation to text ever happens.
* **Text, wrapped.** `wpa_supplicant` and every other C mechanism
  emit lines. The journal captures their standard output and error and wraps
  each line as a record with the line as `msg`.

**The mechanisms' format stays at the bottom of the stack** and never propagates
upward — the same rule `ADR-008` §5 applies to `/etc`.

> **A fourth source arrived 2026-08-24 and it is §11.** Both paths above reach
> the journal through a pipe the supervisor owns *because it spawned the
> process on the other end*. A process the supervisor did **not** spawn — the
> login shell, the editor — has no such pipe, and §11 is the channel it uses.
> The rule that "the mechanisms' format stays at the bottom of the stack" is
> unchanged; what §11 adds is a way in for a format that is already a record.

### 4.1 The kernel is a third source

The journal ingests `/dev/kmsg`. This is worth more than completeness: the
firmware-loading failures that `ADR-004` §8 warns will dominate Stage 2 —
`brcmfmac` not finding its `clm_blob`, NVRAM under the wrong board-specific
name — appear only in kernel messages. Ingesting them means the fiddliest
diagnosis in the project is queryable through the same tool as everything else.

---

## 5. Correlation from the first record

**Ruled 2026-08-22.** Every record carries a correlation identifier and the
actor that triggered the work.

One `rivet apply` touches the supervisor, `netd`, possibly an application and
a mechanism underneath it. Without a correlation identifier those are four
unrelated streams that happen to be adjacent in time — and "adjacent in time" is
exactly what a clock correction destroys (§3.2).

It composes with `ADR-012` §4's actor provenance: a configuration value records
who set it, and now the log records show what that actor's change actually did.
Same question, answered at two altitudes.

**One field now; retrofitting means every earlier record lacks it**, and the
expensive part — threading the identifier through the call path — costs the same
whenever it is done.

---

## 6. Where Ignition sits

`ADR-013` §2.1's placement test answers this directly.

| Concern | Home | Why |
| :-- | :-- | :-- |
| The emitting API — how a program produces a record | **Ignition** | already there; every project of the maintainer's wants it |
| Routing an application's records into Rivet's journal | **the SDK** (`ADR-036`) | "what it means to be a program on Rivet" |
| The journal itself — capture, storage, rotation, query | **Rivet** | would Athena want a daemon collecting other processes' output? No |

**The first row holds off a Rivet, and the SDK holds both rows on one.**
`ADR-036` §4 rules Ignition out of any program a Rivet supervises, so an
application there emits through `rivet_sdk::journal` — which is the second row
and the first at once. A program that brings its own logging framework and
bridges it onto that is making a choice of its own, and `ADR-013` §2.1's fourth
row is where such a choice lives.

### 6.1 CHECKED 2026-08-22. True, and it does not help

**Ignition's logging is built on `tracing`.** Confirmed against Ignition v0.3.0:
it depends on `tracing 0.1` and `tracing-subscriber 0.3` (features `["fmt"]`),
and `logging::init` builds a `Registry`.

**The consequence this section hoped for does not follow.** The hope was that
Rivet's journal could be a Subscriber and the integration would be nearly
free. It cannot be, because **`ignition::logging::init` installs a closed
subscriber**: it constructs `Registry::default().with(reload::Layer<LevelFilter>)
.with(fmt::layer().with_writer(...))` entirely inside the function and hands it
to `tracing::subscriber::set_global_default`. There is no injection point —
the module's whole public surface is `init`, `LogConfig`, `LogGuard`,
`LevelHandle`, `Rotation` and `LogError`, and no function in it accepts or
returns a `Layer` or a `Subscriber`. A second `set_global_default` returns
`LogError::Init`, so the two cannot coexist.

Two further facts sharpen it: the `fmt` layer is configured for **text** output
(`with_ansi(false)` to a rotating file), and `tracing-subscriber`'s `json`
feature is not enabled — so §2's position that a record is structured and text
is a *view* of it is not something Ignition's sink preserves.

**What this means.** The journal is a `tracing` **`Layer`**, not a Subscriber,
and Rivet owns the `Registry` that holds it. Three ways to reach that, in
preference order:

1. **Ignition grows a compose-your-own-layers API** — `init` gains a sibling
   that returns the layer stack instead of installing it, or accepts additional
   layers. This is the right fix, it benefits every consumer, and it is
   `ADR-013` §7's item 2 (Ignition extensions) — an Ignition ADR and a pinned
   tag, not a Rivet change.
2. **Rivet builds its own `Registry`** with the journal Layer and simply does
   not call `ignition::logging::init`, using Ignition for configuration and the
   CLI surface only. Available today, costs nothing, and duplicates the
   filter/reload plumbing Ignition already has.
3. **Applications keep Ignition's file sink and the journal reads the files.**
   Rejected — it makes text authoritative, which §2 forbids.

**Spans still supply §5's correlation for free**, because that is `tracing`'s
property and not Ignition's; it survives whichever of the three is taken.

**Method, and what is still unproven.** Read from Ignition v0.3.0's source and
`Cargo.toml`, in Ignition's own repository; the absence of an injection point was
checked twice, by enumerating the module's public items and by locating every
`Layer`/`Registry` mention in it. **Unproven:** that a journal Layer performs
acceptably as PID 1's supervisor writes to it — that is a Stage 3 measurement,
not a documentation question.

---

## 7. The journal is part of the supervisor, not a service

**This closes `ADR-017` §8 item 2.**

The supervisor captures every service's output, so it must be logging before any
service exists. A journal that were itself a service could fail to start, and
the record of *why* would have nowhere to go — which is the one failure a
logging system must not have.

It is therefore part of `rivet-sysd`, which `ADR-017` §3.1 already
allows to fail and be restarted by PID 1.

Records produced before `/state` is mounted are buffered in memory and flushed
once it is available.

> **Unamended 2026-08-24, and that is the result.** On 2026-08-24 `cmd/rivet`
> became a second writer of `/state/log/journal.jsonl` and `PLAN.md` §10.6
> recommended amending this section to read *"the supervisor and the
> administration surface"*. **The maintainer refused it**, and §11 is what was
> built instead: the supervisor holds the journal open and offers an ingest
> socket, so everything above stands exactly as written — **one writer, and it is
> the supervisor**. §11.1 carries the two reasons the amendment was the wrong
> answer, and both are worth reading before proposing one again.

---

## 8. Storage, retention, and the card

`/state/log`, and every decision here is an SD-wear decision (`ADR-018` §7).

| | |
| :-- | :-- |
| **Batched writes** | never `fsync` per record. A power cut loses the last few seconds of logs, which is an acceptable trade against writing to flash continuously |
| **Size-capped files, capped total** | oldest deleted first. `ADR-017` §3.5's restart-loop hazard is the same hazard: an unbounded log kills the card |
| **Retention is configuration** | a field in the desired-state document like anything else |

---

## 9. Query is an operation

`rivet log` is an operation in the catalogue (`ADR-006` §6.4), emitting
structured output by default and rendering text for a terminal — filtered by
service, severity, time window, correlation identifier or actor.

> **Superseded 2026-08-24 — the operation is `rivet get log`.** The spelling
> in the sentence above predates the catalogue it cites, and `ADR-006` §6.4's
> verb set is closed at nine. §9.1 carries the ruling and the reasoning; the
> rest of this section stands unchanged.

Because it is a catalogued operation rather than a special case, it is reachable
from every adapter (`ADR-006` §3.1): the CLI now, the dashboard later, an MCP
tool for an agent diagnosing its own failed `apply`.

---

## 9.1 Built 2026-08-24, and the verb is a conflict in the record

**A Rivet machine can now be read back remotely**, which is the thing this ADR
was written for. What exists:

| §  | What | Where |
| :-- | :-- | :-- |
| §3 | the JSONL form, **both directions** | `rivet_core::journal` |
| §7 | buffer until `/state`, then flush | `Journal::open_on_state` |
| §8 | `/state/log`, 1 MiB × 8 files, oldest dropped first | `rivet_core::state`, `Journal::rotate` |
| §9 | the query, as a catalogued operation | `rivet_core::operation::get`, `rivet get log` |

**The record moved to `rivet-core`**, out of `cmd/rivet-sysd` and out
of `cmd/rivet-timed`, which had a second copy of it. The thing that forced it
is §9 itself: a **reader** in a third process cannot hold its own copy of a
format §3 makes authoritative, and the failure would be silent — a reader that
disagrees with a writer shows a shorter log rather than crashing. The reader is
tested against a capture of a real guest's console rather than against its own
writer (`AGENTS.md` §4).

### The verb: this ADR says `rivet log` and `ADR-006` §6.4 forbids it

§9 above writes *"`rivet log` is an operation in the catalogue (`ADR-006`
§6.4)"*. `ADR-006` §6.4's verb set is **closed** at nine — `get`, `set`, `list`,
`add`, `remove`, `apply`, `plan`, `export`, `explain` — and says it "is extended
only by amending this ADR". `ADR-006` §8.4 and §8.5 each hit the same wall, for
`reboot` and for `shutdown`, and each recorded the same conclusion: *"`AGENTS.md`
§1 puts adding a tenth out of an agent's reach."*

**So it is built as `rivet get log`**, with `log` as a subject — the shape
`apply` already takes for its four. Nothing about the operation depends on which
it is: the verb is one line in `rivet_core::operation::get` and one match arm
in `perform`. The maintainer's ruling settled the question; the two-line
diff would have been the same either way.

#### Ruled 2026-08-24: `get log` stays, and **§9's opening sentence is the error**

The recommendation put to the maintainer was to amend `ADR-006` §6.4 and add a
tenth verb, on the grounds that the ADR said `rivet log` and the ADR is the
authority. The maintainer's reply was to ask whether `get log` did not fit the
shape of the machine more accurately, and that is the better reading and it is
taken. *(`.rulings/` R-272.)*

**A verb is what you are doing; a noun is what you are doing it to.** Reading
the journal is a `get`, and `ADR-006` §6.4's argument for a closed set is that
*"a reader who knows one command can predict the others"* — worth more than a
command that reads like `journalctl`. A tenth verb would have widened the
surface to match a sentence written before there was a catalogue to fit into.

So the correction lands **here, not in `ADR-006`**: §9's *"`rivet log` is an
operation in the catalogue"* is superseded by this section. The operation is
`rivet get log`, it was already built that way, and `ADR-006` §6.4's verb set
remains closed at nine.

### Records come back as their own lines

Each record in the result is the JSONL line the machine wrote, verbatim, rather
than a re-encoding through `rivet_core::value::Value`. §3 makes that form
authoritative, so a second encoding of the same record would be `ADR-011`'s
failure inside one answer — and `Value` could not carry a record without two
widenings its own documentation reserves for the day something needs them
(dynamic object keys, and floating point for `mono`).

The human reading is §2's *view*, made by the CLI out of `Record::to_text`, so
`rivet --output text get log` prints
`[     1.319] error rivet-timed: REFUSED …` and `--output json` prints the
record. Both are readers of one structure, which is `ADR-006` §4.

### Durability: `write(2)` per record, `fsync` never

§8 rules out an `fsync` per record. It does **not** follow that records should be
buffered in the supervisor, and they are not: each is handed to the kernel as it
is emitted and the page cache coalesces them, so flash sees blocks rather than
lines. What that buys is the case this journal exists for — `ADR-017` §3.1
expects the supervisor to be killed and restarted, and `reboot(2)` syncs, so the
only loss is a power cut, which is exactly the trade §8 named. Buffering in user
space would have made a supervisor crash lose its own last words.

### What is proved, and what is not

**Proved, in the emulator, on 2026-08-24**: the directory is created, the
pre-`/state` records are flushed in order, every subsequent record reaches the
file, and `rivet get log` read 22 records back **from the running guest** into a nushell
table — filtered by severity, grouped by correlation identifier. `smoke.sh`
asserts the open and the flush, and both assertions were driven to failure by
injection.

**Not proved: that any of it survives a reboot.** QEMU has no `/dev/mmcblk0`, so
`/state` there is a directory on the initramfs — writable, which is why the write
path could be exercised at all, and RAM, so `rivet get log previous` has never
selected a boot that actually ended. §8's rotation is likewise untouched: a boot
writes a few kilobytes against a 1 MiB cap. Both are a board's to settle, and
`ADR-026` §4 is the table that says so.

---

## 9.2 §4's capture, built the same day — and both paths, not one

§4 has two paths and the record treated them as different mechanisms:

> * **Structured, natively.** Rivet's own services and applications emit
>   records through the platform crate (§6). No serialisation to text ever
>   happens.
> * **Text, wrapped.** `wpa_supplicant` and every other C mechanism
>   emit lines. The journal captures their standard output and error and wraps
>   each line as a record with the line as `msg`.

**They are one mechanism today, and the difference is a parse.** There is no
channel for "natively" — §6.1 established that Ignition's subscriber is closed
and the platform crate does not exist — so `cmd/rivet-timed` writes its
records to *stdout*, over the same pipe a C daemon writes lines to. So the
supervisor captures both and asks one question of each line: **does it parse as
a record?** If it does it is forwarded as one; if it does not it is wrapped.

That question is answerable only because §9's reader exists (§9.1). It is the
same code, used from the other direction, and it is the reason both halves of §4
landed together rather than one at a time.

### Why forwarding matters more than tidiness

Wrapping a record as another record's `msg` buries its **severity** at `info`.
`rivet get log | where sev == error` would then miss a time client that
failed, which is a functional loss. What a forwarded record keeps is what only
the service knows — the severity, the message, its fields, and its own
correlation identifier as `emitted_corr`.

**What it does not keep is who it says it is.** `svc` is overridden with the
service the supervisor actually started, and `boot`, `mono`, `ts` and `actor`
are the journal's. The thing on the other end of a pipe is a program; `ADR-011`'s
one authority for *who said something* is the supervisor that started it, not
the speaker.

### Generators are captured too

`Kernel::run_to_completion` returns what the program said, so a generator run
before a service starts has its output in the journal rather than on a console
nobody is watching. **Including on the failing path**, which is the one that
matters: an exit code without the generator's own explanation is what makes
somebody go to the machine, and capturing the stream without carrying those
words out would have been a regression rather than a fix.

### The two defects it produced, which were the same defect

Recorded because the second arrived four hours after the first, in the same
change, through a different door — and because neither was visible in 1700
passing tests.

**`Stdio::null()` opens `/dev/null` in the parent, before the fork.** On a
machine that has none, the spawn fails with `ENOENT` **reported against the
program's path, which is present**. `tools/qemu/run.sh`'s root is an initramfs
and the kernel does not auto-mount devtmpfs onto one, so there is no `/dev/null`
there:

1. `cmd.stdin(Stdio::null())` on `spawn` — took every service on the machine
   down on the first boot after it was written, each refused with
   *"this image cannot run the program this service names"*.
2. `Command::output()` on `run_to_completion` — which **nulls stdin by default**
   — took a key generator down the same way.

Both are fixed by setting stdin explicitly to `inherit`, and both are documented
at the code rather than only here. `AGENTS.md` §6 is the rule they illustrate:
until it runs, it is not working.

---

## 9.3 §4.1's kernel messages, and why the emulator cannot see them

`crate::kmsg` in `cmd/rivet-sysd`. `/dev/kmsg` is opened after the
journal reaches the card and before the first `converge`, so **the whole boot's
kernel messages land in the file** — the descriptor starts at the oldest
surviving record, which is what makes this worth doing at all: the supervisor
starts long after the kernel has finished saying anything interesting about
bringing the hardware up.

**Eight syslog priorities onto §10 item 2's four**, pessimistically at the top:
`emerg`, `alert`, `crit` and `err` all become `Error`, because each is a thing an
operator has to act on and there is no level above `Error` to tell them apart
with. The original number is a field, so nothing is lost.

`svc` is **`kernel`**, not the supervisor's — the supervisor is the transport —
and each record gets **its own** correlation identifier, because §5 ties together
the records of one causal chain and a kernel message belongs to none Rivet
started.

### It cannot be exercised under QEMU, and that is still open

`tools/qemu/run.sh` boots an **initramfs**, and `CONFIG_DEVTMPFS_MOUNT` does not
apply to one. Nothing else mounts devtmpfs. **`rivet_core::mount::stage0_plan`
names `/dev` since `15f1acb`** and converges when the kernel got there first
(`ADR-017` §3.1.12); it used to take `/dev` as given, because on a board the
kernel really has mounted it. So under the emulator there
is no `/dev/kmsg`, no `/dev/null`, and nothing else.

**That cost three defects on 2026-08-24** — two spawn failures reported against
present binaries (§9.2), and this, which is not fixable the same way. So this
path is proven against the **build host's own kernel** instead:
`on_a_host_with_dev_kmsg_the_records_arrive_whole_and_parse` opened `/dev/kmsg`,
read 64 records and parsed every one. The parser is additionally tested against
a capture kept verbatim in `cmd/rivet-sysd/tests/fixtures/`.

### Proven on hardware 2026-08-24 — 321 kernel records in one boot

**The emulator still cannot see them and the tripwire still stands; a board
can.** The first card with `devtmpfs` in PID 1's plan (`ADR-017` §3.1.12) read
its own journal back remotely:

```
$ rivet get log | from json | get records | each { from json } | where svc == kernel | length
321
```

Records carry `kernel_mono` and `kernel_metadata` — e.g. `SUBSYSTEM=pci
DEVICE=+pci:0000:01:00.0` — and reach back to the earliest surviving message,
which is the claim this section opens with and the reason the descriptor is
opened where it is. `bcmgenet fd580000.ethernet: GENET 5.0 EPHY: 0x0000` is
among them: the NIC that `ADR-009`'s whole network path depends on, reported by
the kernel into Rivet's own journal rather than only onto a console.

**Eleven of the boot's records were `warn`, and every one was explicable**: two
from the supervisor by design (`ADR-021` §1 and §4 — no stored clock on a first
boot, and the deliberate under-claim that time is unsynchronised), three PCIe
regulator lines, `rdinit=/init failed: -2`, and a GPT pair — `GPT:4194303 !=
15564799` — which is `ADR-018` §10's ungrown `/state` seen from the kernel's
side on a 7.42 GiB card holding a 2 GiB image.

**One cost of this path, observed rather than predicted:** every kernel record
is also written to the console as a full JSON object, so 321 of them scroll the
console hard during the window where PID 1's own plain-text lines are the only
diagnostic there is. That is not a defect of this section, but it is a real
interaction with the same console instrument a service's captured stdout
uses (`ADR-022` §9.2), carried as an open question until
the maintainer's 2026-08-24 ruling settled it: only `warn`, `crit` and `err`
print to the console.

Fixing the loop is a change to PID 1's job list (`ADR-017` §3.1.5 item 1) and is
therefore the maintainer's; the recommendation on record is the same shape
as the board fix — PID 1 mounts `devtmpfs` under QEMU too, non-essential.
`tools/qemu/smoke.sh` holds a **tripwire** that fails the day the
emulator starts delivering kernel records, so the fix cannot land unnoticed.

### What still reaches a console and not the journal

**PID 1's own lines, and nothing else.** `cmd/rivet-initd` predates the journal,
is not a service, and has nowhere to write — the journal is the supervisor's, and
PID 1 runs before it (`ADR-017` §8 item 2). Everything else a Rivet machine
says is now a record on `/state/log`, so the remaining reason to watch a console
is a *boot* — which is the shape the maintainer asked for.

---

## 9.4 `previous` selects the boot that ended — proven 2026-08-24, and only a card could

**This is the one claim about the journal that no emulator can settle**, because
QEMU's `/state` is RAM and a reboot takes the evidence with it. A Raspberry Pi 4
was asked, from a workstation, for the boot it had just finished:

```
boot selector: previous
distinct boot ids IN RECORDS: {05319ec6-c005-422e-857e-9604ca13d971}
returned 489    held 975    unreadable 0
first ts  1970-01-01T00:00:03.300Z
last  ts  2026-08-24T22:59:47.440Z
```

One boot identifier in the returned records, and it is the ended one rather than
the running one. `held` counts both boots; `returned` counts the selection.
`unreadable` is 0. **This sentence cited a §4.2 that does not exist in
this document** — noticed 2026-08-24 while §11 was being written, and corrected
here rather than by inventing one. The field is declared in
`rivet_core::operation::log` and the rule behind it is `rivet_core::journal`'s
*refuse rather than guess* (`AGENTS.md` §5): a line that does not parse is an
error carrying the raw bytes, never a partial record.

**The tail of that boot is `ADR-017` §3.1.10's ordered shutdown and `ADR-021`
§3.3's write-on-shutdown, in the same six lines and 53 milliseconds:**

```
22:59:47.387  supervisor     shutdown requested; stopping services
22:59:47.390  supervisor     writing the running clock to the card now rather
                             than at the hour (ADR-021 §3, §3.3)
22:59:47.412  supervisor     the running clock is written to the card, so the
                             next boot has a floor to restore from
22:59:47.422  rivet-netd     stopping      .434  rivet-netd     stopped
22:59:47.425  rivet-timed    stopping      .437  rivet-timed    stopped
22:59:47.440  supervisor     all services stopped; exiting
```

### The two boots overlap in time, and that is correct

**Boot 2's first record is stamped 44 ms *before* boot 1's last one.** Boot 1
wrote its clock to the card at `.412` and kept running to `.440`; boot 2
restored that floor before the network existed (`ADR-021` §3) and stamped its
early records from it. So the journal legitimately contains two boots whose
timestamps interleave.

This is not a defect and it is not fixable — it is what a machine with no RTC
looks like when it is behaving. It is also the concrete demonstration of why
`get log`'s `boot` input says the journal **cannot be selected by time**, and
why boot identity is a `boot` field rather than a timestamp range. That input's
wording was written before any board had shown the overlap; a board has now
shown it.

**Anyone measuring how long a Rivet board was down must not use the board's
own clock to do it.** Every timestamp in that window is a restored floor. The
console capture — which has the workstation's wall clock — put the HDMI dark for
8 seconds across this reboot.

## 9.5 `--follow` — ruled and built 2026-08-25, and the operation stayed one-shot

**The question `PLAN.md` §20.6 put:** nothing can ask a running Rivet
anything. Every question an operator or an agent asks costs a connection
and a process spawn, and the machine cannot say anything unprompted. The read
side of the supervisor exists and is unreachable.

`rivet get log --follow` is the first answer, and it was chosen because it
needs **no new protocol**: §3 already makes the journal line-delimited JSON with
a stable schema, §7 already gives it exactly one writer, and §8's active file is
already append-only. Streaming it turns polling into subscription with no bus,
no socket and no dependency.

### The ruling: following is a property of the transport, not of the operation

§9's operation **answers once**, from what is on the card at the moment it is
asked. That did not change and must not: an operation that could wait is an
operation with a clock in it, and `rivet-core` may not name one
(`AGENTS.md` §3). An adapter that wants a stream calls the operation — or reads
the file — holds a cursor, and calls again.

What that buys is that every adapter gets a follow of its own shape.
`ADR-006` §3.1's unbuilt adapter will not stream by running this loop; it will
hold the same cursor and push. The alternatives, and why each was rejected:

| | | |
| :-- | :-- | :-- |
| **(a)** | A `--follow` argument on the operation, with the core looping. | The core grows a clock, a sleep and a notion of a result that arrives in pieces. Every operation's return type changes to carry it. Rejected. |
| **(b)** | A second operation, `get log-stream`. | `ADR-006` §6.4's noun set grows a noun that is the same subject twice, and the two would drift. Rejected. |
| **(c)** | **The adapter loops; the core owns the cursor.** | `rivet_core::journal::Follow` decides which records are new and whether the machine restarted — the two decisions with boot identifiers in them (§3.2). The loop, the clock and the file offset are the adapter's. **Ruled.** |

### Three things the design fixes in place

1. **The boot is resolved once and then held.** A selector of `latest`
   re-resolved on every read would silently follow a *different* boot the moment
   the machine restarted, splicing two boots into one stream. A restart is
   reported instead — and a boot identifier that has been *seen before* is a
   re-read rather than a restart, which is what stops a follower walking
   backwards onto a rotated file's contents.
2. **`previous` is refused for following.** It names a boot that has ended, so
   nothing will ever be appended to it; a stream that is silent by construction
   is indistinguishable from a machine doing nothing.
3. **stdout carries records and nothing else.** One JSONL line per record,
   verbatim as §3 requires. The closing summary, a rotation, a restart and an
   unreadable line all go to **stderr**, which is the login shell's
   stray-byte hazard applied to a stream, and the exit status is still 0.

### It is bounded, and that is the point rather than a detail

A follow is the first command in `cmd/rivet` that would not return on its own.
`PLAN.md` §20.6's second half — *nothing bounds the output or the runtime of a
command issued from a workstation* — is answered here for the case that actually has no
bound, and the general case is left alone deliberately: a one-shot operation is
bounded by the work it does, and a watchdog over every command would be a
mechanism with one user.

| Bound | Value | Why it is not the other two |
| :-- | :-- | :-- |
| Deadline | 300 s, `--for <seconds>`, or `--for forever` | A forgotten session ends; a deliberate tail is still reachable by naming it. |
| Records | 5000 | A service in a restart loop (`ADR-017` §3.5) writes faster than a deadline can catch. |
| The reader | the far end closing | `… --follow \| head -5` must end the stream, not crash it. |

**Each of them ends the stream by saying which one it was.** A follow that hit a
ceiling and a machine that went quiet are the same silence otherwise.

### Reading only what was appended, and what rotation does to that

A follower re-reading the whole journal every poll would read up to §8's eight
megabytes twice a second. It reads from a **byte offset into the active file**
instead, and holds that file's **inode** beside the offset.

**The inode is load-bearing and a length comparison is not.** §8 rotates by
`rename(2)`, so after a rotation the file at `journal.jsonl` is a *different
file with the same name* — and it grows. A follower comparing lengths sees a
longer file at the same name and reads from the middle of it. That is not
hypothetical: it is what the first implementation did, and the test named
`a_rotation_carries_the_tail_of_the_old_file_before_the_new_one` is what caught
it.

On a rotation the records the follower had not reached are **not lost** — they
are in `journal.1.jsonl` at the same offsets — so they are read out of the new
name first and append order survives. Only a *second* rotation inside one poll
interval can carry records away, and that is reported as a gap rather than left
silent.

### What is proved, and what is not

Proved by test, on the development host: every bound, the rotation carry-over,
the half-written line held back until it is complete, the boot pinning, the
restart notice, and that stdout carries nothing but records. The clock is a fake
that only advances when the follower sleeps, so a 300-second deadline is proved
in microseconds.

**Proved on hardware the same day.** A Raspberry Pi 4, 2026-08-25, running a
signed system update taken with `trial-restart`: a 150-second follow emitted 24
records — three of backfill and **21 live**, as the installer verified 683 MB
and wrote 640 MiB into the other slot — ended on its deadline, said so on
stderr, and put nothing on stdout that was not a record.

The same board is what put a number on §9.6: a bare `rivet get log` there
would have returned **9553 records**.

**Two things a board has still not settled**, and `PLAN.md` §21.4 carries both:
a rotation *during* a follow, because that journal did not cross §8's megabyte
while anything was watching; and whether 500 ms is the right poll interval under
real load.

---

### 9.5.1 Reversed 2026-08-27: `--follow` becomes an operation input

**The maintainer ruled that `--follow` becomes an input of the operation**,
which is §9.5's option **(a)** — the one that table rejected. The record shows
its own history rather than being edited, which is what `AGENTS.md` §8.3 asks
for, so §9.5 stands above with its reasoning intact and this is what supersedes
it.

**Why it was reversed.** §9.5 decided the question as a *design* question and
answered it well: following is a property of the transport, the loop belongs to
the adapter, and the core stays clockless. What it did not weigh is the
consequence `PLAN.md` §50 found by building the other adapters — **`--follow`
exists in exactly one adapter and no other can see it.** It is not in
`CATALOGUE`, so `rivet mcp` cannot offer it and `rivet-fleet`'s terminal
interface polls instead, with a comment saying why. `ADR-006` §3.1's co-equality
of adapters is the rule that lost: a capability reachable from the CLI and from
nothing else is precisely what that section forbids, and §9.5 created one
without noticing because at the time there was only one adapter.

**So §9.1's *the operation answers once* is amended**: an operation may declare
an input that means *keep answering*, and the catalogue must say so.

### What this does NOT license

**The core still may not name a clock** (`AGENTS.md` §3), and that constraint is
above this ADR rather than inside it. §9.5's option (a) as written — *"the core
grows a clock, a sleep and a notion of a result that arrives in pieces"* — is
still not available. What is being reversed is the **catalogue's silence**, not
the location of the loop: `rivet_core::journal::Follow` already owns the cursor
and the restart decision, and it stays where it is.

### The dependency this ruling has, which is not yet met

**MCP cannot use it today.** An MCP tool call returns one result; a follow that
answers more than once needs **notifications**, which `ADR-032` §4.3 lists as an
open question and nothing has built. So the ruling delivers, in order:

1. **the catalogue tells the truth** — `follow` and `for` appear as inputs of
   `get`, described as `log` ONLY, the way `boot` and `limit` already are;
2. **every adapter must answer for it by name** — an adapter that cannot stream
   refuses the input with a sentence saying so, rather than ignoring a word the
   catalogue advertises;
3. **MCP gains it when §4.3 is ruled**, and not before.

Item 3 is why this is recorded here before it is built: the ruling's stated
purpose is that MCP and the terminal interface gain a follow, and **one of those
two cannot have it until a different open question is answered.** `PLAN.md`
carries the work.

---

## 9.6 A default limit on `get log` — ruled 2026-08-25

**Until this ruling a bare `rivet get log` answered with the whole journal**,
which §8 caps at eight megabytes. That is not a limit; it is the absence of one.
The answer is built as a string in the process, sent back over the wire, and
— for the caller §9 was designed for (`ADR-012`) — spent as an agent's context.

**`limit` now defaults to 200 records**, counting from the end, and the word
`all` is how to ask for everything. A word rather than `0`, because *give me
nothing* and *give me everything* must not be the same request, and `0` is a
number a caller can arrive at by arithmetic.

**The answer says what was applied.** The result carries `limit` beside
`returned` and `held`, so a cut answer says by how much and names the word that
undoes it. An answer that was quietly truncated would be `AGENTS.md` §5's
failure exactly — a log that says *there is nothing about that* when there is.

**Flipping it broke no test in the workspace**, which is `PLAN.md` §20.1's
lesson arriving a second time in the same week: a property no test asserts is a
property nobody chose. Three tests now hold it.

---

## 10. Open

1. **Whether Ignition uses `tracing` — CHECKED 2026-08-22 (§6.1). It does,
   and it does not help as much as hoped:** `logging::init` installs a closed
   subscriber with no injection point, so the journal is a `Layer` in a
   `Registry` Rivet owns. §6.1 carries the three ways forward; the first —
   an Ignition compose-your-own-layers API — is the one to pursue, and it is
   `ADR-013` §7's item 2 rather than work here.
2. **Severity levels**, kept to a small closed set. `ADR-006` §6.4's closed
   vocabulary rule applies — this is exactly the kind of enumeration that grows
   one convenient value at a time until it means nothing.
3. **Whether the fleet dashboard pulls logs or has them pushed.** Pull is
   simpler and needs no buffering-when-offline story. Not urgent; it depends on
   the dashboard existing.

---

## 11. The ingest socket — how a process the supervisor did not spawn files a record

* **Status:** Accepted — ruled 2026-08-24. Closes `PLAN.md` §10.6.
* **Consequence:** §7 is **unamended**.

---

### 11.1 The question, and why the obvious answer was refused

On 2026-08-24 `rivet apply edit` (`ADR-006` §8.9) began appending its record to
`/state/log/journal.jsonl` directly. It had to record *something* — `ADR-012` §4
requires actor provenance and an edit with no account of it is the worst kind of
change — and it could not use §4's stdout route, because **`cmd/rivet` is the
login shell**: its stdout is the operator's session, and a
record written there would corrupt the structured answer being read and reach no
journal at all.

`PLAN.md` §10.6 put three options to the maintainer and recommended (a): amend §7
to read *"the supervisor and the administration surface"*. **He refused it as a
bandaid**, on two grounds:

| | |
| :-- | :-- |
| **It answers the wrong question** | The question is not *may the CLI write too*. It is *how does a process the supervisor did not spawn get a record into the journal*. Answered per-writer, it needs an amendment per writer — and `femto`, and every future tool on the card, are the next ones. |
| **It can corrupt the journal** | Two processes appending JSONL concurrently can interleave a partial line. A record is several hundred bytes; `PIPE_BUF` is 4096 and is about pipes, and nothing promises an atomic `write(2)` of that size to a regular file. `rivet get log`'s **`unreadable`** counter exists to count lines that will not parse; a second appender would have manufactured them. |

Both are right, and the second is the sharper one: the harm is not that the
record is in the wrong place, it is that somebody *else's* record becomes
unreadable.

**Ruled: `rivet-sysd` holds the journal open and offers an ingest socket.** One
writer is preserved. Anything with a record to file sends it.

> **The narrow version of this was already there and nobody had named it.** §4
> has the supervisor ingesting records composed by other processes — that is what
> a forwarded `cmd/rivet-timed` record *is* (§9.2). What it lacked was a way in
> for a process it had not spawned. The socket is the general form of a mechanism
> the design already had, which is why it needs no new vocabulary and no new
> field kinds in the record.

---

### 11.2 A second socket, and not an extension of `/run/rivet/notify`

The supervisor already listens on unix sockets, so this is not a new capability.
`ADR-017` §3.4's `sd_notify` readiness sockets live at
`/run/rivet/notify/<service>`. **They are the wrong place for this**, for three
independent reasons, any one of which would settle it:

1. **`/run/rivet/notify` is a *directory*, one socket per service**, created
   when the supervisor starts that service and named after it. A process the
   supervisor never started has no entry there and no name to have one under —
   which is precisely the population this channel exists for.
2. **`sd_notify` ignores what it does not recognise**, deliberately: the
   convention is extensible and a service sending `WATCHDOG=1` to a supervisor
   with no watchdog is not an error. A journal record sent as a `KEY=VALUE`
   would therefore be **silently dropped** by any build that did not know it,
   which is the one failure a logging system must not have.
3. **Datagram against stream.** The notify socket is `SOCK_DGRAM` because a
   readiness announcement is small, fire-and-forget and fits in one packet. A
   record is up to 16 KiB and must not be truncated, and `SO_PEERCRED` is
   defined for *connected* sockets — see below.

So: **`/run/rivet/journal`, a `SOCK_STREAM` socket beside the notify
directory.** The protocol is the file format, unchanged — line-delimited JSON,
one record per line, and the close of the connection terminates a line exactly as
a newline does. A sender connects, writes what `Record::to_jsonl` produced, and
closes. Nothing is sent back.

#### Why a stream, when a datagram would need no framing

`SOCK_DGRAM` with `SO_PASSCRED` / `SCM_CREDENTIALS` — which is what
`systemd-journald` uses — is the other credible shape, and it is genuinely
simpler: a datagram is atomic, so there is no partial write, no connection that
never sends, and no in-flight state at all.

It was not taken, and the reason is the credential read. On the *server* side of
an unconnected datagram socket `SO_PEERCRED` is not available — `unix(7)` defines
it for connected stream sockets and for `socketpair(2)` — so credentials would
have to arrive as ancillary data on every `recvmsg`, with `SO_PASSCRED` set and
the kernel filling them in when the sender omits them. That is equally
kernel-verified, and it is more moving parts in the one process that must not
fail: an ancillary-data path, a `MSG_TRUNC` path and a control-message buffer,
against one `getsockopt` at `accept(2)`.

**The trade, stated so it can be revisited:** the stream costs bounded connection
state (§11.5) and buys a two-line credential read. If the connection bookkeeping
ever turns out to be the expensive half, the datagram form is the thing to move
to, and neither the record nor the framing would change.

---

### 11.3 Credentials are the kernel's, and they authorise nothing

The supervisor calls `getsockopt(SO_PEERCRED)` on each accepted connection and
puts the result on the record as `ingest_uid`, `ingest_gid` and `ingest_pid`. The
kernel fills those in at `connect(2)` from the connecting process's own
credentials. **A sender cannot write them and cannot choose them.**

This is stronger than what the system already had, and it is worth being exact
about *how*, because the two are easily conflated.

| | `rivet_core::journal::Peer` | `rivet_core::deploy::Principal` |
| :-- | :-- | :-- |
| Built 2026-08-24 for | §11 | `ADR-028` §4.5 |
| Answers | *which process on this machine sent this* | *which caller out there asked for it* |
| Supplied by | the kernel, at `connect(2)` | the sender, out of `RIVET_CONTROL_PLANE_PEER` in its own environment |
| Forgeable by the sender | no | **yes** — it is an environment variable |
| Survives a machine boundary | no; a uid is local | yes; a pinned key is the same key `ADR-025` §2's map names |
| Distinguishes two people | **eventually** — `ADR-008` §7.3 rules full Unix multi-user | today |

**Neither authorises anything, and the distinction is the one `ADR-028` §4.5
draws.** A kernel-verified credential says *who sent this*; it does not make what
was sent true, and it is not a permission check — the ingest socket refuses
nobody on the strength of a uid. What it produces is a record whose attribution
is *checkable*: a claim in it can be contradicted by the kernel's own numbers
sitting beside it.

Both are recorded on an edit, because they answer different questions. An edit's
record carries `principal` (the caller, claimed) **and** `ingest_uid` /
`ingest_pid` (the process, observed). A reader who has both can say more than one
who has either.

**Nothing here assumes one account.** `ADR-008` §7.3 rules full Unix multi-user,
so the actor is written as `uid:<n>` from the first record rather than as a word
that would have to be widened later. Today `rivet` runs as root for every
operator and `uid:0` distinguishes nobody — which is exactly why the *claimed*
principal is kept beside it rather than being replaced by the stronger-looking
number.

---

### 11.4 What the sender keeps, what it cannot claim, and the one trade

The journal composes an ingested record out of three sources, and the split is
`rivet_core::journal::ingest`'s — one function, so there is no second account
of it.

| Comes from | What |
| :-- | :-- |
| **The sender** | `sev`, `msg`, its typed fields, and the `svc` it claims |
| **The journal** | `boot`, `mono`, `ts`, and a freshly minted `corr` |
| **The kernel** | `actor` (`uid:<n>`), and `ingest_uid` / `ingest_gid` / `ingest_pid` |

The sender's own `corr`, `mono` and `boot` survive as `emitted_corr`,
`emitted_mono` and `emitted_boot` — §9.2's treatment of a forwarded record,
unchanged, and what keeps a sender's internal causal chain recoverable.

**The property that follows is the security claim in one sentence: no ingested
record can be made to read as one the supervisor produced.** The supervisor's own
records carry `actor: supervisor`; an ingested one carries `uid:<n>`, whatever it
put in its own `actor` column, and no value of `n` yields the other string.

#### The trade: `svc` is a claim here, and it is not on a pipe

§9.2 says of a captured record: *"`svc` is overridden with the service the
supervisor actually started … a service cannot therefore write a record claiming
to be a different service."* **That does not hold on this socket, and the
difference is structural rather than an oversight.** The supervisor can override
`svc` for a pipe because it started the process at the other end and knows its
name; the whole point of this socket is processes it did not start, so there is
no name to substitute.

Two answers were available and the second was taken:

* **Refuse a record whose `svc` names something the supervisor did not start.**
  Rejected: it would refuse `rivet` and `femto`, which are the senders, and it
  turns a naming question into lost records.
* **Keep the claim and record the kernel's answer beside it.** Taken. `svc` is
  checked against `rivet_core::service::ServiceName`'s rule — so it cannot be a
  path, a control character, or four kilobytes of attacker-chosen bytes in the
  column a reader sorts by — and anything failing that becomes `ingest`, with the
  claim kept as `svc_claimed`.

So a hostile local process **can** write a record saying `svc: rivet-netd`. What it
cannot do is write one whose `ingest_pid` is that service's, or whose `actor` is the
supervisor's, or whose `boot` is a previous one. The lie is visible in the same
record that tells it. A field named in `INGEST_FIELD_NAMES` is dropped from a
sender's own fields and the count reported as `ingest_overrode`, so the kernel's
numbers cannot be shadowed either.

##### Amended 2026-09-04: one name is excluded, and it is the supervisor's own

**The audit of 2026-09-03 found that the trade above extended to `rivet-sysd`
itself, and it should not have.** The paragraph is otherwise unchanged: every
other name is still kept, for the reason given.

The claim above — *"the lie is visible in the same record that tells it"* —
rests on the `actor` column, and it is true. What it does not account for is
that **`svc` is the column a reader filters by.** §9.2 names
`rivet get log | where svc == rivet-netd` as the query the column exists for, and
the same query against `rivet-sysd` returned a local process's chosen text
mixed into the supervisor's own account of the boot. A reader who then checked
`actor` on every row would find it; the point is that they should not have to,
and an operator reading a machine at 2am will not.

So a claim of `rivet_core::journal::SUPERVISOR_SVC` is treated exactly as a
claim of a malformed name: the record's `svc` becomes `ingest` and the claim is
kept as `svc_claimed`. **Nothing is dropped**, which is what keeps this on the
right side of the option that was rejected above. The constant is `rivet-core`'s
and `cmd/rivet-sysd`'s `SELF_NAME` is defined from it, so the exclusion and the
name it excludes cannot drift apart.

#### The mode is `0666`, and here is exactly what that costs

The socket is world-writable. The reasoning:

* The ruling says *anything that has a record to file sends it*. A socket only
  uid 0 can reach does not do that, and `ADR-015` §5.1 gives every application
  its own uid — an application with something to say is the next sender.
* `ADR-008` §7.3 rules full Unix multi-user. A mode that works for uid 0 only is
  a design that assumes one account.

**What an unprivileged local process can do after this change**, stated plainly
rather than left to be discovered:

1. **Add records to the journal**, attributed to its own uid and pid.
2. **Cause older records to be evicted**, by sending enough of them that §8's
   rotation drops the oldest file. That is a denial-of-evidence attack and it is
   the real cost of the mode. **Bounded 2026-09-04 — see §11.4.1**, having been
   unbounded until then: the attack is real and is now slow, because a uid over
   its share loses records rather than evicting everybody else's.

**What it cannot do:**

* forge `boot`, `mono`, `ts`, `corr`, `actor`, or any `ingest_*` field;
* produce a record indistinguishable from one the supervisor wrote;
* **read** anything — the supervisor sends nothing back, and `/state/log` is
  `0700` with files at `0600` (`rivet_core::state`, and the test named
  `the_journal_is_not_readable_by_a_future_unprivileged_account`). So a
  process that can *file* a record still cannot read one, which is the asymmetry
  the mode was chosen for;
* make the supervisor allocate without bound — see §11.5;
* make the supervisor *execute* anything. Unlike `/state/deploy-request`
  (`rivet_core::deploy`), a record names no file and starts no service.

Narrowing the mode later is one `chmod` and a ruling. Widening it after
applications have come to depend on it is not, which is why it is wide now and
argued, rather than narrow now and quietly changed.

---

### 11.4.1 The per-uid rate limit — built 2026-09-04

* **Status:** Accepted — `PLAN.md` §11.2's option **(c)**, which was recommended
  on 2026-08-24 *"when the first application ships"*. Applications have shipped,
  and the audit of 2026-09-03 raised the same finding independently.
* **Consequence:** §11.4's second bullet is struck. The **mode is unchanged** —
  the socket is still `0666` and §11.1's ruling is untouched.

---

**It is a rate limit and it is not authentication, and the difference is the
whole of what may be claimed for it.** §11.3 is already explicit that a
credential on this socket *authorises nothing*: it says which process sent a
record and nothing about whether the record is true. That has not changed. What
is new is a bound on **how much** of §8's rolling storage any one uid may turn
over.

| | |
| :-- | :-- |
| Shape | a token bucket per uid, in `rivet_core::journal::IngestRate` |
| Burst | `INGEST_BURST` records, so an ordinary sender is never touched — `cmd/rivet`'s `apply edit` files **one** record per operation |
| Sustained | `INGEST_PER_SECOND` records per second, **derived from §8's cap and not measured**: at a few hundred bytes a record it is in the region of an hour to turn a ~1 MiB journal over, which is long enough for an operator to catch a machine doing it |
| Over the rate | the record is **dropped**. The connection is not closed, the sender is not refused at `connect(2)`, and nothing is treated as hostile |
| Said | one `warn` record per uid per `INGEST_REPORT_EVERY_MS`, carrying the count since the last one, the uid, the gid and the pid |
| Root | **exempt.** uid 0 can append to `/state/log/journal.jsonl` directly, so a limit on it stops nothing and costs the operator's own records. `PLAN.md` §11.2 makes the same argument against option (b) |

**What it does not stop, stated plainly rather than left to be discovered:**

* A local process can still file records saying anything it likes, attributed to
  its own uid. That is §11.1's ruling and not a defect.
* A process that stays **under** the rate can still fill the journal, given
  time. The limit makes that slow enough to notice; it does not make it
  impossible.
* Several uids cooperating get several buckets. The limit is per uid because the
  uid is the only thing about a sender the kernel vouches for.
* Nothing bounds root at all.
* The bucket table is capped at `INGEST_TRACKED_UIDS` and evicts the least
  recently seen entry, so a caller controlling more uids than that can reset a
  bucket. That takes root, and root is already exempt.
* **The running total can lag.** The first drop in a window is reported at once,
  so an operator learns *that* a uid is over its share; the rest of that window
  is counted and carried on the next note, and a flood that stops inside a
  window leaves its tail counted and never written. Nothing here ticks on its
  own — the limit is only consulted when a record arrives.

**This is option (c) and not option (d).** §8 still means what it said: rotation
drops the oldest file, not the greediest uid's records. A per-uid *share of the
storage* would be the stronger answer and it is an amendment to §8, which is why
it was not taken here.

**What is proved, and by what method:** the bucket's arithmetic, the per-uid
split, the root exemption, the bounded table, a clock that appears to move
backwards, and the supervisor filing exactly one note per window — all by unit
tests in `rivet-core` and `cmd/rivet-sysd`, each watched failing with the limit
removed. **Nothing has been run on a board, and §11.7's sentence still stands:
nothing has ever sent this socket more than a handful of records.** The rate is
therefore a bound derived from the storage it protects and not one anything has
been observed to need — if it turns out to be wrong it will be wrong in the
direction of throttling a legitimate sender, and the `warn` record is what makes
that visible rather than silent.

---

### 11.5 Hostile input, one decision each

Every one of these is a test rather than an intention.

| The input | What happens |
| :-- | :-- |
| **An enormous record** | A line reaching `INGEST_MAX_BYTES` (16 KiB) without a newline closes the connection and the bytes are **dropped**. A prefix of a record is not a record, and `AGENTS.md` §5 refuses rather than guessing at the rest. The supervisor journals a `warn` naming the peer and the byte count. A sender refuses it locally first, so its own record reaches its stderr rather than being lost to a refusal at the far end. |
| **A partial write** | Held. A sender is under no obligation to write its line in one `write(2)`, exactly as a service is under none on a pipe; the partial line is buffered per connection and delivered when its newline or EOF arrives. |
| **A sender that connects and never sends** | Costs one descriptor and `INGEST_DEADLINE_MS` (2 s), then the connection is closed. Complete lines already delivered are kept; an unterminated tail is refused with a `warn`. **The deadline is folded into the supervisor's `poll(2)` timeout**, because a deadline is not a descriptor and a supervisor with nothing else pending would otherwise wait for ever. |
| **A connection that sends nothing at all** | **No record.** That is a probe, not a record that failed to arrive, and manufacturing one per connection would hand anyone who can `connect(2)` a free write into §8's capped storage. |
| **Many senders at once** | `INGEST_MAX_IN_FLIGHT` (16) are accepted; the rest stay in the kernel's listen queue, and when that fills a further `connect(2)` is refused **at the sender**. Back-pressure, not a drop: the sender then takes its own floor. **The listener leaves the `poll(2)` set while every slot is full** — a listening socket with a connection waiting is permanently readable, so polling it while refusing to accept would spin PID 1's only child at 100% for as long as the queue stayed full. |
| **A record claiming a `svc` that is not the sender** | Kept as a claim, bounded by the service-name rule, and contradicted by `ingest_pid` beside it. §11.4 argues it. **Except the supervisor's own name**, excluded 2026-09-04 — §11.4's amendment. |
| **A uid filing records faster than its share** | The record is **dropped** and the drop is journalled, once per uid per reporting window with the count. Not a disconnection and not a refusal at `connect(2)`. §11.4.1. |
| **A line that is not a record at all** | Becomes a `warn` record carrying the first kilobyte of the bytes, cut on a character boundary, with the parse error as a field. Never dropped — *"there is nothing about that in the log"* said falsely is the answer this system must never give. |

#### The floor is stderr

**A machine whose supervisor is not listening is already in trouble, and the
record of that is exactly what must survive.** So a sender that cannot reach the
socket writes **the record itself** — not a summary — to its own stderr, and says
so.

* `cmd/rivet` writes it immediately. stderr and not stdout, because stdout is
  the operator's structured answer (`ADR-006` §4) and a record written into it
  would corrupt the value an agent is parsing. The operation reports
  `journalled: false` and an `unrecorded` sentence naming the socket.
* `cmd/femto` **holds it until the terminal is restored.** It is a full-screen
  editor in raw mode on an alternate screen; a line written to stderr mid-session
  paints over the buffer and is then discarded with the screen. So the record is
  kept, the status line says `NOT journalled` at the moment of the write, and the
  held records are printed after the alternate screen is left.

Neither refuses the edit. A file that was written and not recorded is a smaller
harm than a file that was not written because the journal was unreachable, and
saying *"nothing has been written"* about a file that has been replaced is the
worse of the two lies.

---

### 11.6 This is not a message bus, and the standing position on buses

Rivet has rejected D-Bus twice — `ADR-004` §3.2 over `iwd`, and again for
Bluetooth. It would be easy to read this socket as the same decision arriving
through the back door, so what it is is worth stating precisely:

| A message bus has | This has |
| :-- | :-- |
| a broker process | none — the socket is the supervisor's own listener |
| many message types, discovered at run time | **one**: an `ADR-022` §3.2 record |
| request and response, and therefore method call | **one direction**. Nothing is sent back and no sender waits |
| introspection | none. There is nothing to enumerate |
| bus activation | none. The supervisor is already running or it is not |
| addressing, well-known names, signals, subscriptions | none |

One socket, one message type, one direction. **Adding a second message type to it
is the moment this stops being true**, and is the moment to reopen the question
rather than to extend the socket.

**The maintainer's position on buses, recorded as his position so that it is not
mistaken for this project's doctrine: he is not opposed to a bus on principle,
he will not commit to keeping buses out, and he does not want one incorporated
until other options have been explored.** *(`.rulings/` R-043.)*

That is the standing position, and it is how `ADR-004` §3.2 should be read: as a
trade taken, not as a rejection in principle. **A rejection recorded as principle
calcifies; one recorded as a trade can be revisited when the trade changes.** The
trade today is that Rivet's inter-process needs are few, small and
one-directional, and each has been cheaper to meet directly than through a broker
— the install request is a file, readiness is `sd_notify`, and a record is this
socket. The day that stops being true, the thing to weigh is a bus, not a fourth
bespoke channel.

---

### 11.7 What is proved, and what is not

**Proved on the build host**, by tests that drive real syscalls rather than
fakes:

* the socket binds, at the mode this section names, read back off the
  filesystem;
* a record written by a real client arrives whole, and `SO_PEERCRED` reports
  **this process's own** uid, gid and pid — compared against `getuid(2)`,
  `getgid(2)` and `getpid(2)` rather than against a constant;
* what `cmd/rivet` puts on the wire is what `rivet_core::journal::ingest` —
  the supervisor's own function, not a second copy — reads back;
* the buffering rule, over a `socketpair(2)`: a record split across two writes, a
  line with no trailing newline at EOF, an over-long line, a connection that
  stops mid-line, and a connection that says nothing;
* that no ingested record can carry the supervisor's actor. Driven to failure by
  breaking the override and watching four tests fail before the fix was
  committed.

**Proved in the emulator** — §11.8.

**Not proved by any of it: a Raspberry Pi.** `ADR-026` §4 is the table of what
only a board settles, and two rows of it apply here — `/state` being a real
filesystem that survives a reboot, and §8's rotation, which no emulator run gets
near. **Also unproven: any behaviour under load.** Nothing has sent this socket
more than a handful of records, so §11.4's eviction concern is reasoned rather
than measured, and `INGEST_MAX_IN_FLIGHT` is a bound nothing has reached.

**That last sentence is still true after §11.4.1**, and it is the reason
§11.4.1's rate is described there as derived rather than measured. The limit's
own arithmetic is covered by unit tests; **no process has ever driven this socket
hard enough to reach it**, on a board or anywhere else.

### 11.8.1 PROVED ON A BOARD, 2026-09-06 — both halves, on one record

**`rivet-01`, over the control plane.** `rivet apply edit /state/ingest-probe.txt
--append … confirm` reported `"journalled":true`, and `rivet get log latest all`
read the record back carrying **both** provenance fields at once:

```
"path":"/state/ingest-probe.txt","change":"append","created":true,
"before":0,"after":5,
"principal":"pinned-5108de6435c6a6e501f8905629c908f13ff8a348193d55172f4572b966131107",
"ingest_uid":0,"ingest_gid":0,"ingest_pid":4366
```

**This is §11.3's argument standing up on hardware.** The `principal` is what the
CLI read out of its own environment — self-reported, `ADR-028` §4.5 — and the
`ingest_*` fields are what the kernel said through `SO_PEERCRED` about the
process on the other end of the socket. **No version of `cmd/rivet` can write
`ingest_pid`**, so its presence is proof the record travelled the socket rather
than being appended to the file.

**And it closes `ADR-028` §4.5.5's open half.** That section recorded that
whether `cmd/rivet-mcpd` actually puts `RIVET_CONTROL_PLANE_PEER` into the
environment of the CLI it spawns had **never been observed on a machine**. The
`pinned-` prefix above is that observation: the whole chain — a peer's key, the
daemon's environment, the CLI's read, the socket, the supervisor's stamp — is
one record on a real board. 3,622 records in that journal carry the `ingest_*`
fields.

### 11.8 Proved in the emulator once — and NOT a standing assertion any more

A remote `rivet apply edit … --append … confirm` against a QEMU guest reported
`"journalled":true`, and `rivet get log` read the record back, carrying both
halves at once: the caller the CLI read out of its own environment, and the
`ingest_uid` / `ingest_gid` / `ingest_pid` the kernel supplied for the process
on the other end of the ingest socket.

**Read those two together, because they are §11.3's whole argument in one
record.** The principal is self-reported. The `ingest_*` fields are what the
kernel said. Neither is redundant: the first says *which caller out there
asked*, the second says *which process on this machine sent it*, and only the
second is unforgeable.

**What changed, and why the emulator is no longer where this is proved.**
That demonstration rode on a harness mode that no longer exists.
`tools/qemu/smoke.sh` now boots a pair over the control plane — and **`-M virt`
has no `/dev/mmcblk0p5`**, so pid 1 reports `DEGRADED /state not mounted`, §8's
files never exist, and there is nowhere for an ingested record to land. The
supervisor still accepts one, because the socket is on `/run`; `get log` reads
files, so nothing can be read back.

**So the harness asserts what it can and says what it cannot**, rather than
quietly passing on a guest with no journal — which is the mistake this section
made once already. It checks `get storage` for `/state`, runs the full check when
there is one, and prints the four lines naming what was *not* asserted when there
is not. **§11.8.1 is where the assertion actually lives now**, on a board.

**`ingest_pid=127` is the load-bearing detail.** No version of `cmd/rivet` can
write that field — the supervisor drops any sender's attempt at it and
substitutes the kernel's answer — so its presence is proof the record travelled
the socket rather than being appended to the file. The harness asserts on it, and
on `"journalled":true` **before** reading the journal, because a record that was
never sent and a record that was sent and not written look identical from
`get log` and are entirely different defects.

Three tripwires come with it, and each fails loudly rather than quietly:

* the edit exiting non-zero;
* `"journalled":false`, which means the record took §11.5's stderr floor;
* a journal with no `ingest_uid` in it, or one naming no record for the file
  that was just written.

#### What this run does not settle

* **`cmd/femto`'s half of §11 has never run on a machine.** `tools/qemu/run.sh`
  packs PID 1, the supervisor, the CLI, `rivet-netd`, `rivet-timed` and
  optionally `nu`; it packs no editor. Everything in `cmd/femto/src/journal.rs`
  is covered by unit tests over a scripted terminal and by nothing else.
  `PLAN.md` §11.4 carries the fix and the assertion it should make.
* **`/state` here is a directory on the initramfs**, so the record has never
  outlived the guest that filed it, and §8's rotation is as untouched as §9.1
  said it was.
* **Nothing has been near the caps.** One record per run; `INGEST_MAX_IN_FLIGHT`
  is exercised only by a unit test that opens twenty sockets on the build host.

---

## 12. What the supervisor must record about its own boot — RULED 2026-08-26

**Ruled by the maintainer 2026-08-26**, on a recommendation, out of the gap
`PLAN.md` §25.2 found and §26.4.2 priced.

### 12.1 The gap, stated as it was measured

Two boots of a Rivet board on 2026-08-26 produced **no `rivet-initd` record at
all.** The journal for a whole boot was 165 `kernel`, 34 from a single service
and 1 `rivet-timed`. Nothing said the machine had started, which slot it took, that
`/state` had been grown, that a trial boot had been taken, or that it had been
committed.

**The cost was paid the same morning.** `ADR-018` §5's commit — the event that
decides whether a machine can still roll back — had to be established
**behaviourally**, by restarting the board a second time and reading a device
number out of `/proc/self/mountinfo` (`PLAN.md` §26.4). A supervisor that
narrates nothing about its own startup leaves `ADR-032`'s query socket with less
to report than the kernel ring buffer has.

### 12.2 The ruling: seven events, and no more

The supervisor records these and does not grow the list one convenient value at
a time — §10 item 2's warning about enumerations applies here too:

| | Event | Must carry |
| :-- | :-- | :-- |
| 1 | **boot started** | boot id, the slot booted, the version running |
| 2 | **`/state` grow** | attempted or skipped, the outcome, the resulting size |
| 3 | **trial boot taken** | that this boot is a trial, and of which slot |
| 4 | **trial committed** — or **not**, with the reason | the slot, and what decided it |
| 5 | **service started** | one per service |
| 6 | **service failed** | one per failure, with the reason |
| 7 | **shutdown initiated** | the reason, and whether a reboot or a power-off |

**Seven lines a boot plus one per service**, which is comfortably inside §8's
megabyte and is not a retention question.

### 12.3 Why these seven and not fewer, and not everything

* **1–4 are the A/B mechanism made legible.** They are the ones whose absence
  cost a second reboot, and they are what an operator means by *"did that update
  take?"*.
* **5 and 6 are what `ADR-017` §6's convergence already decides** and does not
  say out loud. A service that never started is currently indistinguishable from
  one that started and did nothing.
* **7 closes the pair with 1.** A journal where every boot has a start and no
  end cannot distinguish a clean restart from a power cut, and `ADR-018` §5's
  rollback path is entered by exactly one of those.

**A full service lifecycle was considered and rejected** for §10 item 2's
reason: it is the enumeration that grows until it means nothing, and §8's
rotation starts mattering. **Reporting this through `ADR-032`'s query socket
instead of logging it was also considered and rejected** — a socket answers
*what is true now* and cannot answer *what happened three boots ago*, which is
the question that was actually asked on 2026-08-26.

### 12.4 This is a record, not authority

Same standing caveat as §11.4: these are the supervisor's own statements about
itself. They are attribution rather than proof, and the authority for which slot
is running remains the kernel's mount table — `PLAN.md` §26.4.1 records why a
binary digest is not a slot identity and fails silently in the direction that
matters.

### 12.5 Amended 2026-08-26: items 3 and 4 are not the supervisor's to record

`PLAN.md` §28.5 Phase 1 item 6, §33. Five of the seven are now written; items 3
and 4 turned out to be in a different process.

**The section's title says *the supervisor* and its table includes the trial boot
and its commit. `cmd/rivet-initd` performs the commit** — `confirm.rs`, `ADR-018`
§5.3.2 — **before the supervisor exists, and PID 1 writes to the console and
nothing else.** Its own `seed` module states the constraint from the other side:
the journal has nowhere to go until `/state` is mounted, which is PID 1's job.

**So §12.1's finding has a better explanation than it assumed.** *"Two boots
produced no `rivet-initd` record at all"* is not a supervisor that narrates
nothing — it is a set of events that happen in a process with no journal to
narrate into. Deriving them afterwards answers *what is true now* rather than
*what pid 1 did*, and §12.4 already draws exactly that line.

`PLAN.md` §33.3 puts three shapes to the maintainer and recommends PID 1 handing
its outcomes to the supervisor, which already receives a provisioning document
and a service set from it.

### 12.6 Item 7 was unwritable, not merely unwritten

Recorded because it is the sharper form of §12.3's argument. `cmd/rivet-sysd`'s
`Event::Terminate` **collapsed `SIGTERM` and `SIGINT` into one variant**, so
*"whether a reboot or a power-off"* had nowhere to come from: the supervisor
could not have said which if it had tried.

It carries the signal now, and the record says both the outcome and **what this
process was told** — `pid 1 sent SIGTERM` — rather than what it inferred
(`AGENTS.md` §5, and §12.4's *this is a record, not authority*). `SIGUSR1` is
deliberately not among the possibilities: §3.1.10 gives it to pid 1, and pid 1
stops this process with `SIGTERM` regardless, so from here a trial restart and a
power-off arrive identically.

### 12.7 Item 1's slot, and the mount it had to avoid

The firmware's answer to *which slot is running* needs a FAT mount of the boot
partition. `rivet_core::operation::stop` already refused to put one on the
shutdown path — *"a machine that cannot be stopped because its card would not
mount is a worse failure than the surprise it prevents"* — and that applies with
more force to the supervisor's startup, which is the one path a board cannot go
around.

`Slot::from_mounts` reads `/proc/self/mounts` and takes the trailing digits of
whatever `/system` is mounted from: `AGENTS.md` §5's *derive, never store*, and
the kernel's own account of which partition is providing the files this process
is executing. `PLAN.md` §26.4.1 records why the alternative — hashing the running
binary — fails silently in exactly the direction that matters.

### 12.8 RULED 2026-08-26: pid 1 hands its outcomes to the supervisor

§12.5 put three shapes to the maintainer for items 3 and 4 — the trial boot and
its commit, which happen in a process with no journal. **Option (a) is taken:
pid 1 hands its outcomes to the supervisor, which records them.**

It is the honest one of the three. The record then says *pid 1 reported*, which
is what actually happened; the supervisor deriving the same facts afterwards
(option (c)) would answer *what is true now* while looking like *what pid 1
did*, and §12.4's distinction between a record and an authority is exactly that.
It needs no journal in pid 1 and no buffering, because pid 1 already hands the
supervisor a provisioning document and a service set.

**Not built.** `PLAN.md` §34.4 records item 4 happening on a board that same day
and going unrecorded, which is the cost until it is.

### 12.9 Item 7's "whether a reboot or a power-off" cannot be answered here

**Measured on a board, 2026-08-26.** `rivet apply restart confirm` — which sends
pid 1 `SIGINT`, §3.1.10's *shut down and come back* — produced this supervisor
record:

```
shutdown requested; stopping services
  shutdown="power-off"  reason="pid 1 sent SIGTERM (ADR-017 §3.1.10)"
```

**That is correct and it is the whole problem.** Pid 1 stops the supervisor with
`SIGTERM` whatever kind of restart it is itself performing — §3.1.5 item 6 makes
the sequence pid 1's, and the supervisor is one step in it. So from this
process's seat a power-off, an ordinary restart and a trial restart are the same
event, and `Terminating::Restart` can only occur if something signals the
supervisor directly, which nothing on a Rivet machine does.

**§12.6 was right that the field was unwritable and wrong about what writing it
would buy.** The supervisor now reports *what it was told*, honestly and
uselessly: the value is `power-off` on every shutdown a board will ever have.

**This is items 3 and 4's problem wearing a different hat**, and it strengthens
§12.8's ruling rather than complicating it: *which kind of shutdown* is pid 1's
knowledge, exactly as *this boot is a trial* and *the trial was committed* are.
It should come through the same hand-off, and until it does the honest reading
of this field is **"the supervisor was stopped"** and nothing more.

**What is NOT being done, and why:** the supervisor could infer the kind from
the `[tryboot]` and `[all]` records on the card. That is `AGENTS.md` §5's
*derive, never store* pointed at the wrong question — it would answer *what will
happen next* while the field claims *what was asked for*, and §12.4 is the line
that separates them.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §6.1 — whether Ignition uses `tracing` was unverified

**Checked 2026-08-22.** The superseded heading word was ~~Unverified~~, and §10
item 1 carried the same question as ~~Whether Ignition uses `tracing`~~. It
does, and it does not help as much as hoped.

### In §9.1 — the mount plan took `/dev` as given

**Changed at `15f1acb`; date not recorded in the file.** The superseded sentence
read:

~~`rivet_core::mount::stage0_plan` mounts `/dev/pts` and `/dev/shm` *inside*
`/dev` and takes `/dev` as given, because on a board the kernel really has
mounted it.~~

The plan names `/dev` now and converges when the kernel got there first
(`ADR-017` §3.1.12).

### In §11 — a field was attributed to a section that does not exist

**Noticed 2026-08-24** while §11 was being written, and corrected rather than by
inventing the section. The superseded citation was ~~§4.2~~.

### In §11.4 — nothing bounded journal eviction

**Bounded 2026-09-04 — §11.4.1.** The superseded item read:

~~**Cause older records to be evicted**, by sending enough of them that §8's
rotation drops the oldest file. That is a denial-of-evidence attack and it is
the real cost of the mode. Nothing bounds it today; `PLAN.md` carries it as a
question for the maintainer, with a recommendation, rather than a rate limit
invented in a commit.~~

The attack is real and is now slow: a uid over its share loses records rather
than evicting everybody else's.
