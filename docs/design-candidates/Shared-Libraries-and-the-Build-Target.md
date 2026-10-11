# Design candidate: Shared Libraries and the Build Target

* **Status:** Proposed. NOT accepted, and deliberately not written as an ADR —
  `ADR-003` §6 already rules the direction and this proposes the *mechanism*
* **Date:** 2026-08-30
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Builds on:** `ADR-003` §5, §6.1, §6.2, §6.3

---

## 1. Context: what two cold agents did without being asked

On 2026-08-30 two agents from different vendors, with no shared context, were
each told to put a database-backed application on a Rivet board. Both rejected
SQLite. Both chose `redb`, a pure-Rust embedded key-value store. Both gave the
same reason, unprompted.

The second wrote it down:

> The environment requires cross-compiling to `aarch64-unknown-linux-musl` on a
> Windows host using `rust-lld`, meaning there is no C cross-toolchain readily
> available. I inferred that I could not use `rusqlite` (SQLite) because
> compiling `sqlite3.c` would fail. […] I would have needed explicit
> documentation on whether a sysroot/C-compiler was available.

Two independent derivations of the same undocumented constraint, on the first
two real applications anyone has built for this platform, is a strong signal.
The question is **what it is a signal of**, and the obvious reading is wrong.

---

## 2. The correction: this is not evidence about linking

The obvious reading is *"static linking is costing us adoption, bring shared
libraries forward"*. That reading does not survive contact with what actually
failed.

`rusqlite` with its `bundled` feature **statically links SQLite**. It compiles
`sqlite3.c` with the `cc` crate and absorbs the result into the binary. It
requires a C compiler targeting `aarch64-unknown-linux-musl`. It does not
require a dynamic loader, a `libsqlite3.so`, or any change to how Rivet links.

The two mechanisms address different failures:

| Failure | Fixed by a C cross-toolchain | Fixed by shared libraries |
| :-- | :-- | :-- |
| `rusqlite`, and every other `-sys` crate built from source | **yes** | no |
| Three binaries each carrying their own copy of musl and `std` | no | yes |
| `dlopen` plugins, proprietary `.so` files | no | yes |
| Running a binary somebody else compiled | no | **no** — see §8 |

So the observed friction sits entirely inside `ADR-003` §6.2's world. Both
agents were doing exactly what that section prescribes — *building for the
target* — and were stopped by the absence of the toolchain to do it with.

**That is §6.3's problem, and §6.3 is already ruled.** It says Rivet must become
documentable as a build target: the triple, the sysroot, and an honest statement
of which platform facilities exist. It also says *"not needed before Stage 3"*.

The new evidence contradicts only that last clause. An agent asked for the §6.3
artifact **by name**, on the first application built here by someone who had not
been told how. It is needed now, and it is cheaper than the thing it was
mistaken for.

---

## 3. The near-term fix, which is mostly already built

`tools/container/Dockerfile` states that the build image *"carries no cross
toolchain of its own"*. It does not need to: Buildroot produces one, and the
container reaches it through `host/bin/aarch64-linux-gcc` and
`aarch64-linux-strip` in the `rivet-br` volume.

**A C cross-toolchain already exists in this build environment.** It is wired
for Rivet's own build and never exposed to application authors, who get
`rust-lld` and nothing else.

Exposing it is plumbing, not design:

1. a documented `CC_aarch64_unknown_linux_musl` / `AR_…` pointing at Buildroot's
   toolchain, set where an application build can see it;
2. the sysroot path stated once, in a place an application author reads before
   the first build rather than after the first failure;
3. the honest facilities list §6.3 calls for — the valuable half, and the part
   nobody writes.

This changes no linking decision, ships no loader, and weakens no guarantee in
`ADR-003` §3. It would have removed the friction both agents hit.

### 3.1 A hazard for the same work: the container is portable, the image is not

`ADR-026` §7 chose Docker over WSL to be platform-independent. That holds for
*Docker*. It does not currently hold for the image, and the difference matters
the moment anyone builds on a host that is not x86_64 — an Apple Silicon
MacBook being the case already intended.

