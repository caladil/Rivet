# ADR-013: Where Shared Abstractions Live

* **Status:** Accepted — the homes and the placement test are ruled. The
  platform crate they call for is built, as `rivetos-sdk`; `ADR-036` is its
  decision record and §6 is the reconciliation.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`ADR-007` describes a configuration model — layered values with precedence,
per-value provenance, desired state separate from observed state, schema
generated from types — that is considerably more elaborate than "load a TOML
file". `AGENTS.md` section 8.4 requires Ignition and divides responsibility as
*the project owns the schema, Ignition owns the mechanics*.

The obvious question was how much of that model Rivet should push into
Ignition. **The maintainer refused the binary** *(`.rulings/` R-033)*:
extending Ignition is right wherever an idea genuinely ought to be abstracted,
and Ignition must not be **polluted** with what only one operating system's
programs want. A crate elsewhere may hold the ideas that are shared across
applications on Rivet, with Ignition as one of several such crates — extended
where it needs to be, and consumed for what it already provides.

**"Extend Ignition" and "keep Ignition clean" are in tension only while there is
nowhere else for a Rivet-shaped abstraction to live.** Naming those homes, and
the test that decides between them, is what this ADR settles. The binary choice
was the wrong frame.

**Where the boundary falls is a fact about the machine a program runs on, not
about who wrote the program.** Ignition holds premises a Rivet denies on purpose
— a home directory, an environment to layer, a writable filesystem, an operator
at a terminal, a binary responsible for its own updates — so a program running
*on* a Rivet takes what it needs from the SDK, and a program that talks *to* one
from a workstation takes Ignition. Both kinds are in this repository:
`cmd/rivet-fleet` and `cmd/rivet-provision` name Ignition and run on a
workstation, and the programs a Rivet supervises take `rivet-sdk`. What Ignition
costs on a board was measured — §7 item 3 for a boot process, and `ADR-036` §4
pillar by pillar for an application.

---

## 2. Decision: four homes, and a test that decides between them

| Home | Holds | Audience |
| :-- | :-- | :-- |
| **Ignition** | abstractions any project of the maintainer's would want, on any platform | Athena, Olympus, Tachyon, Trends, and the tools that talk to a Rivet from a workstation |
| **The Rivet SDK** — `rivetos-sdk` (`ADR-036`) | what it means to *be a program on Rivet* | any software built for Rivet, his or a stranger's |
| **Girder** — chartered, `docs/design-candidates/Girder.md` | how the maintainer structures *his own* programs on Rivet | his applications, and nothing a template teaches |
| **Rivet itself** | what it means to *be* Rivet | Rivet only |

### 2.1 The placement test

One question, asked before anything is written:

> **Would Athena or Trends want this?**
>
> * **Yes** → Ignition.
> * **Only if it were running on Rivet** → the Rivet SDK.
> * **Only the maintainer's own programs would want it, and only on Rivet** →
>   Girder.
> * **Only Rivet could possibly want it** → Rivet.

That is checkable, it names real consumers rather than hypothetical ones, and it
gives "don't pollute Ignition" a definition instead of a feeling. A proposal to
extend a shared crate that cannot name a second existing project that wants the
extension has failed the test.

**The fourth row is the one that needs a discipline under it**, because *the
maintainer's own programs* is a category an agent can stretch to cover anything
he touched. Two rules keep it honest:

* **`examples/templates/` is the operable test.** A template is what a stranger
  copies to start, so it demonstrates the platform: **a thing that belongs in a
  template belongs in the SDK**, and a template linking a preference library
  would teach one person's taste as though it were the contract. Girder is what
  remains after that test refuses.
* **Several of his applications overriding one SDK default is evidence about the
  SDK.** The question to ask is whether an application that reasonably wanted
  the other answer would be badly served by the override. If it would not be,
  the default is a gap and the row is the SDK's.

The fourth row exists because a home was found for something neither Ignition
nor the SDK should carry: `docs/design-candidates/Girder.md` is the survey that
measured it, across four applications of his that had each solved the same
problems separately.

---

