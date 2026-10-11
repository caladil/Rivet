# Design candidate: `rivet-launcher`

* **Status:** Draft. NOT accepted. Five questions are answered with a
  recommendation each; §10 lists what is the maintainer's to rule
* **Date:** 2026-08-31
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Builds on:** `ADR-015` §5.1.1, §5.2.1, §5.3, §6.2; `ADR-017` §4.1, §6,
  §6.3.1, §6.3.2; `ADR-007` §7.2; `ADR-032` §3.2; `ADR-035`;
  `PLAN.md` §104.17.3, §104.18, §104.29.5; `ADR-015` §5.3

---

## 1. Context: the screen is dead on purpose, and this is what un-deads it

`PLAN.md` §104.29.5 records the ruling this document exists to satisfy. With no
`[kiosk]` table in the document, `auto_run` is `None`, `apply_auto_run` clears
`enabled` on every display-declaring application, and **nothing takes the
screen**. An offer to relax that to *"run the only one if there is exactly one"*
was refused:

> auto-launching is `rivet-launcher`'s feature; until the launcher exists no
> kiosk application can function, and that is correct rather than a gap to paper
> over. Trying to bypass that, he added, is a sign that the project is
> incorrect. (`.rulings/` R-321.)

So the dead screen is not a bug to be worked around; it is the absence of a
component. This document designs that component.

The model it has to implement is `§104.17.3`'s, ruled and not up for
re-argument:

Several kiosk applications can be installed and enabled at once, and only one of
them is the auto-launch. Enabled means being on the launcher's list of
applications, and the launcher can then have one auto-run. An application can be
installed and not enabled, in which case the launcher does not offer it.
*(`.rulings/` R-321.)*

**Three states**, and the middle one is new:

| state | document | manifest | what it means |
| :-- | :-- | :-- | :-- |
| installed, not enabled | silent or `enabled = false` | `devices = ["display"]` | invisible. The launcher does not offer it |
| installed and enabled | `enabled = true` | `devices = ["display"]` | **OFFERED** — on the launcher's list |
| offered and selected | plus `[kiosk] auto-run`, or a runtime selection | — | it holds the screen |

### 1.1 The hard problem, stated once

**The launcher cannot start an application. Only the supervisor can.**
`ADR-017` §6 gives the supervisor the whole of service lifecycle and there is
deliberately no manual start or stop anywhere else. `ADR-015` §6.2 gives the
supervisor display arbitration for the same reason: *"it is the only thing that
starts and stops services and therefore the only thing that can know who holds
what."*

Everything below is a consequence of that sentence. The launcher is a program
that **asks**, and every design question is about what the asking is allowed to
be.

---

## 2. Q1 — the launcher is an IMAGE SERVICE, not an installed application

**Recommendation: an image service.** A binary at `/system/bin/rivet-launcher`,
defined in `cmd/rivet-sysd/src/definitions.rs`, running as uid 0, shipped and
replaced by `ADR-018`'s A/B slot exactly as `rivet-splash`, `rivet-netd` and
`rivet-mcpd` are.

| | image service (**recommended**) | installed application |
| :-- | :-- | :-- |
| authority to command the supervisor | **already has it** — `/run/rivet/query` is `0600 root:root` and this runs as root | needs that socket opened to a named uid. §5 argues that is the whole objection |
| §6.2 display arbitration | **out of it entirely.** It declares no devices, contests nothing, and yields | contests `display` with every application it exists to launch; `devices::contested` would name it against each one |
| present on a freshly flashed card | **yes** | no. A fresh card has a dead screen until somebody installs one — the exact state §104.29.5 called acceptable only *until the launcher exists* |
| update path | `ADR-018` A/B: atomic, trial-booted, rolled back by pid 1 if the boot fails | `ADR-015` §7.1 install: atomic per artifact, and **nothing rolls it back** |
| a broken launcher | reverts with the slot, automatically, on a failed trial boot | leaves a board with no way to choose an application; the only repair is a control-plane install |
| blast radius of a defect | uid 0 | its own uid **plus** whatever it can cause the supervisor to start |
| who may ship one | the maintainer, via an image build | the maintainer, via the signing key (`ADR-015` §4.3) |

