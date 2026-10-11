# ADR-058: The Screen Is a Surface, Not a Lease

* **Status:** **Accepted**, 2026-09-20. The maintainer, asked whether the
  better shape is for `rivet-launcher` to be a 3D application handing out 2D
  or 3D surfaces, said that if such a change was ever going to be made, today
  was the best day to make it — then, on the decision, said to do it now
  (`.rulings/` R-309).
* **Date:** 2026-09-20
* **Amends:** `ADR-036` §2–§3, `ADR-015` §6.2
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`ADR-036` §2 rules that a kiosk application is handed **a DRM lease** on the
socket `RIVET_DISPLAY` names, and `ADR-015` §6.2 makes `rivet-launcher` the
one arbiter that holds DRM master and leases the display out. §3.1 states the
reasoning plainly:

> the lease is the screen. A render node cannot modeset and cannot disturb
> whatever owns the display, which is exactly the reach a program that draws
> through a lease needs.

**That sentence is false on this hardware, and 2026-09-20 is when it was
found out.** `examples/probes/gpu-flip` — the first application to ask Rivet
for the GPU's own scanout buffer — established on `rivet-06f5c`:

* `gbm_create_device` **accepts `/dev/dri/renderD128` and refuses the lease.**
* The board carries `card0`, `card1` and `renderD128`. Under `vc4-kms-v3d` the
  display controller and the GPU are **separate DRM devices**: VC4 does KMS
  and cannot render; V3D renders and has no display.

So an application that wants the GPU needs **both** descriptors and a PRIME
bridge between them: render into a linear buffer on V3D, export it with
`gbm_bo_get_fd`, import it on the lease with `drmPrimeFDToHandle`, then
`drmModeAddFB2` and `drmModePageFlip`. The lease is not the screen. It is one
half of the screen, and the half that cannot draw.

### 1.1 What the current shape cost, measured in one afternoon

Getting one probe as far as EGL required **five new confinement grants**, each
found by a build-flash-read cycle, each widening what *every* application
declaring `display` may reach:

| grant | why |
| :-- | :-- |
| `/usr/lib/gbm` — READ, LIST, **EXECUTE** | `libgbm` dlopens its backend from a compiled-in path |
| `/sys/dev/char` | Mesa turns a descriptor into a device through sysfs |
| `/sys/devices` | what those symlinks resolve into |
| `/sys/class/drm` | the other directory Mesa enumerates |
| `/dev/dri` — **LIST only** | Mesa scans the node directory |

**Every one exists because the APPLICATION talks to Mesa.** None of them is
about drawing; all of them are about a userspace GL stack discovering its own
hardware. They are now permanent platform surface, paid by every kiosk
application forever, including the ones that will never use a GPU.

---

## 2. Decision

**`rivet-launcher` becomes a compositor. It owns the GPU and the display, and
hands an application a SURFACE rather than a lease.**

An application asks for a surface and is handed one — a 2D surface it writes
pixels into, or a 3D surface it renders into with GL. The launcher composites
whatever surfaces exist and performs the single page flip.

**What moves into the launcher, once:** mode enumeration, `gbm_create_device`,
EGL context creation, the PRIME export/import across V3D and VC4, `AddFB2`,
`drmModePageFlip`, and vblank handling.

**What leaves every application:** all of it, plus the five grants above.

---

## 3. Why this is the right shape

### 3.1 It is what `SPEC-03` §2 already says

*"An application declares what it needs and is handed it."* Today it is handed
a **lease** — raw hardware — and must then know DRM, GBM, EGL, PRIME,
modesetting and vblank in order to use it. That is not being handed what it
needs; it is being handed the thing the platform did not want to abstract,
along with the obligation to understand it.

A surface is the noun an application actually wants. Handing it one is the
same move the platform already makes for sockets: `ADR-042` §7 lets no
application bind a TCP port, because the supervisor pre-binds and hands the
descriptor over. **The display is the one device where Rivet hands over the
hardware instead of the result.**

### 3.2 The confinement surface collapses

A lessee that does not talk to Mesa needs none of §1.1's five grants — and
needs no render node either. `device_globs`' `Lessee` arm could become
**empty**. An application that draws would reach exactly one thing: the
surface it was handed.

That is strictly smaller, strictly simpler, and removes a class of failure
this project has now hit six times: *the platform promises a library and then
refuses something that library needs*.