## 3. Applying it to what is already designed

| Capability | Home | Reasoning |
| :-- | :-- | :-- |
| Structured logging, configuration loading, a standard CLI surface, self-update of a single binary | **Ignition** | it holds all four, for a program on a machine with a filesystem, an environment and an operator |
| **Layered config with precedence** | **Ignition — extension** | Athena and Trends would both want "this value came from the config, overriding the default" |
| **Per-value provenance and `--explain`** | **Ignition — extension** | same argument; it is the useful half of layering |
| **Schema generated from types** | **Ignition — thin** | every project wants `--schema`; `schemars` does the work, Ignition wires it |
| Closed verb vocabulary (`ADR-006` §6.4) | **Ignition — extension**, tentatively | general, but opinionated enough to want a second consumer first |
| Service registration and lifecycle | **the SDK** | only meaningful on a host with Rivet's service manager (`ADR-017` §3.3) |
| Receiving configuration and settings from the system | **the SDK** | an app on Rivet does not go looking for a file; it is handed what it declared (`ADR-036` §3, `ADR-038`) |
| Receiving **secrets** from the system | **the SDK** | `ADR-020` §8 — an app never opens a secrets file nor hardcodes a path |
| Emitting logs into the system journal | **the SDK** | `ADR-022` §4 |
| Reading machine state from inside the sandbox | **the SDK** | `ADR-036` §8.2 — declared, allow-listed and read-only |
| Exposing an app's own operations to `rivet` | **the SDK** | this is what makes Rivet a platform rather than a distribution. §6.4 rules it the one part of the app-side contract a library must carry, and it is the row with nothing behind it |
| Bridging another logging framework onto the journal; treating an absent data directory as fatal; warning on a setting that will not parse | **Girder** | the platform hands over the journal and the accessor. Which framework a program brings, and what it does when a directory is empty, is the author's answer and several of his agree on it |
| Desired vs observed state | **Rivet** | an application does not have observed state in this sense |
| `plan` / `apply` / drift detection | **Rivet** | system management, not application concern |
| A/B image update and rollback | **Rivet** | see section 4 |
| The operation catalogue itself | **Rivet** | it is the catalogue *of Rivet's* operations |
| A `Kernel` port and its `Errno`, shared between Rivet's own boot binaries | **Rivet** | a foreign application never links these, so the SDK is the wrong home. §7 item 1 |

The encouraging result: **the elaborate parts of `ADR-007` split cleanly.**
Layering and provenance are genuinely general and improve Ignition. Desired
versus observed state is genuinely system-specific and would have been the
pollution.

**The Ignition rows are placements, not adoptions.** A row saying *Ignition —
extension* says where the capability belongs if anyone builds it, and each needs
Ignition's own ADR, its own version and a tag consumers pin (§7 item 2). None of
them is Rivet's to write, and Rivet does not wait on any of them: what a program
on a Rivet gets instead is the SDK row above it.

---

## 4. Image update stays in Rivet

Ignition's self-update replaces one binary with a newer copy of itself.
Rivet must replace an entire `/system` image, attempt a boot, and fall back
automatically if it fails (`ADR-008` §3). These are not the same problem at two
scales; they are different problems that share a verb.

**The image updater is Rivet's**, and it fails the section 2.1 test outright —
no other project of the maintainer's has partitions to swap.

If it later turns out that some *part* of it generalises — signature
verification, or a download-and-verify pipeline — that part is extracted then,
with a real second consumer to shape it. That is `AGENTS.md` section 8.4's
"extract the shared thing early" read correctly: early means *at the second
consumer*, not before the first one is finished.

---

## 5. The failure mode this prevents

Worth naming, because both halves of the maintainer's instruction guard against
a different one.

* **Refusing to extend Ignition** produces duplicated layering code in five
  projects, diverging quietly. This is the failure the first half of his answer
  prevents.
* **Extending Ignition for Rivet's convenience** produces a general-purpose
  library carrying operating-system concepts that four of its five consumers
  will never use, and that constrain it for ever. This is the pollution the
  second half prevents.

