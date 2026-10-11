# ADR-014: Workspace and Crate Layout

* **Status:** Accepted — ruled 2026-08-22, closing the last of the eleven
  questions in `docs/PLAN.md` §4. **§3's first clause amended 2026-08-23** —
  the edge between `rivet-core` and `rivet-config` runs config → core, not
  core → config; §3.2 carries the argument and the cost. §2's list is otherwise
  the destination and **is not a claim about what exists**: as of 2026-08-23
  **nine of the twelve crates it lists exist** — plus `cmd/rivet-image`, which
  it does not list at all. `rivet-render` and **two** of the four adapters had
  not been created. Checked against the root `Cargo.toml`'s member list and
  against `git ls-files`, per `AGENTS.md` §7. **That line was corrected three
  times in one day**, which is the argument for reading `Cargo.toml` rather than
  this sentence; History has the sequence.

  **`rivet-adapter-image` existing is one third of what §2 says it is.**
  §2 calls it *"A/B image store, update, rollback"*; what exists is the boot-slot
  **primitive** — `BootSlots`, one `PiFirmware` implementor, and its own
  `BootMedium` port — and nothing that fetches, verifies or writes a payload.
  `ADR-028` §8 item 1 leaves the signature scheme unruled, so anything that
  verified would be inventing a ruling. See §2.2 for why it was created now and
  what its edges are.

  **`cmd/rivet` existing is not `ADR-006` being implemented**, and the two are
  easy to conflate from this list alone. It is a real binary whose job today is
  the login-shell contract; it implements no
  administrative operation, because there is none in `rivet-core` to adapt.
  `ADR-006` §8.1 is the itemised list of which fraction is built.

  **`rivet-store` likewise is not `ADR-007` §3.4 being implemented.** It
  persists the **machine layer only** — no image defaults, no fleet layer, and
  nothing merges layers, so `--explain` still cannot say where a value came
  from. `ADR-007` §7.6.1 records what was built. `ADR-017` §3.1.9 is the
  amendment that made PID 1 its first caller. Its edges: it depends on
  `rivet-core` and on `rivet-config`, both of which §3's table permits, and
  on nothing else.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`AGENTS.md` §3 states that a boundary held by convention erodes and a boundary
held by the dependency graph does not, because the build refuses rather than a
reviewer noticing. That is only true if the pieces are **separate crates**.

So the workspace shape is not organisation. It is the enforcement mechanism for
`ADR-006` §3 — one core, several adapters, no logic in any interface — and for
`ADR-009` §4, which forbids a dependency's types from reaching Rivet's own
surface.

Everything else in the record constrains this ADR, so most of it is derivation.
Two things were genuine choices and are recorded in §4 and §5.

---

## 2. The workspace

```
rivet-core            every administrative operation as a typed
                        request / result / error. The desired-state model,
                        layering, provenance. Owns the port traits.
                        NO I/O. NO rendering. NO platform types.

rivet-config          the configuration schema — types plus schema generation

rivet-store           persistence of desired state to /state/config
rivet-render          human rendering of core's typed results

rivet-adapter-net     netlink, addressing, routes, DHCP, mDNS
rivet-adapter-wpa     the wpa_supplicant control socket
rivet-adapter-system  procfs, sysfs, mounts — observed state
rivet-adapter-image   A/B image store, update, rollback
rivet-adapter-cec     HDMI-CEC over /dev/cecN — telling the television

rivet-crypto          signature verification for update payloads.
                        A supporting crate, not an adapter — see §2.3
rivet-webauthn        the proxy's WebAuthn relying party checks.
                        A supporting crate, not an adapter — see §2.4

cmd/rivet-initd       PID 1 — mounts, reaping, boot confirmation,
                        and supervising exactly one child
cmd/rivet-sysd        service supervision — allowed to fail
cmd/rivet             the CLI
cmd/rivet-netd        the network daemon
```

**Amended 2026-08-22.** `cmd/rivet-sysd` was added when `ADR-017` §3.1
ruled that PID 1 stays minimal and spawns a supervisor rather than being one.
`cmd/rivet-initd` is correspondingly small — its complete responsibilities are
`ADR-017` §3.1.5.

**Renamed 2026-08-24, ruled by the maintainer — `PLAN.md` §0.4.7 carries the
argument and this is the layout entry.** The crate formerly called
`rivet-init` is `cmd/rivet-initd`, and the one formerly called
`rivet-supervisor` is `cmd/rivet-sysd`. Both moved in one commit, because they
touch mostly the same files and a half-renamed tree is the failure worth
avoiding.

