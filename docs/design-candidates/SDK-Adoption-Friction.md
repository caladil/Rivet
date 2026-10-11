# SDK adoption friction — a running note

* **Status:** **Running note, not a decision.** Nothing here is ruled and
  nothing here is normative. It is the list an author of `rivet-sdk` should read
  before changing its surface.
* **Started:** 2026-09-05, by the Trends port
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. What this file is, and the rule for adding to it

`ADR-036` §2.1 rules that **the port is the instrument**: an application built
against this SDK is how its gaps are found, and every place a port reaches past
the SDK is a gap to report rather than a thing to work around quietly. That
produces findings, and findings evaporate.

`docs/PLAN.md` §130 asked the first `serve` adopter four questions in advance for
exactly that reason, and `ADR-040` §10 is where those four are answered. **This
file is the residue**: the friction a port hit that belongs to no single ADR.

**The rule for an entry.** It names **one specific friction** and **what would
have removed it**. A feeling that something was awkward is not an entry. An entry
whose fix has landed says so and stays, because the next author's question is
usually *has anybody hit this* rather than *is it open*.

Where an entry has an owning document, that document is authoritative and this
is a pointer.

---

## 2. Entries

### 2.1 A template that builds only where it lives — FIXED 2026-09-05

* **Port:** Trends. **Owning record:** `ADR-040` §10.4 item 1.
* **The friction.** `examples/templates/template-network-service` had no
  `.cargo/config.toml`. `.cargo/config.toml` is discovered by walking up the
  *directory* tree, so the template inherited the repository root's copy however
  empty its own `[workspace]` table was — and copying the directory out, which is
  the one thing a template is for, silently dropped the cross linker. Every crate
  compiles for the target and the link then fails with `crt1.o: Relocations in
  generic ELF (EM: 183)`, which names no linker.
* **What would have removed it.** The file, in the template. It is there now, and
  the template was rebuilt outside the repository to prove it.
* **The general shape**, which is likelier to recur than the instance: an example
  inside this repository is tested inside this repository, and `AGENTS.md` §10.5's
  last bullet is about exactly that — a green run in a tree you have worked in is
  not evidence about a clean checkout. **Every template is a claim about a
  directory somewhere else.**

### 2.2 A handed listener is blocking and says nothing — DOCUMENTED 2026-09-05

* **Ports:** Reliquary (still affected), Trends. **Owning record:** `ADR-040`
  §10.4 item 2, and `rivet-sdk/src/listen.rs`.
* **The friction.** `Socket::into_tcp_listener` hands back a blocking
  `std::net::TcpListener`; `tokio::net::TcpListener::from_std` requires a
  non-blocking one and neither checks nor refuses. Both adopters so far are
  asynchronous. The symptom is a runtime worker parked inside `accept`, which is
  invisible at one connection at a time.
* **What would have removed it.** A sentence in the method's documentation — now
  written — or a second, named constructor. The second is `ADR-040` §10.6's open
  question, and the argument for it is that a paragraph did not exist to be
  missed the first time and a name cannot be missed.

### 2.3 The canonical `manifest.toml` is specified only in source — OPEN

* **Port:** Trends. **Owning record:** `ADR-040` §10.4 item 3.
* **The friction.** `tools/qemu/run.sh --app` and `tools/qemu/fixture-signer`
  take a **rendered** manifest, and the only specification of its canonical form
  — key order, which lists are always present, that `settings` is written after
  `devices` — is `rivet_core::package::application::Manifest::render`.
  `cargo rivet --help` lists the keys and the values each accepts and says
  nothing about the order the machine demands them in, and the refusal an
  out-of-order manifest earns reads like a parse error.
* **What would have removed it.** Either a `SPEC-03` section carrying the
  canonical form with an example, or — better, because it cannot go stale — a
  `cargo rivet package --print-manifest` that renders `[package.metadata.rivet]`
  and writes nothing. A harness that needs a rendered manifest could then ask the
  tool for one instead of a person hand-writing what the tool's own doc comment
  says not to hand-write.

### 2.4 Every setting an application wants is a number, and every value is a string — OPEN

