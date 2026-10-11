# Interactive Fleet Deployment — a design candidate

* **Status:** Ruled (§9) and **Phase 1 implemented** — 2026-09-22. Phase 2 (§10.4) outstanding.
* **Date:** 2026-09-22
* **Target Audience:** the maintainer, and the AI engineering system that would build it.

---

## 1. The problem, from the rebuild

`rivet-01` is about to be rebuilt into its final shape, and five applications —
Reliquary, Trends, Aura, TempMon and others — must land on it. Several carry
**post-install steps**: fleet-TLS material has to be staged, a service enabled,
the proxy restarted when a listen layout changed, sometimes a reboot, and — for
a rebuild specifically — each app's **data restored** from its last backup.

Today that is a command-line ritual: `cargo rivet deploy` per app, read the
"refused — here is the command" output, run `cargo rivet secrets`, restart,
repeat. **That ritual is the agent's job or a human typing flags, and the
maintainer has ruled he will not be the one typing it.** The provisioning tool
already set the bar: `cargo rivet provision` is invoked once and is then an
interview — lists, selections, yes/no. This proposes the same shape for
deploying applications to machines that already exist.

---

## 2. What this is NOT

* **Not a new command-line surface.** No `deploy --all --host --confirm`. The
  maintainer types no flags. One invocation, then menus.
* **Not a daemon or a standing orchestrator.** It runs when you deploy, the way
  `provision` runs when you image. The fleet is small and this is a
  human-in-the-loop act, not automation.
* **Not a channel for applications to ship commands.** See §4.1 — this is the
  load-bearing security line.

---

## 3. The flow (the provision mold)

