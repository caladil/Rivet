# ADR-036: The Rivet SDK

* **Status:** Accepted — the first article was ruled 2026-09-01; this is the
  document §104.45.2 said must exist.
* **Date:** 2026-09-01
* **Maintainer rulings, 2026-09-01** *(`.rulings/` R-285)*:
  * This is the point at which the first vestiges of the Rivet SDK are officially
    declared.
  * Ignition is not the fit for Rivet OS, and that is to be admitted.
  * Either Ignition is declared not for Rivet, or a great deal of work is needed
    to make it work on Rivet, and it might make more sense to put all of that
    logic into the SDK instead.
  * Rivet OS has two primary application usage cases: network services and kiosk
    applications.
  * The SDK is to be going before more applications are ported, since porting
    applications exercises the SDK.
  * The `examples/` should be constructed with some of the more popular
    crates.io crates, sqlite in particular, to demonstrate that an application
    can use crates.
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

Rivet has been handing applications things for a while and never called it
anything. The maintainer's 2026-09-01 ruling tabulated six of them (§3 below) and named the pattern:

> **An application declares what it needs and is HANDED it. It does not go and
> take one.**

That is the first article, and it was ruled. **What does not exist is a
document.** The contract lives as a `PLAN.md` section and as behaviour in
`cmd/rivet-sysd`, which is precisely the state `docs/` exists to end. An author
writing the second Rivet application has nothing to read.

This ADR is that document's decision half. It rules **what an application may
assume**; a SPEC will follow with **what an author must do**.

### 1.1 Why now, and not earlier

Because until 2026-09-01 there was one application author and he wrote the
platform too. `aura-controller` and `aura-renderer` ran on a Rivet, and
`examples/probes/kiosk-demo` was written *following Aura* because Aura was the only
working example. A contract inferred from one program is not a contract.

Three things changed in the same week: a second application kind became real
(`rivet-launcher` hosts them), the display rule was ruled rather than
implied, and the maintainer named the next test — **another cold-agent run
against Rivet**. A cold agent reads `docs/`. If the SDK is not written down, the
cold agent measures how well it can guess.

---

## 2. The decision

> **1. There is a Rivet SDK, it is a crate, and it is named `rivet-sdk`.**
>
> **2. It encodes Rivet's *promises* — paths, environment variables, socket
> protocols — and none of Rivet's implementation.** An application is a foreign
> program. It does not link `rivet-core`.
>
> **3. Ignition is not a Rivet dependency.** Its useful third moves into
> `rivet-sdk`, in Rivet's shape.
>
> **4. `examples/` is the SDK's proof and its template set**, and is
> reorganised into probes and templates so a newcomer knows which to copy.
>
> **5. Two application kinds are first-class: network services and kiosk
> applications.** Each gets a template, and each gets one working example.
>
> **6. The SDK comes before the next port.** A port exercises the SDK; it is
> not a substitute for having one.
>
> **7. Every subsystem is reached by declaring it in the manifest** — and a
> declaration is two-sided: plumbing granted, obligations owed. §8A.

### 2.1 Why 6 is a rule and not a preference

**Because it has already gone the other way once, and the record shows what it
cost.** `examples/probes/kiosk-demo` was written *following Aura*, plainly,
because Aura was the only working example. So Aura's
improvisations became the template, and the platform's expectation had to be
*declared later and applied backwards*: §2 above rules every kiosk application a
lessee and, in the same breath, obliges both Aura and `kiosk-demo` to change.

That is the general failure. **An application ported before the contract exists
does not consume the contract — it invents one**, and the next application
copies it. Do that three more times and Rivet's SDK is the union of four
programs' accidents, discovered by whoever writes the fifth.

Ported *after*, the same application is a test: every place the author reaches
past the SDK is a gap in the SDK, and every place the SDK is awkward is a defect
found before it is load-bearing. **The port is the instrument.** Using it as a
workaround for the instrument's absence destroys the measurement.

This is why TempMonitor's Rivet port stays blocked-and-parked rather than
becoming the next thing attempted, and why Aura's raylib patch waits on §5.4
rather than being done ad hoc and normalised afterwards.

### 2.2 This is `ADR-013`'s platform crate, and four of its rulings bind here

**`ADR-013` decided this problem before this ADR existed.** It ruled that a
Rivet-shaped abstraction needs a home that is neither Ignition nor Rivet, gave
that home a placement test, and described the crate as *the library that
software built for Rivet links against in order to be a well-behaved citizen of
a Rivet host*. **That crate is `rivetos-sdk`**, and this ADR is where it was
built. The name `ADR-013` had ruled for it is retired; `ADR-013` §6 is the
reconciliation and its `## History` says why the naming argument decided
nothing.

Four of that ADR's rulings apply to what is decided here, and none of them is
restated in this document's own words because each is checkable where it is:

* **The placement test** (`ADR-013` §2.1) is what answers *does this belong in
  the SDK* — and its fourth row is what answers *and does it instead belong in a
  library of the maintainer's preferences*. A capability that only his own
  programs would want, and only on Rivet, is not the SDK's however many of them
  want it.
* **The scope constraint** (`ADR-013` §6.1): what the SDK carries are shapes
  this project chose, not a general contract for supervised applications. §8.3's
  agreement test is what keeps that honest.
* **The line between what requires the SDK and what does not** (`ADR-013` §6.4):
  socket passing, readiness, structured logs, configuration and secret delivery
  reach a program in any language, because `ADR-017` §3.4 adopted conventions
  rather than inventing them. **Exposing an application's own operations to
  `rivet` is the one part that a library must carry** — and it is the part of
  the contract this ADR has not built. §5.5 and §8.2 rule the other direction:
  what an application may read *of* the machine.
* **The tripwire** (`ADR-013` §7 item 1): a crate that must reach back into
  Rivet's system logic to do ordinary program things is a crate the split has
  failed for. §8.3's `[dev-dependencies]`-only edge is this ADR's compliance
  with it.

---

## 3. The contract: what a Rivet hands an application

Every row is live in this build unless marked. This table is normative; where it
disagrees with a comment in `cmd/rivet-sysd`, this wins and the comment is a
defect.