### 3.3 The hardware topology stops being every application's problem

VC4-and-V3D-are-separate is a Raspberry Pi 4 fact. `rivet-long-term-targets`
names x86 among the machines Rivet is meant to replace, and that machine's
topology is different again. Under the lease model **every kiosk application
encodes this board's layout**; under a compositor the platform knows it once
and an application's code is the same on both.

### 3.4 It removes the reason for exclusivity

`ADR-015` §6.2 point 3 makes `display` exclusive because *"two programs
writing a framebuffer interleave, and with fbdev neither is wrong because
there is no protocol to violate."* A compositor **is** that protocol. Whether
Rivet ever wants two things on a screen at once is a separate decision, but
today the answer is forced by the mechanism rather than chosen.

---

## 4. What this costs, stated plainly

* **A compositor is a real program**, not a refactor. It is the largest single
  piece of work this ADR has ever proposed.
* **`aura-renderer` changes shape.** It stops owning a screen and starts
  owning a surface. Its widget drawing is unaffected; its `rivet_sdk::screen`
  use is not.
* **The launcher becomes a program that may not fail**, and that is not a cost
  — it is a category. The maintainer, on an earlier draft that listed it as
  one, observed that `rivet-initd` or `rivet-sysd` crashing can take down the box
  just as readily, and that this is often true of system programs: they cannot be
  allowed to fail. *(`.rulings/` R-310.)*

  `rivet-initd` is PID 1 and `rivet-sysd` is the supervisor; neither may crash
  and nobody records that as a drawback of having them. The launcher already
  holds DRM master for the life of the machine, so it is **already** in that
  class. What changes is that it acquires a GL context and a composite loop,
  which is **more code held to a standard it is already held to** — a reason
  to write it like `ADR-017` §3.1.5's PID 1 (no unwinding out of main, every
  failure an answer rather than a panic), not a reason to weigh against the
  design.
* **One composite step is added.** On the V3D that is a textured quad per
  surface and is not measurable against a 33.3 ms budget; on a machine with no
  GPU it is a memcpy, which is what applications already do.
* **`ADR-036` §3.0's built, corrected, hard-won lease plumbing is retired.**
  It worked. It is being replaced because the thing it hands over turned out
  to be the wrong noun, not because it was wrong about the noun it chose.

---

## 4.1 It is not a launcher, and saying so is part of the decision

The maintainer, on reading §2, asked what `rivet-launcher` is here — a true
launcher, or a window manager in disguise — because the project should be honest
about what it is building. *(`.rulings/` R-311.)*

**A window manager in disguise, and it already was one before this ADR.**
`rivet-launcher` holds DRM master for the life of the machine, draws a menu,
decides which kiosk application runs, leases it the display, revokes the
lease, and draws again when nothing holds the screen. A launcher starts a
thing and gets out of the way; this never gets out of the way, and the
application it starts is a tenant. Under §2 it also owns a GL context,
allocates surfaces, composites and flips — which Wayland calls a compositor
and X calls a server plus a window manager.

**The name is not cosmetic.** A program called `rivet-launcher` gets written
like a launcher: somebody eventually adds a menu feature carelessly to the
process that owns the screen and may not fail.

### The split

| program | job | may it fail? |
| :-- | :-- | :-- |
| **`rivet-displayd`** | owns the GPU and KMS, hands out surfaces, composites, page-flips, arbitrates input | **No.** `rivet-initd`/`rivet-sysd`'s class, written to `ADR-017` §3.1.5's standard |
| **`rivet-launcher`** | draws the menu and asks the supervisor to start an application | **Yes.** Restart it; the screen survives |

This is the separation `rivet-sysd` already has from the machine's document —
mechanism in one place, policy in another — and it puts the part most likely
to change and most likely to carry a bug **outside** the program that cannot
fail. Today they are one binary, which is why §4 could write *"it may not
fail"* as though it were a cost rather than a fact.

**The launcher becomes `displayd`'s first client**, and that is deliberate: if
drawing a menu through a handed surface is awkward, it is better to find out
there than in Aura.

