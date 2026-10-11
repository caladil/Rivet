# The addon call boundary: what `§6.3` needs before it can be built

* **Status:** Design candidate. **Nothing is built and nothing is decided
  here** — this states what the tree already settles, what it leaves open, and
  the choices that are the maintainer's.
* **Date:** 2026-09-19
* **Ruled:** `.rulings/` R-196 — the addon feature set starts now, and the
  boundary is built before any of the reading source's code moves.
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Why this document exists

`PLAN-Feature-Set-2026-09-12` §13 settles the **shape** of the reading-source addon in
six bullets: it leaves the Aura controller for its own repository as a shared
rivet the controller loads, the controller interrogates it, credentials go
through the controller's existing mechanism, and a fault in it takes the screen
down. §6.3 settles the **argument** for how the crossing must work: a minimal
pointer-and-length interface with versioned serialized messages above it,
because Rust promises no type layout between separate compilations.

Neither is buildable as written. §6.3 is four paragraphs of reasoning with no
interface in it, and §13's interrogation protocol is nine words. This document
is the pass between them: **four findings from the tree that a build plan has to
answer**, the requirements the reading source's code itself imposes, and the
questions that are choices rather than derivations.

**The constraint that makes the order matter.** The reading source is not dead
code. The controller is polling and reporting on `rivet-01` today: Rivet
Monitor's captured journal for the board holds the controller's own reading
lines through the day. Whatever is built here has to land without taking that
off the wall, which is what R-196 D2 chose.

---

## 2. What is already decided, and is NOT reopened here

| Decision | Where | What it means for this work |
| :-- | :-- | :-- |
| An addon is a **library the host loads**, not a process claiming a slot | `PLAN-Feature-Set-2026-09-12` §13 | The crossing is in-process. Nothing here designs a socket |
| Which library joins which application | `ADR-053` §2, and it is built | `accepts-class` / `provides-class`, exact string match, fresh at every discovery |
| The crossing is a pointer and a length, with versioned serialized messages | `PLAN-Feature-Set-2026-09-12` §6.3, `ADR-053` §6 | No direct Rust interface across the boundary, and no shared Rust types |
| Credentials go through the controller's existing mechanism | `PLAN-Feature-Set-2026-09-12` §13 | The addon never opens a secret itself |
| An addon runs **as** the controller | `PLAN-Feature-Set-2026-09-12` §13 | One fault domain, accepted knowingly. The host's manifest is the addon's whole permission surface |
| A class name is free text, compared byte for byte | `ADR-053` §4 | No namespacing scheme to design |

---

## 3. Four findings from the tree

### 3.1 The join hands over a search path, not a list of addons

`rivet_core::service::addon_library_path` returns **one string** — the addon
segment of `LD_LIBRARY_PATH`, every class-matched library's object directory,
sorted by reference. `SPEC-02` §11.3 is explicit that a class-matched library
lands exactly where a declared one does, and the supervisor's job ends there.

So the host learns **where** to look and never **which** objects are there, or
**whether** there are any. `LD_LIBRARY_PATH` resolves a name the loader has
already been asked for; it cannot be enumerated, and a host cannot have been
built knowing the soname of an addon written after it.

**Consequence: an enumeration channel is part of this boundary.** §7 Q1 is the
choice.

### 3.2 The loader has only ever been used at exec time

Every library that has run on a machine — `examples/probes/libgreet` against
`examples/probes/greeter`, on hardware 2026-08-31 (`SPEC-02` §9) — is a
**declared closure resolved by the loader when the program starts**, from a
reference the application's own signed manifest names by digest. An addon is
the opposite case by construction: the host's manifest cannot name it, which is
the whole reason `ADR-053` exists. So the host opens it at run time, and
`SPEC-02` §8 says in four words where that stands — nothing stops it and
nothing supports it.

The tree's only mention of the call is in the image builder, computing what an
application needs:

    grep -rn "dlopen" --include=*.rs cmd rivet-core rivet-sdk examples

— two hits, both in `cmd/rivet-image/src/system.rs`, and both making the same
point: **a `dlopen`ed object is invisible to a `NEEDED` closure**, so nothing
computes it. That is a fact to design around rather than a defect. The host's
closure check will never see an addon, and the guard that an addon's own
dependencies are satisfiable is the one that runs when the **library** is
installed (`SPEC-02` §5) — which is where it belongs, because the library is
what carries them.

### 3.3 The host has to stop being a static binary

`aura-controller`'s manifest declares `library = []`, and its own comment says
what that means: static musl, no loader, no shared library, no closure. **A
statically linked musl program cannot `dlopen`** — its `dlopen` is a stub that
fails — so the controller as it exists today cannot load an addon whatever the
interface looks like.

The move to a dynamic binary is smaller than it sounds, and every piece is
already on the machine:

* the loader ships in the image, root-owned `0755`, A/B protected
  (`rivet_core::package::library::LOADER_PATH`, read off a board in `SPEC-02`
  §9);
