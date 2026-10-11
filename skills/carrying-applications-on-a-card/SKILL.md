---
name: carrying-applications-on-a-card
description: How an application already published to the fleet repository ends up on a newly written card and gets installed on first boot, for any role — server, member, stand-alone or kiosk. Use when asked what a card carries, why an application is or is not on a board after provisioning, how a fleet server gets `rivet-fleetd`, why `rivet-proxyd` is never carried, or how a kiosk's auto-run application reaches the screen.
---

# Carrying applications on a card

**`rivet-mcpd` and `rivet-proxyd` are in the system image; `rivet-fleetd` and
every kiosk application are not** (`ADR-050` §1, `ADR-060`). A **carried payload** is a signed
`.rvt` placed on both boot partitions and installed by the machine itself on
its first convergence — the same artifact `cargo rivet deploy` would send to
an already-running board, by the same installer (`ADR-050` §4.1). This skill
is the whole path: how a product ends up in the fleet repository, how a
carried payload reaches the card, and what happens on first boot.

Not for: publishing an application in the first place
(`skills/deploying-an-application`), the rest of what `cargo rivet provision`
does (`skills/provisioning-a-fleet-server`), or reading a board after boot
(`skills/reading-a-board`) — each is linked below where it applies.

## What decides what a card carries

**The role, not an operator's guess.** `ADR-050` §5 splits "role" into two
independent axes the document already keeps apart — `Fleet`
(`stand-alone`/`server`/`member`) and display (`headless`/`kiosk`) — and §5.1
is the table:

| | image enables | card carries |
| :-- | :-- | :-- |
| server + headless | `rivet-mdnsd`, `rivet-proxyd` | `rivet-fleetd` |
| server + kiosk | `rivet-mdnsd`, `rivet-proxyd`, `rivet-launcher` | `rivet-fleetd`, the kiosk application |
| member + headless | `rivet-mdnsd`, `rivet-proxyd` | nothing |
| stand-alone | `rivet-proxyd`, if anything is to be served | — |

**The proxy is never carried** (`ADR-060`). It is part of the image and runs
under its own uid when the document enables it; `cargo rivet fleet publish`
refuses a `rivet-proxyd` payload, `cmd/rivet-image --application` refuses one,
and `cargo rivet fleet sync` skips one it finds and says so. A proxy fix ships
as a system update (`skills/publishing-a-system-image-and-update`). Its
certificate still rides in the staging bundle, which a member therefore still
needs.

`rivet_config::Document::carried_services` reads this straight off a composed
document — an enabled, non-image service — rather than re-deriving a role the
document has no field for, so a hand-authored document that enables the right
services is carried exactly as an interview-produced one would be.

**Which VERSION, once the repository holds more than one.** `[carry.<product>]
version = "…"` (`rivet-config::carry`) pins it — a table of its own, read once
at build/provision time, never `[services]`'s business (`ADR-050` §5.2, R-159
R20). Unpinned and exactly one version published: that one, silently.
Unpinned and several: **refused**, naming every candidate and suggesting the
newest as the key to set — never chosen silently (`AGENTS.md` §5). Pinned and
absent: refused by name. The pin itself comes from one of two places:

* **The interactive interview** asks, per carried product, newest
  preselected — a blank answer takes it (`.rulings/` R-176 Q6,
  `cmd/rivet-provision/src/main.rs::choose_carried_version`).
* **The register**, for a `cargo rivet fleet`-composed document: a device's
  own pin beats a fleet-wide one, per product, with no role layer between them
  because no role knows a version (`ADR-050` §5.2, R-160 R26) —
  `rivet-fleet set carry`/`set fleet-carry`.

## Getting an application into the repository first

A product has to be **published** before anything here can select it —
`skills/deploying-an-application` is build, package and
`cargo rivet fleet publish`. `cargo rivet fleet list [--at DIR]` confirms what
is there; carrying only ever reads an `application` entry (never `library` —
see Refusals below).

## How it reaches the card

Two routes select the same published bytes by the same rule; neither builds
or signs anything (`ADR-054` §2).

### Provisioning a fresh card

`cargo rivet provision` (`skills/provisioning-a-fleet-server`) resolves each
`[carry.<product>]` pin against the fleet repository, verifies the payload,
and widens an already-built, already-selected image's boot partitions —
`write_image.rs::inject`, `cmd/cargo-rivet/src/carry.rs::gather`
(`ADR-054` §3–§4):