**The last row is what settles it.** The usual reason to prefer an application
is that a third party can ship one. On Rivet there is no third party: §4.3 makes
the maintainer the sole gatekeeper and `cmd/cargo-rivet` is *"a HOST tool and the
only one that holds a signing key"*. So "application" buys **independent
updating** and nothing else, and it pays for that with a new authority path
(§5), a self-contradiction in §6.2's arbitration, and a card that boots to
nothing.

**And the launcher is the successor to `rivet-splash`'s job, which is already
an image binary.** `ADR-015` §6.2 point 4 — *"the splash returns when the owner
exits"* — makes the idle screen a property the supervisor **converges**.
`cmd/rivet-sysd/src/splash.rs` says so in its own words: *"a screen left showing
a dead program's last frame is drift."* The launcher is that same converged idle
state with a menu on it. A component that is the system's answer to *"what is on
the screen when nothing else is"* belongs where the previous answer lives.

**What this costs, stated rather than discovered:**

* A launcher change needs a full image build and an A/B update. It cannot be
  iterated with `cargo rivet deploy`.
* A launcher defect is a root defect. §9 is the honest list of what that means.
* Rivet cannot have two launchers, or a per-machine launcher. If that is ever
  wanted, this decision is the one to revisit — and §5's socket is the thing that
  would have to change with it.

### 2.1 The rejected option, written down so it is not re-proposed

Making the launcher an application requires answering *"which application may
command the supervisor?"* The only available answers are a document key naming
it, or a manifest declaration. Both are the same object: **a grant of
root-equivalent authority over service state, held by one named artifact.** A
socket grantable to one uid is grantable to two, and the day it is granted twice
the display has two claimants that can each start the other's replacement. This
is refused now, before anything depends on it.

---

## 3. Q2 — it asks over the EXISTING control socket, with two new subjects

**Recommendation: `/run/rivet/query`.** It already exists, it is already the
supervisor's control surface, and it already carries a mutating verb.

`ADR-032` §3.2 built it as a query socket; `ADR-017` §6.3.1 made it *"the
supervisor's control surface"*; `apply service-restart` crossed it on
2026-08-29 as the first thing that changes anything.
`rivet_core::operation::service_restart` said the rest in advance:

> a request of `get status` is a Rivet command, and **the day a second one is
> ruled across it needs no new grammar.**

So there is no new grammar. Three subjects join the vocabulary:

```text
get kiosk                  -> what is offered, and what is selected   (§6)
apply kiosk-select <name>  -> make <name> the display's owner
apply kiosk-return         -> give the display back to the launcher
```

with `plan kiosk-select <name>` for the same reason `plan service-restart`
exists — so a caller gets the supervisor's own answer rather than a guess at it.

Wire format: tab-separated, both halves in one module in `rivet-core`, round
trip asserted. `ADR-032`'s reasons, unchanged; a service name cannot contain a
tab, so there is no escaping.

### 3.1 What stops an application from using this to seize the screen

**Two independent things, and the first needs no Rivet code at all.**

1. **The socket is `0600 root:root`.** An application runs as its own uid
   (`ADR-015` §5.1.1) and cannot `connect(2)` to a socket file it cannot open.
   That is the same default-deny §6.2 relies on for `/dev/fb0`, measured the
   same way, and it is the reason Q1's answer is *image service*: making the
   launcher an application means widening this, and there is no way to widen it
   to one uid that is not also a way to widen it to two.
2. **`kiosk-select` is converge-only in `ADR-017` §6.3.2's sense.** The
   supervisor **refuses a name that is not offered** — not `enabled` in the
   document, or not declaring `devices = ["display"]`, or not installed. So the
   verb cannot start a service the document disabled. It chooses among the
   services the document already said may run.

§6.3.2's rule is *"may only ask it to CONVERGE, never to diverge"*, and this
design keeps it **literally** rather than by analogy. The trick is in the
definition of the desired state, and it is §4.1's.

### 3.2 Not the MCP control plane, and not a new socket