Two reasons, and the first is measurable: **`rivet-supervisor` is eighteen
characters and the kernel's `comm` field is fifteen**, so `ps` on a board printed
`rivet-supervi`. The second is that the name claimed one job while the process
is the journal, the `/etc` projection, the clock *and* the services — `sysd`
says *the system*, and both names now match the `-d` of `netd` and `timed`.
Rivet is not POSIX and is not bound by the habit of calling PID 1 `init`.

**The rename is load-bearing in three places that must agree or a board does not
boot**: this crate's name, `cmd/rivet-image`'s `INIT_PATH` (what `cmdline.txt`
puts after `init=`), and `cmd/rivet`'s `INIT_COMM` (what the shutdown
interlock requires `/proc/1/comm` to read, and it fails closed). `rivet-initd`
is thirteen bytes, so it survives `comm`'s truncation; a compile-time assertion
in `shutdown.rs` holds that.

**Ruled 2026-08-22: the full split from the start**, rather than beginning with
the four adapters merged. Splitting later is a refactor with no forcing
function, and the recommendation to merge was optimising for fewer directories
rather than for the property the layout exists to hold.

### 2.1 What is deliberately absent

* **No `rivet-api` crate yet.** The network API is deferred with a named
  destination (`ADR-006` §5.1). When it arrives it is a sibling of
  `rivet-render` and `cmd/rivet`, and if adding it requires touching
  `rivet-core`, the boundary was drawn wrong.
* **No crate of this layout is reachable from an application.** The platform
  crate `ADR-013` §7 called for is `rivet-sdk` (`ADR-036`), and it is a member
  of this workspace for one reason: `ADR-036` §8.3's test, which asserts that
  its constants and its journal encoding still agree with `rivet-core`. The
  edge that test needs is in `[dev-dependencies]`, so an application that links
  the SDK links none of the crates above. The commitment `ADR-013` §7 attached
  to that split still binds every crate here — service and configuration
  plumbing stays separable from Rivet's system logic, so an extraction is a
  move rather than a rewrite.

### 2.2 `rivet-adapter-image` — created 2026-08-23, and why then

§5's last consequence says the layout is *"the destination, not a scaffolding
requirement before the first boot"*, so a crate is created by the first thing
that needs it. This is what needed it, and it is not the update path:

> `BootSlots` and `PiFirmware` were `cmd/rivet-initd/src/slots.rs`, and
> **`cmd/rivet-initd` is a binary crate with no lib target.** Nothing else in
> the workspace could name the port at all.

The maintainer's ruling to put the boot-slot primitive behind a port (x86
has no `tryboot`) cut it on 2026-08-23 so that an x86 machine
would be an adapter rather than a rewrite. It was cut inside the one crate that
cannot export anything, which made it a port only PID 1 could use — and
`ADR-028` §4 rules that an authenticated principal supplies the **authority** to
install an update while saying nothing about which **process** executes it. A
port trapped in PID 1's binary answers that question by accident, in the
direction that is hardest to undo.

**Ruled 2026-08-23: both the trait and the `PiFirmware` implementor move into
`rivet-adapter-image`**, not just the implementor.

That is worth stating plainly because it contradicts `AGENTS.md` §3's diagram,
which puts the port traits in `rivet-core`. **The codebase has never done
that** — `SendRecv` is in `rivet-adapter-net`, `Filesystem` is in
`rivet-store`, `Kernel` is in `cmd/rivet-initd` — and this follows the
codebase's actual, consistent practice rather than the diagram. **The
discrepancy between the two is not resolved here and is the maintainer's to
rule**; it is recorded so that a future session finds it as a known open
question rather than as a defect to "fix" in one adapter at a time.

#### Its edges

* → `rivet-core`, for `ADR-018` §4's partition numbers and `Slot`. Nothing
  else. §3's table forbids adapter → adapter and none appears.
* `cmd/rivet-initd` → it, which §3 permits explicitly — *"`cmd/*` may depend on
  everything and contains no logic"*. PID 1 supplies the only `BootMedium` there
  is, adapting its own `Kernel` exactly as `cmd/rivet-sysd` adapts its
  `Kernel` to `rivet_store::Filesystem`.
* `cmd/rivet-image` → it, as a **dev-dependency only**. That crate's tests ran
  a *verbatim copy* of the `autoboot.txt` parser behind a text-equality drift
  guard, because the original could not be depended on. This change removed the
  reason; the copy and its guard are deleted and the tests call the real
  function.
* `rivet-core`'s `[dependencies]` table is still empty, which is what makes
  every forbidden edge in §3 fail to compile.

#### What it ships no implementor of

There is no `StdBootMedium`, unlike `rivet-store`'s `StdFilesystem`.
`mount(2)` has no `std` equivalent, so a real one means taking `libc` in this
crate and standing a second syscall seam beside the one PID 1 already owns —
`ADR-011`'s second authority, over syscalls. **The moment to revisit that is the
moment `ADR-028` §4's unnamed process turns out not to be PID 1.**