A Rivet-shaped home is what makes both avoidable at once, and its existence is
the actual decision in this ADR.

**The second failure has since been priced rather than imagined.** `ADR-036` §4
takes Ignition's five pillars against what a Rivet offers a program and finds
three dead or wrong, one inert, and the survivor a thin wrapper over two crates
— *"not a port; it is a rewrite that would keep the name"*. That is what
extending Ignition for Rivet's convenience would have cost, measured on a probe
rather than argued.

---

## 6. The platform crate is the Rivet SDK

**The platform crate this ADR calls for is `rivetos-sdk`, and `ADR-036` is its
decision record.** What it is, in the words this ADR wrote before it existed:
**the library that software built for Rivet links against in order to be a
well-behaved citizen of a Rivet host** — to receive its configuration, emit
structured logs, be supervised, and expose its own operations through the same
administrative surface as the system's. `ADR-036` §2 states the same crate from
the other end: it encodes Rivet's promises — paths, environment variables,
socket protocols — and none of Rivet's implementation.

**The name `rebar` is retired**, along with the argument about how to choose it.
This ADR named a crate that had not been written; `ADR-036` wrote one and used
neither that name nor this document, which is the gap the Girder survey in
`docs/design-candidates/` found. A naming argument for a crate that exists
under another name settles
nothing, so §6.2 and §6.3 are retired rather than reconciled and `## History`
says why. **What survives the retirement is the scope constraint (§6.1) and the
line between what requires the crate and what does not (§6.4)** — both of which
bind `rivetos-sdk` exactly as they were written to bind its predecessor's name.

### 6.1 Ruled 2026-08-22: Rivet-specific, not a general hosting contract

The scope question was settled before the name, because it constrains it.

**It is Rivet's, permanently.** The semantics it carries — socket passing,
journal delivery, readiness, the operation catalogue — are shapes *this project
chose*, not a general contract for supervised applications. A name promising
generality it does not have would be the same dishonesty this record refuses
everywhere else (`ADR-005` §5.4, `ADR-003` §6.2).

**`rivetos-sdk` satisfies that constraint by construction**: its name says whose
platform it is, and `ADR-036` §8.3 keeps it in this workspace precisely so a
test can assert that its constants and its journal encoding still match
`rivet-core`'s. A crate promising a general hosting contract could not carry
that test, because there would be nothing single for it to agree with.

### 6.2 Retired 2026-09-07: the crate it argued about how to name is built as `rivetos-sdk`. See `ADR-036` §2.

### 6.4 What requires the SDK, and what does not — ruled 2026-08-22

The app-side contract is six things, and they do not share a nature. Five are
already language-neutral, and **not because Rivet designed for it** — they are
neutral because `ADR-017` §3.4 adopted existing conventions instead of inventing
equivalents.

| Capability | Neutral? | Mechanism |
| :-- | :-- | :-- |
| Socket passing | **yes** | `LISTEN_FDS` — environment variables and descriptors |
| Readiness | **yes** | `sd_notify` — a datagram to `NOTIFY_SOCKET` |
| Structured logs | **yes** | an app writing JSONL to stdout is passed through rather than wrapped (`ADR-022` §4) |
| Configuration delivery | **yes** | a path or descriptor handed in |
| Secret delivery | **yes** | the same, into the app's private namespace |
| **Exposing the app's operations to `rivet`** | **no** | requires Rivet's schema, request encoding and typed error taxonomy |

**Ruled: everything except operations is documented and stable; operations
requires the SDK.**

Two things make this the right line rather than a convenient one:

* **Most of the neutral half is not ours to break.** `LISTEN_FDS` and
  `sd_notify` are stable conventions belonging to somebody else, so documenting
  them is a *compatibility statement* rather than a specification Rivet must
  maintain. A Go daemon can be a well-behaved Rivet service today, with no
  work on our part beyond saying so.
* **Operations is exactly where the SDK should be required**, because it is the
  one surface where Rivet's own types, schema and error taxonomy carry the
  weight. Another language *could* implement the protocol; it would be real
  work, and it would never be tested.