| What arrives | How | Since |
| :-- | :-- | :-- |
| a listening socket | descriptor from 3 upward, `LISTEN_FDS`, `LISTEN_PID`, `LISTEN_FDNAMES` | `ADR-015` §5.2 |
| another application's socket | a path in `RIVET_CONNECT`, granted by `chown` | `ADR-015` §5.3, `cmd/rivet-sysd::endpoints` |
| its own installed files | `RIVET_APP_DIR` | §104.29.2 |
| somewhere to write | `RIVET_APP_DATA` — `/state/app/<name>/data`, `0700`, its own uid | §104.30, `ADR-015` §3.2A |
| its shared libraries | `LD_LIBRARY_PATH`, composed from the declared closure and nothing else | §104.26, `SPEC-02` |
| the journal | stdout and stderr are captured; **and** `/run/rivet/journal` accepts a record from anyone | `ADR-022` §4, §11 |
| **its display** | **a DRM lease descriptor, on the socket named by `RIVET_DISPLAY`** | §2 above, §106.2 — ~~ruled, unbuilt~~ **built**; see §3.0 |

### 3.0 What the display row means now — corrected 2026-09-03

The row above said **ruled, unbuilt** long after it was built, and the audit of
2026-09-03 found it (`PLAN.md` §123). What exists:

* `RIVET_DISPLAY` names a **socket**, not a card. `rivet_sdk::display::Channel`
  reads it; descriptors arrive on it, the first at start and **another whenever
  the launcher re-issues**. `cmd/rivet-sysd` makes the pair and hands the child
  its end (`PLAN.md` §106.2).
* `rivet_sdk::screen::handed()` returns **both** — a `Screen` to draw on and the
  `Channel` the next one will arrive on. It returned only the `Screen` until
  2026-09-03, dropping the channel, which made §8.1's obligation impossible to
  honour: an application worked until the first revocation and then never drew
  again. `Channel::try_next` is how a frame loop asks without blocking, and
  `examples/templates/template-kiosk` is the worked example.
* Dropping a leased `Screen` frees its descriptor and both dumb buffers. It used
  to free neither, so every hand-over cost 16 MB of CMA at 1080p.

**Launcher-side re-issue is still unbuilt** (`PLAN.md` §106.3). The application
side of the contract is complete and has nothing to receive from yet.

### 3.1 The rule that follows, stated before somebody meets it

**An application that opens `/dev/dri/card0` by path cannot hold this screen.**
It will get `EACCES`, and there is no compatibility path. This closes no door
`ADR-015` §4.3 left open — the maintainer is the only publisher — but it is the
kind of thing that costs an afternoon when it is discovered rather than read.

The general form is stronger and worth stating as the article's contrapositive:
**a program that goes and takes a resource is doing the thing this platform
arranges not to need.** Binding its own port is the same error as opening the
card node; both work on Linux and neither works here.

#### 3.1.1 That sentence was not true when it was written — amended 2026-09-04

**A leased application is granted the render node, and only the render node.**
Ruled by the maintainer 2026-09-04 (`.rulings/` R-138), after the audit of
2026-09-03 confirmed the finding it answers (register `R-32`,
`docs/PLAN-Remediation-2026-09-03.md` §5 item 2).

**The paragraph above described an intention, and the code did the opposite.**
`ADR-015` §6.2's `display` class expanded to `/dev/fb0`, `/dev/dri/card*` **and**
`/dev/dri/renderD*`, and the supervisor chowned every one of them to the uid of
any application that declared the class. So an application handed a lease had
two ways around it:

* it could **open `/dev/dri/card0` by path**, because it owned the node — and
  the kernel makes the first opener of a node with no current master into master
  by itself, so the moment the launcher's own master lapsed, the lessee had it;
* it could **write `/dev/fb0`**, which is a different device onto the same panel
  and goes around the lease rather than through it.

Neither gets `EACCES`. *"No compatibility path"* was true only of the modeset
ioctls, and only while the launcher was alive to hold master — which is the
narrowest possible reading of a sentence written to be a rule.

**What is true now.** The class still means one thing to the author who writes
it; what it expands to depends on which side of the arbitration the declaring
service is on (`rivet_core::service::DisplayRole`):

| Role | Granted | Why |
| :-- | :-- | :-- |
| **Arbiter** — holds DRM master for the life of the machine and leases the display out; `rivet-launcher` today | `/dev/fb0`, `/dev/dri/card*`, `/dev/dri/renderD*` | unchanged. It cannot arbitrate a device it cannot open, and narrowing it would leave the machine with nobody able to drive the screen |
| **Lessee** — every kiosk application | `/dev/dri/renderD*` | the lease is the screen. A render node cannot modeset and cannot disturb whatever owns the display, which is exactly the reach a program that draws through a lease needs |

**The role is named by the machine document, never granted by a manifest.**
There is no manifest key that grants it and there must not be: an application
that could make itself the arbiter could take the screen, which is the whole of
what `ADR-015` §6.2 arbitrates. A manifest can only ask — `connects =
["rivet-sysd:kiosk"]` asks to be the launcher, the same declaration the kiosk
control socket is given by — and the document's `[kiosk] launcher` names the
one service whose asking is honoured, the image's `rivet-launcher` when it names
none (`.rulings/` R-210, D3). So **the program given the socket is the program
given the card**, and a machine cannot end up with one without the other. Any
other service declaring the endpoint is refused the socket by name, is a
lessee, and takes nothing from the launcher. Where a compositor is enabled it is
the arbiter instead, named the same way (`ADR-058` §4.2).

**The gate is the build, not this paragraph.** `AGENTS.md` §3 ranks a build that
refuses above a rule a reader has to remember, and `rivet-core` carries a `const`
assertion that the lessee's expansion holds render nodes and nothing else. Adding
a card node back to it does not compile, on any target.

**What it cost, stated plainly: nothing that was working.** Every application in
this repository that draws already requires a lease — `examples/aura/aura-renderer`,
`examples/aura/aura-probe` and `examples/templates/template-kiosk` all take one
through `rivet_sdk::screen::handed()` and open no device node, and `kiosk-demo`
had already stopped opening the card under a supervisor. The one casualty is
`deploy/aura/renderer`, the C++ Raylib port, whose DRM backend opens `card0` by
path; it is superseded by the Rust renderer §6.2B ruled, is not what the board
runs, and is recorded as broken in its own manifest rather than quietly left.