Two facts, both checked rather than assumed on 2026-08-30:

* the base image is digest-pinned to `archlinux:base-20260823.0.578598`, and
  `docker manifest inspect` reports exactly one platform for it: **`linux/amd64`**.
  A digest names one manifest, so the pin *is* an architecture pin;
* the cross toolchain is Bootlin `aarch64--musl--stable-2025.08-1`, published as
  **x86_64 Linux host** binaries.

So on Apple Silicon the image runs emulated, and the toolchain inside it runs
emulated too. The card build plausibly still works. The inner loop is the part
that suffers: `qemu-system-aarch64` emulating aarch64 inside an emulated x86_64
container on a host whose CPU is already aarch64.

**And that last sentence is also the opportunity.** On an arm64 Linux container,
host architecture and target architecture are the same, so building Rivet's own
binaries needs no cross-compilation and no cross toolchain at all — and QEMU
could use hardware virtualisation instead of emulating a foreign ISA. What
prevents it is not the design; it is that both the base image and the toolchain
happen to be x86_64-only artifacts.

This is a packaging problem, it is separable from everything else in this
document, and it should be settled **before** somebody spends an afternoon
discovering it. It also strengthens §3's argument: an image that fetched the
Bootlin tarball by URL against a recorded sha256 — which
`tools/container/Dockerfile` already argues for on provenance grounds — is the
same change that would let it fetch a *different* tarball for a different host
architecture.

---

## 4. The long-term question

Shared libraries remain an expected future option under §6.1. This section is
the design dive, and it exists because the mechanism is cheap to get right at
design time and brutally expensive to retrofit.

### 4.1 What shared libraries would actually buy

Stated honestly, because §6.1 names only one of these:

* **Size.** §6.1's motivation, and real once `uutils`, `nushell` and several
  `rivet-*` binaries share an image (`ADR-002` §5.1).
* **`dlopen` plugins.** Structurally impossible today, and no amount of
  toolchain fixes it.
* **Libraries that cannot be rebuilt** — a vendor `.so` with no source.
* **A shared application runtime**, so ten applications needing the same large
  dependency carry one copy rather than ten.

### 4.2 Prior art, and what each one learned the hard way

**Android** is the closest relative — a fleet of managed devices, image-based
updates, signed artifacts, an unprivileged application layer — and it is worth
being precise about what it actually did, because it is not "shared libraries
for everyone":

* **A small, stable, versioned public surface.** Applications may link only
  against the libraries named in `/system/etc/public.libraries.txt`. Everything
  else in the platform is invisible to them.
* **Linker namespaces** (Android 7+) enforce that in the loader itself, rather
  than by convention. Libraries in one namespace are hidden from another, and
  two libraries with the same name and different symbols do not collide.
* **Applications bundle their own native dependencies** in the APK. The
  vendor-everything model is *already* what Android does for application code;
  the shared surface is deliberately minimal.
* **APEX** (Android 10+) is the part that resembles the shape proposed below: a
  signed, versioned, self-contained filesystem image, mounted by a daemon
  early in boot, verified with dm-verity, updatable independently of the OS.

The lesson is the shape of what they *refused*. APEX exists so that **the
platform** can update its own components off the OTA cycle. It is not a
general-purpose shared-library distribution system for third parties, and
applications still bundle their own native code. Android had already been
burned once — the whole reason linker namespaces exist is that applications had
reached into private platform libraries and broken on every upgrade.

**Nix** contributes the mechanism that makes coexistence safe: every package
lives at a path containing a cryptographic hash of its inputs, so multiple
versions coexist without interference and each consumer resolves to an exact
dependency closure. Dependency hell is not managed; it is made unrepresentable.

**Flatpak** contributes the cautionary half. Applications build against a shared
versioned runtime and bundle whatever the runtime lacks. It works, and its known
cost is the one to design against: when a vulnerability lands in a bundled
library, **every** application bundling it must be fixed individually, and
nothing centrally knows who bundled what.

### 4.3 Is the proposed shape too complex?