**The launcher is always beneath every application's window.** The
compositor keeps it at the bottom of its stack, recognised by its `app_id`
(`rivet_core::compositor::LAUNCHER_APP_ID`). That is what makes the table's
*restart it; the screen survives* true: a restarted launcher creates a new
window, and so does one that starts beside an application the supervisor
resumed at boot, and a stack ordered by arrival alone puts either over what
holds the screen. Wayland gives a client no request to lower its own window,
so this one piece of stacking belongs to the compositor.

### §4.2 Who may drive the screen, and why it is a DECLARATION

`.rulings/` R-138 made the arbiter *the program that connects to the kiosk
endpoint*, because that program was the launcher and the launcher held DRM
master. §4.1 splits it, so the rule moves with the screen: **the arbiter is
the compositor, which provides `socket:wayland`.**

**The declaration asks and the machine document decides** (`.rulings/` R-210,
D3). `[kiosk] compositor` names the one service that may be the compositor and
`[kiosk] launcher` the one that may be the launcher; each defaults to the
image's own, `rivet-displayd` and `rivet-launcher`, so a document that names
neither means what every document meant before the keys existed. A role goes
to the named service only when its manifest also declares the role's endpoint,
and any other service declaring it is an ordinary application. A declaration
alone cannot be the grant, because then any installed application could take
the screen, or take it away from the program meant to hold it, by declaring
one line. The names are read once at boot, with the kiosk socket.

The maintainer set out what that has to survive, in three messages on
2026-09-20: the `KIOSK` setting decides whether a machine is `headless`, `kiosk`
or, one day, `desktop`, and that is how the compositor fits in; headless means no
`rivet-displayd`, kiosk means the launcher, and desktop means COSMIC; and he
could one day see a mobile mode too, or even a `container` mode.
*(`.rulings/` R-312.)*

| mode | who provides `socket:wayland` | what else is different |
| :-- | :-- | :-- |
| `headless` | nobody | — |
| `kiosk` | `rivet-displayd` | `rivet-launcher` draws a menu |
| `desktop` | **`cosmic-comp`** | a session, not one fullscreen application |
| `mobile` | a phone shell | — |
| `container` | nobody | not about screens at all |

**`cosmic-comp` is a compositor in its own right, built on Smithay** — which is
why §5.1 chose Smithay. So a desktop machine does not stack a session on top of
`rivet-displayd`; it runs **a different provider of the same endpoint**, and
its document names it as `[kiosk] compositor`. Under a rule compiled against
the name `rivet-displayd`, adopting COSMIC would be a supervisor change; with
the name in the document it is a document change, and `cmd/rivet-sysd` needs no
edit at all.

Two consequences follow and are enforced by tests in `supervise.rs`:

* **One named compositor, so never two arbiters.** A second provider of
  `socket:wayland` is given what any application is given, so no declaration
  can hand two programs a way to take DRM master from each other — `ADR-015` §6
  point 3 — or leave the machine with nobody drawing.
* **A DISABLED compositor changes nothing.** An image may carry
  `rivet-displayd` and every existing kiosk goes on working under R-138's rule —
  the named launcher holds the screen — until a document turns it on. That is
  the migration, and it ends by itself.

**The mode is not "which compositor".** `container` and `headless` both have no
provider and differ in everything else; that is the only reading under which
`container` and `desktop` are values of one field. The mode stays *what this
machine is for*, and the compositor is one consequence of it among several.

---

## 5. What is NOT decided here

* **Whether two applications may share a screen.** This makes it possible and
  does not propose it.
* **The surface protocol itself** — whether a 3D surface is an EGL handle, a
  dma-buf the application renders into, or a GL framebuffer object bound by
  the launcher. That is `GPU-Backgrounds-2026-09-20` §5's question, now asked
  of the launcher instead of the addon boundary, and it should be ruled with
  the same evidence.
* **What happens to `aura-background-pack`'s frame call.** A pack draws into
  what its host hands it; if the host's surface becomes a GPU surface, the
  pack's boundary question is unchanged in shape and different in content.

---

## 5.1 THE COMPOSITOR IS SMITHAY, and a hand-rolled one was started and stopped

**Ruled 2026-09-20, hours after §2, and the agent had already written ~600
lines of the wrong answer.**

`rivet-displayd` was being written from scratch: GBM, EGL, a composite shader,
a page-flip loop and a shared-memory surface protocol in
`rivet_core::surface`. The maintainer stopped it with one question — whether
writing a compositor of our own was a fool's errand when a well-done, tested
Rust drop-in might already exist that should be looked at instead
(`.rulings/` R-313) — and `ADR-009`'s own table is the answer:

| | Option | When it is right |
| :-- | :-- | :-- |
| 1 | An existing Rust crate | **Default.** |
| 3 | Write it | Only when the component *is* Rivet. |

**Option 1 was never evaluated.** That is the process failure, and it is
recorded because skipping the default is the kind of thing that looks like
diligence while it is happening.

### What decided it

He said that if he ever adopts a desktop, and he might, he will almost certainly
ask whether COSMIC can be adopted first, and asked whether Smithay is then the
only logical choice. *(`.rulings/` R-313.)*

**`cosmic-comp` is built on Smithay.** So "adopt COSMIC" and "adopt Smithay"
are one decision taken at two times, and a hand-rolled compositor makes the
later one a rewrite instead of an addition. His summary is the whole of it:
choosing Smithay now leaves the path to adding a desktop a few months from now as
more work at that time, not a massive rewrite.

### The spike, because `ADR-009` §3.6 rejects by compiling rather than by reasoning

Run before the decision, not after. `smithay 0.4.4`, default features off,
`backend_drm` + `backend_egl` + `backend_gbm` + `renderer_gl` +
`wayland_frontend`, cross-built in the build container:

* **It compiles for `aarch64-unknown-linux-musl`** against the Buildroot
  toolchain, in 15.5 seconds, 152 crates — including `xkbcommon`, `drm-ffi`,
  `gbm` and the whole `wayland-*` tree.
* **Today's confinement grants are what let it work.** Resolving Mesa at
  runtime needs `GBM_BACKEND_DIR`, the sysfs tree and `/dev/dri` listing
  exactly as `gpu-flip` did. Those five grants turn out to be prerequisites for
  Smithay, not merely for the abandoned code.

#### The spike measured a compile and was reported as a link, 2026-09-20

**A claim in the first draft of this section was false and is withdrawn.** It
said *"the binary is STATIC — `readelf -d` reports no `NEEDED` entries at all,
so `ADR-003` §5 needs no exception."* The spike ran `cargo check`, which type
checks and **never invokes the linker**, so there was no binary to read and the
sentence was an inference dressed as a measurement. It is recorded because it
was used as an argument — it is the reason this section claimed Smithay was
*strictly better* than the hand-rolled version on exactly the axis the project
cares most about.

**What a real link says**, run the first time `cmd/rivet-displayd` had a
`main` in it:

| | |
| :-- | :-- |
| `-lgbm`, `-ldl`, `-lgcc_s`, `-lc` | resolved from Buildroot's staging tree |
| `-lxkbcommon` | **not present in this tree at all** — the link named exactly one missing file |
| static | **no.** `libgbm` ships shared-only, so `ADR-003` §5 needs its exception here exactly as `examples/probes/gpu-flip` does |

With `BR2_PACKAGE_LIBXKBCOMMON=y` it links, and what it actually is:

| | |
| :-- | :-- |
| `NEEDED` | `libxkbcommon.so.0`, `libgbm.so.1`, `libgcc_s.so.1`, `libc.so` |
| stripped | **2.85 MB** |
| build | 18.7 s from cold, its own workspace |

So the cost of Smithay is **one Buildroot package** —
`BR2_PACKAGE_LIBXKBCOMMON`, because the `xkbcommon` crate is a dependency of
`smithay` that no feature flag removes — and the same dynamic link recipe
`gpu-flip` already carries. That is a real cost and it is a small one; it does
not disturb §5.1's decision, because `cosmic-comp` would need the same library
on the day COSMIC is adopted.

**`xkeyboard-config` is NOT enabled with it**, and the difference matters:
`libxkbcommon` is 200 KB of code that the link needs, and `xkeyboard-config` is
~5 MB of keymap data that only `xkb_keymap_new_from_names` reads. Nothing here
compiles a keymap yet — the seat is created with no keyboard on it — so the
dataset is not shipped on the chance it is wanted. When a client asks for
`wl_keyboard`, that package and an embedded keymap string should be measured
against `ADR-018` §4's slot budget.

### §5.2 It ran, and what the board said

**Proven on `rivet-06f5c`, 2026-09-20**, slot `b`, version `0.1.99`. The
maintainer watching the panel recognized the launcher and made two further
remarks, about time and a drop shadow (`.rulings/` R-314). The journal is the
rest:

```
rivet-displayd: HDMIA-2 at 1920x1080 @ 60.00 Hz
rivet-displayd: the supervisor handed over the compositor socket
rivet-displayd: composing on /run/rivet/listen/rivet-displayd/wayland
rivet-launcher: type from /system/share/fonts/Geist.ttf
rivet-displayd: a client connected
rivet-launcher: the surface is 1920x1080
rivet-launcher: 1 application(s) offered
```

Zero restarts on either. **§3.2's claim is now a measurement rather than an
argument:** `rivet-launcher` declares `devices = ["input"]`, holds no DRM node,
no render node, no `/usr/lib/gbm` and no sysfs — and drew a menu on a 1080p
panel through a surface it was handed.

**Three defects were found by the board and by none of the 698 tests**, which
is worth recording as a pattern rather than as three incidents:

1. A card that built clean, passed every gate, and **carried no compositor** —
   `$OURS` is passed to one `cargo build` and this binary comes from another,
   so the staging loop never saw it and three guards each returned quietly.
2. A compositor that installed, enabled, and answered `state: idle` — the
   arbiter was still derived from the kiosk endpoint, so it was classed a
   lessee and the display gate disabled it. §4.2 is the repair.
3. A **wonky focus highlight**, visible only on glass: the card's sheen was
   drawn as a rounded rectangle one pixel tall, whose radius clamps to a
   stadium, so every row was a straight bar across the rounded corners.

**One kernel warning per boot, and it is kept on purpose.**
`WARN_ON` at `vc4_hvs.c:1064`, reached from `disable_outputs` on the first
atomic commit: the Pi firmware has already configured the display, so the
driver is asked to stop an HVS channel it did not start. `DrmDevice::new`'s
`disable_connectors: false` would silence it and reintroduce the fault it
guards — a firmware-lit connector staying lit under a compositor that is not
driving it. `screen.rs` carries the trace so nobody spends an afternoon on it.

### §5.3 A screenshot is the compositor's, and it is not an application contract

**The compositor takes the picture, because it is the one program that can see
the screen.** Under a lease, a capture came from whoever held DRM master, so
`cargo rivet screenshot` photographed one program's frame — the launcher's —
and never what a lessee drew. `rivet-displayd` composites everything, so it
writes the file, into its own data directory
(`/state/app/rivet-displayd/data/screen.qoi`).

**A per-application capture hook was rejected**, on three grounds, the first
decisive:

1. It would make the operator's capture depend on the **application's
   cooperation**. A program can refuse or be wedged, and a wedged program is
   exactly when a picture is worth most.
2. It would capture the wrong thing. An application can render only what it
   drew; a screenshot is what is **on the screen**, including what is behind.
3. It would be N implementations of one thing (`ADR-011`): every kiosk
   application would carry an encoder.

`wlr-screencopy` remains the right answer for a **client** that wants pixels —
a recorder, a remote desktop. An operator's capture over the control plane is
a different question, and its answer must not route through any client.

**The wiring is existing machinery.** `rivet-displayd` provides a second
endpoint, `capture`, bound and handed over by the supervisor exactly as the
Wayland one is. `apply input capture` still arrives at `rivet-launcher`,
because it provides the input endpoint, and the launcher **forwards** the
request rather than performing it, under a deadline because it is a draw loop.
The compositor's answer goes to the journal, so an operator who sent the press
sees the outcome without a second question.

**It renders a second time rather than reading back the scanout buffer.**
`DrmCompositor::render_frame` binds, draws and unbinds the swapchain buffer, so
no framebuffer is bound afterwards to read. A capture draws the same elements
into an offscreen renderbuffer and reads that: one extra frame's work only when
asked, and it cannot disturb what is on the glass — looking must not change
what is being looked at. The read-back format is chosen so that GLES's memory
order and the encoder's agree; a swapped red and blue produces a picture that
looks almost right, which is worse than one that looks broken.

### §5.4 An application reaches the compositor through the SDK, not directly

**Ruled 2026-09-20.** Asked where the seam goes — a feature on `rivet-sdk`, or
each application speaking Wayland itself — the maintainer chose **A**: the SDK
(`.rulings/` R-318).