~~**What is still only asserted.** The tests prove the *chowns* — which nodes the
supervisor hands to which uid — and a chown is not a refusal. That an
application trying the old route now receives `EACCES` is a claim about a
running kernel, and settling it takes a board: install a build that opens
`/dev/dri/card0` by path with no lease, select it from the launcher's menu, and
read the journal for the errno it reports.~~ The emulator cannot stand in for it:
`ADR-026` §4 is the table of what it can and cannot settle, and a DRM device
node under devtmpfs is not on the side it can.

#### 3.1.2 The kernel's answer, read off a board — 2026-09-04

**Settled.** `examples/probes/kiosk-demo` was given a `report_dri_nodes` pass
that opens every node under `/dev/dri` by path and prints what happened, then
deployed to the Pi 4 and **selected from the launcher's menu**, so that it was a
real lessee holding a real lease rather than an application with no display.
Its own journal records, unedited:

```text
kiosk-demo: GRANT CHECK — 3 node(s) under /dev/dri, opened by path as uid 35692, HOLDING a display
kiosk-demo: GRANT CHECK /dev/dri/card0 mode 0600 uid 34030 gid 34030 — REFUSED, errno 13 (Permission denied (os error 13))
kiosk-demo: GRANT CHECK /dev/dri/card1 mode 0600 uid 34030 gid 34030 — REFUSED, errno 13 (Permission denied (os error 13))
kiosk-demo: GRANT CHECK /dev/dri/renderD128 mode 0600 uid 35692 gid 35692 — OPENED read-write
```

**Both halves matter and only one of them was the question.** `EACCES` on the
card node is what §3.1 asserts and had never been observed. The render node
**opening** is what says the grant arrived at all — a machine that granted
nothing would produce the same refusal on `card0` and prove nothing, which is
the reading this probe was written to make impossible.

**And it corrects a claim this repository was repeating.** Several comments said
the card node *"stays `root:root`"*. It does not: it is `0600` and owned by
**34030, which is `rivet-launcher`** — the arbiter, exactly as §3.1.1's table
above says it should be. `root:root` was a shorthand for *"not yours"* that
happened to be false, and it was in four places in `kiosk-demo` alone. Corrected
there in the same change. **The conclusion is unaffected**: an application that
is not the arbiter cannot open the card node either way.

**The probe is kept rather than deleted.** The grant is recomputed by the
supervisor at every start, so a transcript in this document cannot notice when
it changes and a check that runs on every launch can.

### 3.2 The manifest is the request

`ADR-015` §3.3: the manifest **is** the permission request. The surface an
author writes:

```toml
[package.metadata.rivet]
kind      = "daemon"        # daemon | oneshot | socket-activated
listen    = ["tcp:8080"]    # Rivet opens these and passes the descriptors
network   = "egress"        # none | group:<name> | egress
state     = "private"       # /state/app/<name>/data, 0700
provides  = ["socket:api"]
connects  = ["photo-index:api"]
memory    = "128M"
restart   = "on-failure"
ready     = "notify"        # none | notify | listening
library   = []              # the declared closure
title     = "Aura"          # what the launcher shows on a card
```

**Two of these are enforced and the rest are reported as unenforced on every
build and start** (`SPEC-01` §5.3). The SPEC must say which, per release, rather
than implying a sandbox that is not there.

---

## 4. Ignition is not the fit

The maintainer put it as a fork: say Ignition is not for Rivet, or do a lot of
work to make it fit. **The fork is real and the answer is the first branch**,
and the reason is not that Ignition is poor. It is that *Ignition's premises are
the ones Rivet deliberately removed.*

Ignition assumes a home directory, an environment it can layer, a platform
config directory, a writable filesystem, an operator at a CLI, and an
application responsible for its own updates. `SPEC-01` §5.2 is the list of
things Rivet does not have, and it denies **every one of those on purpose**.

### 4.1 Pillar by pillar, against what Rivet actually is

| Ignition provides | On a Rivet | Cost to make it fit |
| :-- | :-- | :-- |
| **Config** via `dirs` → platform config dir | `$HOME` is `/` and read-only; there is no XDG. `path_override` is the seam, and a probe built against Ignition and run on a board proved it works | **Already solved** — but what remains is `serde` + `toml` behind a wrapper |
| **Env layering** (`APP_` prefixes) | **Dead.** `SPEC-01` §5.2: there is no `env` key in a manifest, and the only variables an application gets are the ones the supervisor sets | Cannot be fixed. There is nobody to set them |
| **Logging** to a rotating file | **Actively wrong.** Rivet's answer is the journal. A file log writes into the app's one writable directory, is invisible to `rivet get log`, and competes with the record a person actually reads | A journal backend — a real change, and a change to *Ignition*, not to Rivet |
| **Self-update** from a configured source | **Architecturally contrary.** Rivet owns updates: A/B images (`ADR-018`, `ADR-029`) and signed `.rvt` packages. An application updating itself bypasses the signature path, writes to a read-only `/system`, and has no shell to exec | Cannot be fixed. It must be off |
| **Standard CLI surface** | Inert. A daemon is spawned by the supervisor with fixed arguments; there is no operator to type at it | No cost, and no value |

**Three of five are dead or wrong, one is inert, and the survivor is a thin
wrapper over two crates.** That is not a port; it is a rewrite that would keep
the name.

### 4.2 What that costs, honestly

**This section rests on a measurement, not a preference.** Ignition was built
for `aarch64-unknown-linux-musl` and run under a Rivet's supervisor as a probe.
It answered *"does Ignition run on a Rivet at all"* with *yes, and here is what
porting turned out to mean*: every path passed in through `path_override`, and
self-update kept unreachable by hand because nothing can compile it out.

**An application on a Rivet consumes `rivet-sdk`, and nothing in `examples/`
shows otherwise.** The probe was retired once its answer was written here,
because an example is read as a recommendation and this one demonstrated the
pattern this section rules out.

The maintainer's other projects — Athena, Olympus, Tachyon, Trends — keep
Ignition. **This ADR is about Rivet as a host, not about Ignition as a
library.** Nothing here asks Ignition to change.

**Nor is Ignition barred from this repository — the boundary is the machine.**
`cmd/rivet-fleet` and `cmd/rivet-provision` run on a workstation, have an
operator, a home directory and a writable filesystem, and take Ignition for
exactly what `ADR-013` §2 places there. What §4.1 rules out is Ignition inside
a program that a Rivet supervises.

