# ADR-060: The Front Door Is Part of the Operating System — `rivet-proxyd` moves into the image

* **Status:** Accepted
* **Date:** 2026-09-23
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Ruled:** by the maintainer, 2026-09-23 (`.rulings/` R-209)
* **Reverses:** `ADR-050` §4 for `rivet-proxyd` only. `rivet-fleetd` is unchanged.
* **Code:** `rivet-core/src/service.rs` (`PROXYD_SERVICE_NAME`, `PROXYD_PATH`,
  `IMAGE_SERVICE_HOMES`, `IMAGE_SERVICE_NAMES`, `PROXY_IN_IMAGE_SINCE`),
  `rivet-core/src/confinement.rs` (`image_service_policy`),
  `cmd/rivet-image` (`--proxyd`), `cmd/rivet-sysd/src/definitions.rs`
  (`proxyd`), `cmd/rivet-sysd/src/applications.rs` (`provide_image_homes`,
  `retire_carried_image_services`), `cmd/rivet-proxyd` (the default upstream
  name)

---

## 1. Context

`ADR-050` §4 made the path proxy a **carried application**: a signed `.rvt` the
card brings along and the machine installs on its first convergence. The
argument was speed of repair. Infrastructure is the software most likely to
need a fast fix, and a carried payload can be replaced with `cargo rivet
deploy` in about a minute where a system update takes a new image, a trial boot
and a restart.

**That benefit has hardly been drawn on.** The proxy changes rarely, and the
fast-fix route it was carried for has almost never been the route a fix took.

**The machinery that carrying requires has cost something again and again.** A
carried product has a published version in the fleet repository, so a machine
document pins one (`ADR-050` §5.2), and a recorded document keeps pinning it
after the repository has moved on. A document that enables a carried service
demands a staging bundle beside it. A card build refuses when either is out of
step. Each of those has had to be repaired by hand during a reimage, more than
once, and none of them is about the proxy's own behaviour at all.

**So the trade is made the other way.** A speed of repair that went unused is
given up, and the cost that kept being paid goes with it. The proxy becomes
part of the operating system, versioned with it, shipped in every image, and
fixed by a system update.

---

## 2. The decision

**`rivet-proxyd` is an image service** — built into the system image at
`/system/bin/rivet-proxyd` and defined by `cmd/rivet-sysd` like every other
image service. It is not published, carried, installed or uninstalled.

**It still runs in user space, and that is not a compromise.** A service being
part of the operating system says who ships it and when it changes; it says
nothing about what privilege it needs. The proxy needs none — it is handed a
listening socket, reads its certificate, and forwards bytes — so it keeps
exactly the confinement it had as an application:

| | As a carried application (before) | As an image service (now) |
| :-- | :-- | :-- |
| Program | `/state/app/rivet-proxyd/current/bin/rivet-proxyd` | `/system/bin/rivet-proxyd`, read-only |
| Identity | `app_identity("rivet-proxyd")` | **the same uid and gid** |
| Home | `/state/app/rivet-proxyd/` | **the same directory** |
| Ruleset | `confinement::policy` over its manifest | `confinement::image_service_policy` — the same rows, with the program file in place of `current` (`ADR-042` §4) |
| Socket | `tcp:443`, bound by the supervisor on every address | the same |
| Settings | `[applications.rivet-proxyd]`, checked against its manifest | the same table, checked against `IMAGE_SERVICE_HOMES` |
| Staged material | `/state/staging/rivet-proxyd.txt`, delivered into `data/` | the same file, delivered the same way |
| Whether it runs | installed, then `[services.rivet-proxyd] enabled` | **`[services.rivet-proxyd] enabled`** — the role |
| How it is fixed | `cargo rivet deploy` | a system update |

Keeping the uid and the home unchanged is what makes the move cheap: every file
a running proxy already owns — its certificate, key and fleet root — stays where
it is, owned by the uid that will read it after the update.

### 2.1 An image service with a home is a shape, not a special case

`rivet_core::service::IMAGE_SERVICE_HOMES` names the image services that run
this way, each with its program and the settings keys it understands — the
declaration an application makes in its manifest and an image service has no
manifest to make. The supervisor treats every entry alike:

* the definition carries `identity: Some(app_identity(name))`, a confinement
  from `image_service_policy`, and `volume: Some("/state")`, and the
  environment an application is handed — `RIVET_APP_DATA`,
  `RIVET_APP_SETTINGS`, `RIVET_APP_TMP` and `TMPDIR`;
* at boot the home directory is made and owned as an installer would make it,
  `data/` and `tmp/` are made and given to the uid, and a staged bundle
  (`ADR-049`) is delivered;
* `/etc/passwd` gets the account, because it is derived from every definition
  with an identity and a volume;
* the document's `[applications.<name>]` table is delivered as its settings
  file, checked against the declared keys.

**Two things an application gets do not apply.** Launch-time signature
verification (`ADR-015` §4.2) is for programs on writable `/state`; the proxy
is on the system slot, which the update mechanism already verified. And an
uninstall of an image service is refused, as for every image service.

