# ADR-059: The Armada Model — Armadas, Fleets, Members and the operator's tools

* **Status:** Accepted — describes what runs today; `ADR-063` rules its replacement (fleets as the authority, the workstation holding bookmarks), and each part here is rewritten as that migration lands
* **Date:** 2026-09-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

Three symptoms turned out to be one gap. The operator's tools assume a *single
implicit fleet* — the local repo at `~/.rivet/fleet` and the one register
Ignition points at — so there is no way to act on more than one deployment. A
member provisioned on the PC never appears in a fleet server's member list (the
`ADR-034` §2.2.1 promise that members "arrive over the control plane" was never
wired). And "register" means two different things at two levels, which nothing
separated.

This ADR names the levels that already exist implicitly, unifies the
multi-fleet-targeting and provision-to-server problems into one model, and lays
out the lifecycle and the phased build. It **extends `ADR-034`** (the register)
and **`ADR-052`** (the artifact repository); §9 states how.

---

## 2. The model

Two tiers. There is no third.

```
Armada          one operator's whole Rivet estate. Lives on the PC. One per PC.
  ├─ CA          ONE root, shared across everything (§4.1).
  ├─ Repo        ONE artifact repository, org-wide (§4.2).
  ├─ Registry    the list of Fleets — "which deployment am I acting on" (§5).
  └─ Tools       provision, armada, deploy, fleet manager, Rivet Monitor.

Fleet           a collection of Members served by ONE central fleet server.
  ├─ Server      a Rivet board running rivet-fleetd; holds the Fleet's register.
  ├─ Repo        a copy of the Armada repo, serving this Fleet's Members.
  └─ Members     the boards enrolled in this Fleet.

Member          one Rivet board, enrolled in exactly one Fleet.
```

A box that runs on its own is **a Server with no other Members** — a Fleet of
one, named and listed like any Fleet. There is no separate "stand-alone" kind:
`FleetRole::StandAlone` was found to be behaviourally identical to *no role set*
and was removed (ratchet increment #1, 2026-09-22). "Roleless" is simply a box
in no Armada, managed by no one.

---

## 3. Lifecycle

**Provisioning is the ceremony.** One command stands things up and enrolls; there
is no separate "now register it" step, ever.

1. **Bootstrap.** On a clean PC, the first `cargo rivet provision` first asks
   whether this is a new fleet or an existing one, and mints nothing until
   answered (`ADR-063` §8). A new fleet creates the Armada — the CA (already a
   provision concept — kept), the Armada Registry, the artifact repo; joining
   mints nothing and carries an existing workstation's state across instead.
2. **Enroll by role**, during the provision interview:
   * **Server** → its own record is baked onto its card (the `ADR-034` §3.5
     bootstrap record), and the new Fleet is added to the Armada Registry. Works
     with nothing else online.
   * **Member** → its record is POSTed into that Fleet's **running** server
     register (§6.1). Adding a Member therefore requires its Fleet's server to be
     reachable at provision time — normal, since the server predates its members.
3. **Every tool opens with the selector** — Armada → Fleet → Member/Server — so a
   target is always chosen, never assumed (§6.2). The Armada is the one on this
   PC, so in practice the drill-down begins at Fleet.
4. **Backup** is a folder path the Armada tracks; the last N are kept (default 3),
   reusing the retention the repo already has (`prune_keeping_three`).
5. **Rivet Monitor** (§6.3) lists every host. Health monitoring and backups are
   **off by default, per host**, and switched on from each host's flyout. Those
   switches are stored in **Rivet Monitor's own configuration file, which is
   their only authority**; the Armada lists Fleets and how to reach them, and
   holds no Rivet Monitor setting. *(`.rulings/` R-208.)*

### 3.1 Pre-enrollment stays; only the wire is new

The operator mints a device's identity and pin **at provision time** (`ADR-025`
mutual trust); a device does not self-enroll. This is kept — not merely because it
is current, but because self-enrollment would need an admission secret and a
weaker trust story. **The change is only to lay the missing wire** so the
pre-enrollment reaches the Fleet server's register instead of dying on the PC.

---

## 4. Decisions (ruled 2026-09-22)

