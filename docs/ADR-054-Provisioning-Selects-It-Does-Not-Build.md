# ADR-054: Provisioning Selects, It Does Not Build

* **Status:** Accepted and built. §2's sequence is `cargo rivet provision`
  end to end; §3's widened injection, §4's carried-payload verification and
  §5's repository carry are Wave C3, built and unit-tested — §5 carrying the
  repository's text only and declaring `[repos.fleet]` since `R-182`/`R-184`;
  §6's redirect and §7's summary are built. §8's scratch-and-container proof
  has run: a fresh `tools/container/build-card.sh` build and
  `write_image::inject`'s own widening of a selected image agree byte for
  byte, on every file of both boot partitions, given the same resolved
  inputs — see §8.
* **Date:** 2026-09-13, amended 2026-09-14 (Wave C3: §3–§5, §7; item 1's
  byte-for-byte proof and the carried-manifest fix it found: §5, §8; `R-182`
  and `R-184`: §5–§8, a server's card carries text only and its document
  declares the repository it serves; `R-184` D1: §7's summary names
  `fleet sync`)
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Ruled:** `.rulings/` R-174 (the acceptance test — one command, interview to
  a booted card, or a refusal), R-175 (provisioning selects, never builds or
  signs), R-176 (the eight-stage run, W0), R-177/R-177.1 (system images join
  the fleet repository, zstd-compressed), R-178 (the provisioning rebuild's
  seven decisions, P1–P7), R-180/R-180.1 (the redirect for an already-provisioned
  machine, and static addresses at enrollment).
* **Answers:** how does `cargo rivet provision` turn an interview into a
  written, verified card — without this tool ever compiling, signing or
  publishing anything itself?
* **Supersedes:** R-173's and R-174's own framing of what belongs to this
  command. See History for what each said and why it did not last.
