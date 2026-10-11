---
name: provisioning-a-fleet-server
description: Turn a signed system image already in the fleet repository into a written, verified Rivet card — one command, `cargo rivet provision` — for a fresh fleet server, fleet member, or stand-alone machine, or for rewriting an already-provisioned one with `--existing HOSTNAME`. Use when asked to provision a board, write a card, set up a new Rivet machine, or re-provision one that already has a record.
---

# Provisioning a fleet server

`cargo rivet provision` is the one command `SPEC-05` asks for: an interview,
an image already published to the fleet repository, a write, and a verified
card — or a refusal naming exactly what stopped it. It never builds, signs,
or publishes anything itself (`ADR-054`); it **selects** what earlier
commands already built.

## When to use it, and when not to

Use it for a **fresh machine** — a fleet server (carries its own repository
and administers the fleet), a fleet member (joins one), or a stand-alone
machine (neither). All three go through the same interview; the "Fleet"
question is what tells them apart.

Use `cargo rivet provision --existing HOSTNAME` instead for a machine
**already in the register** — redoing a card after a failed boot, or
rewriting one whose card was lost. Provisioning refuses to enroll the same
identity twice by name (R-180); `--existing` writes that hostname's already-
recorded document onto a newly selected image without provisioning it again.
No interview, nothing minted, nothing committed a second time. It writes the
machine's own identity, `~/.rivet/identity/<hostname>`, when that directory
holds one, and otherwise the board identity older machines were written with.

Not for: building the system image (`tools/container/build-card.sh`,
`cargo rivet image`), building or publishing an application
(`skills/deploying-an-application`), minting or resetting the certificate
authority as its own act (`rivet-provision ca`, `rivet-reset` — a missing CA
is *offered* mid-run, but the ceremony is its own command, R-176 Q9), or
anything after first boot (`skills/reading-a-board`).

## Starting the fleet again, keeping the authority

To rebuild a fleet from clean while the certificate authority, its installed
root and the operator's browser certificate stay exactly as they are, empty the
fleet first:

```sh
rivet-reset --keep-authority --dry-run    # the plan: what goes, what stays, per location
rivet-reset --keep-authority --confirm    # do it
```

It removes the operator register, the fleet repository's `register/` and
`documents/` (as one commit whose subject begins `rivet-reset
--keep-authority`), `~/.rivet/provision`, and the Armada's Fleets. It keeps the
authority directory, never opens a certificate store, and keeps the fleet
repository's `manifest.toml` and `objects/`, the Armada's `armada.toml`, the
signing seeds and every identity. **A fleet repository with uncommitted
changes is refused before anything is removed** — commit or discard them, then
run it again. `SPEC-05` §2 has the whole contract.

Prerequisite 2 below is then already met. Two things this does not do, and a
fresh start needs:

* **The Armada is empty until the new server's card is written.** Writing a
  server's card adds it to an empty Armada, with its `rivet-fleetd` operator
  key, and a member needs that record: one provisioned into an empty Armada is
  written without the server's key in its `peers` and is not enrolled into the
  server. So provision the server first and check its report says
  `armada: Fleet "<host>" added` — the section below has the order.
* **Machine identities are kept.** A hostname whose directory under
  `~/.rivet/identity/` already holds an identity is written with that
  identity, and so with its old pin. Name a new directory at the interview's
  Identity field for a new key.

## A fleet-root mode fleet server: the root is minted on the board

`ADR-063` §4.2. The server's card carries a **request** instead of a root and a
leaf from this workstation; on first boot PID 1 mints the fleet's root and the
server's certificate into `/state/secrets` and hands the leaf to `rivet-proxyd`
and `rivet-fleetd`. No workstation authority is read or needed.

* **A new workstation:** the first-provision guard's answer `3` records the
  fleet's name and suffixes; every server document the interview writes after
  that carries the request.