| File | Source |
| :-- | :-- |
| `app-<product>.rvt`, one per carried service | the fleet repository's `[carry.<product>]` pin, resolved to an object |
| `staged.txt` | the `ADR-049` staging bundle, widened for a server (below) |
| `device.seed`, `device.crt`, `peers` | this card's own identity directory |

**The signature check here is against `<rivet root>/keys/rivet-app.pub` —
this workstation's own trust anchor — not the image's own baked-in trust
store**, because widening an existing image only reaches the FAT boot
partitions; `/system` is ext4 and unreachable from a workstation
(`ADR-050` §4.6, `ADR-054` §4). The run states once, as a note rather than a
second refusal, that it could not confirm the image's own store agrees.

**For a `server` document only, the card additionally carries the fleet
repository's TEXT, and none of its objects** (`.rulings/` R-182) —
`manifest.toml` (re-rendered, never copied verbatim, naming only
`application`/`library` entries this workstation holds an object for),
`documents/` and `register/`. Staged to `rivet-fleetd` under a
`fleet-repository/` prefix that cannot collide with the register's own staged
entry. Refused by name, with the measured byte count, if even that will not fit
the staging bundle's 5 MiB cap (`rivet_core::staging::MAX_BUNDLE_BYTES`,
`ADR-054` §5). **The objects are what overran that cap**: a server's
published applications alone came to several megabytes, so a real server's
card could not be written while they rode in the bundle.

**The carried payloads above are unaffected.** `app-rivet-fleetd.rvt` is still
on the card as a boot-partition file, and still installed on first boot. What no longer rides along is the fleet
repository's own copy of every published object — which a server needs to
hand the fleet's software on, and which follows after first boot into
`rivet-fleetd`'s own store (R-184 D1): `cargo rivet fleet sync <host> confirm`,
after `fleet push` (`skills/provisioning-a-fleet-server`, "Then its objects").

**The server's document declares `[repos.fleet]`**, and the server serves that
git repository over the control plane from first boot. `cargo rivet fleet push
<host> confirm` puts the workstation's fleet repository text into it
(`skills/provisioning-a-fleet-server`, "After the write").