* `libc.so` and `libgcc_s.so.1` are both in
  `rivet_core::package::library::PUBLIC_LIBRARIES`, so a dynamic binary needing
  them passes the installer's closure check;
* a dynamic application has run on a board — `SPEC-02` §9's table.

**What the tree has no instance of is a dynamic _Rust_ application**, and §6
proves one rather than assuming it. Two commands say so, which is what this
claim rests on rather than a reading:

    grep -rn "^library" --include=Cargo.toml examples

— eighteen manifests, seventeen of them `library = []`. The one exception is
`examples/probes/greeter`, and its own first line says it is a C program whose
Cargo.toml exists only to carry Rivet metadata. So the dynamic case has been
proven for C, and the Rust toolchain half of it — `-crt-static` off, the loader
in `PT_INTERP`, the closure the installer then checks — is exercised by
nothing the same grep turns up: the same command above names every dynamic
manifest in the tree, and none of them is Rust.

### 3.4 The interrogation protocol is nine words, and the code asks for more

§13 names three things the controller asks an addon for — admin markup, where
it persists, the cadence it will be called on. The reading source's code that
has to survive the move needs more than three, and §4 derives the list from it
rather than inventing one.

---

## 4. What the reading-source addon needs the boundary to carry

Read out of `examples/aura/aura-controller/src/` — `main.rs`'s poller thread,
the relay client and its policy module, `portal.rs`'s save handler — and the
renderer's `controller.rs`.

| Capability | Where it lives today | What the boundary must offer |
| :-- | :-- | :-- |
| **A cadence with backoff** | the poller thread: `POLL_INTERVAL`, `TERMINAL_RETRY`, and a throttle whose retry-after is taken as a floor | the addon states when it wants to be called again, per call — a fixed interval cannot express *the relay just told us to wait* |
| **A reading, held raw** | the shared state holds the value and the instant it was taken | one message per call, carrying the reading and the instant, never a rendered string |
| **Failure that is not a crash** | `Held::failed()` and a complaint written at most once per window | a refusal the host records and survives, since §13's fault domain makes a panic a black screen |
| **Settings** | the source's enabled flag, its display unit and its two thresholds, typed in `config` | the host holds the values; the addon declares what it has and reads them back |
| **Admin markup** | `portal.rs`'s section for the source and its save handler | §7 Q4: markup the host embeds, or a field schema the host renders |
| **A wake on save** | `Nudges::wake` on the source's nudge — a saved setting takes effect at once instead of at the next interval | the host can call the addon out of band, and the addon must accept being called while a poll is due |
| **A credential** | the relay credential, an installed secret reached by path from config | the host opens it and hands over the material; §13 already rules this |
| **Egress** | `ureq` inside the controller, on the manifest's `network = "egress"` | nothing new: the addon's socket is the host's, and the host's manifest is the whole permission request |
| **Persistence** | the reading is in memory and nowhere else | a directory under the host's `state = "private"`, named by the host |
| **The journal rule** | the value is kept out of the journal; trend and age only | the host's logging call, so an addon cannot write a series into the log by accident |

**One thing the boundary does not have to carry: a card.** The controller sends
a reading object on its own wire and the **renderer** composes the card — in
`examples/aura/aura-renderer/src/controller.rs`, with a 900-second TTL, drawn
through the generic composition. So the addon takes no
dependency on `aura-card`, and `docs/RELEASE-BLOCKERS.md` §3 does not stand in
this addon's way. It still stands for a slot addon, which is a different thing.

**What is a real choice: where the policy lives.** The policy module turns a raw
reading into a status using the thresholds and two staleness constants. That
knowledge is the source's rather than Aura's — but the object it produces is
Aura's wire shape, and a shared wire type across two repositories is the
coupling §6.3 exists to prevent. §7 Q3 is the question.

---

## 5. The shape this proposes

Stated so there is something concrete to attack; every part of it is subject to
§7.

* **One exported C symbol**, taking a pointer and a length and returning a
  pointer and a length, each owned by the side that allocated it.
* **A version negotiated in the first call**, and a host that refuses an addon
  whose version it does not understand — with a journal line naming the object
  and both versions, because §13's fault domain makes the alternative to a
  refusal a black screen.
* **A message vocabulary** above it: `Describe` (what are you, what settings do
  you have, what admin surface), `Configure` (here are your settings and your
  material), `Poll` (a reading, or a refusal, and when to call again).
* **Generated glue**: the addon author implements an ordinary Rust trait and a
  macro exports the symbol; the host uses a matching helper. Neither side
  hand-writes an `extern "C"` function.
* **The host owns every clock, every file and every socket.** The addon is
  called; it does not run a thread, open a file or keep a timer.

---

## 6. First articles — run 2026-09-19, and what they found

`examples/probes/addon-probe` (a `cdylib` library artifact declaring
`provides-class`) and `examples/probes/addon-probe-host` (an application
declaring `accepts-class`), following `libgreet` and `greeter`'s precedent.