* **Any workstation, no interview:** add `--fleet-root FLEET [--suffixes
  LIST]` to `cargo rivet provision --document …` (or to `rivet-provision
  stage`). Suffixes are drawn only from `local`, `FLEET.home.arpa` and
  `FLEET.lan`; the default is the first two.
* **An already-recorded machine becoming a new fleet's server:** `--existing
  HOSTNAME --document PATH` keeps its identity and pin, builds the new bundle,
  and records nothing. `--passkeys FILE` (the proxy's `passkeys`, read with
  `get file /state/app/rivet-proxyd/data/passkeys`) carries its passkeys to the
  new card.

After it boots:

```sh
cargo rivet run <host> -- get log latest all      # PID 1's "minted this fleet's root" line
cargo rivet fleet root <host> --out <dir>         # <fleet>-root.crt (PEM) and .cer (DER), fingerprint checked
uv run tools/proof/passkey-browser.py probe --root <dir>/<fleet>-root.cer --address <ip> --name <host>.local
```

`passkey-browser.py` proves TLS against the root alone and WebAuthn in a
throwaway headless browser, so nobody's own devices are used. On a Windows
workstation whose certificate store holds a client certificate the machine's
root issued, the browser half stalls on the proxy's client-certificate request;
a fleet-root machine's root has issued none.

## Prerequisites, in order

Each is checked or offered by the command itself except the last, which is
the operator's/agent's own readiness check (R-179.1) — the tool's preflight
only covers credentials and the repository's existence, not what it holds.

| # | What | Command | Observable |
| :-- | :-- | :-- | :-- |
| 1 | Credentials | `cargo rivet setup` (or `--check`) | Four things minted in order — system seed, application seed, your identity, the board's identity — or, under `--check`, a report of what is already there. Never overwrites an existing seed. |
| 2 | The CA | `rivet-provision ca`, or let the interview offer it | The interview's first question on a workstation with no authority (`ADR-063` §8): `new fleet, or join one? [1, 2, or x]`, with no default. `1` goes on to `fleet name`, `DNS suffixes`, `operator name`, ending `minted <dir>\.rivet\ca`. `2` mints nothing, names `~/.rivet` and `rivet-fleet`'s register as what to carry across from a workstation already in the fleet, and stops — **a second workstation joining a fleet answers `2`**. Piped with no terminal, it refuses unless `rivet-provision --new-fleet` is given. |
| 3 | A fleet repository | `cargo rivet fleet init`, or decline and let preflight offer it | `no fleet repository yet at <root> -- create one? [Y/n]` printed by `cargo rivet provision` itself, before anything else runs. |
| 4 | A system image, published | `tools/card/bump.sh` (Windows `git`) → `tools/container/build-card.sh` → `cargo rivet image --version V --arch TOKEN [--platform TOKEN]` | `cargo rivet image`'s own report names the digest it published; `cargo rivet fleet list` shows a `card` entry for the platform. |
| 5 | The role's applications, published | Build each with its own project's tooling, then `cargo rivet package` + `cargo rivet fleet publish FILE.rvt --trusted-key PATH` (`skills/deploying-an-application`) | `cargo rivet fleet list` shows an `application` entry per product the role carries — `rivet-fleetd` for a server, nothing for a member. **Not `rivet-proxyd`**: it is part of the system image (`ADR-060`), and publishing one is refused. |

**The image of step 4 must carry the proxy for a server or a member.**
`cargo rivet provision` refuses a repository image older than
`rivet_core::service::PROXY_IN_IMAGE_SINCE` for either role, before anything is
written; an image named by a file path has no recorded version, so it is
written with a `NOT CHECKED` line saying so (`ADR-060` §5).

**A server or member needs a staging bundle whatever it carries** — the role
decides it, not the payloads: the proxy is handed its certificate from the
bundle, so a member, which carries no application, still needs one. The
interview writes it at `w`; a document handed to `--document` without one has
it built by `rivet-provision stage` on the way to the write (`ADR-049` §12).

## A fresh fleet: the server, its card, then each member

**Order matters, and the reason is one key.** A member's card must carry, in its
`peers`, the key its fleet server's `rivet-fleetd` presents — without it the
server lists the member and manages nothing on it. The member interview reads
that key from the Armada (`~/.rivet/armada`), and a server reaches the Armada
when its card is written. So, starting from an empty register and Armada with
the CA kept:

1. **The server.** `cargo rivet provision`; answer Fleet `server`, Mode as the
   machine needs, a static address and its nameservers (field 10). Write the
   card. After the verified write
   the report says `register: enrolled …` and `armada: Fleet "<host>" added …,
   with its rivet-fleetd operator key`. **If it says `armada: … NOT added`**,
   the Armada held another Fleet; run the `cargo rivet armada add <host>` it
   names before the member.
2. **Boot it**, and `cargo rivet run <host> -- get status`. The member's
   enrollment in step 3 is sent to this server over the control plane, so it
   must answer.
3. **Each member.** `cargo rivet provision`; answer Fleet `member`. At `w` the
   staging notes say `added this member's fleet server's operator key`. **If
   they say `no fleet server operator key was supplied`**, stop: the Armada has
   no Fleet (step 1's card was not written, or the Armada was emptied since),
   and the card this would write cannot be managed by its server. Write the
   card; the report ends `fleet enroll  <member> is enrolled into Fleet …`.
4. **Deploy.** A person: `cargo rivet deploy`. An agent: `cargo rivet install
   <host> --artifact <digest> confirm` per application, then the steps it
   prints (`skills/deploying-an-application`). A member's card carries no
   application; every one of them is installed here.

`tools/provision/rehearse-member.sh` rehearses steps 1 and 3 through the
interview, in a scratch `--at`, inside `tools/test-all.sh` — with no card
written, so the Armada record step 1 would leave is written by the script.

**If step 4 was skipped, the run refuses before the interview even starts**,
verbatim (R-176 Q8, confirmed in `cmd/cargo-rivet/src/provision_image.rs`'s
`NO_IMAGES_REFUSAL` and reproduced live in `r178-p6-pty-proof/run.log`, in the
maintainer's scratchpad):

```
No images available for provisioning. Deploy a system image to your artifact repository prior to provisioning.
```

**If step 5 was skipped for a required product, the interview refuses at the
version-selection question, before minting a certificate or writing
anything**, naming the fix:

```
NOTHING WAS WRITTEN. <product> must be carried (ADR-050 §5.1) and the fleet
repository at <root> holds no published `application` artifact of that
name. `cargo rivet fleet publish` is how one gets there — publish it, then
run this again.
```

## The run itself, stage by stage

```sh
cargo rivet provision [--document PATH | --existing HOSTNAME] \
    [--image PATH-OR-DIGEST] [--platform TOKEN] [--device PATH] \
    [--staging PATH] [--identity PATH | --pin HEX] \
    [--at DIR] [--not-mine] [confirm] [--verify-only]