Invoked once (name is §8's open question), then fully interactive:

1. **Artifact menu.** The fleet repository's artifacts — product, version, kind,
   arch, size, date. Selectable. **An `Exit` entry leaves the tool from here**,
   and every path returns here.
2. **Host menu.** The register's known machines — name, whether it answers, and
   what version of the chosen artifact it currently runs (so a no-op is visible
   before it happens). Selectable. Identity and pin come from the register; none
   are typed.
3. **Plan and post-install review.** For the chosen artifact(s) × host(s), the
   tool computes the install plus every unmet need and detected consequence, and
   presents each as an approve line with a **danger-class default** — `(Y/n)` for
   safe, `(y/N)` for dangerous (§4.2). Approved in place; nothing is typed.
4. **Execute.** Runs the approved steps, one host at a time, showing each step's
   result. A refused or failed step is shown and stops that host's remaining
   steps rather than being swallowed; other hosts continue.
5. **Return to the artifact menu.** The loop, so the next application is one more
   pass. `Exit` from there ends the session.

---

## 4. Design principles

### 4.1 Applications declare NEEDS; the tool knows REMEDIATIONS

`material = ["fleet-tls"]` is safe because the application says *what it needs*
and the tool knows *how to satisfy it* (stage the fleet cert/key/root from the
operator's own CA). If a manifest could instead carry `post-install = "…"`
command strings, deploying a compromised or malicious `.rvt` would run arbitrary
commands under the operator's identity — a supply-chain hole straight through
Rivet's trust model. So the tool acts on a **fixed, tool-understood vocabulary of
needs** (§5), grown only by teaching the tool a new need-type — never by an
application shipping a script.

### 4.2 Danger class decides the default answer

Every step carries a danger class, and the class — not the step — sets the
prompt default. The maintainer never configures this per step:

* **Safe** → `(Y/n)`: reversible, brief, or non-destructive.
* **Dangerous** → `(y/N)`: destroys data, holds a private key, or takes the
  machine down.

### 4.3 Existing safety gates are preserved, not bypassed

The secret-store classifier that gates a private-key write still gates it here.
An automated, friendly front-end must not become a way around the gate that
exists precisely for the write it is now making easy. The tool asks; the gate
still rules.

### 4.4 Idempotent and re-runnable

Installing a digest a machine already holds is a no-op (`ADR-052`). A need
already met is shown as met and offered nothing. So a run interrupted halfway is
re-run without harm, and a rebuild that redeploys the same versions is honest
about what actually changes.

---

## 5. The needs vocabulary (fixed, tool-understood)

| Need | Source | Remediation the tool runs | Danger | Default |
| :-- | :-- | :-- | :-- | :-- |
| Install the artifact | selection | transfer + install by digest | safe | `(Y/n)` |
| `material` unmet (e.g. `fleet-tls`) | manifest + host state | stage cert/key/root from the operator CA | **dangerous** (private key) + gate | `(y/N)` |
| `enabled` not set | machine document | add `[services.<name>] enabled = true` | safe | `(Y/n)` |
| service not (re)started after install | tool | `service-restart` | safe | `(Y/n)` |
| listen layout changed | tool-detected (manifest diff) | restart `rivet-proxyd` to re-derive the route | safe | `(Y/n)` |
| `settings` undelivered *(deferred — Phase 2, no value source)* | manifest + doc | write the setting into the document | safe | `(Y/n)` |
| **data restore** (rebuild) *(deferred — ruling 2)* | operator choice + a backup | POST the app's own backup to its `restore-path` | **dangerous** (REPLACE) | `(y/N)` |
| reboot required | tool | reboot the machine | **dangerous** (downtime) | `(y/N)` |
| phone pairing expected | manifest hint | mint + display an enrollment code | interactive, not destructive | `(Y/n)` |

The **data-restore** row is the one a rebuild makes central: the point of
rebuilding is to get the applications back *with their data*, and their backups
are what FleetMonitor already pulls to the Windows PC. Making "restore this
app's latest backup" a proffered step is most of the rebuild's value.

---

## 6. What is actually a control-plane gap (and what is not)

**The mutating primitives already exist** and need nothing: install-by-digest,
staging material (secret writes), `service-restart`, editing the machine
document, reboot, and the `restore-path`/`backup-path` fetch. The tool
orchestrates them; it adds no new *power*.

**The genuine gaps are read-side and detection**, and only these are worth
building beneath the tool:

* **6.1 A consolidated "unmet needs" query.** Today `deploy` computes the
  *material* gap inline and prints a sentence; nothing answers "for artifact X on
  host Y, what post-install steps are outstanding?" as data a tool can render.
  This is a read, not a new capability to change the machine.
* **6.2 Listen-layout-change detection.** The proxy-must-restart consequence is
  found by hand today (it cost a debugging session on Trends' two-door). The
  plane could compute it by diffing the installed manifest against the incoming
  one.

Everything else the tool needs, it already has. **No human command-line surface
is added.**

---

## 7. Relationship to the rest

* **Sibling to `provision`, not a replacement.** Provision sets up a *new
  machine*; this deploys *applications* to machines that already exist. Same
  interaction philosophy, different verb.
* **FleetMonitor** already pulls backups on a schedule; this tool is the *push*
  direction (deploy + restore) and would read the same backup store for §5's
  restore row.
* **The agent still uses the flags.** `cargo rivet deploy --rvt … confirm` stays
  exactly as it is — it is the machine/script/agent interface, and this tool is
  the *human* one. Neither replaces the other.

---

## 8. Open questions — for the grill

1. **Batch shape.** Multi-select artifacts × multi-select hosts as one batch, or
   one artifact → its hosts → loop? "Return to the artifact list" reads as a
   loop; "deploy them to hosts" reads as multi-select.
2. **Approve granularity.** Per-step yes/no for everything, or "auto-yes the safe
   ones, stop only for the dangerous ones"? The second is less clicking but hides
   the safe steps.
3. **Does data-restore belong in this tool** (§5), or is restoring an app's data
   a separate act you'd rather keep deliberate and apart from deployment?
4. **Invocation name.** `cargo rivet fleet`? `cargo rivet deploy` with no
   arguments dropping into the interview? Something else?
5. **Danger classes.** Do you agree with §5's table — in particular, is a proxy
   restart safe `(Y/n)`, and is a service restart safe `(Y/n)`?
6. **Missing needs.** Is anything absent from §5 that your five apps actually
   require on a fresh machine?
7. **Scope of the first build.** The whole thing, or a first cut that deploys +
   stages material + enables (the mechanical rebuild), with restore and
   proxy-detection as a second pass?

---

## 9. Rulings — 2026-09-22

The maintainer ruled all seven:

1. **Batch shape:** multi-select artifacts × host(s), one combined review, loop
   back to the artifact list, exit from there.
2. **Data-restore is deferred** — the maintainer's second thought, 2026-09-22:
   hold off on the tool offering restore for now. Deployment and
   data-replacement stay apart; restoring a backup remains the deliberate,
   separate act `cargo rivet restore` already is. It stays catalogued in §5 as a
   known need-type, but nothing in §10 builds it.
3. **Approve style:** per-step prompt with smart defaults — `Enter` takes the
   default, so safe steps `(Y/n)` fly by and dangerous ones `(y/N)` pause.
4. **Invocation:** `cargo rivet fleet` with no arguments drops into the
   interview. It coexists with the existing `fleet` subverbs (`list`, `publish`,
   `push`, …) — bare is the interview, `fleet <verb>` stays as it is.
5. **Danger classes:** §5's table as written (install / enable / service-restart
   / proxy-restart / settings = safe; material / restore / reboot = dangerous).
6. **First build is the mechanical cut**, restore + precise listen-change
   detection in a second pass.
7. **Needs vocabulary is complete** for the five apps as of now.

---

## 10. Implementation plan

### 10.1 Where it lives, and what it reuses

A new interactive mode of `cmd/cargo-rivet`'s existing `fleet` subcommand: bare
`cargo rivet fleet` enters the interview, in the mold of `provision`. **It adds
no new control-plane power.** Everything it does already exists as a cargo-rivet
operation or a control-plane read — the tool is a human front-end that composes
them:

* **Reads** (to compute state and needs): the fleet repository listing (as
  `fleet list` already does); the register (as `deploy <device>` already resolves
  host/identity/pin); and per host, `get status`, `get install`, and `get file`
  on the machine document, the installed `manifest.toml`, and the app data
  directory (to see the installed digest, the enabled flag, the current listen
  layout, and whether material is present).
* **Mutations** (each already implemented): `deploy` (publish + send +
  install-by-digest), `secrets` (stage material), `apply edit` (enable),
  `apply service-restart` (service and proxy), `apply restart` (reboot),
  `backup`/`restore`.

**The one refactor this requires:** those verbs' cores are entangled with their
argument parsing today. Extract each into a callable function that takes a struct
of inputs and returns a result, so the interview calls them in-process rather
than shelling out. This is the bulk of the non-UI work and it is worth doing on
its own — it also makes the verbs testable.

### 10.2 The needs computation (the heart of it)

A pure function: given an artifact's manifest and a host's read-back state,
return the ordered list of steps, each tagged with its danger class (§5). No I/O,
so it is unit-tested directly. Steps and how each is decided:

| Step | Emitted when |
| :-- | :-- |
| install | the host's current digest ≠ the artifact's (else shown as "already current") |
| stage material | the manifest declares `material` and a declared file is absent from the data dir |
| enable | the machine document has no `enabled = true` for the service |
| service-restart | an install happened this pass |
| proxy-restart | the artifact's `listen` differs from the installed manifest's (Phase 2 makes this exact; Phase 1 offers it whenever `proxy-path` is set and an install happened) |
| deliver a setting | the manifest declares a `setting` the document does not carry |
| reboot | never emitted automatically in the first cut; offered only if a step reports it is required |

Data-restore is **deferred** (ruling 2) and is deliberately not a step here.

### 10.3 Phase 1 — the mechanical rebuild

1. **Entry:** `cargo rivet fleet` with no args → the interview; connect to the
   fleet repo and the register.
2. **Artifact menu:** list the repo's artifacts; multi-select; an `Exit` entry
   that leaves, and the loop returns here.
3. **Host menu:** list the register's machines with reach + current version;
   multi-select; credentials from the register, none typed.
4. **Plan:** run 10.2 per (artifact, host); render the steps grouped by host.
5. **Approve:** per-step prompt, default from the danger class (§5). The
   secret-store gate still fires on a material write — the tool's `(y/N)` is not
   a substitute for it.
6. **Execute:** one host at a time; show each step's result; a failed/denied step
   stops that host's remaining steps and is reported, not swallowed; other hosts
   continue.
7. **Loop** back to the artifact menu.

Phase 1 delivers a full rebuild bar the data: install, material, enable, service
and proxy restart.

**As built (2026-09-22).** `cmd/cargo-rivet/src/fleet_deploy_plan.rs` is the pure
planner (§10.2), `fleet_deploy.rs` the interview, wired to bare `cargo rivet
fleet`. Two Phase-1 steps were deliberately **not** surfaced yet, each because
Phase 1 has no honest way to complete it:

* **Setting delivery is deferred to Phase 2.** The planner knows the
  `DeliverSetting` step, but *delivering* a setting needs its **value**, and the
  maintainer has ruled the tool asks for no typing (§2). There is no no-typing
  source for a setting value today, so the interview seeds `settings_present`
  with the artifact's own declared settings, which suppresses the step rather
  than prompting for something the tool cannot carry out. It returns in Phase 2
  once a value source exists (a fleet default, or a reviewed prompt the
  maintainer opts into).
* **"Already current" display is deferred to Phase 2.** There is no cheap read
  of *which digest a host is running* yet (that is Phase 2's `get install`
  parse), so the interview leaves `current_digest` unknown and always offers
  install. This is harmless: an install of an artifact a machine already runs is
  refused by the board before it writes anything (`ADR-052` §7's
  `ALREADY_RUNNING`) and reported as "already running", not a failure.

The no-terminal refusal names the **whole** set of flag-driven verbs the
interview composes — `deploy`, `secrets`, and `run -- apply service-restart` —
not just `deploy`, so a scripted/agent caller redirected out of the interview
lands on tools that can do every step, not only the install (§7).

### 10.4 Phase 2 — precise detection, and parity for the agent path

* **Exact listen-change detection:** diff the incoming manifest's `listen`
  against the installed one so proxy-restart is offered only when it is actually
  needed, rather than on every proxied install.
* **"Already current" precision:** a `get install` read so the host menu and the
  plan show what each machine currently runs before a pass, and a redundant
  install is shown as a no-op rather than offered.
* **Setting delivery:** once a value source exists (above), the suppressed
  `DeliverSetting` step becomes real.
* **The planner advises the agent path too — so everyone benefits.** The
  needs-planner (`deploy_plan::plan`) is a pure, shared brain; today only the
  human interview consumes it. Have `cargo rivet deploy` run the *same* planner
  after a deploy and print the outstanding remediations with their exact
  commands — most importantly the **proxy restart after a listen-layout change**,
  which is the one remediation the flag surface surfaces *nowhere* today (the
  trap that cost a debugging session on Trends' two-door; `memory:
  rivet-proxy-restart-after-listen-change`). Material is already guided
  (`refuse_if_material_missing`), enable is a documented flag (`--enable`, and
  `rivet-sysd` re-reads the document as an install finishes), and reboot is
  already pointed at (`apply trial-restart`). The proxy restart is the hole, and
  one shared planner closes it: the human gets the steps *automated*, the agent
  gets them *advised*, from one source of truth that cannot drift.

Data-restore is **deferred** (ruling 2) — not built in either phase for now. If
it returns, it slots in as one more §10.2 need-type (POST the app's latest
backup to its `restore-path`, dangerous `(y/N)`) with no other change to the
tool's shape.

### 10.5 Cross-cutting invariants

* **Declared needs, never command strings** (§4.1) — the vocabulary is fixed and
  tool-understood; a manifest can never make the tool run an arbitrary command.
* **Existing gates preserved** (§4.3) — the secret-store classifier still gates
  every private-key write.
* **Idempotent** (§4.4) — a met need is shown as met; a re-run after a partial is
  safe.

### 10.6 Testing

* **Unit:** the 10.2 needs function, exhaustively — a manifest + a host-state
  fixture in, the expected step list with danger classes out. This is where the
  correctness lives and it needs no board.
* **The extracted verb cores** get their own tests, which they lack today.
* **Integration:** one QEMU/board round-trip — deploy an app through the
  interview path (driven non-interactively in the test) and assert the same end
  state a hand `deploy` + `secrets` reaches, matching how `screenshot` and
  `backup` are already exercised.

### 10.7 Non-goals

Not a daemon; not a scheduler (FleetMonitor already pulls on a cadence); not a
general multi-tenant orchestrator. It runs when the operator deploys, for a fleet
that is small on purpose.