This is `ADR-015` §3.1's formulation one level up — **support what will be
tested, obstruct nothing else.** The boundary is not a gate Rivet builds; it
is simply where the free integration stops.

**So:** package a binary in any language and it runs, sandboxed and supervised,
and it can be a properly integrated service. Want your application's operations
in `rivet status`, in the dashboard, and as MCP tools for an agent diagnosing
its own failure? **Link the SDK.**

**`SPEC-03` §7 is where the state of each binding is read.** The neutral half is
demonstrated rather than asserted: `examples/templates/template-go-service` is a
Go service on this platform, and `ADR-036` §6.2B's pair — one endpoint, two
controllers in two languages, one renderer that connects to either — is what
turns the language-neutrality claim into something that runs.

**The operations row is the part of this ruling with nothing behind it**, and it
is the last row of §3's table that no crate satisfies. `ADR-036` §8.2 rules the
opposite direction — what an application may *read* of the machine — and does
not reach this one, which is an application publishing its own operations into
Rivet's catalogue.

### 6.3 Retired 2026-09-07: the naming record for an unwritten crate; the name `rebar` is retired. See `ADR-036` §2.

#### Retired 2026-09-07: why the earlier candidate name was withdrawn. See `## History`.

#### Retired 2026-09-07: the recorded name collisions. See `## History`.

---

## 7. Where each of these stands

1. **The platform crate is built, and the evidence this item asked for is about
   a different crate.** `ADR-036` rules `rivetos-sdk` and it does §6's job. The
   deferral this item originally carried — *emerge at Stage 3, out of a real
   consumer* — was discharged the way it asked to be: the SDK was written
   against applications that existed, and `ADR-036` §2.1 makes the ordering a
   rule rather than a preference, because an application ported before the
   contract exists invents one and the next application copies it.

   **The standing commitment behind that deferral still binds.** Rivet's own
   crates are written so the split stays possible — service and configuration
   plumbing kept separate from Rivet's system logic, so extraction is a move
   rather than a rewrite. That is a constraint on every crate, not a phase that
   ended. `rivet-core` holding the mount plan as data with no platform types,
   while the syscalls that perform it live in `cmd/rivet-initd`, is the shape it
   asks for, and `ADR-036` §8.3 is the same discipline stated from the SDK's
   side: `rivetos-sdk` names `rivet-core` in `[dev-dependencies]` and nowhere
   else, so an application that links the SDK never sees the operating system.

   **The tripwire.** A crate that must reach *back* into Rivet's system logic to
   do ordinary program things — read its configuration, log, learn it is ready —
   is a crate the split has failed for. That is the signal to watch for, and it
   is cheaper to notice than to measure.

   **What the tripwire has caught is not the SDK's to answer.** `cmd/rivet-initd`,
   `cmd/rivet-sysd` and `cmd/rivet-timed` each declare a `Kernel` port whose
   `Errno` is identical, and a mount-flag translation is wanted by two of them;
   the doc comments in those files are where the evidence accumulates, including
   the finding that the *translation* wants sharing while the flag sets do not,
   and that it is the clock-and-record half that recurs rather than the
   process-management half. **Those crates are Rivet's own boot binaries, which a
   foreign application never links**, so §3's table puts the row under Rivet and
   the SDK is the wrong home for it. Naming a crate for it before the shape is
   settled is the speculation this item refused in the first place.
2. **Ignition extensions:** layering, provenance and schema emission are
   recommended for Ignition, but each needs Ignition's own ADR and version, and
   `AGENTS.md` section 8.4 requires consumers to pin a tag. **None of it is
   Rivet's to write, and nothing here waits on it** — `ADR-022` §6.1 carries the
   one Rivet went looking for, a compose-your-own-layers API, and records why
   `ignition::logging::init` cannot be reached into as it stands.
