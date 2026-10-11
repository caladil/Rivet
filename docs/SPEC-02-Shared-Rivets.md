# SPEC-02: Shared Rivets

* **Status:** Normative for `0.1.33` and later. Built and proven on hardware
  2026-08-31. §10, the fleet repository, is built and unit-tested as of
  2026-09-12 and has not run against a machine. §11, addon classes, is built
  and unit-tested as of 2026-09-13 and has not run against a machine either.
* **Date:** 2026-08-31, amended 2026-09-12, amended 2026-09-13
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Artifact kind:** `library`
* **Builds on:** `ADR-015`, `ADR-027` §9, `ADR-028` §5.5, `ADR-029` §11,
  `ADR-053` (§11), `docs/design-candidates/Shared-Libraries-and-the-Build-
  Target.md` §4

---

## 1. Context

`ADR-015` §3 has said since 2026-08-22 that an application has no dependency
graph:

> **A package is one static binary, a typed manifest, optional assets, and a
> signature.** … There is no dependency resolution, because there are no
> dependencies.

That is still true of most applications and should remain the default
(`ADR-003` §5, `SPEC-01` §3). What it could not express is the case the design
candidate's §4.1 names: a library that cannot be rebuilt, a `dlopen` plugin, ten
applications carrying ten copies of one large dependency — and, most immediately,
a C or C++ program that links a shared object because that is what its build
system produces.

**The maintainer's order was `wireless → shared .rvt s → display arbitration →
aura-controller`, with `aura-controller` named as the test case.** This
specification is the second item, and §7 says exactly how far the fourth got.

**Nothing in `ADR-029`'s reader, writer or format changed to add this.** A third
kind is a module and a token, which is §11 being true rather than aspirational —
now demonstrated twice.

---

## 2. What a library artifact IS

A `.rvt` whose header `kind` is **`library`**, carrying:

| Member | Class | Count |
| :-- | :-- | --: |
| `manifest.toml` | `Manifest` = 0, and it is **member 0** | exactly 1 |
| a shared object | `Object` = 1 | 1 .. 16 |
| anything else | `Asset` = 2 | 0 or more |

Same container, same header, same signature, same trust store, same verifier as
an application and an update payload. **A library is a payload, not a new
distribution channel.**

It carries **no `<platform>` component** in its name, for `ADR-027` §9's reason
for an application: it is architecture-shaped, not firmware-shaped.

```text
libgreet-1.0.0-aarch64-library.rvt
```

### 2.1 The manifest

```toml
kind = "library"
soname = ["libgreet.so.1"]
needs = []
```

The same strict subset of TOML an application's manifest is, with the same
argument (`ADR-015` §3.2, item 4): it is a build product, the only legitimate
producer is the emitter, and a parser wider than its emitter is a second grammar
with no second producer.

* **`soname`** — what this artifact provides, at least one. It is read out of
  each object's `DT_SONAME` by `cargo rivet library` and never taken from a
  filename.
* **`needs`** — this library's own closure, in the same form an application's is.

---

## 3. The reference, which is the whole design

```text
libgreet-1.0.0-78ab34279df1e7759fec07fb96efdb13c8f075a464c61865439edd8772e272fa
<name>   -<ver>-<64 lowercase hex>
```

**One string doing three jobs**, and it is written down once:

1. the **closure element** an application declares in its manifest;
2. the **directory** the library installs to on a machine;
3. the **identity** of the artifact.

Nothing recomposes one from parts at install time or at start time. Two
spellings of one reference is exactly the class of bug content addressing exists
to remove.

### 3.1 The digest

**BLAKE3 over `[0 .. SIGNED_BYTES)` of the header** — `ADR-029` §3's signed
region.

That region carries the product, the version, the kind, the architecture **and
every member's digest**, so a digest over it covers the whole artifact
transitively, is computable from 4 KiB rather than from the payload, and is *the
same number the signature already attests to*. A second hash over the whole file
would be a second identity for one thing, which is `ADR-011`'s objection in
miniature.

`rivet_core::package::library::identity` is the function. `cargo rivet library`
computes it by reading back the header of the file it just wrote, and
`cmd/rivet-deploy` computes it from the header that just verified — so neither
can differ from what the other would have said.

### 3.2 It is parseable from the right, and that is a convenience only

The digest is the last 64 characters and the `-` before it separates them, so a
person reading a directory can still tell which library it holds. **Nothing
resolves anything with that split.** A version string legitimately contains
hyphens — `ADR-027` §3.2.2 puts a describe string and a program digest in one —
which is why the split is from the right and why it is used for nothing else.

### 3.3 Why the full digest and not a prefix

A truncated digest would be shorter and would invite the argument about how
short is safe. The argument is worth more than the characters.

---

## 4. Where it installs

```text
/state/lib/<reference>/
    manifest.toml           0444
    package-header          0444   the retained signed header, ADR-015 §4.2.1
    lib/
        libgreet.so.1       0555