**In the container, natively — passed.** The handshake, a description, state
surviving between calls, a cadence the addon chose (2 s while unconfigured,
60 s once ready), and a panicking addon answering with a fault while the host
carried on.

**Cross-built for the board — two things no document said.** A `cdylib` for
`aarch64-unknown-linux-musl` needs `-crt-static` off *and* the Buildroot `gcc`
as the linker, because `rust-lld` has no sysroot to resolve `libc.so` in and
says only `unable to find library -lc`. The pair then closes against exactly
`PUBLIC_LIBRARIES` — `libc.so` and `libgcc_s.so.1`, interpreter
`/lib/ld-musl-aarch64.so.1` — so a **dynamic Rust application installs with
`library = []`**, which is §3.3's question answered by measurement.

**On `rivet-06f5c` — it found a platform defect, which is what a first article
is for.** The application installed and ran, the supervisor's class match put
the addon's directory on its `LD_LIBRARY_PATH`, and `dlopen` failed with *no
such file or directory* against a real object that was plainly there.
`rivet_core::confinement::policy` granted the **declared closure only**, so the
loader searched a directory the kernel refused — and because musl reports the
last attempt's errno, a Landlock refusal reads as an absent file. `ADR-053`
§8's own admission (unit-tested; never discovered on a real machine) is where
this was always going to surface.

Fixed by composing both from one function — `service::addon_library_dirs`, which
`addon_library_path` joins and the policy grants — so a directory reaches the
loader's path exactly when it reaches the ruleset. Granted `Optional` rather
than `Required`: an addon removed between a discovery pass and the next spawn
must not stop its host starting.

**Still owed:** the same run on the board with the fix installed, and the
version-mismatch refusal on hardware.

---

## 7. The questions that are the maintainer's

* **Q1 — enumeration.** How does a host learn which addon objects are installed?
  An environment variable the supervisor sets beside `LD_LIBRARY_PATH` (one line
  per object, with its class); an SDK call that asks the supervisor; or a fixed
  soname per class, so a host accepting `reading-source` opens
  `libreading-source.so.1` and needs no list at all. The third is the smallest,
  and it turns two addons of one class into a collision by accident rather than
  by rule.
* **Q2 — the cadence.** The addon returns *call me again in N seconds* per call
  and the host owns the timer; or the host calls on a fixed interval the addon
  declared once. The relay's throttle is the case that decides it.
* **Q3 — where the policy lives.** The addon returns a raw reading and the host
  applies thresholds and staleness; or the addon returns the finished status and
  the host passes it through. The second keeps the source's knowledge with the source
  and needs a shared message shape that is not Aura's own wire type.
* **Q4 — the admin surface.** The addon hands back HTML the portal embeds, or a
  typed field schema the portal renders. Markup is flexible and makes an addon
  part of the portal's attack surface; a schema is narrow and makes every addon
  look the same.
* **Q5 — the message encoding.** Anything self-describing and versioned will do;
  the choice is between one already in this tree and one added for this.
* **Q6 — the slot collision.** `PLAN-Feature-Set-2026-09-12` §13 records that
  `aura-vitals-test` claims the tertiary slot the reading card will take, and the
  platform refuses the second claimant at launch. Repointed or retired, in the
  same change.
* **Q7 — `SPEC-04` §4.5.** It still describes the reading source as a slot addon drawing
  its own card, and the thermometer as a socket contributor, which §13 replaced.
  `docs/RELEASE-BLOCKERS.md` §2 cites §4.5 as the ruling of record. Confirming
  that §13 supersedes it lets both documents be rewritten to one architecture.

---

## 8. Rejected alternatives

| | Why not |
| :-- | :-- |
| **Link the addon at build time** | It is then a dependency rather than an addon: installing one would mean republishing the host, which is the friction `ADR-053` §1 exists to remove, and it couples two repositories' builds |
| **A direct Rust interface across the boundary** | `ADR-053` §6 — no cross-compilation-unit layout guarantee, and the bet fails as memory corruption rather than as a refusal |
| **The addon as its own process, over a socket** | The superseded slot-addon shape. It survives a fault, which is genuinely better, and it is not what §13 ruled. Recorded here so the trade stays visible |
| **A host that scans the addon directory itself** | It would make the host's view of what is installed a second authority beside the supervisor's discovery, which is what `ADR-011` refuses |

---

## 9. What this changes elsewhere, when it is ruled

* `PLAN-Feature-Set-2026-09-12` §13's *not this release* is overtaken by R-196.
* `SPEC-02` §8's *a `dlopen` story* leaves that list, and `SPEC-02` gains the
  section that specifies it.
* `ADR-053` §5 states today that the crossing is a separate pass; the ADR that
  accepts this document is what its §5 will point to.
* `SPEC-04` §4.5 and §12, and `docs/RELEASE-BLOCKERS.md` §2, are rewritten to
  §13's architecture — subject to Q7.
