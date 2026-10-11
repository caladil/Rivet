# ADR-001: Language and Dependency Posture

* **Status:** Accepted
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

Rivet is a Linux distribution whose userland is written in Rust. It runs on a
stock Linux kernel — roughly thirty million lines of C that this project has no
intention of reimplementing. That immediately raises the question every "Rust
OS" has to answer and most answer badly: *how Rust is it, and what happens at
the seams?*

The failure modes at either extreme are both real:

* **Purity theatre.** Declaring hostility to C while running on the Linux
  kernel is incoherent. It also produces bad engineering — a hand-rolled
  WPA2 supplicant is worse software than `wpa_supplicant` in every measurable
  way, and choosing it would be an identity statement, not a decision.
* **Drift.** Without a stated posture, the shortest path is always to shell out
  to an existing C tool. A hundred commits later the project is a shell script
  with a Rust logo.

This ADR fixes the posture so that neither happens by default.

---

## 2. Decision

> **Rivet is a Rust-first Linux, not a Rust-only one. C programs are
> mechanisms; they are never the interface.**

Everything the operator touches — configuration, CLI, logs, errors, service
definitions, failure modes — is Rust, and every C dependency sits behind a Rust
module that owns its lifecycle and translates its behaviour into Rivet's own
types.

The test for whether a dependency is correctly placed is not "is it written in
Rust" but: **if this component were replaced tomorrow, how much above it would
have to change?** The correct answer is "nothing".

---

## 3. Postures considered

| Posture | Description | Time to first boot | Verdict |
| :-- | :-- | :-- | :-- |
| **A. Rust-first, C-tolerant** | Kernel, libc, busybox, supplicant all C. Rust supplies init, networking, service management. | Days | On-ramp only |
| **B. Rust userland, no C in the filesystem** | C exists only as statically linked libc and a small set of named daemons. No C-authored user-facing surface. | Weeks | **Chosen** |
| **C. No libc at all** | `#![no_std]` + `alloc` + `rustix` raw syscalls throughout. | Months | Rejected |

**Posture C was rejected** because surrendering `std` — `std::fs`, `std::net`,
threads — is a permanent tax paid for a claim that is philosophical rather than
operational. See `ADR-003` for the linking decision that captures most of C's
practical benefit without it.

**Posture A is where the project starts and B is where it lands.** The
difference between them is not a rewrite; it is the tier policy below, applied
over time.

---

## 4. The tier policy

Every non-Rust component in the image belongs to exactly one tier, and the tier
is a statement about intent, not about quality.

| Tier | Policy | Members at adoption |
| :-- | :-- | :-- |
| **A** | Rust, now. No C equivalent is acceptable. | PID 1, service management, netlink/addressing, DHCP client, coreutils, findutils, configuration, logging, mDNS |
| **B** | C now; Rust later; **tracked**. | Shell, `grep`/`sed`/`awk`, kernel module loading |
| **C** | C indefinitely, wrapped. Replacement is not a goal. | Linux kernel, `wpa_supplicant`, musl |

### 4.1 Tier B needs teeth

"We will replace it eventually" is a feeling, not a plan, and it decays into
Tier C silently. Two mechanisms hold it:

1. **Every Tier B member has an entry in `docs/PLAN.md`** under open work,
   naming the intended replacement and what blocks it. A member with no entry
   has quietly become Tier C, and that is a decision requiring an ADR.
2. **BusyBox is the coverage bucket, and its build configuration is the
   scoreboard.** Rivet ships BusyBox with a fixed, explicit applet list. Each
   applet replaced in Rust is deleted from that list. The count only goes down,
   it lives in the repository, and it is reported in `README.md`.

### 4.2 Tier C is not a failure

`wpa_supplicant` is the worked example and the reasoning generalises. Depending
on it is what keeps Rivet current with WPA3-SAE, 802.11r/k/v and whatever
follows. Writing our own is what would strand the project on WPA2-PSK for ever,
because we would implement PSK, declare victory, and never touch it again. See
`ADR-004` §3.

### 4.3 Time synchronisation is in no tier, and should be

**Noticed 2026-08-23.** §4's list of Tier A components is *"PID 1, service
management, netlink/addressing, DHCP client, coreutils, findutils,
configuration, logging, mDNS"* — and the clock is not on it, in either
direction. `ADR-021` §7 makes `rivet-timed` a service and `ADR-021` §4.1
rejected `chrony` **because it is another C mechanism to cross-build and wrap**,
which is a Tier A argument made without the word. Verified two ways: the table
itself, and a case-insensitive search of this whole file for `ntp`, `time sync`,
`timed` and `clock`, which returns nothing.

**Time synchronisation is Tier A.** Rust now, no C equivalent accepted — which
is what `ADR-021` already decided, recorded here so the tier table stops
disagreeing with it.

