# ADR-003: libc and Linking Strategy

* **Status:** Accepted. §5 ruled 2026-08-22 — see §6, which corrects §3
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`ADR-001` rejected the `#![no_std]` posture, so Rivet's Rust binaries link a
libc. That makes libc a given. It does **not** settle which one, nor — more
importantly — whether libc is a *system component* or a *link-time detail*.

Those two are very different operational animals, and the difference is most of
what this ADR is about.

---

## 2. Options

| Option | libc present as | Cost | Notes |
| :-- | :-- | :-- | :-- |
| glibc, dynamic | `libc.so` + `ld-linux-*` in the image | Version skew, NSS, loader | The conventional distribution shape |
| **musl, static** | **absorbed into each binary** | **larger binaries; no dlopen** | **Chosen** |
| `no_std` + `rustix` | absent | loss of `std` | Rejected in `ADR-001` §3 |

---

## 3. Decision

**Every Rivet Rust binary is statically linked against musl, targeting
`aarch64-unknown-linux-musl`. The image contains no dynamic loader and no
shared libc.**

The consequences are checkable, and being checkable is the point:

* `/lib` is empty or absent.
* There is no `libc.so` and no `ld-musl-aarch64.so.1` anywhere in the image.
* `ldd` reports `not a dynamic executable` for every binary Rivet ships.

That is a materially different claim from "we use musl". libc is not something
the operating system *has*; it is something each binary *absorbed*.

### 3.1 Why musl over glibc

* **Size and auditability.** musl is on the order of a few hundred kilobytes of
  unusually clean, MIT-licensed C. glibc is millions of lines, LGPL, and
  designed around assumptions a minimal image does not share.
* **It avoids the NSS trap.** glibc's name-service switch `dlopen`s its modules
  at runtime, so a statically linked glibc binary calling `getpwnam` — which
  any daemon that resolves a user does — either fails or silently needs the shared objects it
  was supposed to have eliminated. musl does not do this. Given `ADR-005`
  ships C daemons that do exactly that, this is not a theoretical concern.
* **No version skew.** With nothing dynamically linked, there is no symbol
  versioning surface and no possibility of a binary built against one libc
  meeting another at runtime.

### 3.2 Direct syscalls

Where `rustix` is used with its `linux_raw` backend, syscalls are issued
directly and bypass libc entirely even while musl is linked in. This is not a
purity exercise; it reduces the amount of C actually executed on hot paths and
narrows the surface that has to behave correctly. It is a preference, not a
rule, and it does not change this ADR's decision.

---

## 4. Consequences

* **Rivet cannot run arbitrary prebuilt Linux binaries.** With no dynamic
  loader present, anything shipped as a dynamically linked ELF simply will not
  execute. This is mostly a feature — it is a hard guarantee about what is in
  the image — but it is a real capability that is being given up.
* **Concretely, this breaks:** VS Code's remote-development server (which downloads a
  glibc-linked Node server), vendor-supplied binary blobs, and anything pulled
  from a container image built against glibc.
* **Third-party C components must also be built static against musl.** This
  applies to every Tier B and C member in `ADR-001` §4 — `wpa_supplicant`,
  BusyBox. Each becomes a build recipe the project owns.
* **Binaries are larger.** Irrelevant on the Pi 4; relevant if the Zero W
  compatibility target in `ADR-002` §3 is ever exercised, and a reason to
  measure image size from the start.

---

## 5. Open question

**Is a dynamic loader retained as a deliberate escape hatch?**

The static-only position is clean and defensible, and it is the default this
ADR adopts. The question is whether the inability to run *any* foreign binary
ever proves too expensive in practice — the VS Code remote-server case is the one
most likely to bite, since it is a normal way to work on a remote host and it
is not recoverable by trying harder.

Three shapes, undecided:

1. **Static only.** Accept the constraint permanently. The control plane is the
   supported way to work on a Rivet host.
2. **Loader present, unused.** Ship a musl dynamic loader that nothing in
   Rivet links against, purely so foreign binaries can run. Cheap, and it
   weakens the guarantee in §3 from "there is no loader" to "we do not use it".
3. **A declared compatibility layer.** An explicitly opt-in, separately
   installed glibc environment. Most honest, most work.

This must be decided before anything depends on the guarantee in §3 being
absolute. It is cheap now and expensive later. Tracked in `docs/PLAN.md`.

---

## 6. Ruled, 2026-08-22: static now, not static for ever — and Rivet is a build target

The maintainer's ruling separated two things this ADR had run together, and the
separation is the important part.

**What he ruled.** Static linking stands for day one, but it is a dependency to
be broken sooner rather than later — plausibly not until there is a working
system worth optimising for release — and in the meantime nothing may get sloppy
or assume shared libraries will never be needed. Separately: he does **not**
intend Rivet to be a platform that just any Rust code can target, because a great
deal of Rust code assumes POSIX-like resources Rivet does not provide. His
inclination is therefore that software is **built for the target**.
*(`.rulings/` R-009.)*

### 6.1 Static is the current build, not a permanent invariant

Static linking stands for Stage 0 through 2. **Shared libraries are an expected
future option, not a rejected one**, most likely revisited when a working system
is being prepared for release — where the motivation is size rather than
compatibility. Every Rivet binary carrying its own copy of musl, `std` and the
common crates is real duplication once `uutils`, `nushell` and several
`rivet-*` binaries share an image, and image size is a tracked number
(`ADR-002` §5.1).