**No, and it is close to right.** "Shared libraries built into signed `.rvt`
artifacts published to the fleet" is, near enough, APEX — an existing, shipping,
decade-tested design for exactly this problem on exactly this kind of device.
Arriving at it independently is a good sign, not a warning.

Two amendments, one addition and one subtraction.

**Add content addressing.** Install library artifacts to a path carrying a
digest — `/state/lib/<name>-<version>-<blake3>/` — rather than a bare name.
Rivet already computes and verifies BLAKE3 digests for every payload, so the
identifier exists; this is a decision about the *path*, taken now, costing
nothing. Without it, two applications needing different versions of one library
is an unsolvable conflict and the first one is a support incident. With it, that
case is ordinary. This is the single highest-leverage line in this document.

**Subtract generality — but only the platform's half of it.** Keep *Rivet's own*
public ABI tiny, explicit and versioned. That is Android's
`public.libraries.txt` lesson, and it is the thing that is expensive to take
back: every symbol the platform promises is one it must keep promising across
image updates.

**This does not restrict who may publish a library artifact, and an earlier
draft of this section wrongly implied that it did.** Android gates APEX
publication because Google is the platform and application developers are
untrusted third parties. **A Rivet fleet has no such boundary**: the fleet
operator owns the boards, the trust store, the signing keys and the
applications. Importing Android's restriction would be importing a threat model
this system does not have.

So the answer to *"can a fleet operator compile whatever libraries she needs
into her own `.rvt`s and have them work"* is **yes** — that is the design
working, not a hole in it. What she puts in her own applications' closures is
her business and may be as large as she likes. What stays deliberately small is
only what **Rivet** promises as stable ABI, because that is the promise nobody
can withdraw later.

### 4.3.1 What still bounds her, and none of it is permission

Three limits survive, and they are worth stating precisely because none of them
is *"you may not"*:

1. **She has to be able to build it for the target.** That is Phase A, and today
   she cannot — which is the entire finding this document opens with. Until the
   toolchain is exposed, "compile any library she chooses" is exactly what both
   agents could not do.

2. **Compiling is not working.** `ADR-003` §6.2's distinction applies to
   libraries exactly as it applies to programs. A library that spawns `sh`,
   reads `/etc/resolv.conf` as a source of truth, or assumes an FHS path will
   compile cleanly and fail at runtime, because Rivet provides none of those
   (`ADR-005` §5.4, `ADR-007` §3.6, `ADR-008` §3). Shared libraries answer *can
   it load*. They never answer *will it work*, and no amount of packaging
   changes that.

3. **She owns what she vendors.** This is Flatpak's lesson arriving as a bill.
   The moment a fleet carries operator-built libraries, somebody must be able to
   answer *"which machines are running a copy of this"* when a vulnerability
   lands. Content addressing plus declared closures is what makes that question
   answerable; a bare-name install path makes it unanswerable, which is the
   second reason §4.3's addition is not optional.

### 4.4 The mechanism, in Rivet's own vocabulary

Sketch, not specification:

* **`kind = "library"`** in the application manifest's Rivet metadata, alongside
  the existing `daemon`. Same `.rvt` container, same signature verification,
  same trust store — a library is a payload, not a new distribution channel.
* **Content-addressed install path**, as above. Never `current`; a library has
  no single current version, which is the entire point.
* **Declared closure.** An application manifest names the library artifacts and
  exact digests it needs. The installer **refuses** an application whose closure
  is not satisfied, naming what is missing — refusal at install, never a loader
  error at start.
* **Per-application resolution.** The application sees its declared closure and
  the platform's public surface, and nothing else. Whether that is `RPATH`, a
  namespace, or a mount is an implementation choice; that it is *enforced rather
  than conventional* is not.
* **Reference-counted reclamation.** A library artifact with no application
  declaring it is garbage. Note `apply uninstall`'s existing ruling that data
  outlives its application: the same question arrives here and deserves the same
  explicitness.

### 4.5 Where this is genuinely hard, stated before anyone is committed

* **Platform ABI versioning across image updates.** An application pinned to
  image N's public surface, running on image N+1. This must be checked **before
  a tryboot commits**, not discovered afterwards, or a routine A/B update
  becomes a fleet-wide application outage — precisely the failure the A/B design
  exists to prevent.