### 4.1 One root CA, one-and-done

The Armada has **one** root CA — the operator CA that already exists at
`~/.rivet/ca`. **No new roots and no browser certs are ever installed on the
PC**; standing up a new Fleet must not add a setup cost of that kind. Fleet A is
isolated from Fleet B in **normal operation** (distinct per-machine leaves and
each fleet server's own gate); a **failure of the root CA is out of scope**. The
CA design stays small and defers to `ADR-025`/`ADR-046`/`ADR-055` rather than
inventing a per-Fleet hierarchy.

### 4.2 One Armada-wide artifact repository, served per Fleet

There is **one** artifact repository at the Armada level (today's
`~/.rivet/fleet`, `ADR-052`). Each Fleet **server carries its own copy** and
serves its Members directly — which is what `cargo rivet fleet push`/`sync`
already do. **What leaves circulation is decided at the Armada, for every Fleet
at once** (`.rulings/` R-225, `ADR-052` §7.5): a product is deleted from the
Armada repository, or hidden, which withholds it from every Fleet's copy. A
Fleet holds no exclusion of its own, because a Fleet server is not where an
operator decides what the estate offers. Per-Fleet *subset* filtering remains
a separate pivot, not built; desired-state already narrows what actually
installs.

### 4.3 One Armada per PC

The Armada Registry is the estate of the operator at this workstation. The
selector's drill-down is Fleet → Member/Server; the Armada tier is implicit.

### 4.4 Rename by ratchet, not by sweep

Adopting this vocabulary is **not** a stop-the-world rename of `fleet` →
`armada`, `rivet-fleet`, `rivet-fleetd`, `~/.rivet/fleet`. New and touched code
uses the model's vocabulary; existing surfaces are renamed surgically as they are
touched for other reasons; the naming debt is tracked in §8 and shrinks over
time. User-facing command names change last and deliberately, never as a side
effect.

---

## 5. The two-level register

* The **Armada Registry** lives on the PC and lists *Fleets* — each a name, its
  server's endpoint, and how to reach and trust it — plus the Armada's
  backup-folder and keep-N settings. It holds no Rivet Monitor setting (§6.3). It
  answers *"which deployment"* and is the interactive picker's source. It
  **replaces** the members that wrongly live in `~/.rivet/fleet/register` today.
* Each **Fleet register** lives on that Fleet's server (`rivet-fleetd`'s app-data,
  `ADR-034` §2.2.1) and lists *that Fleet's Members*. The workstation is a
  **client** of it: it reads and enrolls, it is not a second writer.