**This corrects §3.** That section listed "`/lib` is empty or absent" and "`ldd`
reports `not a dynamic executable`" as checkable guarantees, in a tone implying
permanence. They are **properties of the current build**, true today and useful
for reasoning about today's image. They are not invariants, and nothing may be
designed in a way that assumes them for ever. The original text is left readable
because the reasoning in it — particularly §3.1 on musl versus glibc — is
unaffected and still governs.

### 6.1.1 An addon host is dynamic, necessarily — 2026-09-19

§6.1 expected shared libraries to arrive as a **size** decision, revisited when
a working system was being prepared for release. The first application to need
them needed them for a different reason, and the reason generalises:

**A statically linked musl program cannot open an addon.** musl's static libc
supplies `dlopen` as a stub so that callers still link, and it refuses every
argument it is given. So a host that loads a shared rivet by class
(`ADR-053`, `.rulings/` R-196 to R-198) has to be dynamically linked — not to
save space, and not to run foreign software, but because the mechanism does
not exist in a static binary at all.

The cost is small and was measured rather than estimated on 2026-09-19: a
dynamic Rust application's closure is `libc.so` and `libgcc_s.so.1`, both in
`rivet_core::package::library::PUBLIC_LIBRARIES`, and its interpreter is the
`/lib/ld-musl-aarch64.so.1` the image already ships. So such an application
declares `library = []` and the installer's closure check passes with nothing
declared — the guarantee in §3 bends exactly as far as this needs and no
further.

**Two applications are dynamic today** — `aura-renderer`, which links `libdrm`,
and `aura-controller`, which became a host. The first was a device driver's
requirement and the second is the general rule, so the sentence worth
remembering is the one in this heading rather than a list of names.

### 6.2 The more important ruling: Rivet is a build target

Shared library support and *running foreign software* are different questions,
and this ADR previously conflated them. They separate cleanly:

| | Question | Affected by shared libraries? |
| :-- | :-- | :-- |
| **Loading** | Can the binary begin executing at all? | yes |
| **Working** | Will the program actually function? | **no** |

Shared libraries would only ever address the first. The second is decided by
what Rivet *provides*, and Rivet deliberately does not provide much of what
Linux software assumes: no POSIX shell (`ADR-005` §5.4), no `/etc` as a source
of truth (`ADR-007` §3.6), no package manager (`ADR-008` §4), no FHS
(`ADR-008` §3). Rust is no protection here — a crate that spawns `sh`, reads
`/etc/resolv.conf` directly, or assumes an FHS path will compile perfectly and
fail at runtime.

**So Rivet does not promise to run foreign binaries, and should never imply
that it does.** Software for Rivet is *built for Rivet*.

This is the same argument as `ADR-005` §5.4's refusal to ship a partial POSIX
environment: **a partial compatibility promise is worse than none**, because it
invites reliance it cannot support and its failures get attributed to the
software rather than to the platform. Under `ADR-011`, claiming to be a
general-purpose Linux runtime would be asserting an authority Rivet does not
hold.

### 6.3 Consequence

If software must be built for Rivet, then **Rivet must eventually be
documentable as a build target** — the triple, the sysroot, and an honest
statement of which platform facilities exist and which do not. That last part is
the valuable half and it is the part nobody writes. It is the reason §6.2 is a
design commitment rather than a disclaimer, and it was needed well before Stage
3 — see §6.3.1.

#### 6.3.1 DISCHARGED 2026-08-31 — `docs/SPEC-01-Rivet-as-a-Build-Target.md`

**This section once said the document was "not needed before Stage 3"** (see
History). That was disproved by two agents rather than argued with. On
2026-08-30 two agents from different vendors, with
no shared context, were each told to put a database-backed application on a
Rivet board. Both rejected SQLite, both for the same unprompted reason — no C
cross-toolchain they could find — and one asked for this document **by name**.

**A C and C++ cross toolchain was already in the build environment**, produced
by Buildroot and used by `tools/card/build.sh` since 2026-08-28, and it had never
been exposed to anyone building an application. `tools/toolchain/env.sh` is that
exposure and it changes no decision in this ADR: no linking rule moves, no
loader ships because of it, and §3's guarantees are untouched.

`SPEC-01` is the document, and §5.2 is the honest-facilities half this section
called valuable.

#### 6.3.2 And §6.1's "expected future option" arrived — `SPEC-02`

Shared libraries are built as of 2026-08-31 (`SPEC-02-Shared-Rivets.md`), on the
maintainer's order of `wireless → shared .rvt s → display arbitration →
aura-controller` rather than on §6.1's timetable.

**§6.1's motivation is NOT the one that arrived.** That section named *size* —
several binaries sharing one copy of musl. That is still not done and is still a
fair goal. What was built is for **applications**: a program that links a shared
object because that is what its build system produces gets one, signed and
content-addressed, and Rivet's own binaries remain statically linked exactly as
§3 and §5 rule.

**§6.2 is unamended and governs `SPEC-02` completely.** Shared libraries answer
*can it load*; they never answer *will it work*. `SPEC-01` §6.1 is that
distinction with a measurement behind it: a Go binary that started, ran, read its
configuration, and then failed on a facility this platform does not provide.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §6.3 — the build-target document was scheduled after Stage 3

**Changed 2026-08-31**, discharged by `SPEC-01`. The superseded clause read:

~~Not needed before Stage 3,~~

The reason it went is §6.3.1's: two agents from different vendors independently
rejected SQLite for want of a C cross toolchain that already existed and had
never been exposed, which is a Stage-1 cost rather than a Stage-3 one.