`docs/design-candidates/Aura-as-a-Wayland-Client-2026-09-20.md` is the three
options and why A won. The short version is §3.1 read one layer up: **the whole
argument for a compositor was that an application should be handed a result
rather than a mechanism.** Handing it a Wayland connection instead of a DRM
lease would be a smaller version of the same mistake, and it would make every
kiosk application learn a protocol.

**What makes it cheap is that `rivet_sdk::screen::Screen` is five items wide** —
`width`, `height`, `stride`, `what`, `blit`. Modeset, double buffering, page
flips and the missed-event policy are all behind them, and `aura-renderer`
names none of it. So a Wayland implementation of five items is a different
backend and **the same application**: Aura's drawing code does not change at
all.

It stays static. `cmd/rivet-launcher` proved that `smithay-client-toolkit` with
`default-features = false` is pure Rust — 0 `NEEDED` entries — so the SDK gains
no loader and no C library.

**The dependency cost is real and is paid by precedent, not waved through.**
`rivet-sdk`'s `[dependencies]` table is empty *by rule*, and this is the third
exception after `screen = ["draw"]` and `serve`. The bargain is the one the
first exception already struck: a feature nothing enables by default costs a
network service nothing.

### What this supersedes

* `rivet_core::surface` — **removed.** Wayland is the protocol now, and a
  second one in the tree would be a second answer to a settled question.
* `cmd/rivet-displayd`'s `gl.rs`, `shm.rs` and composite shader — **removed.**
* §4.1's split **stands**: `rivet-displayd` is still the program that may not
  fail and `rivet-launcher` is still a client that may. What changes is that
  the first is a Smithay compositor and the second is a Wayland client.

### What survives, and it is not small

`examples/probes/gpu-flip`, the five confinement grants, and the four pitfalls
in §6. None of it was compositor code; all of it is what made this machine
able to run **any** GPU program. Smithay would not have got past
`gbm_create_device` on this board this morning either.

### §5.5 A display comes and goes, and the compositor waits for one

**On a kiosk machine `rivet-displayd` runs whether or not a display is
plugged in, and follows one arriving, leaving and changing at any time.**
Applications start without a display and are handed a surface when one
arrives; an unplug pauses them and never restarts them. On a headless machine
the compositor does not run at all, which the document already decides.
(`.rulings/` R-212, R-236.)

**The device and the display are held separately.** The DRM device, its GBM
allocator and the GLES renderer are opened at start and held for the life of
the process — a client's dma-buf is imported by that renderer, and a client
may connect before any display does. The display being driven — connector,
crtc, mode and swapchain — is built when one arrives and dropped when it
leaves, which clears its crtc. A machine with no DRM device that can drive a
display at all is still refused at start, because waiting cannot fix it.

**The kernel's hot-plug event is the trigger, and the connectors are the
answer.** `rivet-displayd` joins the kernel's uevent multicast group with one
`NETLINK_KOBJECT_UEVENT` socket. A message with `SUBSYSTEM=drm` and
`HOTPLUG=1` — or a lost message, which the socket reports as `ENOBUFS` —
makes it probe every connector again, forced, so a display that has just
arrived is offered its own modes rather than the previous one's. The event is
only a prompt; what is plugged in is always read from the connectors.
Smithay's `backend_udev` was not used: it links `libudev` to read the same
messages, and nothing on a Rivet runs `udevd`.

**The policy is one function** (`hotplug::decide`), given what is driven and
what the connectors say now:

| driven | the connectors say | change |
| :-- | :-- | :-- |
| nothing | nothing plugged in, or nothing offering a mode | keep waiting |
| nothing | a display offering a mode | bring it up at its preferred mode |
| a display | the same connector at the same mode | nothing |
| a display | the same connector at a different mode | swap it for itself at the new mode |
| a display | that connector empty, another usable | swap to the other |
| a display | nothing usable | take it down |

A display that is plugged in and offers no mode — an EDID that will not
read — cannot be driven and is treated as absent.

**A window that connects with no display is mapped and NOT configured.**
xdg-shell lets a compositor send the first configure when it is ready, and a
client may not attach a buffer before it, so the client waits. When a display
arrives every window is configured fullscreen at its mode; when a different
mode arrives every window is configured again, and the same mode again sends
nothing. The alternative — configuring at an invented size — was rejected
because every client this machine runs sizes its whole state once, from its
first configure: Aura allocates its canvas and its widget layer at that size,
and a GPU surface creates its GBM surface at it. An invented size would be
the size those clients drew at on the real display.