```

* **`<volume>/lib`**, a sibling of `app` rather than a child of it: a library is
  not owned by any one application, and putting it under one would make the
  first installer of a shared library its landlord.
* **The inner `lib/` looks redundant and is not.** It is the directory a loader
  is pointed at, and keeping the manifest and the header outside it means the
  loader's search path holds objects and nothing else.
* **An object lands under its MEMBER's name.** This is the one place a library
  differs from an application on purpose: an application's program is named
  after the application because there is exactly one and no `exec` field to
  choose with, whereas a library's objects are found *by SONAME*, so the name is
  load-bearing and cannot be derived.

### 4.1 There is no `current` and there never will be

Design candidate §4.4: *"Never `current`; a library has no single current
version, which is the entire point."*

An application's `current` symlink exists because exactly one version of a
program runs. **Two applications needing two versions of one library is the
ordinary case this design exists to make ordinary**, so a link selecting one
would be a conflict wearing a symlink.

The consequence in the code is a missing field rather than a rule somebody has
to remember: `install::LibraryPlan` has no `Activate` step.

### 4.2 Installing a library that is already there is a no-op and a SUCCESS

The one place a library deliberately behaves differently from an application,
and the difference falls out of content addressing rather than being chosen.

`install::Plan::occupied` **refuses** a second install of an application version
because a version directory is a function of a version *string*, so two
different builds can want one directory and the second would overwrite a running
program. A library's directory is a function of its **bytes**: if the directory
is there, the bytes in it are the bytes being installed, and there is nothing to
write and nothing at risk.

So two applications declaring one library install it once, and no reference
counting is needed to make the second install safe. The outcome token is
`library-already-installed`, and it is the only one in the vocabulary that is
both *wrote nothing* and *succeeded*.

---

## 5. The declared closure

An application's manifest gains one key, **last**, so that a manifest emitted
before 2026-08-31 is not out of order under the canonical-order rule:

```toml
library = ["libgreet-1.0.0-78ab…"]
```

**It is inside the signature**, and `ADR-015` §4.1's reason is sharper here than
for any other key: a closure that could be rewritten outside the signature would
let somebody point a correctly-signed program at code its author never saw.

**Empty is the ordinary answer.** It means the program is statically linked,
which is every Rivet application built before 2026-08-31 and most of them after.

### 5.1 Refusal at install, never a loader error at start

`cmd/rivet-deploy` **refuses** an application whose closure the machine cannot
satisfy, and **names what is missing**:

```text
REFUSED: this application declares 1 library and 1 of them is not on this
machine. NOTHING WAS WRITTEN.

Missing:
  libgreet-2.0.0-56939218889a356254166f6f095dc760f77e29c3b391879a70d2e0dd89cbadd8