`ServiceDef::validate` refuses an image-defined service that carries a
confinement without both an identity and a volume: a ruleset for root would
confine a program that can undo it, and a home with no volume has nowhere to be.

### 2.2 Whether it runs is the role

The proxy starts `enabled: false`, like every image service that is a policy
choice (`ADR-050` §2's tier 2), and the document turns it on. The role tables
already say which documents do — a fleet server and a fleet member both enable
`rivet-proxyd` (`ADR-050` §5.1) — so the tables are unchanged and only what
they cause is different: an enabled service that the image defines, rather than
an application the card must carry.

---

## 3. Moving a machine that already has the proxy installed

**Every machine whose document enables the proxy switches at its next update.**
Nothing is asked of the operator.

On the first boot of an image that carries the proxy, the supervisor looks for
an installed `rivet-proxyd` application before it discovers applications, and
if it finds one it **removes it**: the `current` link and every version
directory go, and the `data/` directory stays, because the image's proxy runs
as the same uid from the same home and reads the certificate and key already
in it. The removal is journalled once, at `Info`, naming what was removed and
what was kept. Every later boot finds nothing to remove and says nothing.

The removal is deliberate rather than the leftover being kept for a rollback. A
machine that rolls back to the previous slot boots an image with no proxy in
it, so the application would be what served 443 there; that was weighed, and
a clean tree was preferred to a copy that could only ever matter after a
rollback.

It is also gated on the image actually carrying the program: an image without
`/system/bin/rivet-proxyd` removes nothing, because deleting the only proxy a
machine has is never the right answer.

### 3.1 The upstream name has a default

The proxy verifies the applications behind it against one name, the
`upstream-name` setting. **With no setting, it uses the first DNS name on its
own certificate** — the certificate the fleet authority issued this machine for
the names it answers to, which is the name every application on the machine is
served under. A setting still wins, and a certificate with no DNS name and no
setting is refused at start, naming both.

---

## 4. Fixes ship as system updates

A proxy fix is a new image and a system update (`ADR-018`). The routes that
used to accept a proxy payload refuse one, each for the same reason — the name
is an image service's:

| Route | What happens to a `rivet-proxyd` `.rvt` |
| :-- | :-- |
| the machine's install door and the on-board installer | refused (`rivet_core::package::verify::not_an_image_service`) |
| `cargo rivet fleet publish` | refused before it is added to the repository |
| `cargo rivet fleet sync` | skipped, and the skip is reported by name |
| `cmd/rivet-image --application` | refused before a partition is written |
| a role's carried payloads | never selected: `Document::carried_services` excludes every image service |

Proxy payloads already in a fleet repository are left where they are: nothing
selects them for a card, and a sync names each one it skips. Retention keeps the
newest few of a product, so with nothing newer published they stay until they
are removed.

---

## 5. What provisioning requires

**A server or member card is refused for an image older than the first release
that carries the proxy.** Such an image has no proxy, and a card that can no
longer carry one either would come up with no front door.
`rivet_core::service::PROXY_IN_IMAGE_SINCE` names that release, and `cargo rivet
provision` compares it against the version of an image selected from the fleet
repository. An image named by a file path has no recorded version on the
workstation; provisioning says it could not check, and writes it.

**A staging bundle is required by role.** A document that enables `rivet-fleetd`
or `rivet-proxyd` needs one, because both are handed material from the card —
fleetd its register and credentials, the proxy its certificate. Carrying used to
be the signal for that; with nothing carried for a member any more, the role is.

---

## 6. `rivet-fleetd` and `rivet-launcher` stay where they are, for now

**The fleet server and the launcher are operating-system features too**, and
they run in user space for good reason, as the proxy does. They do not move
with it. Their move waits for §7: once a component the image ships can also be
updated on its own, it is plain that the proxy, the fleet server and the
launcher all belong in the operating system, and moving them then costs none of
the ability to fix them quickly.

Until then `rivet-fleetd` is a carried application under `ADR-050` §4, and
`rivet-launcher` stays the image service it is, running as root with no
ruleset; nothing here changes how either is built, published, carried or run.
The proxy moves first because the carry machinery around it has had the
repeated cost.

---

## 7. Open, and tabled

**Independently updatable system modules** — a component the image ships, plus
a separately installed newer copy of the same component, with a precedence rule
so that neither one silently overrides the other. It would give back the fast
fix this ADR gives up, and it needs that rule written first: which copy runs,
what an update of the image does to an installed copy, and what a rollback
does.

It is recorded here as an open question. **It does not block this change**, and
nothing is built for it.

---

## 8. Consequences

* The proxy is present on every card from the first boot, and a member's card
  carries no applications at all.
* `[carry.rivet-proxyd]` pins and a published proxy product have nothing left
  to do. A pin in an old document is inert (`rivet_config::carry`'s own rule for
  a product the role does not carry).
* A proxy fix waits for a system update. That is the cost accepted in §1.
* The image grows by the proxy's binary, which is small against the slot.