* **Builds on:** `ADR-052` (the fleet repository: text in git, artifacts by
  digest, deployment selects), `ADR-050` (roles decide what a card carries;
  §4.6 records this ADR's own carve-out), `ADR-049` (the staging bundle and
  its supervisor-side delivery), `ADR-018` §4 (the fixed five-partition
  layout every injection here depends on), `ADR-035` §4 (git served over the
  control plane, which a fleet server's repository text reaches it by).

---

## 1. Context

Three rulings inside one day changed what this command is allowed to do, and
a reader who only has the code would not see why. `R-173` said the
orchestrator writes a card and nothing more. `R-174` reversed that: a real
run against the documented path produced a fleet server with no
`rivet-fleetd` and no register, because finishing the job by hand took three
secret installations and two directory creations — so `R-174` folded
building, signing and publishing a role's own application services into
every `cargo rivet provision` run. `R-175` reversed `R-174` on the same day:
rebuilding and re-signing a system image and a role's applications for every
computer is not what provisioning is for — software is built and signed
**once**, published into the fleet repository, and provisioning **selects**
from what is already there. `R-176` then described the eight-stage run this
settled shape actually takes, and `R-177`/`R-178`/`R-180` are what building
each stage required.

This record states the shape that stood, not the two that did not — see
History for R-173 and R-174's own words and why each was withdrawn.

---

## 2. Decision

**Software flows one direction.** A project — a system image, an
application, a shared library — is built and signed by its own tooling
(`cargo rivet update`, `cargo rivet package`, `cargo rivet image`), published
into the fleet repository (`ADR-052` §2), and selected from there by
whatever needs it next: `cargo rivet deploy` for a running machine, `cargo
rivet provision` for a fresh one. Provisioning never compiles, signs or
publishes anything, on any route, including the interactive interview.

`cargo rivet provision` is eight stages, and a refusal at any one stops the
run before the next begins — nothing partial is ever written:

1. **Locate credentials and the fleet repository.** A missing repository is
   offered (`cargo rivet fleet init`'s own work, declinable), the way a
   missing certificate authority already is inside the interview.
2. **Select a system image.** The platform comes from what the repository
   actually holds — one platform used and named on screen, more than one
   asked before a single image is listed, `--platform TOKEN` with none
   refused by name. That platform's `card` images are offered newest first,
   each checked against this build's own schema version; a mismatch is
   listed and marked, not silently accepted. An empty repository refuses by
   name before the interview ever starts.
3. **Run the interview**, `cmd/rivet-provision`, unchanged in shape — the
   same menu, the same certificate-authority detour, and, since the
   repository is now known, the same application-version questions a role's
   document must answer (`[carry.<product>] version`). It advises nothing
   about building or writing a card — the orchestrator marks it
   (`RIVET_PROVISION_ORCHESTRATED`, `SPEC-05` §4) and writes the card itself.
4. **Check for a staging bundle.** A server or member document with no
   `ADR-049` bundle beside it is refused by name before anything is written.
5. **Plan enrollment.** A server or member's register identifier, derived
   from the card's own control-plane pin, must not already be in the
   operator's register — checked before the write, never after.
6. **Prepare, write and verify the card** — §3 below is what "prepare" means
   as of this wave.
7. **Record**, only once step 6 has verified a card actually written to a
   device: the document is committed to the fleet repository and, for a
   server or member, the device planned in step 5 is enrolled.
8. **Summarise** — §7.

`--existing HOSTNAME` (R-180) takes steps 2 and 6 alone, against the
document already recorded for `HOSTNAME` — no interview, no enrollment
planning, no second commit — for rewriting a card that was already
provisioned without provisioning it again.

---

## 3. Preparing the image: everything beyond `rivet.toml`

**The image a card is written from is selected, never built for the
occasion** — step 2, a whole signed `card`-kind artifact out of the fleet
repository, decompressed once into a scratch working copy. What differs per
computer is only what step 3–5 produced: the document, this card's own
identity, and the applications its role carries. All of it goes onto the
working copy's boot partitions before the write, under the exact names
`cmd/rivet-image` would have used for the same inputs, so first boot needs no
change at all:

| File | Source |
| :-- | :-- |
| `rivet.toml` | the composed document |
| `staged.txt` | the `ADR-049` staging bundle beside the document, widened by §5 for a fleet server |
| `device.seed`, `device.crt`, `peers` | this machine's own identity directory, named by the interview's "Identity" answer or `--identity PATH`. The answer defaults to `~/.rivet/identity/<hostname>`, which the interview mints at `w` when it holds no identity: one identity per machine (`.rulings/` R-189 D2) |
| `app-<product>.rvt`, one per carried service | the fleet repository's `[carry.<product>]` pin, resolved to an object the same way a fresh card build resolves one |

This is `write_image.rs::inject`, widened rather than duplicated: the one FAT
mechanism that already opens an existing image's boot partitions now writes
every one of these files instead of `rivet.toml` alone, on both `boot-a` and
`boot-b` (`ADR-018` §4). **Refused by name, before either partition is
touched, when a file will not fit** — the partition's own free space, read
off the FAT filesystem itself rather than estimated from its raw byte
length, so a partial injection never happens.

`cmd/rivet-image` was considered and rejected as the place to do this
instead: it assembles a **complete** partition file list from raw inputs and
has no mode that opens an existing image and edits it — the very capability
`write_image.rs` already has. Building a second "edit an existing image"
path inside a bin-only crate that cannot be depended on would be new,
untested surface where one already works.

---

## 4. Verifying a carried application before it is placed

`ADR-050` §4 already requires a payload to be verified before a card can
install it — `cmd/rivet-image` does this against the image's **own** trust
store, because at fresh-build time it is the process about to compose that
store. Widening an existing image is a different moment: `/system` is ext4,
and only the FAT boot partitions this tool opens are reachable from a
workstation at all. There is no store here to read.

So a carried application is verified against `<rivet root>/keys/rivet-app.pub`
instead — this workstation's own established trust anchor, the same file
provisioning already reads for the system key when it verifies a **selected
image** (`crate::provision_image::materialise`). A failure is a refusal
before anything is written, on `ADR-050` §4's own rule; what differs is
which key answers the question, and the run states once — as a note, not a
second refusal — that it could not confirm the image's own baked-in store
agrees (`ADR-050` §4.6).

---

## 5. A fleet server's card carries the repository's text, and declares where it will be served

**A fresh `rivet-fleetd` needs its fleet repository, and a card image is not
where it comes from.** For a `server` document, provisioning additionally
carries the fleet repository's **text half** — `manifest.toml`,
`documents/`, the register projection under `register/` — and **none of its
objects** (`.rulings/` R-182). The staging bundle this rides in is hex text
capped at `rivet_core::staging::MAX_BUNDLE_BYTES`, and a real server's
`rivet-fleetd` and `rivet-proxyd` alone overran it; so the application and
library objects follow over the control plane after first boot, as system
updates do, into `rivet-fleetd`'s own digest-keyed store (R-184 D1):
`cargo rivet fleet sync <host>` sends them (`ADR-052` §7.4), and §7 names it
after the push.

**`manifest.toml` is re-rendered from a filtered list, never copied
verbatim.** It names only `application`/`library` entries whose object this
workstation's `objects/` actually holds — never a `card` or `update` entry,
which a server never holds, and never one pruned since publish, which nothing
could ever deliver. `crate::carry::repository_carry` builds that list once and
renders the manifest from it through `crate::fleet::render`, the one function
that owns this file's shape. The byte-for-byte proof (§8) is where the verbatim
copy was found.

It rides the existing `ADR-049` staging mechanism rather than a new one —
the format already carries an arbitrary relative path per entry, and the
supervisor already delivers whatever it is handed into an application's data
directory. This is staged to `rivet-fleetd`, under a `fleet-repository/`
prefix chosen so it can never collide with `ADR-049`'s own
`register/register.toml` entry (`SPEC-05` §4.9), which addresses the same
application for a different purpose — this machine's own single-device
register, not the fleet's whole repository.

**Refused by name, with the measured byte count, rather than silently
dropped**, when the bundle will not fit the cap. With objects out of it, the
text of a repository with a very large `documents/` or `register/` tree is
what reaches it.

### 5.1 The document declares `[repos.fleet]`

**Every `server` document the interview composes declares the git repository
the server serves its fleet repository's text as** (`.rulings/` R-184 D3):
`[repos.fleet]` with an identifier minted when `w` composes the document, in
`rivet_core::identifier`'s grammar. `rivet-sysd` creates `/state/git/fleet.git`
from it on first boot, `rivet-mcpd` serves it (`ADR-035` §4), and `cargo rivet
fleet push <host>` puts this workstation's `main` into it. A `member` or
`stand-alone` document declares nothing. The name is fixed —
`rivetos_fleet_repo::SERVED_NAME` — so what provisioning declares and what the
push assumes cannot disagree.

**Which documents gain it, decided conservatively.** A document the interview
composes gains it, and so does one re-composed from saved answers, because
both pass through the composer. A document provisioning did not compose does
not: a hand-written `--document PATH` is written as given, and `--existing`
(§6) writes the recorded document as recorded — R-180 has that route mint
nothing, and silently changing a machine's desired state on a card rewrite is
not a side effect this tool takes. The summary (§7) names the gap for such a
server instead.

**What this does not make true.** The carried text lands in `rivet-fleetd`'s
data directory at delivery, and the pushed text in `/state/git/fleet.git` — two
copies, arriving by two routes. Wiring `rivet-fleetd` to read either is not
this record's work, and neither copy holds a single object. The pushed copy is
the one a fleet server's repository store is pruned by (`ADR-052` §7.4.4); the
carried copy is frozen on the day the card was written.

---

## 6. Redoing a card without provisioning it twice

`R-180`: an id or hostname already in the register is refused before
anything is written, naming `cargo rivet provision --existing HOSTNAME` in
full. The redirect takes the document already recorded for `HOSTNAME` — the
fleet repository's own copy, committed only after an earlier verified write,
never `~/.rivet/provision/`'s local copy, which can hold a composed document
that was never carried through — runs the same image selection (§2 step 2)
any other run does, and writes and verifies the card. No interview, nothing
minted, nothing committed a second time. §3's widened injection and §4's
verification apply to it exactly as they apply to a first provisioning,
because both are the one `write-image` call every route shares. **Nor is the
recorded document changed**: one recorded before `[repos.fleet]` existed is
written without it (§5.1).

`R-180.1`: enrollment records a static address, when the document configures
one, in the register record's own `interfaces` — so the fleet can reach the
machine immediately rather than waiting to be found by name.

---

## 7. The summary, and the one thing it does not claim

A verified write to a device ends with: the hostname, the role,
the image selected (product, version, a digest prefix), every carried
product and version, the register identifier enrolled (or that none was),
the device written (by path, or as chosen from the menu), and one line naming
what remains unproven.

**A run that writes nothing prints no summary.** On an interactive run that
ends at `N`, a cancelled administrator prompt or no removable device
(`SPEC-05` §4 step 6), the ending instead says nothing was written and nothing
was recorded, and names the `cargo rivet provision --document <path>
--identity <path> --image <digest> confirm` command that writes the same card
later without the interview. It never names the image's working copy, which
the run removes as it exits.

**It never says the board boots.** Writing a device and reading it back
proves the bytes match what was sent (`SPEC-05` §4's own BLAKE3 comparison)
— it does not and cannot prove `rivet-fleetd`, `rivet-proxyd` or
`rivet-mcpd` ever started. `ADR-049` §9.2 and `ADR-050` §11.2 already draw
this line for their own claims; the summary draws it explicitly rather than
leaving a reader to find it, naming the one command that closes the gap:
`cargo rivet run <host> -- get status`, over the control plane, once the
board has powered on.

**A fleet server's summary then names what the server still needs**, in two
numbered lines (`.rulings/` R-182, R-184). Its text: `cargo rivet fleet push
<host>` for the plan and `… confirm` to push — or, for a server document that
declares no `[repos.fleet]` (§5.1), that the push will be refused and
`rivet-fleet add repo --device <host> --name fleet` mints the declaration. Its
objects: not on the card (R-182); after the push, `cargo rivet fleet sync
<host>` shows what would be sent and `… confirm` sends every application and
library object the manifest names into the server's repository store over the
control plane (R-184 D1, `ADR-052` §7.4), a rerun sending only what is still
missing.

---

## 8. What is proven, and what is not

**Proven by unit test**, with fakes for every filesystem boundary except a
scratch one: the widened injection places every file on both boot
partitions, byte for byte, and refuses by name before either partition is
touched when something will not fit; a carried payload is verified and
placed or refused by name for a wrong key or a missing workstation trust
key; a fleet server's document carries the repository's text and **no
object** — not one path under `objects/`, even when an object alone is as
large as the whole cap — a member's carries nothing of it, and text too large
for the cap is refused by name; **the carried `manifest.toml` names only
entries this workstation holds an object for — never a `card`/`update` entry,
and never one whose object has been pruned from `objects/`**
(`carry_tests.rs::the_carried_manifest_names_only_entries_this_workstation_holds_an_object_for`,
the byte-for-byte proof's own finding); a `server` document composed by the
interview declares `[repos.fleet]` with a well-formed minted identifier the
board's own parser accepts, a `member` or `stand-alone` one does not, and a
malformed identifier is refused (`cmd/rivet-provision`); a server's summary
names `fleet push` when its document declares the repository, names the gap
when it does not, and names `fleet sync` after the push for the objects; the staging
path, the identity directory and the fleet repository root all reach the
writer unchanged, across the ordinary route and `--existing`'s redirect
alike; the summary carries every fact this section lists and never the word
the board would need to have booted to earn.

**Proven by a real, scratch fictional run — R-172.1's own posture,
`AGENTS.md` §9**: a fictional server document, a scratch fleet repository
holding a real signed application (built, signed and verified with real
Ed25519 and BLAKE3, never `rivet_core::package::testing`'s fakes), taken
through `crate::carry::gather` and `write_image::inject` together.

**Proven byte-for-byte against a fresh `tools/container/build-card.sh`
build — this wave's closing step, run 2026-09-14.** A fictional server
document, a scratch identity, and a scratch fleet repository holding a real
`card` image and two real signed fixture applications (`rivet-fleetd`,
`rivet-proxyd` — real Ed25519/BLAKE3 packages, R-172.1's posture again) were
taken two ways: once through `tools/container/build-card.sh` directly
(`RIVET_DOCUMENT`/`RIVET_STAGING`/`RIVET_IDENTITY`/`RIVET_FLEET_REPOSITORY`,
the already-widened `staged.txt` extracted from the second route, since
`cmd/rivet-image` itself refuses an empty `--staging` and R-178 P1's
repository carry only happens at provisioning time), and once by selecting
the SAME generic (undocumented) card from the fleet repository and widening
it with `write_image::inject` (`cmd/cargo-rivet/src/provision_tests.rs`'s
gated `c3_byte_proof_*` tests, exercising the identical `run_with_image_selected`
call a live `cargo rivet provision --document … --image <digest>` CLI run
takes). **Every file on both `boot-a` and `boot-b` — firmware, kernel,
device tree, the four boot-config text files, `rivet.toml`, `staged.txt`,
the identity trio and both carried `.rvt` payloads — was byte-identical
between the two routes.** Evidence — the full procedure, every command run,
and the captured diff — is in `c3-byte-proof/README.md` in the maintainer's
scratchpad, outside the repository (per `AGENTS.md` §8.3); the check itself is
repeatable with
`cargo test -p cargo-rivet --bin cargo-rivet c3_byte_proof`, gated on
`RIVET_C3_PROOF_*` environment variables since building the fixtures needs
`docker` and is not something `cargo test` can do unassisted.

**Rerun the same day after `R-182`, and still identical.** With the
repository carry reduced to text, the same procedure on fresh scratch fixtures
widened a generic card whose `staged.txt` held two entries —
`fleet-repository/manifest.toml` and one document, 3,136 bytes where the
first run's held 219,957 with two objects in it — and every one of the 15 files
on each boot partition matched a fresh build given that bundle. Evidence in
`c3-r182/README.md`, in the same scratchpad.

**Not proven by anything in this wave**: that a card built this way boots to
the state its document describes. `ADR-050` §11.2's own reasons are
unchanged — first boot's read of a carried payload needs
`/proc/device-tree/chosen/bootloader/partition`, which `ADR-026` §4 and
`cmd/rivet-initd/src/confirm.rs` are both explicit that `-M virt` does not
emulate — so this cannot be exercised under QEMU at all; only a real card and
a test board close it. That a server built this way creates and serves its
`fleet` repository, and takes a push, is proven on a QEMU guest booted with
the same declaration (`ADR-035` §4.1) — not from a written card, for the same
device-tree reason. That `rivet-fleetd` reads either copy of the text (§5.1) is
not this record's claim. That `cargo rivet fleet sync` fills a server's
repository store is proven on a QEMU guest (`ADR-052` §7.4.7), not from a
written card and not on a board.

---

## History

### 2026-09-14 — `R-182`: a server's card stops carrying objects; `R-184`: its document declares `[repos.fleet]`

**§5 read, until this date**, that for a `server` document provisioning
carries the fleet repository's text half *"plus the bytes of every
`application`/`library` object that manifest names"*, staged under
`fleet-repository/objects/`, and that *"a repository publishing more than a
handful of small applications will reach this refusal"*. Its closing paragraph
said a board serves repositories only once `ADR-035` §4 had its own phase.

**Why both changed.** The objects did reach the refusal, with no handful
needed: the bundle is hex text capped at 5 MiB, and `rivet-fleetd` with
`rivet-proxyd` is about 3.9 MB of bytes, so a real server provision was refused
and Wave C3's proofs only passed on tiny fixture applications. R-182 ruled the
card carries text and the objects follow after first boot. The serving claim
was stale — git over the control plane had been built since 2026-08-28/29
(`ADR-052` History, same date) — and R-184 ruled the remaining pieces: the
server's document declares `[repos.fleet]` (§5.1), `cargo rivet fleet push`
sends the text, and the objects go to `rivet-fleetd`'s own store —
`cargo rivet fleet sync`, which §7's summary now names (`ADR-052` §7.4).

### 2026-09-13 — `R-174` folded building into this command; `R-175` reversed it the same day

**`R-174`'s own decision**, superseded within the day: *"every step between
the interview and the written card belongs to the tool"* was read as
including staged credentials, the role's own application services **built,
signed, published into the fleet repository and pinned**, and the card build
itself — so `cargo rivet provision` cross-built, signed and published
`rivet-fleetd`/`rivet-proxyd` from source, and built a whole card image,
inside every run.

**Why it did not last.** Software this project already builds once, from a
project's own tooling, does not need rebuilding per computer — a fleet
repository exists precisely so that publishing happens once and selecting
happens as many times as there are computers. `R-175` withdrew the
parenthetical `R-174` added to its own acceptance sentence ("(built, signed,
published into the fleet repository and pinned)", "the card build") as the
agent's gloss rather than the maintainer's words, and ruled that provisioning
selects a system image and a role's applications from versions already
published, and never compiles, signs or publishes anything. `R-176` Q11 then
ruled the build-per-run code reverted, keeping only the fixes made alongside
it — `RealPreflight`'s credential survey ahead of the interview, and
`require_staging`'s refusal for a hand-composed document with no bundle.

### 2026-09-13 — `R-173`: the orchestrator only writes, superseded the same day

**Superseded by `R-174` before it was ever built against.** `R-173`'s own
shape — the interview and the write are two separate commands, and an
operator carries the three facts that differ between them by hand — was
overturned the moment a real run against it produced a fleet server with no
`rivet-fleetd` and no register: finishing the job by hand took three secret
installations, two directory creations, three file writes, five ownership
changes and two service restarts (`SPEC-05` §10, the manifest read off
`rivet-01` on 2026-09-07). The remedy that stood in the end was neither
`R-173`'s nor `R-174`'s in full — `R-175`/`R-176` kept `R-174`'s one-command
acceptance test and replaced its "the tool builds everything" reading with
"the tool selects everything already built."