```

A refusal at any stage stops the run there; nothing partial is ever written
(`ADR-054` §2).

**1 — Preflight.** Credentials, checked before the interview runs; a missing
fleet repository is offered (declinable) the way a missing CA is inside the
interview.

**2 — Image selection**, before the interview, so a refusal here costs no
typing. If the repository holds images for more than one platform, it asks;
with one, that one is used and named on screen. That platform's `card`
images are offered newest first; a schema mismatch is **listed and marked**,
never silently skipped or accepted — seen for real:

```
--- rivet 1.0.0-pty-mismatch is not selectable: needs schema 999999, this tool speaks 1
```

The selected image is verified against this workstation's own
`keys/rivet-system.pub` and decompressed into a scratch working copy.
`--image` names a selection directly (a real path, or a fleet-repository
digest or unambiguous prefix), skipping the menu — the route an agent with
no terminal takes.

**3 — The interview**, `cmd/rivet-provision`, unchanged in shape — skipped
by `--document PATH` or `--existing HOSTNAME`. Real excerpts from
`r178-p6-pty-proof/run.log`: the menu (`1 Hostname … 6 Fleet … 7 Timezone …`,
a field that reads `fix 7: required` until answered), the Fleet choice
(`1 server  2 member`; the word itself is accepted too), and — since R-178 P3, right after `w`
— a version pick per carried product, newest preselected:

```
  rivet-fleetd — published versions:
    1)  0.1.1 (newest)
    2)  0.1.0
  carry [1]:
```

`w` then composes the document, mints the machine's certificate, writes the
document and its `ADR-049` staging bundle locally, and re-reads the document
back through `rivet_config::Document::parse` — the same parser a board
refuses with.

**The Identity field (9) is one identity per machine** (`.rulings/` R-189
D2). It defaults to `~/.rivet/identity/<hostname>` and follows the hostname
until you type another. When that directory holds no identity, `w` mints it —
`device.seed`, `device.crt`, and a `peers` naming your operator key — and
prints `minted identity <dir>`. Editing and quitting mint nothing. An identity
whose pin is already on another hostname's register record is refused at
`w`, before anything is written, naming that record's hostname and id; pick
or mint another directory. Do not name `~/.rivet/identity/board` for a new
machine: `rivet-08ecc` already holds that key.

**The Nameservers field (10) is asked only for a static address.** The menu
has ten questions; the tenth appears when Addressing is `static` and is absent
under `dhcp`, whose lease names the nameservers. Its prompt offers the gateway,
and a bare Return takes it:

```
  up to three IPv4 addresses, in the order asked, e.g. 192.168.1.1, 1.1.1.1
  Nameservers [192.168.1.1] >
```

Type up to three, separated by commas or spaces. A fourth, or anything that is
not an IPv4 address, is refused at the prompt by name — the same things the
board's parser refuses — and so is an empty list. `w` writes the answer as
`[network.resolver] nameservers = [...]`. Without it a static machine has no
resolver at all, and every lookup fails inside the application that made it.

**4 — The staging bundle.** A server or member document with a bundle beside
it uses that one — what an interactive run always has. One with none has it
BUILT: the run calls `rivet-provision stage --document … --identity … --at …`
(the installed sibling, version-checked like the interview), prints
`--- staging bundle: none beside <doc>, so rivet-provision stage built <path>`
with every file's name and size, and goes on. It needs the authority under
`--at` and the machine's identity (`--identity`, or the board identity); either
missing is refused by path, nothing written. **Never built:** a `--staging
PATH` that names nothing, and `--existing HOSTNAME`, where a new bundle would
mint a server a new `rivet-fleetd` key its members do not trust. The
`ADR-060` image check runs before this, so a refused image mints nothing.

**5 — An enrollment check**, for a server or member, before the write, not
after (R-176 Q10). A stand-alone document, or the `--existing` redirect,
needs none of it.

**6 — The write**, `cargo rivet write-image` called directly with the image
step 2 selected. It widens the image (`ADR-054` §3): the document,
identity (`device.seed`, `device.crt`, `peers`), and one `app-<product>.rvt`
per `[carry.<product>]` pin are placed on **both** boot partitions, under the
exact names a fresh build would use. Each carried payload is verified
against this workstation's `keys/rivet-app.pub` first. **The reachability check
reads the `peers` file the card will carry**: the identity directory's, which
replaces the image's own. A published image is generic and carries none. If
that `peers` does not name your operator key, the run refuses, naming that
file, before anything is written. Before the gate rehearsal of 2026-09-14 the
check read only the image, so every run of a published image was refused
"carries NO peers file" after the interview. **A fleet server
additionally carries the repository's text half — and none of its objects**
(R-182): `manifest.toml`, `documents/`, `register/`, refused by name, with the
measured byte count, if even that will not fit the 5 MiB staging cap
(`rivet_core::staging::MAX_BUNDLE_BYTES`). Its document, composed by the
interview, also declares `[repos.fleet]` — the git repository the server will
serve that text as (R-184). Then the device, and **the bare command goes all
the way to it**: typed with no flags at a terminal, it lists the removable
devices and you type a number, the plan is printed (`WRITING THIS ERASES
EVERYTHING …`), and `Write it? [y/N]` is asked (default no; `confirm` means
already answered and skips it). Only `y` makes Windows ask for administrator;
then the elevated write locks and dismounts the disk's volumes, and the device
is read back and BLAKE3-compared against what was sent (`SPEC-05` §4.10–§4.12
has the full sequence and a real transcript).

**`N`, a cancelled administrator prompt, or no removable device** writes
nothing, records nothing, and names the command that writes the same card
later without the interview, with this run's own paths and digest filled in:

```
Nothing was written to a device and nothing was recorded. To write this same card later without redoing the interview:
  cargo rivet provision --document <…\provision\<host>.provisioning.toml> --identity <identity dir> --image <digest> confirm