**A client waits for that configure as long as the compositor answers.** The
SDK (`rivet_sdk::composited`, `rivet_sdk::gpu`) and the launcher's own client
send a `wl_display.sync` each time their configure patience passes in silence.
A compositor that answers is alive and waiting for a display, and the wait
continues; one that does not answer is refused, which is the protection the
patience existed for. The launcher starts the `[kiosk] auto-run` application
before it waits, so a kiosk booted with its screen off still starts what it
is for.

**An unplug pauses frame callbacks and nothing else.** The output leaves the
space; its `wl_output` global is disabled at once, which withdraws it from
every registry, and removed at the next display change, because removing a
global outright kills a client whose bind was already on the wire. Windows
stay mapped and their clients keep running with their state intact. Frame
callbacks are sent only when a frame was put on the glass, so with no display
none is sent; one asked for during an unplug is held and answered by the
first frame after a display returns. A client that paces itself on callbacks
idles; a client that paces itself on a timer, as the SDK's do, keeps drawing
into buffers the compositor releases without showing.

**A frame wait is bounded, and an unplug cannot run it out** (`.rulings/`
R-210, D18). `rivet_sdk::composited` follows each frame with one
`wl_display.sync` and waits for its answer for at most the same patience the
start-up wait uses (`CONFIGURE_PATIENCE`, ten seconds). The wait is for the
answer to the sync and never for a frame callback, and the compositor answers a
sync whether a display is connected or not, so only a compositor that has
stopped answering altogether reaches the deadline. Then the SDK prints what it
was waiting for and closes the surface; the application exits on `closed()`
and its restart policy brings it back. The start-up wait is unchanged: it goes
on as long as the compositor answers.

**A capture with no display is refused at once, with a reason**, and the
published capture is withdrawn whenever there is no display — on an unplug,
at a start with nothing connected, and on a refused capture — because
`cargo rivet screenshot` fetches whatever the path holds, and a file left
there would be fetched as the screen of now.

### §5.6 The compositor runs under its own uid, and holds no capability

**`rivet-displayd` is an image service with a home** (`ADR-060` §2.1): the uid
`app_identity` derives from its name, the image-service Landlock ruleset, and
`/state/app/rivet-displayd` as an application has it (`.rulings/` R-210, D1).
Every client's bytes reach it — surfaces, dma-bufs and protocol messages — so a
defect in a protocol handler is reached by any application that draws, and it
must not run with root's reach. Being part of the operating system decides who
ships it, not what it may touch.

**What it needs, and how each is had without root:**