* **Rollback with pinned closures.** Slot A and slot B may carry different
  platform ABI versions. The desired-state document's meaning becomes
  slot-dependent unless closures are resolved per-slot.
* **The bundled-CVE problem.** Flatpak's. If applications may bring arbitrary
  library artifacts, something must be able to answer *"which machines are
  running a copy of this library"*. Content addressing plus declared closures
  makes that answerable; nothing else here does.

---

## 5. Blueprints, not cookbooks

### 5.1 The rung this is missing

`ADR-009` §3's ladder prefers an existing Rust crate, then **an existing C
component, wrapped**, then writing it. Rung 1 needs nothing from this document:
Cargo is already the blueprint, and a pure-Rust crate builds for the target
today. Rung 3 is Rivet's own source.

**Rung 2 is the one that cannot be stood on.** Wrapping a C component means
building that component for `aarch64-unknown-linux-musl`, repeatably, and there
is today neither an exposed toolchain (§3) nor any repeatable way to describe
the build. `ADR-009` §5's gate — *"every dependency must build for
`aarch64-unknown-linux-musl`"* — is therefore being enforced by the absence of a
mechanism rather than by judgement. Both agents in §1 hit exactly this and
resolved it by dropping to rung 1 and finding a pure-Rust substitute.

That substitution was the right call and stays the default. But a ladder whose
second rung cannot be stood on is a one-rung ladder.

### 5.2 Why a blueprint and not a document

**This project has already made this argument once**, in
`tools/coldagent/preflight.sh`:

> A checklist is something the person about to skip it is the one reading.

A page explaining how to build SQLite for Rivet is a cookbook. It is correct the
day it is written, it is never executed, and nothing detects the day it stops
being true — the same failure mode that script exists to prevent in a different
domain. **A blueprint is executed, so it fails loudly.**

A blueprint is a checked-in, runnable definition: the source and its hash, the
configuration and build for the Rivet target, and what it produces. It runs in
the container (§3), which is what stops *"it built on my machine"* from being
part of the answer.

### 5.3 The property that makes it worth doing properly

If a blueprint fully determines its output, then **the blueprint is the
library's identity**, and §4.3's content-addressed path is derived from it
rather than asserted alongside it. That is the Nix derivation model, and it
collapses three separate ideas into one mechanism:

* *"which machines run a copy of this library"* (§4.3.1) becomes answerable by
  construction, because the digest names the inputs;
* rebuilding a fleet's closure after a vulnerability is bumping one blueprint
  and re-resolving, rather than conducting an audit;
* two versions coexisting is ordinary rather than a conflict.

**Blueprints are orthogonal to the linking question and must not be sequenced
behind it.** A blueprint that builds SQLite so an application can absorb it
*statically* is useful immediately, under today's static-only build, with no
loader and no library artifacts. Shared `.rvt`s change what is done with the
output, not how it is produced.

### 5.4 Pinning: what it buys, and the one thing it does not

**A blueprint pins its upstream version and source hash.** Ruled by the
maintainer 2026-08-30, and it is the right default for the reason he gave: a
pinned blueprint keeps working. It is reproducible, it does not rot the way a
cookbook does, and it either builds or fails loudly. Set against rung 2 being
unreachable at all, that is unambiguously better than what exists now.

Rivet already holds this pattern: `tools/container/Dockerfile` pins its base
image by digest and its toolchain to `aarch64--musl--stable-2025.08-1`. A
blueprint pinning its source is the same discipline one level out — and it needs
the build environment pinned too, or one blueprint produces different bytes on
different days and §5.3's identity property is lost.

**What pinning does not buy is knowing when to unpin.** This is the honest
residue of the maintenance objection and it should not be waved away: a pinned
SQLite compiles forever and also carries that version's vulnerabilities forever.
Pinning converts an *availability* problem into a *currency* problem, which is
the better trade — an unbuildable dependency is discovered by a developer at the
worst moment, whereas a stale one is at least discoverable on purpose. And it is
the currency problem that §4.3's content addressing and §4.4's declared closures
make tractable, because they are what let anyone answer *which machines carry
this version* without an audit.