```

The alternative reaches an operator as a service that will not run, whose only
account is a loader saying it cannot find a file, with nothing saying which
artifact to go and fetch.

**The check runs after every member verifies and before anything is written.**
The order is the argument: the manifest is a member, so reading it before it
verified would be trusting bytes the signature had not yet vouched for, and
refusing after a write would leave an unrunnable application on the machine.

### 5.2 Satisfaction is a `stat`, not a search

A reference **is** a directory name, so the installer asks one question per
reference — *is `<volume>/lib/<reference>` there* — and no directory is listed.
That is a property of content addressing rather than an optimisation: there is
no matching rule to get wrong.

### 5.3 The build tool refuses earlier still

`cargo rivet package` reads the program's `PT_INTERP` and `DT_NEEDED` and:

* refuses an interpreter that is not `/lib/ld-musl-aarch64.so.1` — a glibc
  cross-compiler's output, which would fail at `execve` with `ENOENT` and
  nothing to diagnose it by;
* refuses a dynamic binary that needs something outside `PUBLIC_LIBRARIES` and
  declares no closure at all;
* reports the SONAMEs beside the declared references otherwise.

It cannot check that the declared libraries *provide* those SONAMEs, because a
reference names an artifact the build tool does not have. The board can and
does.

---

## 6. Resolution, and exactly what it is not

`cmd/rivet-sysd` composes **`LD_LIBRARY_PATH`** from the object directories of
the declared closure, in the order the author wrote them, and from nothing else.
`rivet_core::service::library_path` is the function.

`None` rather than an empty string when the closure is empty: an empty
`LD_LIBRARY_PATH` is not the same as an absent one — the loader reads it as a
single empty path, meaning the current directory.

### 6.1 This is resolution. It is NOT isolation.

It decides what the loader **finds**. It does not stop a program that wants to
from opening a path it can read: `/state/lib` is root-owned and
world-searchable, because the unprivileged uid an application runs as has to
traverse it — the same argument `ADR-015` §4.2 makes about `/state/app`.

Enforced resolution in the loader is Android's linker namespaces and needs the
namespace machinery `ADR-015` §6 records as not existing on this build.

**Saying so is `ADR-005` §5.4's rule.** A partial promise about isolation is
worse than none. What this *does* promise is that an application's declared
dependencies resolve to the exact bytes it declared, which is the property
content addressing exists for.

### 6.2 Why an environment variable is not an `exec` key wearing a hat

`ADR-015` §3.2's rule is that a manifest may not name a *program*, because the
signature covers the bytes of one binary and naming another would escape it.
A closure entry cannot do that: it is content-addressed, so the only thing a
manifest can point at is a directory whose name is a digest of an artifact this
machine verified and installed. An author who writes a reference for bytes the
machine does not hold gets a refusal at install, not a different program.

### 6.3 A reinstall's new closure reaches the definition — found by doing it

A definition is built once, when discovery reads a manifest, and
`installed::reconsider` took exactly one field out of a re-reading:
`enabled`. That was harmless only while every other manifest-derived field was
either unenforced or derived from the name and therefore unable to go stale.
**The declared closure is the first manifest value that reaches `execve`.**

On 2026-08-31 `greeter` was reinstalled declaring `libgreet-2.0.0-…`, `current`
pointed at the new manifest, the board answered
`library = ["libgreet-2.0.0-…"]` — and `/proc/<pid>/environ` answered
`LD_LIBRARY_PATH=…libgreet-1.0.0-…`.

`Supervisor::take_application_program` closes it. **Nothing is restarted**: a
running process keeps the environment it was `execve`'d with and cannot be given
another, so the change applies at its next start. Whether an install should
restart a running application is `ADR-017` §6's decision and `PLAN.md` §19.4's
open question.

---

## 7. The platform's side

### 7.1 The loader

**`/lib/ld-musl-aarch64.so.1`**, shipped in the image, `0755`, root-owned.

Not a choice: it is the string the cross toolchain writes into a dynamically
linked binary's `PT_INTERP`, and the **kernel** opens it before the program's
first instruction. On musl the loader *is* the C library, so "ship the loader"
turned out to be one file.

It is **optional** in `cmd/rivet-image`, and a card without it is a supported
card: every Rivet binary is static, so such an image boots, converges and is
reachable exactly as every image before 2026-08-31 did. What it cannot do is run
a dynamic application — and the installer's closure check is what says so.

`tools/card/build.sh` verifies it landed by reading `/lib` back off the finished
image with `debugfs`. That check is new and `/lib` is the first directory
outside `/system/bin` the build installs into.

### 7.2 The public surface

`PUBLIC_LIBRARIES` is `["libc.so"]`. See `SPEC-01` §4.

### 7.3 Trust

The application key is entitled for `application` **and** `library`. One key,
because they are one trust domain: a library is a payload an application author
publishes for their own applications. `ADR-028` §5.5's separation of the SYSTEM
key from the application one is untouched — the system key may sign neither.

**A card built before 2026-08-31 refuses every library install at the
entitlement check, naming the key.** That is the correct answer for that card
and it is not a silent one.

---

## 8. What is NOT built

* **Automatic reclamation.** §12 removes a library that a **newer build of its
  own product** supersedes, and §13 removes one an operator names. A library no
  application declares, nothing supersedes and nobody names stays installed —
  an addon whose accepting application was uninstalled remains until it is
  removed by name. Design candidate §4.4's last bullet, and it inherits `apply
  uninstall`'s open question about data outliving its application.
* **Platform ABI versioning across image updates.** §4.5's first hazard: an
  application pinned to image N's public surface running on image N+1. The
  surface is one library and musl is ABI-stable, so nothing has broken; **that
  is a property of the current surface and not a mechanism.**
* **A `dlopen` story.** Nothing stops it and nothing supports it.
* **A fleet-wide answer to "which machines carry this library".** `cargo rivet
  fleet drift` answers it for one machine at a time (§10.5); nothing asks every
  machine and collects the answers. Design candidate §4.3.1 item 3 is why that
  matters, and it is a fleet-side job.
* **Blueprints.** `SPEC-01` §7.

---

## 9. What was proven, on hardware, 2026-08-31

Board `devpi`, `0.1.33-2934f1d`. Each read off the machine:

| | |
| :-- | :-- |
| the loader | `/lib/ld-musl-aarch64.so.1`, 1027520 bytes, `0755`, arriving by A/B update and confirmed by pid 1 |
| trust | `may_sign: ["application","library"]` |
| a library installed | `/state/lib/libgreet-1.0.0-78ab…/lib/libgreet.so.1` |
| an application resolving it | `greeter: tick 0 — libgreet 1.0.0 — resolved through a Shared Rivet (answer 42)` |
| the environment it got | `LD_LIBRARY_PATH=/state/lib/libgreet-1.0.0-78ab…/lib` |
| an unsatisfied closure | refused at install, naming the missing reference, with the running version untouched |
| two versions coexisting | `libgreet-1.0.0-78ab…` and `libgreet-2.0.0-5693…`, side by side |
| the closure changing | after reinstall and restart, `greeter: tick 0 — libgreet 2.0.0 …` |

`examples/probes/libgreet` and `examples/probes/greeter` are the artifacts, and they are also
the first evidence behind `ADR-015` §3.1's four-month-old claim that a non-Rust
binary is a valid package. It is C, and it is dynamic.

---

## 10. The fleet repository

`ADR-052` is the decision that a fleet repository holds every artifact and that
deployment selects from it; `PLAN-Feature-Set-2026-09-12` §2 decision 6 puts
shared rivets in it and decision 7 makes them deliberate. This section is what
a `library` artifact is inside that repository. Numbered after §9 rather than
beside §4, so that no section above moved.

### 10.1 Publishing

    cargo rivet fleet publish libgreet-1.0.0-aarch64-library.rvt --trusted-key <key> [--at DIR]

A library is published by the same verb and the same code as an application:

* **Verified as `cargo rivet inspect` verifies it** — the header signed by the
  one key given, every member's bytes the ones the signature covers, the §2
  shape (`Library::open`: member 0 the manifest, one to sixteen objects, no
  platform), the §2.1 manifest parsed, and the §3 reference recomputed from the
  header that verified. A file that fails any of it is refused and nothing is
  stored.
* **The key given is trusted for every kind.** That is `inspect`'s posture — an
  author checking their own artifact — and not a machine's trust: §7.3's
  entitlement is checked by the machine, at its store's door and at install.
* **Stored at `objects/<digest>`**, where `<digest>` is BLAKE3 over the whole
  file (`ADR-052` §3). The bytes are never committed to git.
* **Recorded in `manifest.toml`** as one `[[artifact]]` with the header's
  `product` (the library's name), `version`, `arch`, `kind = "library"`, no
  `platform`, the file's `size`, and when it was `added`.
* **Idempotent by digest.** Publishing the same file again reports it and
  changes nothing.

### 10.2 Two digests, and what each one names

A library in a fleet repository has two BLAKE3 digests, over two different
ranges, and **they are different numbers**:

| | computed over | names | where it appears |
| :-- | :-- | :-- | :-- |
| **the store digest** | the whole `.rvt` file | the artifact as a transfer and a stored object | the manifest's `digest`; `objects/<digest>` in the repository; `/state/artifacts/objects/<digest>` on a machine (`ADR-052` §7.2.1); what `apply install` names |
| **the reference digest** | `[0 .. SIGNED_BYTES)` of the header (§3.1) | the artifact as a library | the last 64 characters of the reference; `/state/lib/<reference>`; an application's `library = [...]` |

**The manifest records the store digest only.** A library's reference is
derived from its stored object's header whenever a tool needs it —
`cmd/cargo-rivet`'s `library::reference_of`, the function `cargo rivet
library` prints references with — rather than kept as a second field that could
disagree with the bytes. Nothing converts a reference into a store digest.

### 10.3 From the repository to a machine

    cargo rivet install --artifact <digest or unambiguous prefix> [--at DIR] --host … --identity … --pin … confirm

1. **The workstation** resolves the entry, and hands `install` a copy of the
   object named by `ADR-027` §7's grammar with the `library` kind token —
   `libgreet-1.0.0-aarch64-library.rvt` — recomposed from the manifest's fields
   rather than from whatever the file was called when it was published.
2. **The transfer** puts it in the machine's artifact store under its store
   digest, through the door's three gates (`ADR-052` §7.2.3): the digest, the
   signature with the key's entitlement to sign a `library` (§7.3), and the
   architecture. The store keeps the three newest of each product, a library
   included, grouped by the name's `<product>` (`ADR-052` §7.2.2).
3. **The install** names the store digest. The installer verifies again, sees
   `kind = "library"`, and writes §4's directory — or answers
   `library-already-installed` (§4.2).

Three consequences, each a property of the code rather than a rule to remember:

* **`--enable` is refused for a library.** A library is not a service;
  `rivet_remote::deploy` refuses `--enable` for any kind but `application`.
* **Deploying an application sends that application and nothing else, and
  refuses first when the machine cannot satisfy its closure** (`.rulings/`
  R-157 R15). Before anything is sent, deploy reads `/state/lib` the way
  `cargo rivet fleet drift` already does, and refuses when a declared
  reference (§5) is not there — naming every missing one and the exact command
  that deploys it. **This is a check, never an automatic send**: nothing on
  the workstation fetches a library on an application's behalf, so each
  library is still deployed on its own, by its own `--artifact`. §5.1's
  refusal at install remains the backstop for whatever reaches a machine by
  some other route.
* **A library `.rvt` given to `deploy --rvt` is published first.** `deploy`
  verifies it against the application signing seed, stores it in the
  repository as §10.1 describes, and sends the repository's copy by step 1's
  path (`ADR-052` §7). A library the repository already holds is not an error.

### 10.4 What the repository holds is a library by its bytes

The repository does not read a library's manifest after publish and does not
index SONAMEs. Which library an application needs is the reference in that
application's own signed manifest (§5); the repository is where the artifact
with that reference is kept, and §10.2's derivation is how a tool finds it.

### 10.5 Drift

`cargo rivet fleet drift` compares the libraries a machine holds with the
libraries the repository holds, beside its comparison of applications:

* **The machine side** is one `get file /state/lib`. Every entry whose name
  parses as a §3 reference is one installed library. An entry that does not
  parse is listed under its own heading and compared with nothing; a listing
  the machine cut (`entries_truncated`) says so, and what it did not name is
  not reported as absent. A `/state/lib` with nothing at it is zero libraries.
* **The repository side** is every `kind = "library"` entry, with its reference
  derived from its object (§10.2). An entry whose object is missing refuses the
  whole comparison before the machine is asked anything: skipping it would
  report its library as unpublished.
* **`LIBRARY MATCHES`** — the installed reference is, character for character,
  a published one. The digest is part of it, so a different build of the same
  version is not a match.
* **`LIBRARY NOT MATCHED`** — no published reference is this exact string
  (`.rulings/` R-157 R17). Where a published library's name followed by `-`
  begins the installed reference's `<name>-<version>`, the line carries a
  labelled hint — *"closest published name: libgreet (a guess from the
  name)"* — with every reference that product holds printed beside it; the
  longest matching name wins when more than one could apply. Without a
  matching name, the reference is printed alone.

**The hint reads the name out of a reference, which §3.2 says is a convenience
for a person.** It decides only whether a hint is worth printing, never
whether something matches — every non-match is `LIBRARY NOT MATCHED`, hint or
not. Because a version is free text, a name that is itself a published name
plus a hyphen — `libgreet-extra` beside `libgreet` — carries that hint; that is
pinned by a test rather than hidden.

A `get file /state/lib` that fails is reported as the libraries being
unreadable, and the applications beside them are still reported.

### 10.6 What has run

**Unit tests, on real signed artifacts and a fake machine.**
`cmd/cargo-rivet/src/fleet_tests.rs` publishes a library built with real BLAKE3
and Ed25519 and checks what the manifest, `list` and `deploy --artifact` make
of it; `fleet_drift_tests.rs` publishes one and compares it against a `get file`
answer in each of the shapes above. **No library has been published into, or
deployed from, a fleet repository on a board or in QEMU**, and no drift has read
a real machine's `/state/lib`.

---

## 11. Addon classes

`ADR-053` is the decision; this section is what it requires of a library
artifact and of discovery. Numbered after §10 rather than beside §5, so that
no section above moved.

### 11.1 The declaration

A library's manifest gains one optional key, **last**, so that a manifest
emitted before this section is not out of order under the canonical-order
rule (§2.1's `soname` and `needs` are unaffected):

```toml
provides-class = "temperature-sensor"
```

`crate::package::library::Manifest::provides_class` reads it back. Absent
means the same as it does for every other optional key in this project's
manifests: this library joins no application by class. An application's own
`accepts-class` is the mirror of this key, in
`rivet_core::package::application::Manifest` — `ADR-053` §3 has both —
**except that the application's side is a LIST** (`.rulings/` R-162 R28):
an application may accept several classes, `["temperature-sensor",
"humidity-sensor"]`, while a library still declares exactly one — a library
provides one kind of thing.

### 11.2 Matching is exact, and it is a fresh reading every time

A class is a plain string, compared byte for byte
(`rivet_core::service::addon_library_path`) against every entry in the
application's accepted-class list. There is no prefix match, no
case-folding and no version component — a library whose class is
`"temperature-sensor-v2"` does not join an application that accepts
`"temperature-sensor"`, on `SPEC-02` §10.5's own precedent: `LIBRARY NOT
MATCHED` is exact-or-nothing there too, and a hint is cosmetic rather than a
second matching rule. **A repeated entry in the accepted list cannot join
one installed library twice** — matching walks the installed libraries, not
the accepted list, so there is nothing for a duplicate to multiply.

Nothing about a class is cached. `crate::libraries::discover_installed`
lists `<volume>/lib` and reads every found library's manifest at each
discovery pass, the same pass that already re-derives every application's
definition from what is on `/state` now (`crate::applications::discover`).
An addon installed after a machine last booted becomes available at the next
discovery — including the one `crate::installed::Watch::look` runs
immediately after an install finishes, with no reboot required.

### 11.3 Where a class-matched library resolves to, and which object in it

Exactly where an explicitly declared one does — `crate::package::install::
library_object_dir(volume, reference)`, §4's install layout. The two
mechanisms differ only in how the reference is found: §5's declared closure
names one by exact bytes in the application's own signed manifest; a class
match finds it by asking every installed library what it provides. Both
land in the same `LD_LIBRARY_PATH`, the declared closure first and any
class-matched libraries after, sorted by reference for a result that does
not depend on the order a directory listing happened to return.

**A library artifact may carry sixteen objects (§2) and exactly one of them
is the addon: the library NAMES it** (`.rulings/` R-201). The manifest gains
a second optional key, after `provides-class` and last in the canonical
order for §11.1's reason:

```toml
soname = ["libpack-codec.so.1", "libpack.so.1"]
provides-class = "aura-background-pack"
addon-soname = "libpack.so.1"
```

The key is named for the key its value comes from: **its value must be one
of `soname`'s entries, character for character.** The objects it does not
name are the library's own private dependencies rather than further addons
— naming them all would make one installed artifact indistinguishable from
several installed artifacts of one class, which §11.5 refuses.

**A library carrying exactly one object writes no key**, because there is
nothing to choose between and a key would be a second spelling of a fact the
manifest already states once. That is the shape every addon in this project
ships as, and it is why the rule needs no compatibility shim.

Three shapes are **refused**, at `cargo rivet library` when the artifact is
emitted and again by `Manifest::parse` when a machine reads one back:

| Refused | Because |
| :-- | :-- |
| a class, several objects, and no `addon-soname` | nothing may choose for the author, and the refusal names every object the library carries, because *which one should I have said* is the next question |
| an `addon-soname` naming an object the library does not carry | a name with nothing behind it installs perfectly and is handed to a host that cannot open it |
| an `addon-soname` with no `provides-class` | nothing would ever load it, so the author believes they have shipped an addon and has not |

**The order of a list is a weak contract**, and that is the whole of why the
key exists. An addon join is a thing two signed manifests depend on; resting
it on which object a build happened to emit first fails silently and much
later, when a reordered build produces a perfectly valid artifact that joins
the wrong object and nothing says so until something behaves oddly on a
board. `rivet_core::package::library::Manifest::addon_object` is the one
place the resolution is written, and `crate::libraries::discover_installed`
asks it rather than reading `soname` itself.

### 11.4 The host is TOLD which objects it was joined to

`ADR-053` puts a directory on `LD_LIBRARY_PATH`; it does not say what is in
it. The supervisor therefore also composes **`RIVET_ADDONS`**
(`rivet_core::service::ADDONS_ENV`) into the application's environment:

```text
RIVET_ADDONS=aura-background-pack=/state/lib/<ref-a>/lib/libpackone.so.1:aura-background-pack=/state/lib/<ref-b>/lib/libpacktwo.so.1
```

* **`<class>=<absolute path to the object>`, colon-separated.** Colon is what
  a list separator already means in a handed variable here — `ADR-017` §3.4's
  `LISTEN_FDNAMES` — and the rule that comes with it holds: **no token may
  contain the separator**, which no class and no install path can.
* **Sorted by reference**, on §11.3's determinism argument, and composed from
  the same matched list as `LD_LIBRARY_PATH` and the Landlock ruleset
  (`rivet_core::service::addon_matches`) — so a directory is named exactly
  when it is searched and exactly when it is granted. Those first two
  disagreed once, on a board, and the object `dlopen` reported missing was
  plainly there.
* **Absent, never empty, when nothing joined** — an empty variable is a value
  a careless reader turns into one nameless entry.
* **A library with no addon object is left out of the naming** and still
  joins the path and the ruleset; the supervisor says so once, at `Info`.
  §11.3's refusals mean no artifact this build emits or accepts can be in
  that state, so the sentence exists for the day the manifest grammar widens
  — the alternative to it is a supervisor that panics to say the same thing.

`rivetos_addon::host::installed(class)` reads it back, and
`rivetos_addon::host::Loaded::open_path` opens one of the objects it names.
**An application whose supervisor sets nothing falls back** to the naming
rule this replaced, `lib<class>.so.1`, because an application is installed
independently of the image that supervises it and a host built today may run
on an older board.

### 11.5 Two libraries of one class

**Both join, and the host decides what that means.** Every library whose
class matches reaches the path, the ruleset and `RIVET_ADDONS`; the platform
arbitrates nothing, for `ADR-053`'s original reason — an arbitration rule
invented without a case in front of it is a policy nobody wrote.

What has changed since `ADR-053` is that the host can now *see* both, so the
two shapes of host are both expressible:

* A host that wants **every** library of a class — a renderer drawing through
  each installed background pack (`.rulings/` R-199 D1) — enumerates and opens
  each by path.
* A host that wants **the** library of a class calls `Loaded::open(class)`,
  which opens the one when there is one and **refuses, naming every
  candidate, when there are several**. `.rulings/` R-200 makes a class a role
  and a role one library, so two is an operator having installed two things
  for one job — and a host that picked silently would answer *which one* by
  accident on the next boot.

### 11.6 What this does NOT do

* **It does not read a class out of `SPEC-02` §10**'s fleet repository, and
  the fleet repository does not index it. A class is matched **on the
  machine**, from what is actually installed — the same reason §10.4 gives
  for the repository not indexing SONAMEs.
* **It does not let an addon call into its host.** The host calls the addon,
  through `rivetos-addon`'s three symbols; the addon answers and initiates
  nothing. `ADR-053` §5 named that whole boundary as unbuilt and it is half
  built now — the host's direction exists, the addon's does not, and the
  addon holds no handle it could call back through.

---

## 12. Retention — one build per product, and a declared library is never evicted

`.rulings/` R-202. **A machine keeps one build of each library product under
`<volume>/lib`, and removes the rest at the next install, naming each one it
removes.**

### 12.1 Why it exists, and why it bit immediately rather than rarely

§3 makes a library's identity the digest of its bytes, and **Rust builds are
not byte-reproducible across rebuilds**. So *the same source, built twice* is
two references and two directories — not an edge case but the ordinary result
of rebuilding anything.

Both directories carry the same `provides-class`, so §11.5's
`Loaded::open(class)` refuses to choose between them. It is right to: R-200
makes a class a role and a role one library, and a host that picked silently
would answer *which one* by accident on the next boot. But the consequence,
before this section, was that **shipping a new version of an addon put out the
card that addon feeds**, and nothing removed a library but `apply
remove-directory` against `/state/lib` by hand. Updating an addon had no
supported form at all.

Retention closes it from the other end: a rebuilt addon supersedes its own
product's older build at install, so **the two-of-one-class state never arises
from an upgrade**, and §11.5's refusal is left doing the job it was written
for — two genuinely different products claiming one role.

### 12.2 The rule

Exactly `ADR-052` §7.2.2's arithmetic, over a second store, and
`rivet_core::artifact::evicted` is the one implementation both use:

* **grouped by product** — the `<name>` half of a reference (§3), split from
  the version by the rule `ADR-027` §7's file name already uses: the version
  begins at the first `-`-separated token that begins with a digit;
* **ordered by version**, componentwise over the numeric core, so `0.3.10` is
  above `0.3.9`;
* **a version this build cannot order is retained apart** from those it can,
  and a reference carrying no version at all forms a group of one — neither is
  ever deleted first for not being understood, and neither pins by outranking;
* **installation time is the tie-break and only the tie-break**, taken from
  the directory's modification time. `ADR-021`: a Pi's clock is a floor that
  never goes backwards, which orders two installs within one machine's life
  and says nothing about a date;
* **every eviction is reported by name**, in the supervisor's journal, with
  the path and the rule that removed it.

**One, not `ADR-052` §7.2.2's three, and the difference is not an
inconsistency.** That constant keeps three `.rvt` payloads of a product in the
machine's *artifact store* so that a rollback is local, and it still does:
rolling a library back is installing the older digest the store already holds.
This one governs how many are **unpacked at once**, and any number above one
leaves exactly the state the rule exists to prevent.

### 12.3 The guard, which is what makes it safe

**A library named by reference in any installed application's declared closure
is never evicted**, whatever retention would otherwise say.

The two kinds of library are not alike here:

| | how the machine found it | what eviction costs |
| :-- | :-- | :-- |
| a class-joined addon (§11) | **discovered** — nothing names it; the machine chose it by reading manifests | nothing: the next start joins whatever is installed |
| a declared library (§5) | **named by exact reference inside a signed manifest** | that application does not start — §5.1's failure, arriving later and by another route |

So retention walks every installed application's closure first and takes
anything named there **off the table**, rather than holding it back inside the
ordering. The difference is load-bearing: a protected library that still
occupied its product's one place would evict the version an operator had just
installed for an application about to arrive — and §5.1 would then refuse that
application for an unsatisfied closure. Retention would be producing the exact
refusal §5.1 exists to raise, against itself.

**The cost of that choice, stated.** A library product that is *both* declared
by one application and class-joined by another can still end up with two
installed — the declared one and one retained build — and §11.5's refusal
applies to them. Nothing on any machine is in that shape today, and the
alternative breaks §4.1's ordinary case, which is the one this design exists
for.

### 12.4 A fault never evicts

`cmd/rivet-sysd`'s `retention` module, at the install edge, before discovery
re-reads what is installed — so the `LD_LIBRARY_PATH` and `RIVET_ADDONS` that
discovery composes (§6, §11.4) describe the set that is left.

**Every failure to learn something ends the sweep with nothing removed**, and
that is the opposite of the posture §5's readers take. An application whose
manifest cannot be read is an application whose closure is unknown, and a
guard computed from an unknown closure is a guard with a hole in it; a board
that needs a directory tidied by hand is a smaller problem than one whose
application will not start after the next reboot.

**It runs at an install and not at a boot.** An install is the operator asking
for something, which is what licenses a sweep to delete; a supervisor that
pruned `/state` on its way up would be removing an operator's artifacts in the
one pass nobody is watching.

---

## 13. Removal — an operator names a library and it goes

```
rivet apply uninstall-library <product|reference> [confirm]
```

**A product name removes every build of that product under `<volume>/lib`; a
full reference removes exactly that build.** Without `confirm` it plans and
changes nothing. With it, the request is handed to the supervisor, which
decides again, removes, and recomposes every service's library path **without a
reboot**. §12 retires a build its own newer build supersedes; this is how a
library nobody supersedes is retired on purpose — an addon that is no longer
wanted, or one replaced by a different product of the same class (§11.5).

### 13.1 The selector

* Parsed as a **full reference** (§3) first, then as a **product name**. A
  product name obeys the component rule a reference's name does — letters,
  digits, `.`, `_` and `-`, not beginning with `.` or `-`, never containing
  `..`, at most 160 characters — so a selector is always one path component
  below `<volume>/lib`.
* A product selects by §12.2's grouping: `aura-background-pack` never selects
  `aura-background-pack-extra`.
* Every directory under `<volume>/lib` that parses as a reference is
  selectable, including an install interrupted before its header was written.
  An entry a removal has set aside is never selected.

### 13.2 What refuses it

| Refused | Because | Code |
| :-- | :-- | :-- |
| nothing under `<volume>/lib` matches | there is nothing to remove; the refusal lists what is installed | `E_MACHINE_REFUSED` |
| an installed application's **declared closure** (§5) names a build it selects | that application would not start at its next start — §12.3's guard, for §12.3's reason | `E_UNSAFE` |
| an installed application's manifest cannot be read or parsed, or the application root cannot be listed | the closure is unknown, and a guard computed from an unknown closure has a hole in it (§12.4) | `E_MACHINE_REFUSED` |
| `<volume>/lib` exists and cannot be listed | what is installed is unknown, and an unlistable root is not an empty one | `E_MACHINE_REFUSED` |

**A refused request removes nothing**, not even the builds that happened to be
safe.

**An application that only accepts the library's class (§11) does not refuse
it.** Removing an addon on purpose is what the verb is for. The plan names each
accepting application against the build it would lose, and that application
starts without it next time — §11's class join finds what is installed, so
nothing else changes.

### 13.3 The channel, and who decides

* `/state/uninstall-library-request` is a **twin** of `apply uninstall`'s
  request, not a generalization of it: mode 0600, written to a temporary and
  renamed, at most 1024 bytes, one line `from=<client> [key=<label>] --
  <selector>` with exactly one word after the marker. One request waits at a
  time; a second is refused while the first is on the card.
* **The command decides first, and the supervisor decides again when it takes
  the request up; the supervisor's answer governs.** An application can be
  installed between the two, naming a build the request selects. Both call one
  function over one closure reader, `rivet_core::package::closure`, which
  retention (§12) uses too, so the plan and the act cannot disagree about what
  an application declares.
* The supervisor **removes the request before acting on it**, so a crash costs
  one request and never a loop, and does not take one up while the machine is
  stopping — the request stays on `/state` for the next boot.

### 13.4 Removal, and the library set without a reboot

* Each selected build is **renamed aside and deleted by a helper**
  (`ADR-017` §3), and journalled by name with its reference and path. A build
  that cannot be removed is journalled as still installed, and the request as
  only partly done.
* **Then the supervisor re-runs the install edge's discovery** — without
  retention, because nothing was installed — so the `LD_LIBRARY_PATH` and
  `RIVET_ADDONS` every service is given (§6, §11.4) describe what is left.
* **Nothing running is restarted.** A running process keeps the environment it
  was started with, and restarting it is the operator's decision
  (`apply service-restart`). The journal names each running application that
  had a removed build on its library path or joins one by class, and says it
  takes the change at its next start.
* **The artifact store is not touched.** The library's `.rvt` stays in
  `/state/artifacts`, so putting it back is `apply install <digest>`.

### 13.5 Who may ask

Whoever may ask for `apply uninstall`, and nobody else: an authenticated
control-plane peer may; the break-glass console and a build with no machine
refuse both alike.

### 13.6 What it does not do

* **A removal made by hand through the file surface is still invisible until a
  reboot.** Only an install and this verb re-run discovery; `apply reload`
  re-reads the document and not the library set.
* It does not restart consumers, and it does not reclaim anything nobody named
  (§8).

---

## History

### In §8 — "a library artifact no application declares is garbage and nothing collects it"

**Rewritten 2026-09-19 by `.rulings/` R-202**, which built §12: a machine now
keeps one build of each library product and removes the rest at the next
install. The sentence was written when nothing under `<volume>/lib` was ever
removed by anything, and reading it as still true would have left an agent
believing an upgrade could only ever add a directory — which is precisely the
belief that made shipping a second build of an addon dark a card. The bullet
keeps its place because the *reference-counted* half is genuinely unbuilt:
retention answers *"a newer build of this product arrived"* and nothing
answers *"nobody needs this any more"*. It read:

~~**Reference-counted reclamation.** A library artifact no application
declares is garbage and nothing collects it. Design candidate §4.4's last
bullet, and it inherits `apply uninstall`'s open question about data outliving
its application.~~

### In §11.3 — a class-matched library joined by its FIRST declared object

**Superseded 2026-09-19 by `.rulings/` R-201**, hours after it was written.
The enumeration channel (`PLAN-Addon-Feature-Set-2026-09-19` T2) had to name
one of a library's up-to-sixteen objects to its host and took the first
declared one, which made the order of a build product's list into a contract
two signed manifests depend on. The maintainer ruled an explicit key
instead, because that contract fails **silently and much later**: a
reordered build emits a perfectly valid artifact that joins the wrong
object, and nothing says so until something behaves oddly on a board.
Nothing had shipped under the old rule — no signed manifest carried a
multi-object addon library — so no artifact anywhere was built against it,
and there is no compatibility shim. The paragraph read:

~~**A library artifact may carry sixteen objects (§2) and exactly one of
them is the addon: the FIRST entry of `soname`.** A library that provides a
class provides one kind of thing, so the objects after the first are its own
private dependencies rather than further addons — and naming them all would
make one installed artifact indistinguishable from several installed
artifacts of one class, which §11.5 refuses. The rule is written here rather
than inferred because `soname`'s order is otherwise only a build product's
order, and a reader deserves to know that it is load-bearing.~~

**What survived into the body** is the half of that paragraph the new rule
still rests on: the objects a library does not join by are its private
dependencies, and enumerating all of them would make one artifact look like
several addons of one class.

### In §11.4 — "it does not let an addon call into its host, or a host call into an addon"

**Superseded 2026-09-19**, when `rivetos-addon` shipped (`.rulings/` R-196
through R-198) and the host's direction of that boundary was built. The
bullet claimed neither direction existed; one now does, and a reader acting
on the old text would have concluded there was no way for an application to
use an addon at all. The bullet read:

~~**It does not let an addon call into its host, or a host call into an
addon.** `ADR-053` §5 names that boundary and says why it is a separate,
unbuilt pass.~~

### In §11.4 — "it does not resolve a conflict between two installed libraries of one class"

**Superseded 2026-09-19** by §11.5. The platform still arbitrates nothing,
which is why the paragraph's reasoning survives above rather than here; what
changed is that a host could not previously tell there were two, because it
derived one filename from the class and asked the loader for it. The bullet
read:

~~**It does not resolve a conflict between two installed libraries of one
class.** Every library whose class matches joins; nothing here chooses one
over another or refuses a second. `ADR-053`'s own report flags this as a
question to stop at if it is ever forced by a real addon, rather than
guessing an arbitration rule with no case in front of it.~~

### In §8 — "which machines carry this library"

**Superseded 2026-09-12**, when `cargo rivet fleet drift` began comparing
libraries (§10.5). The bullet read:

~~**Anything that answers "which machines carry this library".** The information
is now *there* — the digest is in the path — and no tool asks the question.~~

It was true until drift read `/state/lib`; what remains unbuilt is the
fleet-wide collection, and the body says so.

### In §11.1 — an application's `accepts-class` was one string

**Superseded 2026-09-13** (`.rulings/` R-162 R28): the section originally
described a single value on both sides of the join, mirroring `ADR-053` §4's
original argument that a `Vec` cost nothing to declare but nothing in the
plan needed it. The maintainer ruled the application side into a list while
no signed manifest yet carried the key, so there was no installed base the
narrower form was protecting — `ADR-053`'s own History has the full
argument. `provides-class` was not part of the ruling and is unchanged: one
library, one class.