Proven byte-for-byte on 2026-09-14 (`c3-byte-proof/` in the maintainer's scratchpad):
a card widened this way and a fresh `tools/container/build-card.sh` build
given the same resolved inputs produced **zero differences** across all 15
files of both `boot-a` and `boot-b`, including both `app-rivet-fleetd.rvt`
and `app-rivet-proxyd.rvt` — and again after R-182, with a text-only
`staged.txt` (`c3-r182/`, same place).

### Building a card directly, with applications already on it

`tools/container/build-card.sh` → `tools/card/build.sh` also carries
payloads, two ways, given `RIVET_DOCUMENT` and `RIVET_IDENTITY`:

* `RIVET_FLEET_REPOSITORY=<dir>` — resolves the document's role requirement
  and its `[carry.<product>]` pin by digest, the identical resolution
  `cmd/rivet-image/src/fleet_payloads.rs::resolve` performs for provisioning.
* `RIVET_APPLICATIONS="<path> <path> …"` — explicit, already-built `.rvt`
  paths, additively with `RIVET_FLEET_REPOSITORY` if both are given.

This is `cmd/rivet-image`'s own build-time path (`ADR-050` §4.3–§4.4): each
payload is verified against **the trust store this build is about to bake
into the image** — `keys/rivet-app.pub`, refused (not merely warned) if
`RIVET_APPLICATIONS` is non-empty and that key is missing. This is the route
the c3 byte proof's "Card B" used to establish the fixture the widened route
was then checked against — the two routes agree because they resolve, verify
and name payloads the same way, not because one calls the other.

## First boot

PID 1 (`cmd/rivet-initd`) copies each `app-<product>.rvt` off the card into
`/state/carried/<product>/payload.rvt` **only when that product's record
directory is absent** — the whole of the idempotence: a second boot of the
same card copies nothing, and nothing on the card is ever modified. The
supervisor (`cmd/rivet-sysd::carried`) then asks `rivet-deploy` — the same
supervised installer `cargo rivet deploy` uses — to install it on the first
convergence loop, and `ADR-049`'s staged credentials are delivered into the
application's data directory by the discovery pass that install triggers.

* **Success:** the payload is removed; the record directory stays, so the
  next boot never re-copies it. `rivet get status` (`skills/reading-a-board`)
  should show the application `started`.
* **Failure:** the machine keeps converging — it does **not** refuse to come
  up, because `rivet-mcpd` is tier 1 and stays reachable regardless
  (`ADR-050` §4.2, §10). The payload is **left in place** under
  `/state/carried/<product>` for `rivet apply install <path> confirm` to
  retry by hand, or for the next boot to retry once; one bad payload never
  stops a second one beside it. `rivet get install` carries the installer's
  own account of what went wrong; `rivet get status` shows the machine
  `DEGRADED` rather than the application `started`.

Confirm with `skills/reading-a-board`'s `-- get status` (state, `restarts`)
and `-- get install` (read the `correlation` this install returned, not a
bare `finished`).

## A kiosk's auto-run application

**`rivet-launcher` and the auto-run application are two different tiers.**
`rivet-launcher` ships **in the image**, `enabled = false`, and the document
turns it on (`ADR-050` §2, tier 2 — no new mechanism, it runs as uid 0 with no
per-application confinement). The application named by `auto-run` is tier 3
and reaches the card exactly like `rivet-fleetd` — carried,
installed on first convergence, **and separately enabled** in
`[services.<name>]`, because the supervisor starts nothing the document has
not enabled (`SPEC-05` §5.1). An `auto-run` naming an application that is
absent, or installed but not enabled, is the same black screen by two
different routes.

**The gap `SPEC-05` §5.1 names rather than hides:** nothing at card-composition
time can confirm the named application will actually install — the tool
asks, prints the risk, and does not compose `rivet-launcher`'s `auto-run`
unasked on the assumption that it is usually right.

## Refusals and traps

| Situation | What happens |
| :-- | :-- |
| A required product has no published `application` at all | Refused before the interview even asks the question — `NOTHING WAS WRITTEN. <product> must be carried (ADR-050 §5.1) …` — or at the direct build, the identical resolution in `fleet_payloads.rs` |
| A required product is unpinned and the repository holds several versions | Refused, naming every candidate; the newest is suggested as `[carry.<product>] version = "…"` — never chosen silently |
| A pinned version is not published | Refused by name, naming what the repository actually holds for that product, if anything |
| A carried payload fails signature verification | Refused before anything is written — the workstation's `keys/rivet-app.pub` (provisioning) or the image's own baked-in store (a direct build) |
| A `library` of the same name is published | Not a candidate at all — reads as "not found", `ADR-050` §4.5; only `kind = "application"` is ever carried |
| A file will not fit a boot partition, or a server's repository text exceeds the 5 MiB staging cap | Refused by name with the measured byte count; nothing partial is ever written |
| An install fails on first boot | The machine converges anyway and reports `DEGRADED`; the payload is kept for a retry, never for a second attempt on the same boot |
| A `rivet-proxyd` payload is published, carried or synced | Refused by `fleet publish` and `rivet-image --application`, skipped and named by `fleet sync` — the image provides it (`ADR-060`) |
| A server or member card from an image older than `rivet_core::service::PROXY_IN_IMAGE_SINCE` | `cargo rivet provision` refuses it: that image has no proxy (`ADR-060` §5). An image named by a file path is not checked, and the run says so |

## What is unproven

**No board has booted a card carrying `rivet-fleetd` or any kiosk application
through this path.** The c3 byte proof establishes that the
two build routes place identical bytes — it says nothing about what a board
does with them. `ADR-050` §11.2 and `ADR-054` §8 are the standing accounts,
and the reason is structural, not a gap in effort: first boot's read of a
carried payload rides the same boot-partition mount `rivet.toml` is read
through, which needs `/proc/device-tree/chosen/bootloader/partition` — a
property `-M virt` does not emulate at all (`ADR-026` §4). Only a real card
and a test board can close it, with

```sh
cargo rivet run --host <addr>:7777 --identity <dir> --pin <hex> -- get status
```

showing the carried application `started`. Also unproven: that `fleet sync`
fills a real board's repository store (it is proven on a QEMU guest only), and
that `rivet-fleetd` reads the repository text the card carried. Also unproven: `rivet-launcher`
running as an uid-0 image service rather than a confined application —
nothing has watched it draw (`ADR-050` §11.2's own words). Whether a
`library` can ever be carried, and whether a failed carry should keep
retrying forever, are both named as open in `ADR-050` §12, not decided.

See `docs/COMMANDS.md` for `cargo rivet provision --help` and
`tools/card/build.sh`'s own header comment for the exact current flags.