One operational note: upstream URLs die. The recorded hash proves an artifact is
the right one; it does not make it fetchable. Whether Rivet caches blueprint
sources is a separate decision and it should be taken deliberately rather than
discovered.

### 5.5 The risk that remains: do not become a distribution

Buildroot carries thousands of packages and a community to keep them current.
Every blueprint Rivet *ships* is a version, a patch set and a vulnerability feed
somebody must track, and that cost recurs where design work does not.

The scope that survives:

* **The blueprint FORMAT is the deliverable.** A small, well-specified,
  executable definition an operator can write herself is worth more than any
  collection Rivet curates.
* **Rivet ships only what Rivet itself needs**, and says so plainly rather than
  implying a library set that does not exist — `ADR-005` §5.4 again: a partial
  promise invites reliance it cannot support.
* **Rung 1 stays the default.** Much of what would need a blueprint has a
  credible pure-Rust equivalent and `ADR-009` already prefers it. The
  irreducible set is smaller than it first appears, and every blueprint not
  written is maintenance not incurred.

### 5.6 If Rivet goes public: contributed blueprints

Contribution fits this mechanism unusually well, and for one specific reason:
**a blueprint is a recipe, not a binary.** It is text — a source URL, a hash, a
build — so it is reviewable, diffable and arguable in a way a compiled artifact
never is. This is the model Homebrew, Nixpkgs, the AUR, vcpkg and Buildroot all
run on, and it is well understood.

The property that matters most here is that **contribution transfers no trust**.
Whoever uses a contributed blueprint builds it herself, in her own container,
and signs the result with her own key (§4.3). She never installs an artifact
somebody else signed. §4.3's earlier claim — that a fleet has no untrusted
third party — survives going public *precisely because* what is shared is the
instructions and not the output.

**And determinism turns review into something stronger than reading.** If a
blueprint fully determines its output (§5.3), then two people building it
independently must arrive at the same digest. A mismatch is evidence, not a
mystery. That is a better check on a contributed blueprint than any diff
reading, and it is another reason §5.3 is a constraint on the format rather than
a nice-to-have.

What genuinely changes, and none of it is a reason not to do it:

* **A blueprint is arbitrary code execution on the builder's machine.** It runs
  in the container and fetches from the network. The pinned hash protects
  against a compromised upstream; it does **not** protect against a blueprint
  author who pins a malicious tarball and its matching hash. Review is the only
  defence there, and the build environment's own isolation is worth deciding
  deliberately rather than inheriting.
* **Licensing stops being abstract.** A blueprint that builds a copyleft
  component makes obligations concrete for whoever ships the result
  (`ADR-010`). That belongs in the blueprint's own metadata, where it is
  mechanically visible, rather than in a contributor's memory.
* **§5.5's maintenance gravity arrives faster in public than in private.** A
  contribution channel is exactly the thing that turns a format into a
  distribution.

The governance shape that keeps §5.5 true is a **two-tier split**, and it is
worth stating before the first contribution rather than after: a small set
Rivet itself needs and vouches for, and a clearly separate tier that is
contributed, versioned and *not* endorsed. Homebrew's core-versus-tap line, or
Nixpkgs' maintainer split. **Contributed must never silently read as
endorsed** — `ADR-005` §5.4's rule once more, since an implied guarantee about
somebody else's build script is the least supportable promise in this document.

### 5.7 The demand-side survey, measured 2026-08-30

§5.5 asserts the irreducible set is smaller than it looks. That is testable
against real software rather than argued, so it was: the maintainer's own
projects, which are the intended first consumers.

**The Rust projects, from their resolved dependency graphs:**

| Project | Crates | Needing a C toolchain on Rivet |
| :-- | --: | :-- |
| Ignition | 58 | **none** |
| Reliquary | 200 | `ring` |
| Trends | 401 | `ring`, `libsqlite3-sys` |
| TempMon | 450 | `libsqlite3-sys`, plus a GTK desktop stack |