```

**7 — Recording**, only once step 6 has verified a card actually written to
a device. The document is committed to the fleet repository and, for a
server or member, the register gains the record step 5 planned. Then the
summary (`ADR-054` §7) — real shape, from `provision.rs::summarize`:

```
--- summary ---
 hostname        example-c3-server
 role            server
 image           rivet 1.0.0-c3proof db90801cf9c6
 carried
    rivet-fleetd 0.1.0
 register id     <derived id>
 device          <device path>

UNPROVEN: that example-c3-server boots as this document describes. Writing
and reading back a device proves the BYTES match; it does not prove
`rivet-fleetd`, `rivet-proxyd` or `rivet-mcpd` ever started. Once the board
has powered on, `cargo rivet run example-c3-server -- get status` (SPEC-05
§4) is the command that closes that gap.

NEXT, for a fleet server, once `get status` answers:
 1. its text   `cargo rivet fleet push example-c3-server` shows the plan, and
    `cargo rivet fleet push example-c3-server confirm` pushes this
    workstation's fleet repository (`main`) into the server's `fleet`
    repository over the control plane.
 2. its objects   NOT carried on this card (R-182). After the push, `cargo
    rivet fleet sync example-c3-server` shows what would be sent, and `cargo
    rivet fleet sync example-c3-server confirm` sends every application and
    library object the manifest names into the server's repository store
    over the control plane (R-184 D1). A rerun sends only what is still
    missing.