---

### 2.3 `rivet-crypto` — added to §2's table 2026-08-24

**The crate existed before the table listed it**, which is the failure this
entry closes rather than a new decision. It is in `Cargo.toml`'s members, it is
what `ADR-029` §11's single verifier is built out of, and §2 is meant to be the
authority on what the workspace contains. A reader checking the layout against
the record would have found a crate the record did not know about — `ADR-011`'s
problem in the small.

**It is a supporting crate, not an adapter, and the distinction is the one §3
already draws.** An adapter is named for the *outside world* it translates to:
netlink, a control socket, procfs, a block device. `rivet-crypto` translates
to nothing — it is pure computation over bytes, with no syscalls and no
platform types, and it could as easily live inside `rivet-core` were it not
for §3's rule that core takes no dependencies at all. That rule is exactly why
it is a sibling: signature verification needs a vetted implementation, core may
not depend on one, so the dependency is pushed one crate out and core states
the requirement as a port.

So it sits with `rivet-config` and `rivet-store` — crates that support core
without adapting anything — and the `rivet-adapter-` prefix continues to mean
what it says.

### 2.4 `rivet-webauthn`

**§2.3's argument, for a different dependency.** `rivet-webauthn` (package
`rivetos-webauthn`) holds every check a WebAuthn relying party makes over what
a browser sent — `ADR-063` §4.3. It is pure computation over bytes: no syscall,
no platform type, no file, no clock. It depends on `ring` for signatures and
SHA-256, `ciborium` for CBOR (`.rulings/` R-230), `serde_json` and `base64`,
and on no Rivet crate. Its one consumer is `cmd/rivet-proxyd`, which keeps
everything with state — challenges, sessions, the credential file, randomness.

It is a crate rather than a module of the proxy so that every refusal is tested
on any host without a socket or a container, and it is not `rivet-core` because
`rivet-core` takes no dependencies (§3).

---

## 3. The isolation rule

Stated so it has teeth, per `AGENTS.md` §10.1. Each forbidden edge fails to
compile because the dependency is simply not listed — no reviewer, no lint, no
discipline required.

> **`rivet-core` depends on `rivet-config`, and on nothing that touches the
> world.**

**The first clause was reversed 2026-08-23 — see §3.2.** The second clause, and
every row of the table below, is unchanged.

| Forbidden edge | Why |
| :-- | :-- |
| core → any adapter | `ADR-006` §3 — the core owns operations, adapters perform them |
| core → any transport | the same, and it is what makes a future API cheap |
| core → `rivet-render` | rendering is an adapter concern; `ADR-006` §4 |
| core → Ignition | `AGENTS.md` §10.1 — configuration is I/O, loaded at the composition root |
| core → `clap` or anything CLI-shaped | the CLI is one adapter among several, and must not be privileged |
| adapter → adapter | adapters are siblings; shared behaviour belongs in core's traits |
| a dependency's types in core's public API | `ADR-009` §4 — reuse implementations, never interfaces |

**`cmd/*` may depend on everything and contains no logic.** It is the
composition root: it wires adapters into the core and exits. Ignition appears
here and nowhere else.

### 3.1 The check that this is still true

The dependency graph is the gate, but a gate nobody looks at can still be
widened by someone adding a line to a `Cargo.toml`. The cheap standing check is
that **`cargo test -p rivet-core` must need no network, no filesystem beyond
temporary paths, no device and no root.** If it starts needing one, the boundary
has leaked, and that is `AGENTS.md` §4's rule applied here.

### 3.2 Amended 2026-08-23: the edge between core and config runs the other way

**`rivet-config` depends on `rivet-core`, and on nothing that touches the
world.** §3 originally ran that edge the other way; the rest of the sentence —
*and on nothing that touches the world* — is unchanged and is the load-bearing
half:
`rivet-core`'s `[dependencies]` table is still empty, which is what makes
every forbidden edge in the table above fail to compile rather than fail review.

**Neither edge had ever been created**, which is why this reads as an amendment
rather than a contradiction. `rivet-core` had no dependencies at all and
`rivet-config` depended only on `toml_edit`; the sentence above described an
intended direction that nothing had yet acted on. `ADR-007` §7.3's `[network]`
table forced the choice on 2026-08-23:

* The document carries interface names, addresses and prefix lengths.
* Those are `rivet-core::net`'s types — `InterfaceName`, `Ipv4Cidr`,
  `StaticAddressing` — which already carry the kernel's own `dev_valid_name()`
  rules and their own validation.
* `ADR-009` §4 forbids a second declaration of a type Rivet already owns, and
  a second address parser is the plainest available instance of `ADR-011`'s
  second authority.

So the crate that turns text into types must be able to see the types.
Cargo forbids a cycle, so **this decides the direction permanently**: nothing in
`rivet-core` may ever depend on `rivet-config`. That is the cost, and it is
stated here rather than discovered later.