A `-sys` suffix is a poor proxy and most matches are false positives:
`linux-raw-sys`, `dirs-sys`, `js-sys` and `web-sys` compile no C at all, and
`windows-sys`, `core-foundation-sys`, `security-framework-sys`, `jni-sys`,
`ndk-sys`, `objc-sys` and `vswhom-sys` are for platforms that are not this one.
TempMon's GTK stack — `gtk-sys`, `gdk-sys`, `pango-sys`, `libdbus-sys` and
siblings — is a tray application that would not be deployed to a headless board
at all.

**So the Rust side needs roughly two blueprints — SQLite and a TLS backend — to
cover four projects.** §5.5's claim holds. Note that the second may not need
writing: `ADR-009` §3.7 already surveyed the TLS stack for Rivet's own use, and
a project aligning with that decision needs no blueprint rather than a new one.
SQLite is the one both agents in §1 reached for, from opposite directions.

### 5.7.1 And then Aura, which is why it goes first

Aura is not a Rust project. It is a Go controller and a **C++20 renderer** using
Raylib, drawing GPU-accelerated output through **Wayland** under the Cage kiosk
compositor, with **HDMI-CEC** via libcec, currently targeting Raspberry Pi OS.
Its own `docker/Dockerfile.arm64` names the shape of the problem: it
cross-compiles with `gcc-aarch64-linux-gnu` and `g++-aarch64-linux-gnu` —
**glibc, not musl** — against `libgl-dev`, `libwayland-dev`, `libxkbcommon-dev`
and `wayland-protocols`.

Choosing it first is right, and it is worth being explicit that it **exceeds
what this document proposes**. Blueprints would cover part of it — Raylib is
already fetched from source by CMake, and Wayland, xkbcommon, libcec and a
compositor are ordinary C sources to build. Four things are not blueprint
problems at all:

1. **A C++ toolchain targeting musl**, and libstdc++ built against musl. §3's
   Bootlin toolchain includes a C++ compiler, so this is tractable, but it is a
   different claim than "expose the C compiler" and should not be smuggled in
   under it.
2. **A second and third language.** `ADR-003` §6.3's build-target document is
   scoped as though the answer is Rust. Aura needs Go and C++ as well.
3. **A GPU and display stack.** Mesa for the Pi's V3D, a kernel configured for
   DRM, and `/dev/dri` existing at all. That is a kernel-configuration and
   image-composition question (`ADR-002`, `ADR-005`), not a library one.
4. **Device access and a compositor as a service.** A Wayland compositor is a
   long-running system service with privileged device access, which is
   `ADR-015`'s sandboxing model and `ADR-017`'s supervision model, not an
   application concern.

**This is `ADR-003` §6.2's distinction at full scale.** Every library above
could be built and linked and Aura would still not draw a pixel without items 3
and 4. Aura is therefore an excellent first target for exactly the reason it is
an uncomfortable one: it fails in the places where the plan is thin, and it
fails there early rather than after three easy blueprints have made the approach
look proven.

### 5.8 What finishing Aura earns Rivet the right to say

> **Rivet favours Rust. C++ and Go are proven.**

That is a claim worth the work, and it is worth being exact about why this
particular sentence is safe when it sits three sections away from an argument
that partial compatibility promises are worse than none (`ADR-005` §5.4).

**Because "proven" is not "supported".** *Supported* is open-ended: it commits
to software nobody has tried, invites reliance the platform has not earned, and
cannot be checked. *Proven* names an artifact. It says a real application in
that language was built for this target and runs on it, and it points at the
blueprint that did it. A reader can rebuild it and find out.

**And the blueprint is what keeps the claim honest over time.** A documented
claim of language support degrades silently — it is true the day it is written
and nothing detects the day it stops being. A claim whose proof is an executable
blueprint in CI fails loudly the moment it stops being true, which is §5.2's
argument arriving somewhere unexpected: the blueprint is not only how the
library gets built, it is the evidence for what Rivet is allowed to say about
itself.

This also gives `ADR-003` §6.3's build-target document its shape. It is not one
statement about Rust; it is per-language, and each language's section is worth
exactly as much as the artifact behind it — with the honest facilities list
§6.3 already calls for, which is where *"and here is what Rivet does not
provide"* belongs.