### 4.4 C can enter the image inside a Rust binary, and §4 does not see it

**The gap, found by the `nushell` survey of 2026-08-23 (`ADR-005` §5.6.3).**

§4's opening commits that *every non-Rust component in the image belongs to
exactly one tier*. That sentence quietly assumes a C component arrives as **a
program Rivet drives** — which every one so far has (`wpa_supplicant`,
BusyBox). A Rust crate that statically links a C library does
not arrive that way and is invisible to the whole apparatus.

`nushell` would put **`libgit2`, LMDB, `zlib` and `blake3`** in the image,
compiled into `nu`, in no tier and on no scoreboard. All four arrive through a
single crate (`fff-search`), reached unconditionally, with no feature
combination that avoids them — verified by building.

**This is an accounting gap, not a violation, and the distinction matters:**

* **§4.1's BusyBox applet scoreboard is undisturbed.** These are not applets and
  the count is unaffected. Reading them as a regression would be wrong.
* **The audit surface is genuinely different.** A C mechanism Rivet *drives*
  sits in the boot path or beside it; C linked into a leaf binary that nothing
  else links against cannot reach PID 1, the supervisor or the CLI. `ADR-009`
  §3.3's "wrong altitude" hazard is the one that does **not** apply.
* **What breaks is a claim elsewhere.** *"The image contains
  exactly one C mechanism"* becomes false the day `nu` ships, and that sentence
  is the kind a reader trusts.

**Ruled here: a C library statically linked into a Rust binary is a *transitive*
C component.** It is listed wherever the tier table lives, attributed to the
Rust crate that brings it, and it is **not** on §4.1's applet scoreboard. It
gets a tier for the same reason everything else does — so that its presence is a
decision somebody made rather than a fact nobody wrote down. On the evidence
available it is **Tier C**: nobody is going to rewrite `libgit2`, and pretending
otherwise would be the Tier B entry that quietly becomes Tier C which §4.1
exists to prevent.

### 4.5 `gdbserver` and `strace` — Tier C, ruled 2026-08-24

**`gdbserver` and `strace` are both Tier C.** The argument taken is `ADR-005`
§7.6's, which is §4.2's applied to a different domain: nobody is going to
rewrite `gdb`'s remote serial protocol or `strace`'s syscall tables in Rust, and
§4.1 exists precisely to stop a Tier B entry that nobody intends to act on.
`strace` in particular is a wrapper around the kernel's own `ptrace` and
`PTRACE_GET_SYSCALL_INFO`, and its value is that it tracks the kernel's syscall
set rather than this project's opinion of it. The full argument, and the
counter-argument, are in `ADR-005` §7.6.

**A gap remains open, and the ruling sharpened it rather than closing it.**
`ADR-005` §7.4 ruled the two onto a `--debug` build only, so **neither is on any
card by default** and §4's table carries two entries that are true of no shipped
image. `cmd/rivet-image --gdbserver` and `--strace` are optional and default
off. Adding a *present on some images* column is a change to this ADR's shape
and was not part of the ruling; until it is made, the table says Tier C and this
paragraph says where they actually are.

**Why this belongs in the table at all.** §4's opening commits that *every
non-Rust component in the image belongs to exactly one tier*, and a tier is a
claim about what the image is. Recording these two makes the table's coverage a
decision somebody took rather than an oversight of the kind §4.3 found for the
clock.

**§4.1's BusyBox applet scoreboard is undisturbed** — neither is an applet and
the count is unaffected, exactly as §4.4 established for the transitive C
libraries.

---

## 5. Consequences

* **Every Tier B and C dependency is wrapped before it is used.** A component
  invoked directly from more than one place in the tree is a defect, because it
  cannot then be replaced without touching all of them.
* **Prefer a process boundary to an FFI boundary.** Where a C component is
  available as a daemon with a socket protocol, drive the socket. No `unsafe`,
  no ABI coupling, no C in the build graph, and the component can be replaced
  or restarted independently. `ADR-004` §4 applies this to `wpa_supplicant`.
* **A C component's own configuration file is an implementation detail and
  should not exist where avoidable.** The operator configures Rivet; Rivet
  configures the mechanism at runtime.
* **The image is measured.** Image size, boot time, and BusyBox applet count are
  tracked numbers, because leanness that is not measured is leanness that is
  being lost. See `ADR-002` §5.

---

## 6. What this ADR does not decide

* The workspace and crate layout, and where the isolation gate falls. Pending.
* Whether a dynamic loader is retained as an escape hatch — `ADR-003` §5.

---

## 7. Randomness — ruled 2026-08-28