| | why not |
| :-- | :-- |
| `ADR-035`'s MCP over TLS on 7777 | it is the **remote operator's** plane. A local root program reaching a process on the same machine through TLS with a pinned fleet identity is a loop through the network stack for nothing — and `rivet-mcpd` can be disabled by the document, while the screen must work on a board with no network at all |
| a second unix socket, launcher-only | two control surfaces for one supervisor is `ADR-011`'s violation with extra syscalls. §6.3.1 already named this socket the control surface |
| a file under `/state`, like `deploy_request` | that shape was chosen because an install *must survive a reboot* (the maintainer's 2026-08-24 install-request ruling). A menu selection must **not** — see §7.2 |

---

## 4. Q3 — how the display changes hands

### 4.1 The desired state gains a selection, and that is the whole mechanism

Today `apply_auto_run` clears `enabled = false` on every display-declaring
service that `[kiosk] auto-run` does not name. That is correct while nothing can
start them later, and **wrong the moment the launcher can**: a service whose
definition says `enabled: false` is one `ADR-017` §6.3.2 refuses to start, so
the launcher's whole job would be a divergence request.

**Proposal: the supervisor holds a display selection, `Option<String>`.**

* `enabled` for a display-declaring service means **offered**, and is left
  exactly as the document wrote it. `apply_auto_run` stops writing it.
* A display-declaring service starts iff `enabled && selection == Some(name)`.
* The selection is initialised at boot from `[kiosk] auto-run` (§8) and moved at
  runtime by `apply kiosk-select` / `apply kiosk-return`.

The desired state for the display is therefore **the offered set plus a
pointer**, and moving the pointer inside that set is convergence, not
divergence. `ADR-015` §6.2 point 3 — one owner at a time — is now enforced by
the pointer being an `Option<String>` rather than by refusing the second
declarer at discovery, which `§104.17.3` already established is the wrong place
for it.

### 4.2 The launcher is NOT running while an application owns the screen

This is the sub-decision the sequence turns on.

| | launcher exits on handoff (**recommended**) | launcher stays resident |
| :-- | :-- | :-- |
| who guarantees it stops drawing | **the supervisor** — it is not a running process | its own code, on the honour system |
| DRM master / framebuffer release | the kernel, on process death. Unconditional | an explicit close it must remember to do |
| learning that the application exited | it does not need to; the supervisor starts it again on the stop edge — the same edge `splash::restore` fires on today | needs an out-of-band notification, or a poll |
| a launcher crash while an app runs | nothing happens; it is not running | `restart = always` redraws the menu **over** the running application |
| cost | a second process for the escape gesture (§4.4) | none |

**Recommended: it exits.** The supervisor already owns "restore the idle screen
on the stop edge" and this is that mechanism with a different program on the end
of it. `cmd/rivet-sysd/src/splash.rs::restore` becomes
`launcher::start_menu`, and the splash keeps its own separate job (§10 item 6).

### 4.3 The sequence, precisely

Boot, no `[kiosk] auto-run`, one or more offered applications:

1. Pid 1 draws the splash (`cmd/rivet-initd/src/splash.rs`), as today.
2. The supervisor converges. Selection is `None`, so it starts
   **`rivet-launcher menu`** — root, `restart = on-failure`.
3. `menu` opens `/dev/fb0` and every `/dev/input/event*` read-only. It is root:
   **no device grant is involved**, and it declares no devices, so
   `devices::contested` never sees it.
4. `menu` sends `get kiosk`, draws the offered applications, and takes
   up/down/enter (§6, §7).
5. The user presses ENTER on `X`. `menu` sends `apply kiosk-select X`, reads the
   acknowledgement, closes its framebuffer and input descriptors, and **exits
   0**.
6. The supervisor records `selection = Some(X)`. **It arms the next step on the
   launcher's stop edge — the `SIGCHLD` — and not on having sent the reply.**
   The framebuffer mapping and the DRM master are released when the process
   dies, not when it says it will die.
7. On that stop edge: `devices::grant(X, …)` chowns `/dev/fb0`, `/dev/dri/*` and
   the input nodes to `X`'s identity, exactly as it does today. `X` starts.
8. The supervisor also starts **`rivet-launcher watch`** — see §4.4.

Coming back:

9. The user holds BACK for two seconds. `watch` sends `apply kiosk-return` and
   exits 0.
10. The supervisor sets `selection = None` and stops `X`: `SIGTERM`, then
    `ADR-017`'s existing escalation to `SIGKILL` after the grace period.
11. On `X`'s stop edge — the same edge, whether it was killed, quit by itself or
    crashed — `devices::revoke(X, …)` returns every node to `root:root`, `watch`
    is stopped because its desired state is now false, and
    `rivet-launcher menu` is started again.

**An application that exits on its own** skips steps 9 and 10 and arrives at 11
by the same path. That is the point of putting the restore on the stop edge
rather than on the return request: there is one edge and one restore, and a
crash is not a special case.

### 4.4 The escape gesture is the SYSTEM's, and that needs a second reader

`examples/probes/kiosk-demo` already detects the two-second hold and deliberately does
nothing with it. Its comment is the requirement:

> **The gesture is PROVEN here, not acted on.** This program does not exit:
> stopping a kiosk application is the supervisor's, and an example that did it
> would be claiming a power `ADR-015` gives it no way to have.

So the way out must not depend on the application's cooperation — otherwise an
application that ignores it strands the screen and the only remedy is a reboot
or a remote operator.

**Recommendation: `rivet-launcher watch`** — the same binary, a second mode,
supervised, root, `restart = always`, desired state *"running iff an application
holds the display."* It opens **only** the input nodes. It never touches the
display. On a two-second BACK hold it sends `apply kiosk-return` and exits.

**This works because evdev permits more than one reader.** A node is only
exclusive to a process that calls `EVIOCGRAB`, and nothing in this workspace or
in `examples/probes/kiosk-demo` does — verified by grep across both trees, and it is a
property that must be **asserted in a test on any application this platform
ships**, because an application that grabbed the nodes would silently disable the
escape.

Rejected alternatives:

| | why not |
| :-- | :-- |
| the application implements BACK-hold-to-exit | it is a contract nothing enforces, and the failure mode is a stranded screen. `kiosk-demo`'s own comment argues against it |
| evdev decoding inside `rivet-sysd` | puts device parsing and a per-event wakeup into pid 1's **only child**, which `ADR-017` §3.1 says must not fail; and the supervisor would be a second reader on nodes it has itself chowned away, invisible to any operator looking at ownership |
| a magic key sequence handled by the kernel (SysRq) | it kills a process without telling the supervisor, so the selection and the device grant are left inconsistent |

### 4.5 What happens if the application ignores the request to exit

Four statements, in order of how far they get:

1. **`SIGTERM` may be ignored.** `ADR-017`'s existing escalation to `SIGKILL`
   after the grace period applies unchanged; no new mechanism is proposed.
2. **`SIGKILL` cannot be caught, blocked or ignored.** The process dies.
3. **The kernel releases the framebuffer mapping and drops DRM master on process
   death.** There is no userspace lease to reclaim and nothing for Rivet to
   revoke beyond the `chown` it already performs. A dead process cannot hold the
   screen.
4. **The residual failure is a process wedged in an uninterruptible ioctl** — `D`
   state, inside a hung GPU. `SIGKILL` is delivered and not taken. Nothing in
   userspace fixes that; the answer is a reboot, which is the same answer this
   machine has today, reachable over the control plane.

**And one cosmetic window that should be stated rather than found:** between the
kill and the launcher's first frame, the screen shows the dead application's last
frame. That is §6.2's drift, bounded by one process start, and the launcher's
first act is to clear the screen.

**If both launcher modes are dead** and an application holds the screen, the
machine is still reachable: `apply kiosk-return` is the same request over the
same operation, and §10 item 5 asks whether the CLI and the MCP plane should
expose it.

---

## 5. Q4 — the app list comes from the supervisor, over the same socket

**Recommendation: a new read subject, `get kiosk`.** The supervisor answers with
one line per **offered** application — installed, `enabled` in the document, and
declaring `devices = ["display"]` — plus which one is selected.

```text
selected	aura-controller
app	aura-controller	true
app	kiosk-demo	false
app	mesa-demo	false
```

`ADR-015` §6.2's own argument is why this is the supervisor's to answer:

> The supervisor arbitrates, because it is the only thing that starts and stops
> services and therefore the only thing that can know who holds what.

`devices::contested`'s signature is that argument written as a type — it takes
*every* definition at once, because no single definition is wrong and only
something holding all of them can see the answer. The launcher holds none of
them.

**Rejected: the launcher walks `/state/app` and reads manifests itself.** Three
reasons, and the second is the fatal one:

1. It cannot see `enabled`. That lives in `/state/config/machine.toml`, and a
   launcher that read the document would be a **second reader deriving one
   answer** — `ADR-011` exactly, and the same shape as `ADR-007` §3.6.6's
   reverse-lookup problem.
2. It would duplicate `applications::discover`'s definition of what an installed
   application *is*: the `current` symlink, the version directory, the header,
   and `ADR-015` §4.2's verify-at-every-launch. Two definitions of "installed"
   is one more than the machine can be right about.
3. `§104.17.4` item 1 — *"a newly installed application has no definition until
   the machine restarts"* — means the supervisor's list and the directory's
   contents legitimately differ, and only one of them is what would actually
   start.

### 5.1 The display name is an open question, deliberately

There is no `title` in the manifest today, so the launcher shows the application
**name**. That is `aura-controller` rather than *Aura*, which is legible but
not chosen. §10 item 2 is the maintainer's.

---

## 6. Q5 — `[kiosk] auto-run` is the boot-time initial selection

**Recommendation: the supervisor applies `auto-run` directly at boot and the
launcher does not run at all on that boot.**

| | supervisor applies it (**recommended**) | launcher starts and hands off |
| :-- | :-- | :-- |
| an appliance whose launcher is broken | **still boots into its application** | boots to nothing |
| what the user sees | the application | a menu flash, then the application |
| code paths | one selection, two writers (the document at boot, the launcher at runtime) | one writer, but the launcher gains a mode that exists only to be skipped |
| boot timing | the supervisor's convergence, as today | depends on a userspace menu program's start time |

The first row decides it. `auto-run` is the **appliance** configuration — it is
what a machine ships with when it is one thing rather than a menu — and making
the appliance's boot depend on the menu program is exactly backwards.

### 6.1 The trap, named before it is built

**`auto-run` is consulted ONCE PER BOOT and must never be re-applied.**

If the supervisor re-derived the selection from the document on the stop edge,
then on an `auto-run` machine holding BACK would stop the application and
immediately start it again. **The launcher would be unreachable on exactly the
machines that have one configured**, and the symptom would be a screen that
blinks and comes back — a failure that reads as a crash loop rather than as a
policy.

So: `auto-run` is read at boot, into the selection, and after that the selection
is the supervisor's own state. Coming back from `X` lands in the menu. Picking
`X` again is one ENTER. Making a choice survive a reboot is a document edit,
which is already true of everything else on this machine.

### 6.2 What this does to §104.29.5's dead screen

Nothing about the ruling changes, and the symptom disappears:

| document | before the launcher | after |
| :-- | :-- | :-- |
| no `[kiosk]`, one app `enabled = true` | **dead screen** (correct, per §104.29.5) | the launcher's menu, offering it |
| `[kiosk] auto-run = "X"`, `X` enabled | `X` runs | `X` runs, unchanged |
| `[kiosk] auto-run = "X"`, `X` not enabled | warned, dead screen | warned, and **the menu** — which is where the operator fixes it |

The strictness §104.29.5 protects is untouched: `enabled` still never means
*runs*. What changes is that the state it leaves the machine in now has
something in it.

---

## 7. State, and where it does not live

### 7.1 The selection is memory, not a file

`AGENTS.md` §5's *derive, never store*, and `ADR-032` §3.2's own justification
for the query socket existing at all: *"which services are running … exist only
in `cmd/rivet-sysd`'s own `Supervisor`, and no file holds them."* The selection
is the same kind of fact.

### 7.2 A selection must NOT survive a reboot

This is the opposite of `deploy_request`'s ruling and the reason is the same
reasoning run the other way. An install must survive a reboot because the
operator asked for a thing that takes minutes and 640 MiB. A menu selection is a
button press, and a selection that outlived the boot would mean **a machine that
does not boot to what its document says.** `[kiosk] auto-run` is where "what this
machine runs on the screen" is written down, and a second, invisible, persistent
answer to that question is `ADR-011`'s violation with a reboot in front of it.

Consequence, stated: **a power cut loses the user's choice and the machine
returns to `auto-run`, or to the menu.** That is correct and should be the
documented behaviour rather than a discovered one.

---

## 8. What the launcher draws, and with what

`cmd/rivet-splash` is the precedent for every part of this and should be reused
rather than re-derived: it reads the framebuffer geometry with
`FBIOGET_VSCREENINFO` rather than assuming it, it carries a bitmap font, and
`crate::pixel` already knows RGB565 and 32-bit layouts.

**Recommendation: the launcher draws to `/dev/fb0`, not to DRI/GBM.** The menu
is a list of text. `§104.17.1` records what a GLES stack costs — eight shared
libraries, a `dlopen`ed GBM backend, and a linking arrangement that needed GCC
as the driver — and none of it buys anything a menu needs. The applications the
launcher starts may use the GPU; the launcher does not need to.

The input vocabulary is `§104.18`'s, already written and already tested in
`examples/probes/kiosk-demo`: up/down/left/right/enter from a keyboard or either shape
of D-pad, BACK decided on the release, `BackHeld` at two seconds. `§9` below
records where the scaffold put it.

---

## 9. What is scaffolded, and what is deliberately not

**Written by this change:**

* `cmd/rivet-launcher/` — a crate in the workspace that **compiles and does
  nothing**. `src/input.rs` carries `§104.18`'s ruled vocabulary and its tests,
  ported from `examples/probes/kiosk-demo` so that the launcher owns it rather than
  borrowing from an example. `src/main.rs` parses the two modes this document
  names and **refuses to run either**, because the supervisor half does not
  exist.

**Deliberately NOT written**, because each depends on a ruling in §10:

* the three control-socket subjects, in `rivet-core`;
* the supervisor's selection state, and the `apply_auto_run` rewrite it needs;
* a `definitions.rs` entry for either mode — **nothing starts this binary**, and
  a launcher wired into the image before it draws would be a service that exits
  immediately in front of `restart`;
* any change to `cmd/rivet-image`, so the binary does not reach a card;
* the framebuffer drawing itself.

### 9.1 The honest state of it

Nothing here has run on a board. The vocabulary's tests pass and prove a
mapping, not a launcher. **Until the supervisor can be asked to select an
application, this crate is a place to put code and not a program.**

---

## 10. What is the maintainer's to rule

1. **§2 — image service or application.** The recommendation is image service,
   and it is the decision every other section is built on. Changing it changes
   §3 completely.
2. **§5.1 — does the manifest gain a `title`?** A label is what a person reads
   off the menu. If it is in the manifest it is inside the signature
   (`ADR-015` §4.1) and the operator cannot change it; if it is in the document
   it is a fact about the artifact that the machine's owner can rewrite, which
   §3.2.1's direction test would have to be applied to.
3. **§4.1 — `enabled` stops being rewritten for display applications.** This
   changes what `rivet get status` reports for them: today they read
   `enabled: false` after `apply_auto_run` has cleared them, and under this
   design they would read `enabled: true` and simply not be running. That is
   more honest and it is a visible change to an operator-facing surface.
4. **§4.4 — a second root process reading input the whole time an application
   runs.** It is what makes the escape independent of the application. It is
   also a process an operator did not ask for, and it means every kiosk
   application on this platform is forbidden from calling `EVIOCGRAB`.
5. **§4.5 — should `kiosk-select` and `kiosk-return` be reachable from the CLI
   and the MCP control plane?** `kiosk-return` converges toward the safe state
   and is the remote remedy for a stuck application, which argues for yes.
   `kiosk-select` from a workstation would make a headless kiosk test possible
   without a keyboard on the board. Both widen who can move the screen, to a
   population that already has root.
6. **§4.2 — what remains of `rivet-splash`?** The recommendation keeps it: pid 1
   draws it before the supervisor converges, and the launcher cannot. But
   `splash::restore`'s stop-edge job moves to the launcher, and the two should
   not both be claiming to be the idle screen.
7. **Should an application with `devices = ["display"]` but no `"input"` be
   offered?** It can be launched and can draw, and the only way out of it is the
   system gesture — which works, because §4.4's watcher is reading the nodes
   whether the application declared them or not.
8. **Aura.** Under this design Aura needs no BACK handling of its own and no
   change at all. Whether it is `auto-run` on this board, or offered beside
   `kiosk-demo`, is the first real use of the menu and is his call.