* **Port:** Trends. **Owning record:** `ADR-038` §10.2 and §4.4; `ADR-040` §10.4
  item 4.
* **The friction.** `ADR-038` §10.2 accepted text-only values *unwillingly*.
  Trends' two settings are `max-readers` and `max-page-size`, both integers, so
  **100% of its settings must be written quoted in the machine document**. §4.4
  is right that a line which cannot be carried is ignored rather than refusing
  the document — and the result for an operator who typed `max-readers = 3` is a
  daemon that starts, serves every request correctly, and runs on numbers nobody
  chose, with one journal record saying so. Reliquary's two settings are also
  integers.
* **What would have removed it.** Nothing about the storage: §10.2 is a decision
  and this is its stated cost arriving. What would remove the *silence* is the
  application being able to see that a value was refused — today
  `rivet_sdk::settings::get` answers `None` for *unset*, *unparseable* and
  *refused by the machine* alike, and only the first of those three is ordinary.
  §10.4's open question about a running application and a changed setting is the
  same surface.

### 2.5 An adopter reaches the SDK by a path to a sibling checkout — OPEN

* **Ports:** Reliquary, Trends. **Owning record:** each port's own manifest says
  it about itself.
* **The friction.** `rivetos-sdk` is unpublished and this repository is untagged,
  so an adopter cannot pin a git dependency and writes
  `path = "../../../Rivet/rivet-sdk"`. `AGENTS.md` §8.4 forbids that in general
  and is right: it resolves on one machine and nowhere else.
* **What it cost each port, and the two costs are different.** Reliquary put its
  Rivet root in its one workspace and accepted the path. **Trends could not**:
  cargo resolves every member of a workspace before it builds any of them, so a
  member whose path dependency is absent takes `cargo metadata` down for the
  whole workspace — including the `cargo test -p trends-core` that is that
  project's strongest portability gate. A machine with no Rivet checkout beside
  it would report a portability break in somebody else's repository. So Trends'
  Rivet platform is a **second workspace**, at the cost of a second `Cargo.lock`
  and building the shared crates twice.
* **What would have removed it.** A tag on this repository. One tag turns both
  ports' path dependencies into git dependencies in the shape Ignition already
  uses, and folds Trends' second workspace back into its first.
* **The same fact reached this repository's own tooling.**
  `tools/container/exec.sh` grew `RIVET_APP_SRC` on 2026-09-05 so an adopter's
  tree can be built in the container beside this one — and it has to mount this
  repository at `/src/Rivet` **whatever the checkout is called**, because a
  linked worktree is named after the agent that made it and an adopter's manifest
  counts directory levels to a sibling called `Rivet`. That block stops being
  needed on the same day the tag exists.

### 2.6 A manifest key is a contract with the MACHINE, not with the packaging tool — OPEN

* **Port:** recorded by `examples/templates/template-network-service` before the
  Trends port and carried here because it is a general trap, not a template one.
* **The friction.** `cargo rivet package` accepts the keys **this checkout's**
  `cargo-rivet` knows. A board running an older image refuses the whole payload
  — *"the manifest cannot be read: a key is not one this build defines"* — after
  the signature and the members have verified. `title` is the instance that was
  found this way.
* **What would have removed it.** Nothing in the tool alone: the two ends are
  genuinely different versions. What would remove the *cost* is the refusal
  naming the key and the image version that would accept it, so the answer is
  *update the image* rather than *the artifact is corrupt*.

---

## 3. What is NOT friction, and is recorded so it is not "fixed"

Two things the Trends port expected to be friction and were not. Both are worth
leaving alone.

* **The SDK's `serve` documentation answered every question about `serve`.**
  `PLAN.md` §130.1 predicted that a first adopter would end up reading
  `cmd/rivet-fleetd`'s source instead of the module's own documentation, and
  called anything learned that way a documentation defect. It happened **zero
  times**. `ADR-040` §10.4 records it as the good result it is.
* **Adopting the handed descriptor is one line, and nothing downstream moves.**
  Predicted by the migration plan for both ports and true for both. The whole of
  what replaces a `bind` is `listen::socket(name)` and a conversion.