**It is not a cost that bites.** §2 gives `rivet-core` "the desired-state
model, layering, provenance", and the natural worry is that layering will need
`Document`. It does not: a layer is a *value* of the same schema, so layering
composes documents rather than being described by one, and `rivet-config` is
where both the schema and its composition belong. If a genuine need ever appears
for core to hold a document, the answer is to move the shared types down — not
to reverse this edge, which cannot be done.

---

## 4. Binary packaging: separate, static, revisit later

**Ruled 2026-08-22:** `rivet-initd`, `rivet` and `rivet-netd` are separate
crates producing separate, statically linked binaries.

### 4.1 The fact that decides it

The maintainer asked whether to start with shared libraries, since the record
already expects to end there (`ADR-003` §6.1). The answer turns on something not
previously written down:

> **Rust has no stable ABI, so Rivet's own crates cannot practically be shared
> libraries.**

`dylib` is explicitly unstable and exists for rustc's internals rather than for
distribution; `cdylib` means passing everything through a C ABI, which would
discard the generics, traits and typed `Result`s that the entire `ADR-006`
design is built on.

So the duplication that shared libraries *could* remove is musl and `libstd` —
a few megabytes across three or four binaries. **The largest duplication, which
is `rivet-core` appearing in every binary, is not addressable this way at
all.** The ceiling on the benefit is much lower than it appears.

### 4.2 Why static now, and it is not about size

All three reasons static-versus-shared usually matters have already been ruled
out from under this question: size is not currently a constraint, `ADR-003` §6.1
already ruled static is not a permanent guarantee, and `ADR-003` §6.2 already
ruled Rivet is a build target rather than a runtime for foreign binaries.

What remains is the argument that actually earns the decision: **debuggability
during bring-up.** A static binary has no loader to fail, no missing `libc.so`,
no version mismatch. Stages 0 to 2 are spent asking "why will this not boot",
and removing an entire class of failure from that question is worth more than
ten megabytes.

Boot binaries would want to be static regardless — a PID 1 that cannot find its
loader is a kernel panic with no diagnostic.

### 4.3 Nothing is foreclosed

Switching later is a **build-configuration change, not a redesign**: Buildroot
has a global static/shared toggle for the C mechanisms, and Rust has
`crt-static`. No crate boundary, no ADR and no part of the image layout depends
on the answer.

A multicall bundle — one executable invoked under several names, as `uutils`
does — also remains available and would recover most of the duplication with no
dynamic linking at all. It is additive whenever a tracked number says it is
worth doing, and **the code layout is deliberately not coupled to the binary
layout** so that it stays that way.

---

## 5. Consequences

* **The first crate created must be `rivet-core`**, and it must compile
  against nothing that performs I/O. Establishing that on day one is free;
  establishing it after five adapters exist is a refactor.
* **`cargo-deny` is configured with the workspace** (`ADR-010` §4.1), so the
  licence allowlist and advisories are live from the first dependency rather
  than retrofitted onto a large tree.
* **Stage 0 needs only a fraction of this.** `cmd/rivet-initd`,
  `rivet-core` and `rivet-config` are enough to boot and mount —
  `cmd/rivet-sysd` joins them as soon as anything needs supervising.
  The rest are
  created as the stage that needs them arrives — the layout is the destination,
  not a scaffolding requirement before the first boot.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### Status block — how many of §2's crates existed

**Corrected three times on 2026-08-23**, which is the argument for reading
`Cargo.toml`'s member list rather than this sentence. The sequence, in order:

~~as of 2026-08-23 six of the twelve crates it lists exist~~
~~seven, later the same day~~
~~**eight, later the same day again** — `cmd/rivet` and then `rivet-store`
were created~~

The fourth value, nine, is in the status block, and the crate that took it there
was `rivet-adapter-image`. `AGENTS.md` §10 carries the general form of this
failure and the count that `tools/docs/claims.py` now derives instead.

### In §2 — the two daemon names

**Renamed 2026-08-24**, ruled by the maintainer (`PLAN.md` §0.4.7), in one
commit. The superseded names were ~~`cmd/rivet-init`~~ and
~~`cmd/rivet-supervisor`~~. The measurable reason is in the body:
`rivet-supervisor` is eighteen characters and the kernel's `comm` field is
fifteen.

### In §3.2 — the edge between core and config

**Amended 2026-08-23**, when `ADR-007` §7.3's `[network]` table forced the
choice. The superseded clause read:

~~**`rivet-core` depends on `rivet-config`**~~

Neither edge had ever been created, which is why this reads as an amendment
rather than a contradiction. The rest of the sentence — *and on nothing that
touches the world* — is unchanged and is the load-bearing half.