**The earlier half of that measurement is `ADR-013` §7 item 3**, which put
Ignition against a boot process and found two conditions attached — every path
passed explicitly, and `ignition::update` unreachable, which cannot be compiled
out because Ignition declares no cargo features. `cmd/rivet-netd` and `cmd/rivet`
both cite it as the reason Ignition is absent from their manifests. This section
is the same finding one level up: the conditions are not a boot binary's special
case, they are what a Rivet is.

---

## 5. What `rivet-sdk` provides

The design constraint that shapes all of it: **an application is a foreign
program.** If it links `rivet-core` it drags the operating system into a
program that wanted a temperature reading. The principle is that an application
does not link `rivet-core`, and a Rivet's promise to it is a path; the SDK's
whole job is to make that promise checkable without importing the machine.

So: ~~`rivet-sdk` depends on **no Rivet crate**~~, and as little else as
`ADR-001`'s tier policy permits. It is constants, small readers, and thin
protocol clients.

**Amended 2026-09-04 by `ADR-040` §6, for the optional-feature case only.** The
`serve` feature depends on `rivetos-tls`, and the rule is struck rather than
reinterpreted because a reader should see that it moved. What holds is the
sentence above it, which is the reason the rule existed: an application is a
foreign program and must not have the operating system dragged into it.
`rivet-tls` is `rustls` plus pinned-Ed25519 verification, depends on no other
crate in this workspace, contains nothing of the operating system, and is
reached only by an application that asked to serve. **With no feature named,
`rivet-sdk` still links `libc` and nothing of Rivet's** — the default path is
unchanged. The alternative was a fourth independent `CryptoProvider`
construction inside the SDK, which `ADR-011` is precisely about; `rivet-proxyd`
had already written the third.

### 5.1 The journal — the maintainer's item (1)

Two paths exist and the SDK should expose both, because they answer different
questions:

* **stdout and stderr**, captured by the supervisor for a process it spawned.
  The zero-dependency floor, and the one that works before anything is
  initialised.
* **`/run/rivet/journal`**, the ingest socket. `0666` by deliberate design
  (`ADR-022` §11.4), with `SO_PEERCRED`'s kernel-verified uid, gid and pid
  stamped by the supervisor. One record, written and closed; no reply, no retry;
  16 KiB max; **stderr is the floor when it is not there.**

The second is what lets an application emit a *structured, severity-tagged*
record rather than a line of text — and, critically, it is the answer for any
process the supervisor did not spawn.

**A `tracing` subscriber over the ingest socket is the obvious shape**, and it
is exactly the thing Ignition's file logger should have been here. This is the
clearest instance of the maintainer's direction that such logic belongs in the
SDK (`.rulings/` R-285).

### 5.2 Configuration

Not Ignition's model. Rivet's promises are paths, so:

* read from `RIVET_APP_DIR` (shipped defaults) and `RIVET_APP_DATA` (what the
  application owns and may rewrite);
* **no environment layer**, because there is no environment to layer;
* fail loudly on a malformed file rather than falling back to defaults — a
  service silently running on defaults writes its data somewhere the maintainer
  is not looking, which is the defect TempMon — a separate repository of the maintainer's, not a crate in this tree — records from the other side
  of the family.

### 5.3 The handed environment

Typed accessors for §3's table, so an author never parses `LISTEN_FDNAMES` by
hand or hard-codes `/state/app/<name>/data`. Every one of these is a place a
program can be wrong in a way that compiles.

### 5.4 The kiosk surface — the maintainer's item (2)

What an application needs in order to be a good citizen of `rivet-launcher`:

* **accept a leased DRM descriptor** rather than opening the node — §2 above,
  and the thing `examples/probes/kiosk-demo` must do before Aura can;
