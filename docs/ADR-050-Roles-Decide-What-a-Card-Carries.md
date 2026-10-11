# ADR-050: Roles Decide What a Card Carries — the payload tiers and the interview they shape

* **Status:** Accepted. **§2.1, §4, §5.1, §5.2 and §9 are built, with tests,
  and no machine has booted one** — §11 says what is proved, by what method,
  and what is not. **§6, the two-phase interview, remains a design on paper**:
  `cmd/rivet-provision`'s interview is still the single flat list it always
  was.
* **Code:** `rivet-core/src/carried.rs` (the names three programs agree on),
  `rivet-core/src/state.rs` (`CARRIED`), `cmd/rivet-image/src/application.rs`
  (the build-time verification) and `src/main.rs` (`--application`,
  `--launcher`), `cmd/rivet-initd/src/carried.rs` (the courier),
  `cmd/rivet-sysd/src/carried.rs` (the installs, and §10's ruling),
  `cmd/rivet-sysd/src/definitions.rs` (`launcher`), `tools/card/build.sh` and
  `tools/container/build-card.sh` (`RIVET_APPLICATIONS`),
  `rivet_config::Document::carried_services` (§5.1's table read off the
  document), `cmd/rivet-image/src/fleet_payloads.rs` (resolving a role's
  products, and §5.2's pin), `rivet-config/src/carry.rs`
  (`[carry.<product>] version = "…"`), `cmd/rivet-provision/src/main.rs`'s
  `services_for` and `cmd/rivet-fleet/src/compose.rs`'s `role_services` (§5.1's
  table, realised for a hand-provisioned machine and a register-composed one),
  `cmd/rivet-provision/src/stage.rs::compose` (§5.1's front door, staged for a
  member exactly as for a server — `.rulings/` R-160 R25),
  `rivetos_register::{Fleet, Device}::carry` and
  `cmd/rivet-fleet/src/compose.rs::carry` (§5.2's pin, composed from the
  register — R-160 R26)
* **Superseded in part:** §4 for `rivet-proxyd`, which is part of the system
  image since `ADR-060` (`.rulings/` R-209). `rivet-fleetd` and every tier 3
  payload are still carried as §4 decides.
* **Date:** 2026-09-07
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Ruled:** by the maintainer, 2026-09-07, in conversation (`.rulings/` R-304).
  The tiers are his: `fleetd` is a must-have, `proxy` is the same kind of thing,
  and the launcher is the one he would most like provisioning to leave off for a
  headless machine and add, with Aura, for a kiosk. So is the consequence: the
  role is selected in provisioning, it drives which software is installed, and it
  also drives whether the operator is prompted about enabling `mdnsd` or about
  enabling Aura (or auto-starting it).
* **Answers:** why `SPEC-05` §1.1 could not be satisfied as written, and what
  replaces it.
* **Builds on:** `ADR-049` (card-time material and the ownership hop),
  `ADR-015` (applications, and `rivet-deploy` as the installer), `ADR-029`
  (the payload format), `ADR-004` (first-boot provisioning), `ADR-017`
  (services and `enabled`)

---

## 1. Context — a sentence that could not be true

`SPEC-05` §1.1 is the maintainer's, and it is the acceptance test the whole
provisioning effort was written against: when the device boots up, `fleetd`
comes up running just like `mcpd` does. *(`.rulings/` R-303.)*

**It could not be satisfied by any card this project could build**, and the
reason was a distinction nobody had written down: **`rivet-mcpd` is in the
system image and `rivet-fleetd` is an application.** `cmd/rivet-image` had no
`--application` flag. A card carried a kernel, a system image, a provisioning
document, a TLS identity and — since `ADR-049` — a staging bundle. It carried no
programs that live under `/state/app`.

So a first boot had no `rivet-fleetd` to start, no `rivet-proxyd`, and on a
kiosk machine no `rivet-launcher` and nothing for it to launch. `ADR-049`
removed every piece of hand-work *around* those applications — the credentials,
the register, the ownership — while leaving the applications themselves absent.

**This ADR is the other half**, and §4 is built: a card carries signed payloads
and the machine installs them on its first convergence. What that makes true is
bounded by §11.2 — the bytes are verified, carried and handed to the installer,
and **no Raspberry Pi has booted one.**

---

## 2. Three tiers, and the middle one is why `launcher` felt different

The maintainer's instinct separated `fleetd`/`proxyd` from `launcher` without
naming the axis. The axis is **what decides whether it runs**.

| tier | what it is | where it lives | what decides it runs |
| :--- | :--- | :--- | :--- |
| **1 — system** | the machine cannot be administered without it | the image | nothing; it always runs |
| **2 — platform, optional** | Rivet's own components, but a policy choice | the image | the **document**, via `enabled` |
| **3 — applications** | programs built *on* Rivet | `/state/app` | installation, then `enabled` |

**Tier 1:** `rivet-initd`, `rivet-sysd`, `rivet-mcpd`, `rivet-netd`,
`rivet-timed`, `rivet-deploy`.

**Tier 2:** `rivet-mdnsd`, `rivet-bluetoothd`, **`rivet-launcher`**, and
**`rivet-proxyd`** (`ADR-060`).

**Tier 3:** `aura-controller`, `aura-renderer`, and every adopter —
`tempmon-rivet`, `reliquary-rivet`, `trends-rivet`.

### 2.1 Tier 2 already exists and needs nothing built

**`rivet-bluetoothd` ships in the image with `enabled = false`, and the document
turns it on.** That is precisely a headless machine leaving the launcher off
while a kiosk machine adds it, and it is already how this system works. `rivet-launcher` belongs on
that shelf: it is `cmd/rivet-launcher` in this repository, it is versioned with
the operating system, and whether a machine offers a menu is a property of the
machine rather than of a program somebody installed.

**No new mechanism is required for the launcher.** What is required is that the
image carry it — it does not today — and that the role compose its `enabled`.

**Built 2026-09-07, and it supersedes a ruling six days old.**
`cmd/rivet-launcher`'s own header records the maintainer's 2026-09-01 ruling
that the launcher makes more sense as an application (`.rulings/` R-305), and
`deploy/launcher/Cargo.toml` is the manifest that shipped it as one. That
placement is superseded here; the
manifest and its deploy recipe are **kept**, because deploying the launcher as
an application is how it is iterated on without building a card.

**What the move costs is a hand-written definition**, and it is stated because
it is the kind of thing that drifts. An image service has no manifest, so
`cmd/rivet-sysd`'s `launcher` restates by hand what that file declares: the
kiosk endpoint (`.rulings/` R-138 — the line that decides whether the program is
the display **arbiter** or a lessee that draws nothing, and whose absence is
silent), the input socket it provides, and both device classes. A test reads
`deploy/launcher/Cargo.toml` and fails if the two disagree.

**And the launcher runs as uid 0**, where the application form ran as its own
uid with a Landlock ruleset. The device grant becomes a no-op, because a
devtmpfs node is `0600 root:root` and root already opens it. **Tier 2 does not
itself decide privilege**: `rivet-proxyd` is tier 2 and runs under its own uid
and a ruleset composed without a manifest (`ADR-060`, `ADR-042` §4), because
nothing it does needs root.

---

## 3. Aura does not go in the image, and the boundary is the point

`aura-renderer` and `aura-controller` live in `examples/`. They are there
because they are things built **on** Rivet, and `examples/` is the demonstration
that a foreign program can be a first-class citizen of this platform.

**Ship Aura in the operating system and the next question is why not TempMon.**
There is no principled answer, and the boundary that makes `examples/` mean
anything is gone. A platform that ships one appliance has chosen that appliance.

So Aura is **tier 3 and stays tier 3**, and reaches a kiosk card as a *carried
payload*. That is the mechanism §4 decides.

---

## 4. The decision: payloads are CARRIED, not baked

`fleetd` must be present on a fleet server's first boot, and a kiosk's
application on a kiosk's. **The card carries signed `.rvt` payloads, installed
at first boot.** This gives first-boot completeness *and* keeps `cargo rivet
deploy` as the way to fix any of it afterwards, because a carried payload and a
deployed one are the same artifact by the same route.

**The alternative is to move a program into the system image.** It satisfies
§1.1 literally and it is less machinery, and its cost is the speed of repair: a
fix to a carried program is a `cargo rivet deploy` of about a minute, while a
fix to an image program is a system update — a new image, the inactive slot, a
trial boot and a restart (`ADR-018`, `ADR-012` §7.1). That cost is weighed per
program, against what carrying costs in pins, bundles and refused card writes.

**`rivet-proxyd` is the one program the weighing went the other way for**
(`ADR-060`): it changes rarely, so the fast fix went unused while carrying it
kept costing repairs. It is part of the image, and no card carries it.
`rivet-fleetd` stays carried; the same argument has been raised for it and not
ruled.

### 4.1 Both halves already exist

* **`rivet-deploy` is already a supervised installer**, deliberately separated
  from the login session — *"an update writes 640 MiB, and doing that from the
  login shell means a session owns a long write. A connection that drops
  mid-write owns a half-written slot."* It already takes a request and performs
  `ADR-029` §9.2's sequence.
* **The card already stages material**, as of `ADR-049` — the boot partition,
  PID 1 as courier, and destruction of the secret half.

**So this is joining two built things, not inventing a third.** A card carries
`.rvt` files; PID 1 lands them in `/state`; `rivet-deploy` installs them on the
first convergence, at which point `ADR-049`'s staged credentials and register
are delivered into the data directories the installation just created.

### 4.2 The risk this accepts, stated plainly

**A carried payload can fail to install, and then a machine boots without its
fleet server.** A baked-in binary cannot.

**This is survivable because `rivet-mcpd` is tier 1.** The control plane is in
the image, it comes up regardless, and an operator can always reach the machine
to see what happened and install it by hand. A failure here costs a diagnosis,
not a card.

**That is the argument that decides it**, and it is the one to attack if this
ADR is ever revisited.

### 4.3 A card build needs the application PUBLIC key, and no seed at all

**Corrected 2026-09-07 during implementation; the original claim is in the
History, and the risk it named is real and is now closed.**

A card build **signs nothing**, so it needs no seed. Signing happens in two
other places: `cargo rivet package` signs a payload with the application seed
and `tools/update/build.sh` signs an update with the system seed. What a card
build takes are the two **public** keys it writes into the image's trust store
(`ADR-028` §5.5.6), and `tools/card/build.sh` has taken both —
`keys/rivet-system.pub` and `keys/rivet-app.pub` — since 2026-08-25.

**The failure §4.3 was written to prevent is nonetheless real, and it was one
line away.** The application key was a **warning** when absent, on the reasoning
that a card which cannot verify an application is merely a card that runs no
applications. That was true of a card that carries none. A card asked to carry
payloads with no application key in its trust store is a card whose every
payload is refused at the entitlement check **on the board**, hours later, with
the account of it in a journal on a headless box.

So the absence is now a **refusal**, in both places that can see it:

* `tools/card/build.sh` refuses when `RIVET_APPLICATIONS` is non-empty and
  `keys/rivet-app.pub` is absent, or when `RIVET_NO_TRUST` is set. It names the
  key and the command that makes one.
* `cmd/rivet-image` refuses a `--application` against an empty trust store, and
  — the stronger check — **verifies every payload against the trust store the
  image is about to carry**, so a payload signed by a key this particular card
  will not accept is refused on the workstation by the same verifier the board
  would use. §4.4.

Size is not a consideration: `fleetd` 2.1 MB, `proxyd` comparable, Aura's two
halves 4.9 MB, against a 2 GiB image.

### 4.4 The verification uses the image's own trust store

`cmd/cargo-rivet`'s `inspect` verifies a `.rvt` against **one key handed over on
the command line**, which is the honest shape of a developer checking their own
artifact. That is not the question a card build has to answer.

The question is *will this card install this payload*, and only the thing
composing the card knows what it will trust. So `cmd/rivet-image` builds its
`TrustSet` from the same `--trusted-key` entries it is about to serialise onto
`/system`, and hands them to `rivet_core::package::read::open` — the verifier
`cmd/rivet-deploy` runs on the machine (`ADR-029` §11, `ADR-011`: one
authority). Every member's digest is checked before anything is read out of the
metadata, and `Application::open` then applies the kind's own structural rules.

**A payload signed with the SYSTEM key is refused here**, by the entitlement
rule rather than by a special case, which is the mistake somebody actually makes
— signing with the key they already had.

### 4.5 Only `application` payloads are carried, and `library` is refused by name

An `update` is `ADR-018` §5's A/B mechanism and does not arrive as something the
supervisor hands to `cmd/rivet-deploy` on a running machine.

A **library** is refused for a narrower reason and it is a real gap rather than
a principle. A library installs to a content-addressed directory whose name
carries a digest (`rivet_core::package::library::Reference`), so nothing on the
machine can work out from the product name alone whether the install landed —
and that is exactly the question `cmd/rivet-sysd` asks the filesystem to decide
whether to keep a payload (§10). Carrying a library needs that question answered
first; until then it is `cargo rivet deploy` after the first boot, and the
refusal says so.

### 4.6 A provisioning-time carry verifies against the WORKSTATION's key, not the image's own store

**Added 2026-09-14, `ADR-054`.** §4.4 is exact for `cmd/rivet-image`, which is
composing the card's `/system/trust` at the very moment it verifies a payload
against it. `cargo rivet provision`/`write-image` widening an EXISTING,
already-built image (`ADR-054` §3) is a different moment: `/system` is ext4,
and only the FAT boot partitions this tool opens are reachable from a
workstation at all — there is no store here to read.

So a carried payload is verified against `<rivet root>/keys/rivet-app.pub`
instead — this workstation's own established trust anchor, `cargo rivet
setup`'s own output, the same file `crate::provision_image::materialise`
already reads for the system key when it verifies a selected card image. A
failure still refuses before anything is written, on §4's own rule; what
changes is which key answers the question, and the run states once, as a
note rather than a second refusal, that it could not confirm the image's own
baked-in store agrees. An operator whose `RIVET_APP_KEY` diverged from
`keys/rivet-app.pub` when the image was built is the one case this cannot
catch — the same class of gap `write-image`'s own `trust_note` already lives
with for the document's signing key, not a new one.

---

## 5. The role is TWO axes, and collapsing them breaks the maintainer's own machine

**`rivet-01` is a fleet server AND a kiosk.** A single `role` enum cannot express
it, and the mistake is easy because "role" sounds singular.

| axis | values | decides |
| :--- | :--- | :--- |
| **fleet role** | `stand-alone`, `server`, `member` | `fleetd`, `proxyd`, the listen address, the register |
| **display role** | `headless`, `kiosk` | `launcher`, `[kiosk]`, `auto-run`, the application |

The document already keeps these apart — `Fleet` is a field and `[kiosk]` is a
table — and that separation is correct and is preserved.

### 5.1 What the combination decides

| | image enables | card carries |
| :--- | :--- | :--- |
| server + headless | `mdnsd`, `proxyd` | `fleetd` |
| server + kiosk | `mdnsd`, `proxyd`, **`launcher`** | `fleetd`, **the kiosk application** |
| member + headless | `mdnsd`, `proxyd` | nothing |
| stand-alone | `proxyd`, if anything is to be served | — |

**A server and a member both enable the front door.** `cmd/rivet-provision`'s
`services_for` and `cmd/rivet-fleet/src/compose.rs`'s `role_services` — its own
second copy, kept apart on purpose (that module's own header) — each enable
`rivet-proxyd` for both roles, with a test on each side. Since `ADR-060` the
proxy is part of the image, so `Document::carried_services` does not list it
and a member's card carries no application at all.

**A server's card and a member's card both carry a staging bundle**, whatever
the right-hand column says. The role decides it, not the payloads: the front
door both roles enable is handed its certificate from the bundle, so a member —
no payload — still carries one, and a stand-alone card carries none. A document
that reaches the card writer without its bundle has one composed by
`rivet-provision stage` rather than being refused (`ADR-049` §12).

### 5.2 Several published versions of one product: the document pins one, or a build refuses

Added 2026-09-13 (`.rulings/` R-159 R20). §5.1 says WHICH products a role's
card carries; it says nothing about WHICH VERSION once a fleet repository
holds more than one for the same product — the normal case the moment
`cargo rivet fleet publish` has run twice for one thing, since retention keeps
three (`ADR-052` §7.1). Before this, `cmd/rivet-image
--fleet-repository`'s resolution (`ADR-050` §5.1, §9) succeeded only when
exactly one version existed, so a repository with any history refused every
card build.

**The machine document pins it**, in a table of its own rather than a fourth
key on `[services.<name>]`: `[carry.<product>] version = "…"`
(`rivet_config::carry`). `[services]` is the supervisor's, read on an
already-running machine (`ADR-017` §4.1); a pin is read once, by
`cmd/rivet-image --fleet-repository`, **before the machine it will run on
exists** — mixing the two would make one table answer to two authorities at
two different moments, the split `ADR-017` §4.1 already drew, read a second
way. `rivet-config`'s schema publishes the key as `not-asserted`: neither
`immediately` nor `on-next-restart` is true of a key nothing running ever
re-reads.

`cmd/rivet-image/src/fleet_payloads.rs::resolve` takes the pin
(`rivet_config::Document::carried_version`) and decides:

* **Pinned, and the repository holds that version:** carries it, whatever
  else is published.
* **Pinned, and it is not in the repository:** refused by name — the pin, and
  what the repository actually holds for the product, if anything.
* **Unpinned, and the repository holds exactly one version:** carries it —
  every card built before this ruling, and every product with one published
  version today, is unaffected.
* **Unpinned, and the repository holds several:** refused, naming every
  candidate. The newest is suggested, as a sentence naming the key to set —
  never chosen silently (`AGENTS.md` §5).

**The register also composes a pin, fleet-wide and overridable per device**
(`.rulings/` R-160 R26). `services` layers fleet default → role default →
device (`ADR-007` §3.4, `cmd/rivet-fleet/src/compose.rs::services`); a version
pin has **no role layer**, because no role knows a version, so
`rivetos_register::{Fleet, Device}::carry` and
`cmd/rivet-fleet/src/compose.rs::carry` merge just the two layers
`service_memory` already has — a device's own pin wins, per product rather
than per table, and a fleet-wide pin fills the gap otherwise. The merge is
rendered into the composed document's `[carry.<product>]` table, which stays
what `cmd/rivet-image --fleet-repository` reads (`Document::carried_version`
is the one reader, whichever tool produced the document). A hand-authored or
`cmd/rivet-provision`-produced document, and one composed through
`cmd/rivet-fleet` from the register, therefore both carry a pin the same way.
`rivet-fleet set carry` (one device) and `rivet-fleet set fleet-carry` (every
device, unless its own record says otherwise) are the two verbs, on
`set service`/`set fleet-service`'s footing exactly.

---

## 6. The role is a GATE, not a field

Today `Fleet` is field 6 of a flat form, sitting **after** questions it
determines. If the role decides which questions exist, it has to be answered
before them.

**So the interview is two phases.** The role questions are asked first and are
not part of the numbered field list. The field list is then **composed for that
combination**.

**A stand-alone machine never sees an `auto-run` field at all** — not greyed
out, not marked N/A, absent. A field that cannot apply is a question the
operator has to work out the irrelevance of.

### 6.1 Why not conditional fields in one list

Because the numbering is the interface. `1` is Hostname and `6` is Fleet, and an
agent extending the menu on 2026-09-07 appended new fields specifically to keep
`1`–`8` stable. Conditional fields fight that: a kiosk answer that inserts two
questions renumbers everything below it, and the operator's muscle memory is
part of the tool.

**Two phases keeps numbering stable *within* a role**, which is the property that
actually matters.

---

## 7. Ask, or decide — the razor

Both halves were ruled by the maintainer on 2026-09-07, on the same evening, and
neither was named at the time.

**DECIDE when the role knows the answer.**
A fleet server listens on `0.0.0.0:7777`. Asking is how a headless board came up
answering only loopback and had to be rebuilt — `SPEC-05` §5. The role knows;
the operator should not have to.

**ASK when only the operator knows.**
The timezone, because a wrong or absent one is **silent** UTC — nothing fails,
every timestamp is simply wrong (`SPEC-05` §5.0). Which application auto-runs,
because the tool cannot know what will be installed and naming an absent one
takes the display and produces a black screen with no message on it.

**This razor is what keeps a role-driven interview from becoming either a wizard
that nags or a wizard that assumes**, and every gap found on 2026-09-07 sits on
one side of it or the other.

---

## 8. Why this was not seen sooner

`SPEC-05`'s §2 question 4 — why this was not thought of sooner, put by the
maintainer (`.rulings/` R-302) — has an answer here rather than an excuse.

**The menu is a flat form because it was written when the role meant almost
nothing.** It selected a word that appeared in a document. The moment the role
decides which *software is installed*, a flat form cannot express it — and every
defect found during the `rivet-01` bring-up is the same shape: **something the
role knew, and the form had no way to carry.**

The services. The listen address. The timezone. `upstream-name`. An `auto-run`
naming an application that was never installed. Each was an operator asked to
supply, or to remember, something the role already determined.

---

## 9. Consequences

**Built** — `cmd/rivet-image` carries signed application payloads,
verified at build time against the trust store the image will hold rather than
on a headless board; `rivet-launcher` ships in the system image with
`enabled = false`; `rivet-deploy` installs carried payloads on the first
convergence, and `ADR-049`'s staged material is delivered by the discovery pass
that install triggers; a card build asked to carry payloads with no application
key **refuses**.

**Built, 2026-09-13** — the CARRIED PAYLOADS derive from the role, closing the
half of "the composed document, the staged material and the carried payloads
all derive from the two role answers" that did not need the interview to
change. `rivet_config::Document::carried_services` reads §5.1's table back off
whichever document exists — a fleet server's carries `rivet-fleetd`, a kiosk's
its auto-run application — without needing a
`role` field the document does not have, and `cmd/rivet-image
--fleet-repository DIR` resolves each by digest out of the fleet repository and
carries it, refusing rather than guessing when a product has been published
zero or more than once (`AGENTS.md` §5). `PLAN-Feature-Set-2026-09-12` §4.

**Built, 2026-09-13, R-159 R21** — a fleet MEMBER's document also enables
`rivet-proxyd`, closing the gap between §5.1's table (which always said so)
and `cmd/rivet-provision`'s `services_for` and `cmd/rivet-fleet`'s
`role_services` (which did not).

**Built, 2026-09-13, R-159 R20** — a machine document may PIN which version of
a carried product `cmd/rivet-image --fleet-repository` resolves, closing the
half of the refusal above that a normal, once-published-twice repository hit
every time: unpinned-and-ambiguous still refuses, now suggesting the newest as
the key to set; pinned-and-present carries exactly that version; pinned and
not published refuses by name. §5.2 is the design and `rivet-config::carry` is
the code.

**Not built** — `cmd/rivet-provision`'s interview becomes two-phase (§6). The
document is still composed the way it always was; what changed is what a build
does with whichever document it is handed, so a hand-authored document that
enables the right services is carried exactly as a two-phase interview's would
be.

**`SPEC-05` §1.1 is satisfied for a kiosk fleet server as far as a workstation
can show it** — the card carries `fleetd` and the kiosk application, the image
carries `proxyd` (`ADR-060`), and the machine is designed to come up running
them with no operator step
between the write and the screen. §11.2 is what that claim does **not** yet
include.

---

## 10. A carried payload that fails to install does NOT stop the machine

**Ruled 2026-09-07, during implementation.** This was §10's second open
question; the wording it replaces is in the History.

**The machine comes up without the application and says so loudly.** It does not
refuse to converge, and the argument is §4.2's own read forward: the risk this
ADR accepts is survivable *because* `rivet-mcpd` is tier 1 and the operator can
always reach the machine to see what happened. **A machine that stopped
converging over a bad payload would be a machine with no control plane** — on
hardware with no shell, which turns a diagnosis into a card. Refusing to
converge would therefore destroy the only property that makes §4.2 acceptable.

It is also what the rest of this system already does. `ADR-017` §3.2 has no
dependency graph: a service that needs something that has not arrived **retries**,
and every failing service on a Rivet is reported and carried on from.

What that means concretely, in `cmd/rivet-sysd`'s `carried` module:

* **The payload is left where it is**, under `/state/carried/<product>`, so
  `rivet apply install <digest> confirm` retries it and so does the next boot.
* The failure is an `Error` record naming the application, the path, and the
  fact that `rivet get install` holds the installer's own account of why.
* **It is not retried again on this boot.** `ADR-017` §3.5's restart loop is an
  SD card destroyed 640 MiB at a time, and a payload that failed once will fail
  the same way four seconds later.
* **One bad payload does not stop the good one beside it.** A fleet server's
  card carries two, and a machine that came up with neither because the first
  was broken would be the dependency graph §3.2 does not have.

### 10.1 Success is read off the filesystem, not off an exit code

The obvious implementation is to watch the installer's exit status, and it does
not work: a `RestartPolicy::Never` instance is `State::Done` **whichever way it
exited** — `Supervisor::reap` sets `Done` for any exit a policy does not want
restarted — so `State::Failed` is unreachable for the installer and a watch
reading the state would delete the payload of every failed install. That was the
first shape of this module and its own tests caught it.

The question that actually wants answering is *is it installed*, and the machine
has the answer: the manifest under `current`, which is the same file
`cmd/rivet-sysd`'s `applications::discover` reads to decide the same thing
(`AGENTS.md` §5 — ask the kernel, never a cached verdict). A `/state` that will
not answer reads as **not installed**, which keeps the payload; the other
direction deletes the only copy of a program on the strength of a card that is
failing.

---

## 11. What is built, what is proved, and what is unproven

### 11.1 Built and tested

* `rivet_core::carried` — the names `cmd/rivet-image`, `cmd/rivet-initd` and
  `cmd/rivet-sysd` agree on, and the round trip between them. 7 tests, including
  every name on a real boot partition that must **not** read as a payload and
  every name that would escape `/state/carried`.
* `cmd/rivet-image` — `--application PATH`, repeatable, verified against the
  image's own trust store before a partition is written. 7 tests over **real
  signed payloads** built with the same writer `cargo rivet package` uses and
  sealed with real Ed25519: the entitlement refusal, an unknown key, an empty
  trust store, a tampered member, a file that is not a package, and that the
  card's name comes from the signed header rather than the file. 5 more assert
  the bytes reach the boot partition unchanged and that two payloads under one
  name are refused.
* `cmd/rivet-initd` — the courier. 9 tests, including that a payload whose
  record directory already exists is **not** carried again (the whole of the
  idempotence), that nothing on the card is removed, overwritten or mounted
  writable, and that PID 1 verifies nothing and installs nothing.
* `cmd/rivet-sysd` — the installs and §10's ruling. 11 tests, including that a
  payload that will not install leaves the machine converging and says so, that
  it is not retried on that boot, that one bad payload does not stop the next,
  that an operator's install goes first, and that the payload is removed on
  success while the record directory is kept.
* `cmd/rivet-sysd::definitions` — `rivet-launcher`, disabled, with a test that
  reads `deploy/launcher/Cargo.toml` and refuses to let the definition drift
  from the manifest it replaces.
* **Built 2026-09-13 — selection.** `rivet_config::Document::carried_services`:
  a document with no service enabled carries nothing; a fleet server's document
  carries `rivet-fleetd` and not the image services `rivet-proxyd` and
  `rivet-mdnsd`; a disabled service is not carried; an image service is never
  carried whatever the document says. `cmd/rivet-image::fleet_payloads`, over a
  hand-built scratch fleet repository (`manifest.toml` beside `objects/`, the
  same shape `cargo rivet fleet init`/`publish` leave): a unique published
  `application` resolves to its object by digest; a product never published is
  refused by name; an uninitialised repository (no `manifest.toml` at all)
  refuses the same way as an empty one; two published versions of one product
  are refused rather than guessed between (`AGENTS.md` §5); a `library` of the
  same name is not a candidate (§4.5); a manifest naming bytes the object store
  does not have is refused as inconsistent. Each fix behind these was broken by
  hand and its test watched fail before being restored (`AGENTS.md` §4).
* **Built 2026-09-13 — R-159 R21, a member carries the front door.**
  `cmd/rivet-provision`'s `services_for`: a member's answers compose a document
  enabling `rivet-proxyd` and `rivet-mdnsd` but not `rivet-fleetd`, and
  `Document::carried_services` then lists nothing — both are image services
  (`ADR-060`). `cmd/rivet-fleet/src/compose.rs`'s `role_services`: the
  identical enabling against a register `Device` with `fleet_role = Member`,
  so the two composers agree.
* **Built 2026-09-13 — R-159 R20, pinning a version.** `rivet-config::carry`:
  the table's one key round-trips through `Document::carried_version`; absent
  is one state whether the `[carry]` table is missing or empty; a name this
  document does not pin reads back `None`. `cmd/rivet-image::fleet_payloads`:
  a pinned version present among several resolves to exactly that one; a
  pinned version absent — whether or not the product was published at all, and
  whether or not OTHER versions of it were — is refused by name; the existing
  unpinned-and-ambiguous refusal now also names the newest published version
  and the `[carry.<product>] version = "…"` key that would pin it, still never
  choosing it. Each fix behind these was broken by hand and its test watched
  fail before being restored (`AGENTS.md` §4).
* **Built 2026-09-13 — R-160 R25, a member's front door.**
  `cmd/rivet-provision/src/stage.rs::compose`: a member's plan stages exactly
  `tls-cert`, `tls-key` and `tls-root` for `rivet-proxyd`, under the member's
  own name and from its own leaf certificate; a server's plan is unchanged
  (every `rivet-fleetd` file, the register, and the same three proxy files);
  a stand-alone plan stages nothing; a member's key is never the server's,
  and the public root is the same for both — proved against a scratch CA
  minted in the test, never `~/.rivet/ca`. Each fix behind these was broken
  by hand and its test watched fail before being restored (`AGENTS.md` §4).
* **Built 2026-09-13 — R-160 R26, the pin composed from the register.**
  `rivetos_register::{Fleet, Device}::carry` and
  `cmd/rivet-fleet/src/compose.rs::carry`: a device's own pin beats the
  fleet's and the fleet's applies otherwise — `service_memory`'s two-layer
  merge, with no role layer beneath it (no role knows a version); the merge
  reaches the composed document as `[carry.<product>]`, read back through
  `Document::carried_version`; no pin anywhere composes no `[carry]` table at
  all; a pin for a product the device's role does not carry composes and is
  inert, `rivet_config::carry`'s own rule. An existing register written before
  `carry` existed — fleet-wide or per-device — still loads, with an empty map.
  `rivet-fleet set carry` (one device) and `set fleet-carry` (every device,
  unless overridden) are the two verbs, `set service`/`set fleet-service`'s
  shape exactly. Each fix behind these was broken by hand and its test
  watched fail before being restored (`AGENTS.md` §4).
* **Built — a bundle for a document that has none.**
  `cmd/rivet-provision/src/stage_document.rs`: a server's document is staged
  with the proxy's three files, `rivet-fleetd`'s five and a one-record register
  naming the server; a member's with the proxy's three alone, its `peers`
  gaining the key the Armada records for its server; a repeat run reuses the
  operator credential; a missing authority or identity is refused by path with
  nothing written. `cmd/cargo-rivet/src/provision.rs`'s `ensure_staging`: a
  member document with no bundle has one built and handed to the writer — that
  test was watched failing with the build removed — while a named `--staging`
  and the `--existing` route are refused rather than built.
  `tools/provision/rehearse-member.sh`, inside `tools/test-all.sh`, walks a
  fresh server and then a member through the interview itself and reads back
  both bundles, both documents and both prepared images: `rivet-fleetd` is the
  server card's only payload, the member's card has none, neither carries a
  `rivet-proxyd` payload, and both cards' `peers` name the server's
  `rivet-fleetd` key.

### 11.2 Unproven

**No machine has booted a card built this way**, and this ADR inherits
`ADR-049` §9.2's sentence unchanged. What is asserted above is that the payloads
are verified, carried, landed and handed to the installer correctly according to
fakes and to a card image read back off the disk. What is **not** asserted is
that a Raspberry Pi boots such a card and that `rivet-fleetd` reaches `started`.
**Selection (§9, built 2026-09-13) does not change this**: it decides which
bytes a build carries and is proved the same way, by fakes and by a scratch
fleet repository — not by anything booting.

**R-159 R21's gap is closed: R-160 R25 stages a member's front door.** A
provisioned member's document enables `rivet-proxyd` (§5.1) and
`cmd/rivet-provision/src/stage.rs::compose` now stages that application's
`tls-cert`, `tls-key` and `tls-root` for a member exactly as for a server —
its own leaf certificate and key from the fleet CA, for its own name, plus the
same public root (§11.1's R25 bullet). Proof that a member's front door
actually answers waits on the same thing every card in this ADR waits on —
this section's own opening paragraph: no machine has booted one yet.

**And this cannot be closed under QEMU, checked rather than assumed.** First
boot's read of a carried payload rides the identical boot-partition mount
`rivet.toml` is read through — `cmd/rivet-initd/src/provision.rs`'s
`mount_boot_filesystem` calls `rivet_adapter_image::boot_partition_device`,
which refuses rather than assuming partition 1 when
`/proc/device-tree/chosen/bootloader/partition` is absent. `ADR-026` §4 and
`cmd/rivet-initd/src/confirm.rs`'s own header say `-M virt` does not emulate
that property at all, and `tools/qemu/seed-card.sh` exists for the identical
reason: a card's own first-boot provisioning "cannot run under `-M virt`". So a
card built to carry `rivet-fleetd` and booted with `run.sh --card` would refuse
at the same mount `rivet.toml` needs, before carrying is ever reached — a run
that would prove only the refusal this ADR already knows about, not the
carrying. There is no QEMU form of this proof; only hardware answers it.

Closing it takes a card written from an image built with `RIVET_APPLICATIONS`, a
**test** board to put it in, and

    rivet run --host <addr> --identity <dir> --pin <hex> -- get status

showing `rivet-fleetd` `"state":"started"`, with `/api/fleet` returning the
record `ADR-049` injected. **The portal's HTML is not evidence** — it ships its
empty-register sentence as a JavaScript string at all times.

**And `rivet-launcher` as an image service is the least proven thing here.** It
has run as an *application*, with a per-name uid, a Landlock ruleset and a
device grant derived from its manifest; §2.1 makes it uid 0 with no confinement
and its declarations restated by hand in `definitions.rs`. The device grant is a
no-op for root and the arbitration is decided from `connects` either way, so the
reasoning says it works — nothing has watched it draw.

---

## 12. Open

* **Whether a stand-alone machine runs `proxyd` at all** depends on whether
  it serves anything, which the interview cannot know. Probably an ask. It is
  one `enabled` line either way, since the image carries the proxy.
* **Whether a LIBRARY can be carried**, which §4.5 refuses for a stated reason
  rather than a principle. It needs a way for the machine to decide whether a
  content-addressed install landed, from what the card knows.
* **Whether an adopter's payload can be carried too** — a card that arrives with
  TempMon already on it. Nothing here forbids it and nothing requires it.
* **Whether a failed carry should be retried on a later boot for ever.** It is
  today: the payload stays, and every boot tries once. A card whose payload can
  never install therefore spends one install attempt per boot for the life of
  the machine. That is cheap and visible, and it is not obviously right.

---

## History

### 2026-09-23 — §4 carried the proxy too, and argued against baking either in

`ADR-060` (`.rulings/` R-209) moved `rivet-proxyd` into the system image. §4
said:

> ~~`fleetd` and `proxyd` must be present on a first boot. There were two ways to
> get them there, and this ADR takes the second.~~
>
> ~~**Rejected — move them into the system image.** It satisfies §1.1 literally
> and it is less machinery. It was rejected for a cost measured the same day:
> `rivet-proxyd` was fixed and redeployed to `rivet-01` in about ninety seconds
> with `cargo rivet deploy`. … **Infrastructure is the software most likely to
> need a fast fix, and baking it in makes its fix the slowest one available.**~~

The proxy turned out to change rarely, so the fast fix it was carried for was
hardly used, while carrying it kept producing stale version pins, demanded
staging bundles and refused card writes. The speed-of-repair argument is kept
in §4 as the cost weighed for each program; for the proxy it lost. §2's tier
list, §2.1's statement that every image service is root, §5.1's table and the
built-and-tested lines in §9 and §11 were rewritten to match.

### 2026-09-07 — §4.3 said a card build needs both signing SEEDS

The original text:

> **4.3 A card build will need BOTH signing seeds**
>
> Payloads are signed with the application seed and the image with the system
> seed (`ADR-028`). A card carrying payloads therefore needs both present at
> build time; `tools/card/build.sh` and `tools/container/build-card.sh` take one
> today.
>
> **Named because it is the kind of thing that is discovered at the worst
> moment** — the night a fleet is being built, by a build that refuses.

**Changed the same day, when it was implemented and the code disagreed.** A card
build signs nothing, so it needs no seed; it needs the two **public** keys, and
it has taken both since 2026-08-25. The paragraph is kept because the *risk* it
named was real and had a different cause — the application key's absence was a
warning rather than a refusal — and because a reader who finds the old sentence
elsewhere should be able to see what replaced it.

### 2026-09-07 — §10 did not say what a failed payload does

The original text:

> * **What happens when a carried payload fails to verify.** §4.2 accepts the
>   risk; it does not say whether the machine should refuse to converge, or come
>   up without it and say so loudly. The second is more in keeping with
>   `ADR-017` §3.2, which has no dependency graph and lets an application retry.

Ruled the same day, in the direction it leaned, and moved into the body as §10
with the argument that decides it: refusing to converge would take `rivet-mcpd`
down, and §4.2's acceptance of this risk rests entirely on the control plane
coming up regardless.