Under `ADR-011`, this is the difference between the system stating what it
knows and the system asserting an authority it does not hold.

---

## 6. The display, and a capability that can actually be enforced

Raised 2026-08-30 while building the boot splash, and it belongs here because
Aura (§5.7.1) is the first application that will want the screen.

### 6.1 Nothing restores the display, because nothing owns it

`/dev/fb0` is a mapping of the scanout buffer. There is **no acquire and no
release**: a program writes, and when it exits the last pixels it drew stay on
the screen for ever. So an application that takes the display and quits leaves
its own final frame behind, and the boot splash does not come back.

DRM/KMS is where Linux solves this — a master holds the device and the kernel
restores when it drops — and **`/dev/dri` is absent on this board**, which §5.7.1
already records as one of the four things blueprints cannot supply.

**Restoring is the easy half and it is already possible.** `cmd/rivet-splash` is
a one-shot blit: it opens, draws and exits, holding nothing. *"Go back to the
splash"* is therefore just running it again, and the only real question is who
decides when — which is the supervisor, on the edge where a service that draws
stops.

### 6.2 The permission IS enforceable, and that makes it unusual here

Asked plainly by the maintainer: *should an application have to declare it will
use the HDMI device, and can that be enforced or is it decorational?*

**It is enforceable, and it is the exception rather than the rule.** Measured on
the board:

```text
/dev/fb0   rw-------   root root
```

An application runs as its own uid (`rivet_core::service::APP_UID_BASE`), so it
**already cannot open the framebuffer**. The kernel refuses, today, with no Rivet
code taking part. That is a consequence of `ADR-024`'s devtmpfs with **no udev**:
nothing relaxes the kernel's default `0600 root:root` the way a distribution's
udev rules would with `0660 root:video`.

**Why this differs from the manifest's other keys.** Both cold agents of
2026-08-30 reported the same thing and the packaging tool says it out loud: this
build does not enforce `restart`, `ready`, `state`, `network` or memory. Those
need namespaces, cgroup wiring and a socket-passing supervisor — machinery that
does not exist. **The display needs none of it**, because it is a *filesystem
object* and Unix has arbitrated those since the beginning.

So the capability has two halves and only one of them is work:

| | |
| :-- | :-- |
| **Undeclared may not draw** | already true, by default-deny. Nothing to build |
| **Declared is granted** | the supervisor chowns the node, or adds a supplementary gid, when it starts that service |

The grant is **per boot**, because devtmpfs recreates the node on every boot.
That is the right lifetime: a permission that outlived the manifest that asked
for it would be a permission nobody could audit.

### 6.3 What it does not buy, stated so nobody assumes it

**Exclusivity.** Two applications that both declare the display both get it, and
they interleave — with fbdev neither is wrong, because there is no protocol to
violate. That is also enforceable, but as **policy the supervisor applies** —
refusing to start a second declarer — rather than as a file mode. It is a
different decision and should be taken as one.

**And it is not a security boundary against root.** Anything running as uid 0
writes to the framebuffer whatever the manifest says. The capability describes
what an *application* may do, which is the population it is for.

### 6.4 RULED the same day, and it is NOT far off

`ADR-015` §6.2 carries the ruling: the supervisor arbitrates, one owner at a
time, the splash returns when the owner exits, and **the feature-set is required
before `aura-controller` is implemented**.

**This section first proposed waiting** — *"not solving it now"*, on the grounds
that a second claimant would make the question concrete. The maintainer
corrected that: Aura is intended shortly after wireless, so the second claimant
is one item away and the wait was a guess dressed as prudence. The sequence is
**wireless → display arbitration → `aura-controller`**, and the middle item is
the one nobody had written down.

---

## 7. Recommended sequencing