`ADR-001` said nothing about randomness until this section, and a rule that
said *"there is no RNG in this project's dependency graph at all"* had
nonetheless been enforced for months — out of `cmd/cargo-rivet/src/key.rs`'s
module documentation, `rivet-crypto/Cargo.toml`'s comment block, and `PLAN.md`
§57.3. **A constraint that blocks a design decision and lives nowhere in the
design record is folklore, and this one blocked one** (`PLAN.md` §68).

### 7.1 The blanket rule is retired, and it was false besides

**Rivet is full of randomness and always has been.** `ADR-024` §6 configures
the BCM2711's RNG200 and reasons at length about entropy *crediting*; the
kernel credits the firmware's `/chosen/rng-seed`; the machine mints its own device identity
on the board at first boot. An operating system that had no randomness could
not mint a key at all.

**And the dependency-graph claim was not true.** Checked 2026-08-28:
`Cargo.lock` carries **three** `getrandom` entries — `0.2.17`, `0.3.4` and
`0.4.3` — plus `rand` and `rand_core`. `0.2.17` arrives by
`redox_users` → `dirs-sys` → `dirs` → Ignition → `cmd/rivet-fleet`, behind a
Redox-only `cfg` that no build this project makes ever compiles; the other two
are reachable from no workspace member at all, even with `--target all`.

What is true is narrower and worth keeping as an **observation**: no binary
that ships on a card links one. `rivetos-initd`, `-sysd`, `-cli`, `-mdnsd` and
`-gitd` each count zero for `aarch64-unknown-linux-musl`. That is a fact about
the image, not a prohibition on the workspace, and it is not a reason to refuse
a dependency.

### 7.2 `getrandom` is not an RNG, and the distinction is the whole ruling

`getrandom` is a shim over `getrandom(2)` on Linux and `BCryptGenRandom` on
Windows. **It generates nothing; it asks the kernel.** `rand` and `rand_chacha`
are RNGs — userspace PRNGs seeded once and iterated.

So a Rivet daemon that takes a nonce from `getrandom` is **doing what this
section asks**, through a crate rather than an `open()`. Refusing it on
"no RNG" grounds refuses the correct implementation in favour of a hand-rolled
one, which is `ADR-009` §5's trade upside down.

### 7.3 The property that survives

> **The randomness Rivet depends on comes from the operating system.** Rivet
> implements no generator, seeds no pool, and iterates no stream cipher to
> produce key material. Where it needs bytes it asks the kernel — directly, or
> through a crate whose entire job is to ask the kernel.

That is a statement about *where entropy originates*, and it is satisfied by
`getrandom(2)`, by `BCryptGenRandom`, by `/dev/urandom` and by the `getrandom`
crate equally. It is **not** satisfied by a userspace PRNG in the trusted path,
and a proposal to add one is a decision for this section.

`ADR-029` §7.1's reason for Ed25519 is unaffected and is worth restating
because it is a real property rather than a policy: **Ed25519 needs no
randomness at signing or at verification**, so the update path has no entropy
requirement at all. Nothing in this ruling adds one.

### 7.4 What this changes

1. **`cargo rivet key new` is admitted** (`PLAN.md` §68). It reads from the
   operating system through `getrandom` and writes a seed. It moves no entropy
   source: `head -c 32 /dev/urandom` and `getrandom(2)` are the same kernel
   call. What it buys is a **recorded, reproducible, cross-platform**
   procedure for the highest-value secret this project has — and the previous
   arrangement had that procedure living in one person's shell history, which
   is exactly what `tools/card/build.sh`'s own header exists to condemn.

   It also removes a platform seam: Windows is a development host, and it has
   no `/dev/urandom`, so *"just ask the operating system"* silently meant
   *"go to WSL"*.

2. **`getrandom` is not grounds to refuse a dependency.** A candidate that
   brings it is judged on everything else it brings. `PLAN.md` §57.3's
   objection to `rivet-smbd` on these grounds is withdrawn.

### 7.5 What it does not change

* **The seed is still never committed** (`AGENTS.md` §8.3), still has no
  default location, and `cargo rivet key` still reads no environment variable
  and looks in no well-known directory — every one of those absences is a
  design decision nobody has taken.
* **`ADR-028` §5.6's root key stays deferred.** The offline discipline is the
  feature and a generator does not supply it.
* **No userspace PRNG enters the trusted path** without amending §7.3.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §4.5 — `gdbserver` and `strace` had no tier

**Changed 2026-08-24**, when the maintainer ruled both Tier C. The section was
titled ~~Two components have no tier, and that is on purpose~~, and its two
superseded sentences were:

~~**`gdbserver` and `strace` are in no tier**, and this section exists so that
the table's silence is a recorded gap rather than an oversight of the kind §4.3
found for the clock.~~

~~**The recommendation, recorded and not taken, is Tier C for both**~~

The recommendation was taken on 2026-08-24 and is now the section's opening
statement. The gap the section was written about is not closed by the ruling and
is stated in the body.