**The PC is a pure client, with a brain.** Member lists are read live from each
Fleet server (`rivet-fleetd`'s `GET /api/fleet`), cached on the PC with a **24-hour
TTL** — fast on repeat use, revalidated when stale, so the cache cannot silently
drift. There is no authoritative PC-side member store.

---

## 6. The tools under the model

All are interactive; **none takes a command-line flag the operator types** (the
standing rule). A tool invoked with no terminal refuses and names its flag-driven
agent sibling, as the deploy interview already does.

### 6.1 Enrollment transport — a control-plane subject, not a portal endpoint

Enrolling a member is a **control operation**, so it rides the control plane
(`rivet-mcpd`, `:7777`, the operator's pinned transport), not `rivet-fleetd`'s
`:4443` browser portal. The register is `rivet-fleetd`'s app-data, but the
control plane is precisely what already writes into an app's data —
`apply install-secret` does exactly this — so a member record is one more such
write. The mechanism (built, §10): a new `apply` **subject**, `enroll-member`,
handled as root by the `rivet` the control plane already spawns; it composes the
path under the server's `rivet-fleetd` `register/devices/<id>.toml` and chowns to
fleetd's uid. `rivet-fleetd` stays a reader and lists the member on its next
`GET /api/fleet`. The operator sends it with the same identity/pin `deploy` and
`secrets` use; the member record travels hex-encoded, as `install-secret`'s bytes
do. This fires from `provision` after the local enroll (§3.2).

> An earlier draft routed this through a new `rivet-fleetd` portal endpoint. The
> maintainer corrected it: there is one control plane and the write goes
> through it, and letting where the data sits dictate where the write goes was
> the wrong turn. (`.rulings/` R-315.)

### 6.2 The shared selector (converge-first, then extract)

Four adopters want the same interactive selection: **provision**, **`cargo rivet
armada`**, the **deploy tool**, and **`cargo rivet shell`** (one day). The
primitives already exist as *divergent copies* — `write_image::choose_with`
(single-select), `fleet_deploy::multi_choose_with`/`parse_device_choice`
(multi-select), `approve_with` (`[Y/n]`/`[y/N]` by danger class),
`refuse_unless_terminal` — plus `rivet-provision`'s own interview in a separate
binary.

Per **converge-first, parameterize-second**: do **not** build a knob-laden
selector framework up front. When the `armada` tool becomes the forcing third
adopter, converge the real implementations into one shape and lift the shared
primitives into a crate both `cargo-rivet` and the separate `rivet-provision`
binary can depend on (a small `rivet-interview`-style crate, or Ignition's
already-adopted CLI patterns). The Armada→Fleet→Member drill-down is composed on
top of those primitives and is Armada-specific.

### 6.3 The tools

* **Provision** — bootstraps the Armada if missing (§3.1); enrolls the box into
  the chosen Fleet by role (§6.1). The Fleet is chosen from the Armada Registry,
  in the interview.
* **`cargo rivet armada`** (new) — the interactive CRUD surface over the Armada
  Registry: list/add/edit/forget Fleets, the backup-folder and keep-N settings,
  and (with an agent/flag twin for scripts) Armada backup/recovery.
* **Deploy tool** (`cargo rivet fleet`, bare) — its host menu reads the chosen
  Fleet server's register (the client model, §5), not the PC-local one.
* **Fleet manager** (`rivet-fleet` today) — becomes Fleet-scoped: reads and edits
  the chosen Fleet server's register (client model), CRUD on Members,
  interactive.
* **Rivet Monitor** (was FleetMonitor) — Armada-aware: it reads the Armada
  Registry's Fleets and **lists every host, and none is monitored or backed up
  until enabled**. Each host has a flyout with a **health-monitoring** switch and
  a **backup** switch, both **off by default, per host**. The switches are stored
  in **Rivet Monitor's own configuration file, and that file is their only
  authority** — the Armada lists Fleets and how to reach them and holds no Rivet
  Monitor setting, and no Rivet tool shows or changes one. Rivet Monitor is a
  separate project that only reads the Armada, so a key it does not understand
  in a Fleet record is skipped rather than refused. This is "appears ≠ acted on"
  — the same rule that keeps a glob from sweeping a dev board into production.
  **One authority is the point**: a switch in the Armada that nothing reads
  beside a switch in Rivet Monitor that decides would be two answers to one
  question, and the one displayed in the Armada would be the one that does
  nothing (`ADR-011`).

---

## 7. What already exists under other names

| Model concept | What it is today |
| :-- | :-- |
| Armada (PC operator role) | `~/.rivet/ca`, `~/.rivet/fleet` repo, `~/.rivet/identity/fleet-operator` |
| Armada CA (one root) | the operator CA at `~/.rivet/ca` (`ADR-025`/`ADR-046`) |
| Armada Repo | `~/.rivet/fleet` objects + manifest (`ADR-052`) |
| Armada Registry | **new** — replaces the members wrongly in `~/.rivet/fleet/register` |
| Fleet server | a `rivet-fleetd` board (rivet-01) |
| Fleet register (Members) | the server-held register `ADR-034` §2.2.1 defines — the one nothing populates |
| Fleet repo | the server's own repository store, fed by `fleet push`/`sync` |
| Member enroll wire | **built** — `apply enroll-member`, a control-plane subject (§6.1, §10) |
| Armada backup / recovery | `fleet objects backup/restore`, `key backup`, `backup-secrets` — scattered, unnamed as a set |

Most of the model is a **reorganization of real machinery**, plus §6.1's one
missing wire.

---

## 8. The rename ratchet (§4.4) and its inventory

The migration is the inventory shrinking to zero. Code written or edited for any
reason adopts the model's vocabulary for the names it touches and strikes its
line; it does not rename neighbours it did not otherwise touch. Behaviour names
(`cargo rivet fleet …`) change last and deliberately.

**Inventory (living):**

* ~~`FleetRole::StandAlone` in `rivetos-register`~~ — **done, ratchet #1, 2026-09-22.**
* ~~`rivet-config/src/fleet_role.rs` — the second, duplicated copy of the enum
  still carries `StandAlone`.~~ — **done, ratchet #2, 2026-09-22** (`rivetos-config`;
  the document-load shim maps a stored `"stand-alone"` to `None`, mirroring
  ratchet #1).
* ~~`FleetRole::ALL`/`RULE` still list three words, and `SPEC-05` §5 still
  describes three roles.~~ — **done, ratchet #2 (cont'd), 2026-09-22.**
  `FleetRole::ALL`/`RULE` in `rivetos-config` now offer exactly `{server,
  member}`; the provisioning interview's generated Fleet question drops
  `"stand-alone"` with them (`interview_tests.rs` pins it), and `SPEC-05` §5
  now describes two roles, noting the retired spelling still loads as no role.
* ~~`cmd/rivet-provision` — its interview uses a plain `fleet: String` with
  literal `"stand-alone"`/`"server"`/`"member"`, not the enum.~~ — **the menu
  side done alongside ratchet #2, 2026-09-22**: the Fleet question's offered
  choices now come from `FleetRole::ALL` (`{server, member}`) and no path
  produces the literal `"stand-alone"` fresh any more. `services_for` and
  `stage::compose` still compare against the literal string — a legacy saved
  answer can still carry it and it must keep meaning no role — so the `&str`
  comparisons stay as deliberate backward-compat, not an oversight; reconcile
  the plain `fleet: String` itself with the enum when the selector converges
  (§6.2).
* Doc/prose still saying "three roles" / `stand-alone` as a role: `ADR-050`,
  `ADR-054`, `ADR-004`, `ADR-007`, `README.md`, and the provisioning
  skills — fix as touched. (`SPEC-05` done above.)
* `~/.rivet/fleet/register` (members-on-the-PC) → the Armada Registry / Fleet
  registers split (§5).
* FleetMonitor → Rivet Monitor (§6.3).
* `rivet-fleet` (binary) → Fleet manager, Fleet-scoped (§6.3).

---

## 9. Relationship to ADR-034 and ADR-052

* **`ADR-034` (the register)** ruled the register lives on a fleet server and the
  workstation is a client. This ADR keeps that and **adds the transport that was
  never built** (§6.1), and names the second register the PC does hold — the
  Armada Registry of Fleets (§5), which is *not* the member register `ADR-034`
  governs.
* **`ADR-052` (the artifact repository)** is the Armada repo (§4.2); `push`/`sync`
  are how a Fleet server gets its copy. Unchanged in mechanism; renamed in concept.

---

## 10. Implementation status (as built, 2026-09-22)

This section first phased the *wire* into "Phase 1" and the *model* into "Phase
2". That was **backwards** — the wire's inputs (which Fleet, where the record
belongs, how the server is reached) come from the model — and the maintainer
corrected it. The model was built first; the wire draws from it.

### Built, tested, and — for the board half — shipped and verified

* **The enroll wire.**
  * **Server: `apply enroll-member`**, a new control-plane *subject* in
    `rivetos-core` (`operation/enroll_member.rs`). It is `install-secret`'s
    write-and-chown act with the path composed under a Fleet server's
    `rivet-fleetd` `register/devices/<id>.toml` and the owner fixed at fleetd's
    uid; it refuses a machine that is not a fleet server. **Not the portal
    endpoint an earlier draft imagined** (§6.1) — the register is fleetd's data
    and the control plane is what already writes into an app's data
    (`install-secret`). 6 unit tests.
  * **Client: `enroll_member`** in `cargo-rivet` — renders the member's
    `Device`, hex-encodes it, sends `apply enroll-member … confirm` over the
    pinned control plane (`secrets`' shape). 12 unit tests.
  * **Shipped and verified on rivet-01.** The subject is in `rivetos-core` → the
    `rivet` binary, so it rode a **system update** (card build → `build-update`
    → `deploy … confirm` → `trial-boot b` → `trial-restart`), not an app deploy.
    rivet-01 runs **0.1.109 on slot b, committed and healthy**, and
    `get catalogue apply enroll-member` confirms the subject is present and
    reachable — verified without writing a real member into the production
    register.
* **The Armada Registry model** (`rivetos-register::armada`): `Armada`/`Fleet`/
  `Server`/`Settings` + store (§5), unit-tested.
* **`cargo rivet armada`**: the CRUD tool over it (§6.3), unit-tested. **rivet-01 is
  bootstrapped into the Armada Registry** as a Fleet.
* **Provision wiring** (§3.2): for a Member, `provision` enrolls it into its
  Fleet's server after the local enroll (`enroll_member_into_fleet`) — reported and
  never fatal; a single Fleet is auto, several are deferred to the selector.
  For a **Server**, `provision` adds the new Fleet to the Armada Registry after
  its card is written and verified and its record enrolled, carrying the
  `rivet-fleetd` operator key its staging bundle names
  (`cmd/cargo-rivet/src/armada.rs`'s `bootstrap_provisioned_server`) — when the Armada holds no
  Fleet or already holds that one; an Armada holding others is left alone and
  the report names `cargo rivet armada add`. That key is what the next member's
  interview stages into its `peers` (`ADR-049` §12).

### The one remaining step — gated on hardware

The full acceptance test — **provision the dev board as a Member → it appears in
rivet-01's member list → deploy to it** — needs the dev-board card written, the
maintainer's physical step. Everything upstream is in place and rivet-01 answers
`apply enroll-member`.

### A bootstrap detail worth a ruling

`cargo rivet armada`'s "add a Fleet" gates on the device's `fleet_role == Server`.
rivet-01's register record carried only the free-text `role = "fleet-server"`
label, not the `fleet_role` field, so `fleet_role` was set to `server`
(`rivet-fleet set fleet-role`) before the add. Whether the tool should instead
honour the label, or let the operator declare any reachable device a server, is a
small open call — noted, not decided.

### Later phases

* **The deploy tool reading the Fleet server's member register** behind the
  24-hour cache (§5) — it still reads the PC register; re-point it when the
  selector converges.
* The **shared selector** (§6.2), **Fleet manager**
  re-pointing, and the remaining §8 ratchet items (the `cmd/rivet-provision`
  plain-`String` reconciliation, the doc/prose sweep, the other renames).
  Ratchet #2 (`rivet-config`'s enum, `ALL`/`RULE` and the menu) is done.
  Per-Fleet repo *subset* filtering (§4.2) remains a future pivot.

---

*This ADR was promoted from `docs/design-candidates/Armada-Model.md` after the
2026-09-22 grill. The candidate is retired.*

---

## History

### 2026-09-23 — Rivet Monitor's switches left the Armada

**Why it changed.** The Armada carried per-Fleet `monitor` and `backup` flags,
and nothing in Rivet ever read them: `cargo rivet armada` only displayed and
toggled them. Meanwhile Rivet Monitor built the flyout and the default-off rule
itself, **per host**, stored in its own configuration file. That left two
switches for one fact, one of which controlled nothing. The maintainer ruled that
Rivet Monitor's own per-host switches are the only authority for whether a
machine is health-monitored or backed up, and retired the Armada's flags from the
schema, because a setting with no reader is worth nothing. A Fleet record
that still carries the two keys loads, and they are dropped on its next save.
`cargo rivet armada` lost `toggle-monitor`, `toggle-backup`, their menu entries
and its list columns.

Superseded text, verbatim:

* §3, item 5: ~~**Rivet Monitor** (§6.3) shows every host but monitors or backs
  up none until the operator enables it per host.~~
* §5: ~~plus the per-host Monitor and Backup flags (§6.3) and the Armada's
  backup-folder and keep-N settings.~~
* §6.3: ~~Each host has a flyout with **Monitor** (default off) and **Backup**
  (default off); the flags live on the host's Armada Registry entry.~~
* §10: ~~**rivet-01 is bootstrapped into the Armada Registry** as a Fleet
  (`monitor off, backup off`).~~