| Phase | What | Cost | Blocked on |
| :-- | :-- | :-- | :-- |
| **A** | Expose the cross toolchain; write `ADR-003` §6.3's build-target document | Low; toolchain exists | nothing |
| **A2** | The blueprint format, pinned, running in the container (§5) | Low–medium | A |
| **B** | Ship the loader; define and version the public ABI surface | Medium | A |
| **C** | Library artifacts: `kind = "library"`, content-addressed, declared closures | High | A2, B, and §4.5 |
| **D** | Anything about foreign binaries | — | see §8 |

#### The maintainer's actual order, given 2026-08-30

The table above sequences by cost and it is **not** the order this will be
built. His:

> after wifi is shared rvts and aura-controller is THE test case for shared
> rvts.

combined with `ADR-015` §6.2's ruling, gives:

**wireless → shared `.rvt`s → display arbitration → `aura-controller`**

**That moves Phase C from "high cost, later" to next-but-one**, and this
document had it last. Two corrections follow and both are this document's
errors rather than changes of plan:

* **Phase C has a named first consumer and it exists.** Aura is C++20 on Raylib
  drawing through Wayland with HDMI-CEC (§5.7.1) — the hardest case in the
  survey, deliberately chosen as the test rather than a gentle one. A mechanism
  whose first user is the hardest user gets designed honestly or not at all.
* **The phases are not as separable as the table implies.** Aura needs a musl
  C++ toolchain (A), blueprints to build Raylib and Wayland (A2), the loader and
  a public ABI (B) and library artifacts (C) — so "A is worth doing whatever is
  decided about B and C" is true and nearly useless, because all four are on the
  path to the same application.

**A and A2 are still worth doing first**, and they are what today's evidence
calls for. A2 is deliberately not numbered behind
B: a blueprint that builds a C library for an application to absorb *statically*
is useful under today's build, with no loader and no library artifacts (§5.3).

**Two decisions are wanted before Phase C rather than during it**, because each
is free now and a migration later:

* **the content-addressed path** (§4.3) — a path-naming choice today;
* **that a blueprint fully determines its output** (§5.3) — which is what makes
  that path derivable rather than merely asserted, and it constrains the
  blueprint format, so it has to be decided while the format is being written.

Separately, `ADR-026` §7's platform-independence claim needs either honouring or
qualifying for non-x86_64 hosts (§3.1). That is not sequenced here because it is
not on this path — it is a packaging job that blocks an intended MacBook build.

---

## 8. What this does NOT propose

**Running foreign binaries.** `ADR-003` §6.2 stands unamended: shared libraries
answer *can it load*, never *will it work*, and Rivet deliberately does not
provide what Linux software assumes — no POSIX shell (`ADR-005` §5.4), no `/etc`
as a source of truth (`ADR-007` §3.6), no package manager, no FHS
(`ADR-008` §3, §4).

The libc question makes this sharper still. Most third-party Linux binaries are
glibc-linked, and musl is not ABI-compatible with glibc: glibc's internals leak
into the binaries it produces. `gcompat` exists and is a partial answer at best —
its own documentation limits it to the ELFv2 ABI, which most glibc installations
do not use, and musl targets feature-compatibility rather than
bug-compatibility with glibc by design.

So "bring any program with as little friction as possible" resolves into two
goals that must not be run together:

* **Bring any *source* to Rivet and build it here.** Achievable, largely by
  Phase A, and the goal both agents were actually pursuing.
* **Bring any *binary* to Rivet and run it.** A libc compatibility decision,
  `ADR-003` §5's option 3, and not made cheaper by anything in this document.

`ADR-005` §5.4's argument governs: **a partial compatibility promise is worse
than none.**

---

## 9. Sources

External prior art cited above:

* [APEX file format — Android Open Source Project](https://source.android.com/docs/core/ota/apex)
* [Linker namespace — Android Open Source Project](https://source.android.com/docs/core/architecture/partitions/linker-namespace)
* [Namespaces for native libraries — Android Open Source Project](https://source.android.com/docs/core/permissions/namespaces_libraries)
* [How Nix Works](https://nixos.org/guides/how-nix-works/)
* [Dependencies — Flatpak documentation](https://docs.flatpak.org/en/latest/dependencies.html)
* [musl FAQ](https://www.musl-libc.org/faq.html)
* [gcompat](https://github.com/Stantheman/gcompat)