```

(The `NEXT` block is a fleet server's only, and wrapped here for reading.)

## Agents without a terminal

**Without a terminal and with neither `--document` nor `--existing`, the run
refuses** rather than launching an interview nothing can answer — verified
live (`r178-p6-pty-proof/run.log` never reaches this path; the refusal text
is `cmd/cargo-rivet/src/provision.rs`'s `NO_TERMINAL_REFUSAL`, unchanged
since built):

```
cargo rivet provision has no terminal to run the interview at, so nothing
was composed and nothing was written. An agent should compose a document
directly -- rivet_config::Document::parse validates it, the same parser a
board refuses with -- and pass it with --document PATH: `cargo rivet
provision --document PATH --image <digest-or-path> [--platform TOKEN]
[--device PATH] [confirm]` makes the same write this command would have
made, skipping the interview entirely.
```

That route needs `--identity PATH` or `--pin HEX` too, for step 5's
enrollment check, since no interview ran to supply a pin. `--at DIR` reaches
the whole run for a scratch rehearsal — the fleet repository step 2 lists
and step 7 commits into, and step 5's register — never only the credential
check `write-image --help` describes on its own. A real, proven CLI
invocation, captured verbatim, is in `c3-byte-proof/provision-cli.log` in the
maintainer's scratchpad.

## Refusals, and what to do about each

| Refusal | Cause | Do |
| :-- | :-- | :-- |
| No images | repository empty, or nothing for the requested platform | Publish a system image: `tools/container/build-card.sh` then `cargo rivet image` |
| Schema mismatch | image built against a different `rivet_config::SCHEMA_VERSION` | Publish an image built against the current schema, or provision with a build of `cargo-rivet` that speaks the image's schema |
| Unknown platform | `--platform TOKEN` named has no images | Omit `--platform` to be asked, or name one the refusal lists as having images |
| Missing staging bundle | `--staging PATH` names nothing, or `--existing HOSTNAME` on a workstation that never staged that machine — every other route builds one | Name the right bundle, or run `--existing` where the machine was provisioned |
| `building one failed: … no certificate authority at …` / `no identity at …` | a document with no bundle, and `rivet-provision stage` could not build one | Mint or restore the CA under `--at`; pass `--identity DIR` naming the machine's identity |
| A carried app fails its signature | the artifact's signature does not verify against this workstation's `keys/rivet-app.pub` | Confirm which `RIVET_APP_SEED`/seed signed the published artifact and that this workstation trusts its public half |
| A file will not fit a boot partition | the document, identity, carried apps and (for a server) repository text exceed the FAT partition's own free space | Nothing was written to either partition; the image's boot partition is undersized or carrying too much — build one with more room, or carry fewer applications |
| `fleet push`: serves no `fleet` repository | the server's document declares no `[repos.fleet]` | `rivet-fleet add repo --device <hostname> --name fleet` mints the declaration; its report names how it reaches the machine |
| `fleet push`: NOT A FAST-FORWARD | the server's `main` holds a commit this workstation lacks | Reconcile by hand; the tool never forces |
| `fleet sync`: `missing` | the manifest names an object `objects/` does not hold | `cargo rivet fleet objects restore <dir>` from a backup, or publish it again; the rest were sent |
| `fleet sync`: `refused` | the server's door rejected the object (signature, member bytes, or a `card`/`update` kind) — the reason is quoted | Republish a good artifact; sending the same bytes again is refused again |
| `fleet sync`: `PRUNE NOT RUN` | the sync did not complete, or the server's `main` is not this `main` | Finish the sync; `fleet push <hostname> confirm` first |
| `fleet sync`: 404/405 on `/repository-objects/` | the server's `rivet-mcpd` predates the store, or `rivet-fleetd` is not installed | Update the machine; confirm `rivet-fleetd` is installed |
| Already provisioned | the hostname or its derived register identifier is already enrolled | Use the redirect the refusal names: `cargo rivet provision --existing "<hostname>" [--image PATH] [--device PATH] [confirm]` |
| `The identity given for <host> is already "<name>"'s (register id <id>, …)` | the Identity answer (or `--identity`) holds a key another machine's record carries | Pick another identity directory, or name an empty or new one so `w` mints it. For that record's own machine, use `--existing "<name>"` |
| No terminal | interactive interview needs one; none is attached and neither `--document` nor `--existing` was given | Compose a document and pass `--document PATH --image <digest-or-path>` |
| `NOTHING WAS WRITTEN. The card this run would write — its peers file comes from <dir>\peers, which carries a peers file naming N key(s), and none of them is yours` | the identity directory named by the interview's Identity field (or `--identity`) has a `peers` that omits this workstation's operator key | Add the operator key to that `peers`, or name an identity whose `peers` names it |
| `<host>.provisioning.toml ALREADY EXISTS … overwrite it? [y/N]` at `w` | an earlier run wrote the document and was refused, or quit, before the card was written | Answer `y` to compose it again. Nothing was recorded for the earlier run |

**Traps found by the gate rehearsal of 2026-09-14:**