* **declare `title`** so a card shows a name rather than a service id
  (§104.42's schema half is built; the reading half is not);
* the narrow kiosk socket vocabulary — `select`, `return`, `restart` — from the
  application's side.

### 5.4A The display module is EXTRACTED from the launcher, not written beside it

Ruled 2026-09-01, and it is the maintainer's argument rather than a preference:
if the SDK library is written from the bones of `rivet-launcher` itself, that
also helps keep the OS and the SDK in sync, because moving one means moving the
other. He put it as a question. *(`.rulings/` R-286.)*

**Yes, and it is a stronger guarantee than §8.3's test.** That test *detects*
drift between two implementations. Shared code makes drift *impossible*, because
there is only one. `rivet-launcher` becomes the SDK's first consumer — and it is
the one that runs on hardware at every boot, so a change that breaks the display
surface cannot reach a release without breaking the machine's own screen first.

> **THIS HAS NOT HAPPENED, and saying so is the point of this block.**
> Recorded 2026-09-03, after the audit counted what is actually in the tree
> (`PLAN.md` §123): the launcher's `qoi.rs` is byte-identical to the SDK's, its
> `text.rs` is the SDK's minus the glyph cache, its `drm.rs` is `screen.rs`
> minus double buffering, and `examples/probes/kiosk-demo/drm.rs` is a **third**
> copy of the modeset. The launcher uses `rivet_sdk` for exactly one listen
> socket.
>
> So *"drift impossible"* describes the ruling and not the code, and a reader who
> takes this section as a statement of fact will be wrong about four files. The
> extraction is `PLAN.md` §123's Wave 5.6 and is **the one item in that plan
> that is a decision rather than a defect** — it is the largest single change in
> it and it touches the program that arbitrates the screen, so it waits on the
> maintainer rather than being drifted into.

**The pattern is already established one door over.** `cmd/rivet-launcher`'s
manifest says of the kiosk protocol: *"so both ends of the socket are parsed from
ONE module (`rivet_core::kiosk`) rather than from two that could drift."* This is
that reasoning applied to drawing.

#### The split is already clean, measured rather than hoped

`cmd/rivet-launcher/src/drm.rs` separates **how the display was obtained** from
**what is drawn on it**:

```rust
pub struct Screen { .. }
pub fn take(path: &str) -> Result<Screen, String>   // opens the node, becomes master
impl Screen {
    pub fn blit(&mut self, canvas: &[u8]);
    pub fn release(self);
}
```

Acquisition is one function. The surface takes a canvas the caller rendered. So
a lessee needs **`Screen::from_fd(fd)` beside `take(path)`** — a second
constructor, and the blit path is identical either way.

Roughly by file: `drm.rs`, `text.rs`, `input.rs` and `qoi.rs` are the reusable
half; `ui.rs`, `offered.rs`, `farewell.rs` and `kiosk.rs` are the launcher's own
program. Only `drm.rs`'s shape has been read closely; the other three are judged
by role and must be checked before they move.

#### What the launcher does NOT share, and it matters

**The launcher is the master; every application is a lessee** (§2 above). So the
*acquisition* halves genuinely differ and must not be pretended into one. What is
shared is the drawing, the text, the input and the encoding — which is most of
the code and none of the privilege.

#### Two costs, both real

1. **`unsafe` enters the SDK.** `drm.rs` is FFI — *"a modeset is a sequence of
   ioctls and there is no safe wrapper in this tree."* `rivet-sdk` currently
   carries `#![forbid(unsafe_code)]`. That becomes a per-module allowance, and
   the forbid stays everywhere else. An application linking the SDK should know
   this changed.
2. **A network service must not pay for a framebuffer.** The display half goes
   behind a `display` feature, off by default, so a `tokio`/`axum` service links
   no DRM ioctls and no `fontdue`.

**This is not built and is not this sequence's step.** It follows the lease
(§9 step 4), because a lessee constructor with no lease to construct from is an
API written against a guess.

### 5.5 Introspection — the maintainer's item (3)

The ask: an application that wants to see what *should* be visible of the OS —
configuration, networking, the journal, process status — **without opening the
sandbox.**

The control plane already answers all of it. `rivet get` reads eighteen
subjects: `schema`, `desired-state`, `memory`, `process`, `thermal`, `boot`,
`network`, `device`, `cpu`, `trust`, `storage`, `log`, `install`, `slot`,
`file`, `diff`, `catalogue`. But that is a **host→board** surface over TLS, and
an application on the board asking about its own machine is a different
question with three possible answers:

| | Verdict |
| :-- | :-- |
| the application shells out to `rivet get` | **Impossible.** No shell (`ADR-005` §5.4), and `/system` binaries are not in its closure |
| the application links `rivet-adapter-system` | **Rejected.** That is Rivet's internals in a foreign program — the thing §5 exists to prevent |
| **a local introspection socket, declared and handed** | **Taken in principle** — it is the first article applied to the machine itself: an application *declares* it wants to see, and is *handed* a socket |

The third is the Rivet-shaped answer and it respects the constraint exactly:
what the socket answers is a **policy decision about a bounded subject list**,
not a capability leak. An application never gains a permission; it gains a
correspondent that answers questions.

**Ruled in full the same day — see §8.2.** Three grants, not one:
`system-info` for machine facts, `log = "own"` for this application's own
journal records, `log = "system"` for all of them. `file` and `diff` are
excluded permanently; `desired-state` and `trust` are deferred.

---

## 6. `examples/` is the SDK's proof

Ten directories exist and every one of them was written to settle a question.
None was written to be copied. That is the gap the maintainer named.

### 6.1 Probes and templates are different things

* **A probe** answers *can this work on a Rivet at all* — `sqlite-demo`,
  `mesa-demo`, `socket-demo`, `libgreet`/`greeter`, `hello-rivet`. They are
  evidence, and they stay while what they demonstrate is something an author
  may do; a probe whose answer was *don't* is retired once that answer is
  written into the record (§4.2).
* **A template** is what an author copies to start. There are none.

The directory must say which is which, or a newcomer copies a probe and inherits
a question rather than an answer.

**Made structural 2026-09-01.** This was first implemented as an
`examples/README.md` — which is *documentation*, and the maintainer pointed out
that the classification should be in the layout: some way of making clear that a
probe is not an example of how to write code, with `examples/templates/` and
`examples/probes/` as the shape he suggested. *(`.rulings/` R-287.)*

So it is `examples/templates/` and `examples/probes/`, and a reader who never
opens the README still cannot mistake one for the other. **A README can be
skipped; a path cannot.**

**Package names were deliberately NOT changed** in the move. `[package] name`
becomes the service name, the derived uid and the data directory, so renaming
`template-network-service` would orphan what is installed on a board. Directory
moves are free; package renames are not, and the directory already carries the
meaning.

**Aura left `examples/` entirely**, to `deploy/aura/`. Its two directories were
packaging scaffolding for the maintainer's private application rather than
examples of anything — and §6.2B had already ruled that Aura is not an example.
A **9.2 MB compiled binary** went with them: a build artifact of that private
application, tracked in git, larger than every source file in this repository
combined.

### 6.2 What must exist

| | |
| :-- | :-- |
| **template: network service** | manifest, `listen`, descriptors from `LISTEN_FDS`, journal, `RIVET_APP_DATA`, graceful stop |
| **template: kiosk application** | manifest with `title`, a leased display descriptor, the launcher's input vocabulary, the narrow socket |
| **a working network service** | deliberately basic — the maintainer's word. It must *work*, not demonstrate |
| **a working kiosk application** | the Hello World. `examples/probes/kiosk-demo` is the candidate once it accepts a passed descriptor |

**The kiosk template cannot be finished before the lease is built** (§104.45.3).
That is a real ordering constraint and not an excuse: the network-service
template has no such dependency and goes first.

### 6.2B The example set, ruled 2026-09-01 — a screensaver pair, and no C++

The maintainer's plan, which supersedes §6.2's *(kiosk template)* row: write and
test a Go and a Rust version of the screensaver controller, and write a new Rust
screensaver renderer. There is no C++ example: the tooling exists and people can
build what they like, but the project focuses on modern languages such as Rust
(or Go). The Go controller and the Rust renderer ship as pre-installed demos on
the default image. *(`.rulings/` R-288.)*

| | |
| :-- | :-- |
| **screensaver controller, Go** | state and an endpoint. Ships **pre-installed** |
| **screensaver controller, Rust** | the same endpoint, the other language |
| **screensaver renderer, Rust** | the kiosk half — a lessee. Ships **pre-installed** |
| ~~kiosk example, C++~~ | **not written** |

#### Why two controllers and one renderer

**Because `provides` is an interface, and two implementations are what prove
it.** The renderer declares `connects` once; either controller can satisfy it,
in a different language, with no change to the consumer. That is §105.5's
language-neutrality claim turned into a working demonstration instead of a
sentence in `PLAN.md`.

#### Why no C++, stated so it does not read as a slight

The C toolchain is exposed (`tools/toolchain/env.sh`), `libgreet`/`greeter`
prove dynamic C, and `mesa-demo` proves the GPU. **Nothing is being withdrawn —
an author can build C or C++ for a Rivet today and the probes that say so
remain.** What changes is what this project *maintains and holds up as the
pattern*, and that is Rust and Go.

Aura therefore stays the maintainer's own application rather than becoming an
example. §6.1's earlier reasoning — that lifting it would teach two forbidden
patterns and carry personal data through a scrub — is unchanged and now moot.

#### Pre-installed is a second decision, and it reopens §8.4

§8.4 ruled `rivet-launcher` **installed, not enabled**, because the majority
case is headless. **A screensaver renderer on a headless machine is the same
case**: it would be started, find no display, and fail. So the default image
gains a second package and the same question with it.

The shape that follows from §8.4's reasoning: **installed by default, enabled by
the same one-line desired-state change that enables the launcher.** Recorded as
following rather than ruled — the maintainer said *pre-installed*, and whether
that means enabled is his to say.

#### Aura merges the Rust renderer, which retires §8.1's largest risk

The maintainer, 2026-09-01: Aura in its official form will take in the Rust
renderer from `examples/` in due course — and, on keeping his own application,
he can still install and run his own Aura against his own health and environment
readings. *(`.rulings/` R-288.)*

**§8.1 recorded a risk that this removes.** That section takes the strong form of
the re-issuance obligation and says plainly that *rebuilding a GL context mid-run
may be genuinely beyond raylib, and Aura is the first application this applies
to.* The answer is no longer *patch raylib to accept a leased descriptor* — it is
that Aura adopts a renderer written as a **lessee from its first line**. The
awkward case stops being a case.

It also settles the division cleanly:

* **`examples/` carries the pattern** — maintained, Rust and Go, no private data.
* **Aura stays the maintainer's own application** — private, with real health and
  environment data, and never an example. `§6.1`'s scrub problem does not arise
  because nothing is lifted.

**A platform whose maintainer runs a private application on it is better evidence
than any example**, and that is worth keeping rather than collapsing.

#### The GPU question this opens, and it is NOT answered here

`rivet-launcher` and `examples/probes/kiosk-demo` are **CPU** renderers — DRM dumb
buffers, no GPU (`§5.4A`). Aura's current renderer is **GLES on V3D**. So *"Aura
merges the Rust renderer"* means one of two different things and this ADR does
not choose between them:

1. **Aura moves to CPU rendering.** Simplest, and re-issuance stays nearly free.
   Whether the ice-blue rain holds 1080p on a Pi 4's CPU is **unmeasured** — and
   guessing at it is exactly the kind of claim this project does not make.
2. **The Rust renderer grows a GPU path** — EGL + GBM + GLES from Rust, which
   nothing in this tree has done and which `§5.4A` deliberately left to tier two.

**The screensaver renderer should be built CPU-first regardless**, because that
is where the lease contract is cheapest to prove (`§9` step 4). What Aura needs
after that is a measurement, not a decision taken now.

#### It is gated, and the gate is the lease

The renderer is a kiosk application, so §8.1 obliges it to accept a re-issued
descriptor, and it cannot be written before §9 step 4. **The two
controllers are not gated** and can be built and proven now — which is why the
Go one exists already.

### 6.2A The first template found an SDK gap on its first page, as predicted

§2.1 argued that a port ported *after* the contract exists is an instrument:
*"every place the author reaches past the SDK is a gap in the SDK."* That was a
prediction. Writing `examples/templates/template-network-service` tested it within a
dozen lines.

**Turning a handed descriptor into a listener needs `FromRawFd`, which is
`unsafe`.** Without the SDK owning it, every Rivet network service would open
with the same `unsafe` block — each author making the same safety argument
privately, none of them able to check it.

And the argument is not theirs to make. *Is this descriptor really an open,
listening socket?* is answerable only by the platform that opened it, called
`listen` on it and passed it as fd `3 + n`. An application asserting that is
asserting something it has no way to know.

So `Socket::into_tcp_listener` and `into_unix_listener` moved into the SDK,
consuming the `Socket` so a descriptor cannot be owned twice. **The crate's
`#![forbid(unsafe_code)]` became `#![deny(...)]`** with one documented allowance,
which is a real change to what an application links and is recorded here rather
than left in a commit message.

**The template contains no `unsafe`.** That is the measurement: the gap existed,
it was closed in one place, and the thing it was blocking now reads like ordinary
Rust.

### 6.3 The examples use ordinary crates.io crates, deliberately

Ruled 2026-09-01. **An example that uses only `std` proves the wrong thing.** It
demonstrates that Rivet can run a program nobody would write, and leaves the
question an author actually has — *can I use the crates I use everywhere else* —
answered by inference.

**More of that answer already exists than the examples show.** Every one of
these is running on the board today, and none of it was written to prove the
point:

| Crate | Where it already runs | What it settles |
| :-- | :-- | :-- |
| `rusqlite` + `bundled` | `examples/probes/sqlite-demo` | a `-sys` crate compiling **C from source** for `aarch64-musl`, WAL on `/state` — `PLAN.md` §103.3 |
| `rustls` + `ring` | `rivet-tls`, `cmd/rivet-mcpd` | pure-Rust TLS, and `ring`'s assembly, on the control plane itself |
| `tokio` (multi-thread) | `rivet-smbd`, `rivet-mdnsd`, `rivet-timed` | async runtimes work; `mdnsd` resolving `devpi.local` is the proof |
| `serde`, `clap` | throughout | unremarkable, which is the point |

So the network-service example should be **an ordinary Rust web service** —
`tokio`, `axum`, `serde`, `rusqlite` — because three of those four are already
proven here and the fourth is exactly what the example is for. Nothing about it
should look like it was written for an unusual operating system, except the
manifest and the fact that it is handed its socket instead of binding one.

**Each example declares the dependency class it proves**, so the set is a matrix
rather than a pile: pure Rust, vendored C, dynamic C against the platform's own
libraries (`greeter`/`libgreet`), the GPU (`mesa-demo`), async, TLS.

### 6.4 `SPEC-01` §5.2 needs a positive counterpart

Today the honest-facilities list says only what is **absent** — no shell, no
`$HOME`, no D-Bus, no zoneinfo, no `pkg-config` for the target. That is the half
that prevents a wasted afternoon, and it is not the half an author reads first.

The SDK SPEC must carry the other half: **what is known to work, and what it
cost.** The table in §6.3 is its seed. The rule for entry is this project's rule
everywhere else — a row goes in when something ran on the board, not when it
compiled.

---

## 7. Consequences

**A second contract to keep.** Every change to what the supervisor hands an
application is now a change to a published surface. That is the cost of having
one, and it is the point.

**`SPEC-01` §5.2 becomes SDK documentation.** Its honest-facilities list — no
shell, no environment, no `$HOME`, no zoneinfo, no D-Bus — is what an author
must read before choosing a dependency. The SDK SPEC should reference it rather
than restate it (`ADR-011`).

**Stale comments in `cmd/rivet-sysd` are now defects.** Several say the
`LISTEN_FDS` handoff is not built; it is — `supervise.rs` composes
`LISTEN_FDNAMES` and `kernel.rs` passes the descriptors. §3's table is the
authority and those comments contradict it.

**The cold-agent test gets a fair subject.** A cold agent reading `docs/` will
find the contract instead of inferring it from `aura-controller`.

---

## 8. Answered 2026-09-01

All four were put to the maintainer and ruled the same day. They are kept as
questions-with-answers rather than rewritten into the body, because *what was
uncertain* is part of what a reader needs.

### 8.1 The display is a channel, and that IS what declaring a display means

**Corrected 2026-09-01 — there is no `kind = "kiosk"`.** This section was
written naming one, and `ProgramKind` is `daemon | oneshot | socket-activated`.
The declaration that makes an application a kiosk one is
**`devices = ["display"]`** (`ADR-015` §6.2), which is what
`Supervisor::offered_displays` and `rivet-launcher` both actually read.

The ruling below is unchanged; only the key it named was wrong. It is corrected
where it stands rather than quietly rewritten, because *a document written the
same day is not therefore true* is the lesson worth keeping — this was the
newest staleness in the corpus within hours of being committed.

The maintainer's question was whether the channel is probably part of the
contract for being a kiosk application. *(`.rulings/` R-289.)*

**Ruled: re-issuable, and not as an opt-out.** A kiosk application must accept a
descriptor more than once. That unlocks the exit confirmation (§104.36.4),
display hot-plug, §104.40's runtime selection, and getting shutdown messages
back onto HDMI (§104.43, currently regressed).

The recommendation on the table was a manifest opt-out — `display = "once"` for
programs that cannot rebuild a GL context. **The maintainer's form is stronger
and it is the one taken**, because the 2026-09-01 ruling already ruled the harder half:
*"an application that cannot be patched cannot hold the screen here."* An
opt-out would have re-opened that on the SDK's first day.

**The risk is stated rather than hidden.** Rebuilding a GL context mid-run may
be genuinely beyond raylib, and Aura is the first application this applies to.
If it proves undeliverable, that is **data that returns to this ADR** — not a
reason to weaken the contract in advance.

### 8.2 Introspection is read-only, allow-listed, and split by grade

The maintainer asked for a read-only allow list, and then suggested that reading
one's own log, or the system log, might be something requested in the manifest
separately from system information. *(`.rulings/` R-290.)*

**Ruled, and the second request refines the first into three grants rather than
one.** Reading *your own* log is a far smaller thing than reading the journal,
which carries every other application's output.

| Declaration | What it grants |
| :-- | :-- |
| `system-info` | machine facts only: `boot`, `cpu`, `memory`, `thermal`, `storage`, `network`, `slot`, `install`, `catalogue`, `schema` |
| `log = "own"` | journal records where `svc` is this application, and no others |
| `log = "system"` | the whole journal — a materially larger grant, and it should read like one |

**`file` and `diff` are excluded permanently.** They read a caller-named path;
that is an arbitrary-read primitive and no declaration makes it safe.
**`desired-state` and `trust` are deferred** — the first can carry the wireless
PSK (`ADR-020`), the second is the trust store.

### 8.3 `rivet-sdk` is in this workspace, and a test holds it honest

**Ruled as recommended**: the SDK declares its own constants, links no Rivet
crate, and a test asserts agreement with `rivet-core`. That test can only exist
if both are in one workspace, which settles the location as a consequence rather
than a preference.

**The test is bigger than constants, and that was found while pinning the wire
format.** The ingest socket takes a **JSONL `Record`** — `journal::ingest` calls
`Record::from_jsonl`. So the SDK must *encode* a record `rivet-core` reads back
identically, and a drifted encoding is worse than a drifted constant: a wrong
path fails loudly, a wrong field name is silently demoted to a `warn` record
carrying the bytes.

So the agreement test asserts **round-trip**: a record the SDK writes parses
through `rivet_core::journal::Record::from_jsonl` to the same severity, service,
message and fields.

Mechanically, `rivet-sdk` names `rivet-core` in `[dev-dependencies]` and nowhere
else. Dev-dependencies are not linked into consumers, so an application that
depends on `rivet-sdk` never sees `rivet-core` — the edge exists only for the
test that proves the two agree.

### 8.4 The launcher ships installed, not enabled

The maintainer's reasoning was that turning the launcher on is one step, for the
people who want kiosk mode. *(`.rulings/` R-291.)*

**Ruled as recommended.** The majority case is headless — the long-term target
is replacing every home server — and enabling is a one-line desired-state
change, which is what `ADR-011` is for. A server deployment gets no surprise
process holding DRM master.

---

## 8A. The manifest is the declaration surface for every subsystem

*(Numbered `8A` because it arrived with §8's answers and belongs in reading
order beside them — the same device as `ADR-015` §3.2A.)*

The maintainer generalised the four answers into one rule: an application that
wants to be a kiosk application declares so in its manifest and is given the
operating system's plumbing to be one; an application that wants read-only access
to system state declares that in its manifest and is given the plumbing needed to
read non-secure data about the system. *(`.rulings/` R-292.)*

**This is §3's first article applied to subsystems rather than to resources**,
and it completes `ADR-015` §3.3's *"the manifest IS the permission request"*
with the half that section did not say: **a declaration is two-sided.** The
platform provides plumbing; the application takes on obligations in exchange.
`devices = ["display"]` is the worked example — it grants a leased display and
*owes* re-issuance handling (§8.1).

What follows from it:

* **A new subsystem is a new declaration, never an ambient capability.** Nothing
  is available because a program happens to run here.
* **The SDK's shape is derived, not invented.** For each declaration there is
  one module, and an application that did not declare it cannot call it in a way
  that works.
* **The refusal is legible.** An undeclared subsystem fails saying *you did not
  declare this*, which is a sentence naming its own fix.

**GPIO is the named future row and is deliberately not built.** The maintainer
called it not a primary usage case for Rivet, but an idea for the future
(`.rulings/` R-292). It is recorded here so that when it arrives it is understood as a row in an
existing table rather than as a new mechanism — which is the whole value of
writing the rule down before the second case.

---

### 8.5 RULED 2026-09-01: NO — and the failure it prevents is not the one it looks like

**Asked by the maintainer** on being shown that the fleet register had the
problem: whether others could benefit from a durable write in the Rivet SDK.
Then, having thought about it, he asked whether any example exists in which
people would expect a kiosk-mode application to survive a power cut and come
back with no state lost, and whether he was missing something. *(`.rulings/`
R-293.)*

**Ruled: the SDK does not gain a durable write.** What it gains instead is a
rule that costs nothing and fixes the failure that actually happens — §8.5.2.

#### 8.5.1 The question was about the wrong property

Losing no state at all is not what `write`-then-`rename` buys, and framing it that
way is what made the answer look obvious. A torn write does not cost you the
last few seconds; it leaves a file that is **zero bytes or truncated**. The
application reads its own state on the next boot, fails to parse it, and exits.
The supervisor restarts it. It fails again.

**On a locked kiosk that is `PLAN.md` §108's restart-loop detector firing and a
fault splash** — a machine that does not come back until somebody with a
control-plane key intervenes, which on a headless board in a hallway is the
expensive outcome by a wide margin.

So the expectation nobody holds is *"my kiosk lost no data"*. The expectation
everybody holds is **"my kiosk turned back on"**.

#### 8.5.2 THE RULE: an application treats its own unparseable state as ABSENT

Corrupt file, missing file, empty file — **one path, and it is the cold-start
path.** Never an exit, never a panic, never a refusal to start.

This is cheaper than durability and it is strictly better for the majority
case, because it also covers the failures a `sync` cannot: a half-finished
write by a previous version, a file truncated by a full `/state`, a schema
change nobody migrated.

It belongs to the application, not to the platform, which is why it is a rule
and not a function. It is stated on `rivet_sdk::app::data` — where an author is
standing at the moment they decide to keep something — and in
`examples/templates/template-kiosk`.

#### 8.5.3 The maintainer's ownership argument is the load-bearing one

His argument had two halves. The first asks whether a kiosk application owns
its state at all. The second says that even a kiosk-mode application built for an
end user and needing state, such as a point of sale, would be saving frequently
anyhow. *(`.rulings/` R-293.)*

**The first half is right and decides this.** A kiosk that renders somebody
else's data — departures, a dashboard, signage — does not own anything. It can
discard its file and re-fetch, and §8.5.2's rule is a complete answer for it.
That is most kiosks.

**The second half runs backwards, and it is worth writing down because the
intuition is a common one.** The exposure is the time spent *inside* a write.
An application saving every two seconds spends far more of its life in that
window than one saving hourly. Frequent saving reduces how much is lost and
*increases* how often a file is caught torn — which is the failure that matters.

#### 8.5.4 The cost was misjudged, and the ruling does not rest on it

An earlier draft of this section argued the SDK would need a new crate and a
`cfg(unix)` seam in its public surface. **Both were wrong.** `rivet-sdk` does
not build on Windows at all — it is an SDK for applications running on a Rivet,
which is Linux — and it already depends on `libc`. The sequence is about thirty
lines with no new dependency and no seam.

Recorded because it changes what this ruling means: it is **not** *"too
expensive"*. It is *"aimed at the wrong failure"*, and would have been declined
at any price.

The one real cost stands and is not the deciding factor: a second copy of a
sequence whose ORDER is the entire guarantee (`rivet_store::atomic`).

#### 8.5.5 What would reopen this, stated precisely enough to recognise

**The first application that holds state it cannot re-fetch from anywhere.**
The honest example is a point-of-sale queue: sales taken while the network is
down exist in no other place, and §8.5.2's rule would throw them away.

When one appears, **the answer is probably not the sequence argued about here.**
A queue wants an **append-only file with a discardable torn tail** — a
half-written record at the end is detectable and droppable, every complete
record before it survives, and correctness does not depend on `fsync` ordering
at all. Which is a different mechanism, reached by a different argument, and is
the reason this is not being built speculatively.

---

## 9. Sequence

1. **SPEC-0N — the SDK contract**, written against §3. Nothing blocks it.
2. **`rivet-sdk`**: the handed environment (§5.3) and the journal (§5.1).
   Nothing blocks these either, and they are what every application needs.
3. **The network-service template and its working example** (§6.2).
4. **Answer §8 item 1**, then build the lease (§104.45.3 items 1–3).
5. **`examples/probes/kiosk-demo` accepts a descriptor**, then the kiosk template.
6. **Aura**, which is the maintainer's tree — and it is a *port*, so by §2.1 it
   comes after 1–5 and its friction is data about the SDK.

Steps 1–3 are unblocked today. Step 4 is a decision, not work.

**Nothing new is ported until step 3 is done.** §2.1 is the argument; the
practical form is that the network-service template is the first thing that
gets to say whether `rivet-sdk` is any good, and it should say so before a real
application depends on the answer.

---

## History

The body above states what is true now and is complete without this section.

### §3.1.1 — the arbiter was whichever single application declared the kiosk

**Superseded 2026-10-03 by `.rulings/` R-210, D3.** The role was derived from the
declaration alone:

~~The supervisor reads the declaration it already has — `connects =
["rivet-sysd:kiosk"]` … Two applications claiming it means neither is the
arbiter, matching the existing refusal to give the socket to either of them.~~

It changed because that let any installed application take the launcher's
socket and card away by declaring the same endpoint. The machine document now
names the launcher, and the declaration is a request the document must back.

### §4.2 — the Ignition probe was kept as history

**Superseded 2026-10-10, by the maintainer, while the public tree was being
prepared.** The section read:

~~`examples/probes/ignition-demo` stays. It is not wasted and it is not deleted:
it is the measurement that produced this section, and the record shows its own
history.~~

It changed because a public reader takes an example as a recommendation, and
this one showed Ignition inside a program a Rivet supervises — the one place
§4.1 rules it out. Its findings moved into §4.2's body, and git keeps the code.