| need | how it is granted |
| :-- | :-- |
| the card node it drives | the `display` class's **arbiter** set (`ADR-015` §6.2), chowned to its uid by the supervisor before the spawn and appended to the ruleset with read, write and `ioctl` |
| the render node Mesa's `renderonly` opens | the same set, but **shared rather than chowned**: `root:render 0660`, with the `render` group added to its supplementary groups — see below |
| Mesa's device lookup: `/sys/dev/char`, `/sys/devices`, `/sys/class/drm`, `/dev/dri` listed, `/usr/lib/gbm` loadable | `confinement::image_service_policy` over the definition's own `devices`, by the same function that grants it to an application declaring `display` (§1.1's five grants) |
| the image's shared libraries, `libEGL` and its closure among them | the `/lib` row every confined program has |
| **DRM master** | **no capability.** The kernel makes the first process to open a primary node while none holds master its master, unasked. `CAP_SYS_ADMIN` is needed only to take master back after dropping it, and the compositor holds the node for its whole life and never drops it |
| hot-plug: the kernel's uevent multicast group | **no capability.** The kernel's uevent netlink family admits an unprivileged socket to its group, and Landlock does not mediate netlink |
| the capture file | its own data directory, read and write |
| `wayland` and `capture` | bound by the supervisor and handed over; it opens no socket path |

**It declares `display` and not `input`.** It reads no input device: its seat
carries no keyboard and no pointer, and the gesture vocabulary reaches the
launcher, which declares `input` itself. Under its own uid a declared class is
a chown of every node the class expands to, so declaring `input` would hand the
compositor every keyboard on the machine for nothing. When the compositor
arbitrates input, as §4.1's table means it to, it declares the class again.

**The render node has two holders and is shared through a group.** An
application that renders declares `gpu` and is granted the same render node,
so while the compositor ran as root the node could be chowned to that
application and the compositor reached it anyway. Under its own uid the
compositor is a second non-root holder, and a chown to one uid takes the node
from the other: whichever was granted last could open it, so the compositor
was refused it when an application started first, and the application would
be refused it at its next restart. So a render node is `root:render 0660` and
the supervisor adds the `render` group to every service it grants the node —
the compositor, and each application declaring `gpu` — whichever starts first.
Card nodes stay chowned to one holder, because DRM master is exclusive, and no
node is ever `0666` (`ADR-015` §6.2).

**The arbiter set includes `/dev/fb0`**, which the compositor never opens. It
is the class's set for whichever program holds the screen, and the launcher
holding it with no compositor enabled uses the same rows.

**Two things a machine still depends on, and neither is a capability.** No
other process may hold master on the display device when the compositor
starts: a root compositor would be refused in that case too, with `EBUSY`,
because a second master is refused to anyone. And the files root wrote into
its data directory stay root's: the supervisor gives the directory to the uid,
and the compositor replaces a capture by renaming over it, which a directory's
owner may do whoever owns the file.

---

## 6. The evidence this rests on — corrected after the probe succeeded

`gpu-flip` reached the glass at **57.3 fps, 1920x1080, `V3D 4.2.14.0`**, as an
ordinary kiosk application through its lease, with no privilege beyond
`devices = ["display"]`. The maintainer confirmed magenta on the panel, which
is the last frame's predicted colour. **Two claims this ADR was first written
on were wrong, and they are struck rather than quietly edited.**

* ~~`gbm_create_device` accepts the render node and refuses the lease.~~
  **False.** That test ran before `/sys/class/drm` and listing `/dev/dri` were
  granted; the lease failed for the same enumeration reason the render node
  failed for a cycle later. GBM works on the lease.
* ~~An application needs a PRIME bridge between V3D and VC4.~~ **False.**
  Mesa's `renderonly` layer bridges them *inside* a GBM device made on the
  display device. The buffer handed back is already a VC4 buffer, and
  importing it again answers `EINVAL`.

**What survives, and it is what §1.1 already called the argument:**

1. **Five confinement grants**, each found by a failure, each now permanent
   platform surface paid by every application that declares `display` —
   including every one that will never touch a GPU. Unaffected by the
   corrections above: they were needed, and they are needed because the
   *application* talks to Mesa.
2. **Eight build-and-flash cycles and four non-obvious pitfalls** to get one
   program to the glass: the window config's visual id (a first query returns
   only `XR30`/`AR30`, because `eglChooseConfig` treats `RED_SIZE 8` as a
   minimum and sorts deeper channels first), `GBM_BO_USE_SCANOUT`, PRIME
   versus `renderonly`, and the enumeration grants. **Every future kiosk
   author hits all four**, and writes the same three hundred lines to get
   past them.
3. `EGL_NOT_INITIALIZED` carries no cause, so the probe had to report its own
   reachability to make progress at all — which is not an API an application
   author should have to work around.

**The case narrowed and did not weaken.** §3.3's "every application encodes
this board's topology" is retired: `renderonly` already hides it. What
replaces it is better evidenced — not *the topology is duplicated*, but **the
three hundred lines of GBM, EGL, config matching, mode setting and flipping
are duplicated**, in every application, along with the platform surface that
lets them run.

That is the same argument `aura-background-pack` settled for backgrounds, one
layer down.

---

## History

The body above states what is true now and is complete without this section.

### §4.2 — the arbiter was whoever provided `socket:wayland`

**Superseded 2026-10-03 by `.rulings/` R-210, D3.** §4.2 made the role a pure
declaration and ruled that two enabled providers meant nobody was the arbiter:

~~**the arbiter is whoever provides `socket:wayland`.**~~

~~**Exactly one enabled provider, or nobody is the arbiter.** Two would hand
two programs a way to take DRM master from each other.~~

It changed because a declaration that is also the grant lets any installed
application make the machine's screen nobody's: one more provider of the
endpoint, and the compositor lost the screen with it. The document now names
the compositor and the launcher, and a declaration is only a request.