* **An image selected from the repository is decompressed into a scratch copy
  that is removed when the run ends.** There is no way to write the
  provisioned result to an image FILE. Only a literal `--image PATH` is
  modified in place and kept. An interactive run never names the scratch copy;
  a run with no terminal and neither `--device` nor `confirm` still reports
  `the image is ready at <…\image-work-N\…img>`, a file gone once the command
  returns, with nothing committed or enrolled.
* **A repeated `w` reuses the fleet-server operator key its previous staging
  bundle carries**, so `peers` gains it once. Deleting
  `~/.rivet/provision/<host>.staged.txt` between runs mints a new key, and the
  old one stays in `peers`.

## After the write

**The summary never claims the board boots.** Writing a device and reading
it back proves the bytes match what was sent; it proves nothing about
whether `rivet-fleetd`, `rivet-proxyd` or `rivet-mcpd` ever started
(`ADR-054` §7, `ADR-049` §9.2, `ADR-050` §11.2). This cannot be exercised
under QEMU at all — first boot's read of a carried payload needs
`/proc/device-tree/chosen/bootloader/partition`, which `-M virt` does not
emulate (`ADR-026` §4). Only a real card and a test board close it.

Once the board has powered on, the summary names the exact command:

```sh
cargo rivet run <hostname> -- get status
```

(`<hostname>` resolves through `rivet-fleet`'s register once step 7 has
enrolled it; see `skills/reading-a-board` for what to read next — `started`,
not merely installed.)

**A DHCP machine needs no `--host`.** Its record carries no address, so the
command asks this workstation's resolver for `<hostname>.local`, which the
board's `rivet-mdnsd` answers. It prints the address it dials, IPv4 first, and
still pins the register's key. If the name does not resolve, the refusal names
`<hostname>.local`; give `--host <address>:7777` for that run. The printed
address is how to see that some other machine on the LAN answered the name.

### A fleet server: put the repository's text on it

The card carried the fleet repository's text into `rivet-fleetd`'s data
directory, and the server's own `fleet` git repository starts empty. Once it
answers:

```sh
cargo rivet fleet push <hostname>            # the plan: both mains, what a push would do
cargo rivet fleet push <hostname> confirm    # push main, then read it back
```

It pushes `main` of `~/.rivet/fleet` (`--at DIR` for another) over the control
plane as your operator identity, resolved through the register like `run
<hostname>`; `--host`, `--identity` and `--pin` override. It **refuses**:
uncommitted changes (commit them first), a repository with no `main`, a server
with no `fleet` repository (a document that declares no `[repos.fleet]` — one
recorded before R-184, or rewritten with `--existing`), an identity the server
does not accept, and anything that is not a fast-forward. **It never forces.**

The objects do not travel with it: they are not in git and the card does not
carry them. Proven on a QEMU guest (`tools/qemu/adr-035-git-proof.sh`), not on
a board.

### Then its objects

After the push, send the application and library objects the manifest names:

```sh
cargo rivet fleet sync <hostname>                    # the plan: would send / skipped / missing
cargo rivet fleet sync <hostname> confirm            # send only what the server lacks, then read the store back
cargo rivet fleet sync <hostname> --prune confirm    # and remove what the server's manifest stops naming
```

Same resolution and overrides as `fleet push`. The objects go over the control
plane into `rivet-fleetd`'s own store, `/state/app/rivet-fleetd/data/repository-objects`,
each verified at the server's door (`ADR-052` §7.4). **Done** is the last line
reading `SYNCED`, with exit zero; anything `missing`, `refused` or `failed`
exits non-zero, and a rerun sends only what is still not held. `--prune` runs
only after a complete sync, and only once the server's `main` is this `main` —
push first after removing an entry.

Proven on a QEMU guest (`tools/qemu/adr-052-objects-proof.sh`), not on a board.

### Fleet as code is a separate repository, and provisioning leaves it off

`rivet-fleetd`'s poller and the portal's Save (`ADR-044`) work against a
repository of `fleet/machines/<label>.toml` documents that is **not** `fleet`.
Turning it on is two edits to the server's own document, then a push:

1. declare a second repository, e.g. `[repos.machines]` with a minted `id`
   (`rivet-fleet add repo --device <hostname> --name machines`);
2. set `[applications.rivet-fleetd] fleet-repository = "machines"` — a
   repository NAME on this server, never a URL, and never `fleet`, which is
   refused;
3. push a first commit holding each machine's `rivet get desired-state`
   verbatim, with a git that presents your operator identity to the server's
   control plane (`tools/qemu/adr-044-fleet-code-proof.sh` shows the `git -c
   http.*` settings, with throwaway keys).

`rivet-fleetd` runs no git: it asks its own `rivet-mcpd` for
`/fleet-as-code/{head,machines,file,commit}` with its operator identity, through
the register record provisioning wrote with `role = "fleet-server"`. So:

| Symptom in the server's journal | Cause | Do |
| :-- | :-- | :-- |
| `refused no-branch` | the repository has no commit yet | push the first commit |
| `holds no record with role = "fleet-server"` | the server's own register record lost its role, or the register was hand-written | put `role = "fleet-server"` back on exactly one record, the server's own |
| `did not answer /fleet-as-code/head` | the server's control plane is down, or its `peers` lacks `rivet-fleetd`'s operator key | `get status`; the key is what provisioning added to `peers` |
| `answered 404 with a body that is not a refusal` | the server's `rivet-mcpd` predates these operations | update the machine |
| `refused stale-head` on Save | somebody else's commit landed first | reload the page; nothing was committed |

Proven on a QEMU guest, with `rivet-fleetd` confined, converging itself; not
on a board, and not across a second machine under confinement.

## Safety

* **The disk write is the operator's.** Never write a card, or answer
  `Write it? [y/N]`, on someone's behalf.
* **Seeds are never printed**, on any path through this command or
  `write-image`.
* **`--at DIR` for every rehearsal.** It redirects the interview's CA and
  saved-answers notebook, the fleet repository default, and the register
  checked for a duplicate — never the maintainer's real `~/.rivet`. Confirmed
  by a real ConPTY run whose `HOME`/`USERPROFILE` pointed at a directory that
  must never come to exist, and asserting afterward that it still did not
  (`SPEC-05` History, 2026-09-14).
* **`--not-mine`** is required to proceed writing a board someone else
  operates, whose `peers` file does not name your own key — you will not be
  able to reach the card this writes.
* **Never write a disk or touch a board from an agent session** unless the
  operator has asked for that specific write and is present for the `Write
  it? [y/N]` and administrator prompts.

## What is proven, and by what

Unit-tested throughout (`cmd/cargo-rivet/src/provision_tests.rs`,
`provision_image_tests.rs`) with fakes — no terminal, no subprocess, no disk
beyond a scratch directory. The interactive interview, in a real terminal,
is proven by a `portable-pty`/ConPTY harness
(`cmd/cargo-rivet/tests/pty_interview.rs`,
`r178-p6-pty-proof/README.md` in the maintainer's scratchpad). That the widened
injection matches a fresh `tools/container/build-card.sh` build byte for
byte, on every file of both boot partitions, is proven in
`c3-byte-proof/README.md`, in the same scratchpad. **Not proven anywhere in
this tree**: that a card produced this way boots to the state its document
describes — see "After the write" above.

`docs/COMMANDS.md`'s `cargo-rivet` entry documents the top-level binary only
— it does not include `provision`, `write-image`, `image`, `fleet` or
`setup`'s own `--help` text (`tools/docs/commands-manifest.tsv` generates one
entry per **binary**, and these are sub-verbs of one). Run `cargo rivet
provision --help` (etc.) directly for the current text, quoted above from a
build of this tree on 2026-09-14. See `ADR-054` for why provisioning selects
and never builds, and `SPEC-05` §4.5–§4.12 for the twelve-step operator
script this command collapses six of into one.
