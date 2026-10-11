# Rivet in a Container — design questions for the maintainer

* **Status:** Questions only. No decision taken, no ADR. For a design session
  before any ADR is written.
* **Date:** 2026-10-02
* **Reads with:** `AUDIT.md` in this folder.

---

Each question states the decision, the options, and a lean — so the session is a
reaction, not a blank page. The leans are the author's, held loosely. They are
grouped: **A** shape the thing, **B** decide behaviour, **C** scope the work.

---

## A. Shape

### Q1. What exactly gets extracted, and what is it called?

The enabling work is pulling the supervisor out of the binary into a library
both `cmd/rivet-sysd` and `cmd/docker-sysd` link. The question is the seam.

* **(a)** Extract the whole supervisor domain — discovery, document application,
  settings, the `supervise` engine, health, retention — into one
  `rivet-supervisor` library. Both binaries become thin composition roots over
  it. The `Kernel` trait moves to the library; `LibcKernel` and a new
  `DockerKernel` are the two implementors.
* **(b)** Extract only the convergence engine and manifest handling; leave the
  rest in `cmd/rivet-sysd` and have `docker-sysd` re-implement the thin parts.
* **(c)** Do not extract; `docker-sysd` is its own crate that duplicates.

**Lean: (a).** Only (a) gives the manifest contract one home (`ADR-011`). (c) is the fork the audit warns against; (b) draws the seam in
a place that will move. The cost of (a) is a real but mechanical refactor, and
it is a forcing function: anywhere the extraction snags is a place the binary
quietly assumed a Pi, which we want to find regardless.

### Q2. One application per container, or a mini-fleet per container?

* **(a)** One `.rvt` per container — `docker-sysd` reads one manifest, runs one
  service. Docker-idiomatic; the image *is* the product. Compose multiple with
  `docker compose`.
* **(b)** A full mini-fleet in one container — the proxy plus several
  applications, as a board carries them.

**Lean: (a) first, keep (b) possible.** A single-service container is the thing
an adopter wants (a Trends server), is simplest, and matches how Docker is used.
The extracted library should not *preclude* (b), but (b) is not the first
article.

### Q3. Where does `docker-sysd` live, and how does it relate to the build
containers and the QEMU harness?

`tools/container/` already runs builds in Docker; the QEMU harness runs a whole
emulated board. `docker-sysd` is a third, lighter thing: userland supervision of
one service, no kernel boot.

* **(a)** New `cmd/docker-sysd` binary + a `tools/` recipe that builds the
  runtime image. Distinct from both existing tools.
* **(b)** Fold into the QEMU harness as a "no-emulation" mode.

**Lean: (a).** It is a different lifecycle (a shipped runtime, not a test rig)
and conflating it with the harness would overload both.

---

## B. Behaviour

### Q4. Confinement: Landlock, or lean on the container boundary?

A container is already an isolation boundary. Landlock works inside one only if
the kernel supports it and seccomp does not block the syscalls — not guaranteed
across Docker hosts.

* **(a)** Best-effort Landlock: apply it when available, journal plainly when
  not, never fail the launch for its absence.
* **(b)** Rely on the container as the boundary; do not attempt Landlock.
* **(c)** Require it; refuse to run where it is unavailable.

**Lean: (a).** It matches the board's own "NOT ENFORCED, and say so" posture and
never fakes a confinement it does not have. (c) would make the runtime unusable
on common hosts; (b) throws away protection that is often available.

### Q5. Per-application uids inside the container — keep or collapse?

On a board each app gets its own uid (`ADR-015` §5.1.1). In a single-service
container the container already isolates the one service.

* **(a)** Keep uid minting — the manifest's contract is honored identically,
  and a mini-fleet (Q2b) would need it anyway.
* **(b)** Run the one service as one unprivileged uid; the container is the
  boundary.

**Lean: (a) for fidelity, but this is genuinely open** — (b) is simpler and a
single-service container may not need the ceremony. Depends on Q2.

### Q6. Resource limits — honor the manifest's, or defer to Docker?

The manifest can declare memory/cpu caps; on a board these become cgroup limits.
Inside a container, cgroup delegation is restricted and Docker has its own
`--memory`/`--cpus`.

* **(a)** Defer to Docker: `docker-sysd` refuses (does not silently ignore) a
  manifest memory/cpu cap and documents that the container runtime enforces it.
* **(b)** Attempt an in-container cgroup when delegation is available;
  best-effort like Landlock.

**Lean: (a).** Two authorities over one limit is the thing to avoid; the
container runtime is the natural one here. Refuse-and-say, never fake.

### Q7. Configuration surface — how is a container told its desired state?

A board reads the desired-state document and is managed over the control plane.
A container needs an equivalent.

* **(a)** A mounted `machine.toml` (the same document schema), read at start —
  declarative, versionable, Docker-idiomatic.
* **(b)** Environment variables mapped to settings.
* **(c)** Keep `rivet-mcpd` in the container for live management.

**Lean: (a), optionally (c) later.** One document schema everywhere keeps the
authority single. Env (b) is a second schema. The control plane (c) is worth
having for parity but is not needed to *run* a server and can come second.

### Q8. Does the container carry the control plane at all?

* **(a)** No control plane by default — the container is configured
  declaratively and observed via logs/Docker. Add `rivet-mcpd` only when a
  product wants live management.
* **(b)** Always include it, for parity with a board.

**Lean: (a).** Smaller surface, fewer ports, matches the single-service shape.
It also sidesteps carrying the per-source governor work into a context where
Docker's own network controls already apply.

---

## C. Scope

### Q9. Confirm the refuse-list.

`docker-sysd` refuses, rather than fakes, anything it cannot honor. Proposed
refuse-list: **kiosk manifests** (display/DRM/compositor/input), **anything
A/B-, card-, or tryboot-shaped**, and (Q6) **manifest resource caps**. Confirm,
and name anything to add or remove.

**Lean:** as listed. The test: if the container cannot provide it, refuse at
launch with the reason, never start a service that believes it has something it
does not.

### Q10. First article — prove one real server, or design the general runtime?

* **(a)** Build `docker-sysd` just far enough to run **one** real server (Trends
  or another adopter's) end-to-end in a container, and let that shape the general design.
* **(b)** Design the general runtime fully first, then adopt a server onto it.

**Lean: (a).** The project's own style is a first article that finds what the
design missed — exactly as the on-board flood test found the journal gap the
unit tests could not. Pick Trends or another adopter's server as the subject.

### Q11. State and backups in this mode.

`/state` becomes a named Docker volume. The backup/restore contract is Rivet's
(sealed, typed). In a container, does backup stay Rivet's operation, or does the
operator back up the volume with Docker's own tools?

* **(a)** Keep Rivet's backup operation — one backup story across board and
  container.
* **(b)** Let the volume be backed up by ordinary Docker/host tooling; Rivet's
  backup is a board concern.

**Lean: open, and it depends on Q8** — Rivet's backup today runs through the
control plane. Without a control plane in the container, (b) is simpler; with
one, (a) keeps parity. Worth deciding once Q8 is settled.

### Q12. What guarantees the subset never drifts from the board?

The audit's central risk: two composition roots, one manifest contract. How is
"strict subset, refuses what it cannot honor" *enforced*, not just intended?

* **(a)** The extracted library owns the manifest parse and the refusal rules;
  both binaries get them for free, and a test asserts `docker-sysd` refuses each
  unsupported feature by name.
* **(b)** Convention and review.

**Lean: (a).** Make the library refuse, so the refusal is one implementation
tested once — the same reasoning as one verifier, not two (`ADR-028` §2).