3. **Ignition against a boot process — measured, and it is why two manifests
   name it as deliberately absent.** The build half is confirmed; the PID 1 half
   is confirmed only with conditions attached.

   * **It builds.** `cargo check --target aarch64-unknown-linux-musl
     --all-features` against Ignition v0.3.0 completes clean — no errors, no
     warnings. Its whole transitive tree is pure Rust over `libc`/`rustix`;
     there is no C dependency and therefore no cross-toolchain problem. **Not
     proven:** `check` does not link, so a static musl link is inferred from the
     absence of C, not demonstrated.
   * **It is usable from PID 1, but not as written.** Nothing panics, nothing
     exits, and `unsafe_code` is `forbid`en — the failure paths degrade rather
     than abort (`logging::init` falls back to an inert guard and prints to
     stderr; the config layers resolve to `None` and are skipped). Two
     constraints follow, and they belong in whatever crate depends on it:
     - **Every path must be passed explicitly.** Ignition's defaults resolve
       through `dirs::config_dir()` and `dirs::home_dir()`, which mean nothing
       in PID 1's environment — no `HOME`, no user, an empty environment at
       boot. `ConfigBuilder`'s `path_override` and `LogConfig::dir` both exist,
       so this is a usage rule, not a defect.
     - **`ignition::update` must not be reachable from PID 1.** It spawns
       processes via `std::process::Command`, and its self-update model
       conflicts with `ADR-018`'s A/B image updates. Ignition declares **no
       cargo features at all**, so it cannot be compiled out — this is a
       discipline enforced by review, or an Ignition change (see item 2).
   * **This measurement is consumed rather than filed.** `cmd/rivet-netd` and
     `cmd/rivet` both cite it in the block of their manifests that records what
     is deliberately absent: the second condition cannot be compiled out, and a
     self-updater reachable from a boot binary on a machine whose update model
     is A/B images is a second updater. `ADR-036` §4 reaches the same
     conclusion from the application side, for the whole platform.
   * **The subscriber is the real constraint** — `ADR-022` §6.1 carries it,
     because it changes the journal's design rather than Rivet's usage.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §7 item 1 — the platform crate's timing was open

**Ruled 2026-08-22: it emerges at Stage 3.** The superseded question was:

~~Does the platform crate exist from Stage 1, or emerge at Stage 3?~~

### In §7 item 3 — Ignition from PID 1 was unverified

**Checked 2026-08-22**, and the answer split in two; the body carries both
halves. The superseded label was ~~Unverified and load-bearing~~.

### §6.2 and §6.3 — the platform crate was named `rebar`, and the name is retired

**Retired 2026-09-07 by the maintainer**, on being shown that this ADR had ruled
a split of this kind and named a crate for it: ~~the platform crate is
`rebar`~~. It was `ferris` for part of a day before that, withdrawn because the
crab was becoming the whole project's mark and `use ferris::prelude::*;` teaches
a reader something about Rust rather than about this platform.

**Why the change matters rather than being a rename.** `ADR-036` built the crate
this ADR called for and used neither the name nor this document, so the record
carried a ruled name for an unwritten crate beside a written crate doing its
job. `docs/design-candidates/Girder.md` §3.3 found it while surveying what four
of the maintainer's applications had each built separately, and the
reconciliation is §6: `rivetos-sdk` is the platform crate, the scope constraint
and the language-neutrality line are what carried over, and the naming argument
decided nothing that survives.

Two things were kept out of §6 deliberately. **The naming criteria** — that the
crate's name is the most-typed one in the project, that it is read at every
import line, that `rebar` collides with an abandoned crates.io package and with
Erlang's build tool — are an argument about choosing a name, and the crate has
one. **The `ferris` withdrawal** is the same. `git log` holds both in full, and
`290d07a` is the commit that named `rebar`.

### §2's table gained a fourth home

**Girder was chartered 2026-09-07** and is the row this ADR had no place for.
The three-home split answered *general, Rivet-shaped, or Rivet's own*, and the
question that had no answer in it is a thing that is Rivet-shaped **and** only
the maintainer's own programs would want — a preference encoded once instead of
copied into four applications. Without the fourth row that work has to be
argued as an SDK feature, and an SDK feature is what a template teaches a
stranger, which §2.1's discipline now says it must not.
