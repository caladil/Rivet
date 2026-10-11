---
name: publishing-a-system-image-and-update
description: Build a card image and publish it into the fleet repository for provisioning, or build a system update, publish it, and send it to a running machine's inactive slot. Use when asked to cut a new Rivet release, publish a card image, build or ship a system update, or roll an update out to a board.
---

# Publishing a system image and update

Software flows one direction (`ADR-052` §2, `R-175`): built once by its own
tooling, published into the fleet repository, then **selected** by whatever
needs it next. This skill covers the *build and publish* half for the two
kinds only this tree's own tooling produces — a **card image** (provisions a
fresh machine) and a **system update** (updates one already running) — and,
for an update, the *send* half too.

## When to use it, and when not to

* **A new card image** — cut when the software a fresh machine should be
  provisioned with has changed. Ends at `cargo rivet fleet list` showing a new
  `card` entry; a machine is provisioned from it by
  `skills/provisioning-a-fleet-server`, not by this skill.
* **A system update** — cut to patch a machine already running. Ends with the
  board's own `get slot` confirming the new build is running and committed.

Not for: **provisioning** a machine from a published image
(`skills/provisioning-a-fleet-server`); **deploying an application**
(`skills/deploying-an-application` — a different artifact `kind`, signed with
a different key, `ADR-028` §5.5.2); **reading** a board once an update has
landed (`skills/reading-a-board`, linked below for `get slot`/`get install`).

## The version, and when to bump it

`Cargo.toml`'s `[workspace.package] version` is the single source
(`ADR-027` §10.1); a card image's own name and an update's `--version`
argument both trace back to it. Bump before cutting a release meant to be
published, never mid-work:

```sh
tools/card/bump.sh            # refuse a dirty tree, bump the patch, commit alone, print `git describe`
tools/card/bump.sh --dry-run  # say what it would do; changes nothing
```

**Windows Git Bash only — it refuses under WSL by name**, because WSL's `git`
reads this whole tree as modified (`AGENTS.md` §10.5) and a commit made there
writes CRLF blobs nothing pins. **Patch only.** The script has no path to the
minor or major field; incrementing either is the maintainer's own edit
(`AGENTS.md` §10.6, `.claude/CLAUDE.md`'s "never increment the minor
version"). The sequence is refuse-dirty → bump → commit-alone → build
(`ADR-027` §10.2) — committed *before* the build so a version and a commit
stay a permanent 1:1 mapping and no image is ever stamped for a number that
was minted and not recorded.

```sh
RIVET_DESCRIBE=$(tools/card/bump.sh | tail -1) tools/container/build-card.sh
```

`bump.sh` itself has not been run by an agent. Without a bump, `RIVET_DESCRIBE`
is Windows `git describe --tags --always --dirty` on a clean tree; the gate
rehearsal of 2026-09-14 built that way.

## Building a card image

```sh
tools/container/build-card.sh
```

One command, same on every host: it mounts the repository and a Buildroot
output tree into a container and runs `tools/card/build.sh` unmodified inside
it — `sgdisk`, `mkfs` and `mtools` live only in that container's `archlinux`
distro (`AGENTS.md` §10.5). `RIVET_BR_OUTPUT` names the Buildroot tree; without
one at `$HOME/.rivet/buildroot-*` the script refuses by name.

**Run generic — no `RIVET_DOCUMENT`.** A card image is never built *for* one
computer (`R-175`); provisioning is what puts a machine's own document,
identity and carried applications onto a copy of this image later
(`ADR-054` §3). Without `RIVET_DOCUMENT` the card carries `cmd/rivet-image`'s
own template — a schema version and nothing else — and boots reachable by
nobody until provisioning widens it. That is correct for an image destined for
the repository, not a defect.

**Output:** `out/rivet-<version>-<describe>-aarch64-rpi4-card.img`, where
`<describe>` is the SHA `bump.sh` minted (`git describe --tags --always
--dirty`, asked of Windows `git` — a `-dirty` describe is refused by
`build.sh`, on the ground a card whose version does not match its commit is
worse than no card). The script's own report echoes what it built and what it
trusted (`ADR-028` §5.5.5 — a card built here trusts `keys/rivet-system.pub`
for `update`, `extension` and `card`, and `keys/rivet-app.pub` for `application` and
`library`, by default; the build prints `--- trust:` lines naming both.
`RIVET_NO_TRUST=1` stamps a development-only card instead, loudly, on every
boot).

**The image trusts the committed `keys/*.pub` unless told otherwise.** If your
seeds are not the committed keys' private halves, copy your public keys under
`keys/` and set `RIVET_TRUSTED_KEY` and `RIVET_APP_KEY` to them, as `cargo rivet
setup` prints. The build refuses a key path outside the repository.

**Trap: `out/` keeps only the newest 2 card images by default**
(`RIVET_KEEP`, `tools/card/build.sh`'s own retention, added after `out/` once
reached 36 GB). A build removes older `*-card.img` — and the matching
`out/debug/<describe>` tree — the moment it finishes. `cargo rivet image`
with no `--img` picks the newest `*-card.img` it finds, so this is usually
what you want; pass `--img PATH` explicitly if you built more than one image
since and need an older one, and set `RIVET_KEEP=0` before a session that must
keep every build.

**Run 2026-09-14 by the gate rehearsal**, generic, with `RIVET_OUT` naming a
scratch directory and `RIVET_KEEP=0`. The Buildroot tree comes from the
`rivet-br` docker volume, and with warm caches the build took under a minute.
`RIVET_OUT` confines the retention to that directory.

## Publishing a card image — `cargo rivet image`

One command wraps, compresses, signs and publishes an **already-built** card
image (`R-177` D1); it never builds one itself (D2):

```sh
cargo rivet image --version V --arch aarch64 [--platform TOKEN] \
    [--img PATH] [--signing-key PATH] [--out-dir DIR] [--at DIR]
```

* `--version` and `--arch` are **required** — this tool did not build the
  bytes it is handed, so it cannot compose a version claiming a commit
  (`ADR-027` §3.2.1). `tools/card/build.sh` is given no version: it names the
  image `rivet-<Cargo.toml version>-<RIVET_DESCRIBE>-aarch64-rpi4-card.img`,
  and the gate rehearsal passed that middle part, e.g. `--version
  0.1.87-v0.1.77-444-gf2f7e4d0`.
* `--platform` defaults to `rpi4`, the only token with a `BootSlots`
  implementor today; `rpizw` and `uefi` are named in `ADR-027` §3.1 and have
  none.
* Signed with the **system** seed — `--signing-key`, else `$RIVET_SYSTEM_SEED`,
  else `<rivet root>/keys/rivet-system.seed` — never the application one
  (`ADR-028` §5.5.2's mask puts `card` on `rivet-system` beside `update` and
  `extension`).
* Compressed with **zstd** (`R-177.1`) — the one kind whose one member this
  container format's codec byte may legally compress at all, because a card
  is decompressed exactly once, on the workstation, when provisioning writes
  it, and never installed by a board (`ADR-029` §13/R-107 keeps every
  board-installed kind uncompressed).
* Named `<product>-<version>-aarch64-rpi4-card.rvt`, composed and with no
  override (`ADR-027` §7, §9.5).
* `--at DIR` is the fleet repository to publish into. Default `~/.rivet/fleet`.
  **Always pass a scratch `--at` for a rehearsal** — never a maintainer's real
  one.

A real scratch run, against one of the pre-built images above and a freshly
minted scratch `rivet-system` seed:

```
[3/5] compressing …-card.img with zstd            [3/5] done (1s)
[4/5] signing and sealing the header               [4/5] done (0s)
[5/5] writing …\rivet-0.1.87-scratch1-aarch64-rpi4-card.rvt
[6/5] publishing into the fleet repository          [6/5] done (0s)
wrote    …\rivet-0.1.87-scratch1-aarch64-rpi4-card.rvt
product  rivet
version  0.1.87-scratch1  (taken as given: this tool did not build the image)
arch     aarch64
platform rpi4  (Raspberry Pi firmware (tryboot, ADR-018 §5) — from the boot port, ADR-027 §3.1)
kind     card
schema   1  (read from the image's own rivet.toml, R-177 item 3)
plain    2147483648 bytes
stored   100308959 bytes  (zstd, 4.7% of plain — R-177.1)
key      75cb0258acceebf869ed9814deb5706b11fdea1c7c8a07178a4fed7c5d2c0911 (id 6da9214929cd6dbf…)
fleet    CREATED the fleet repository at …
MADE  6123524f3c72f54a3429669e89438d0846ce10949cac79a52fb3d0db49fc9772
```

**Confirm with `cargo rivet fleet list [--at DIR]`** — one line per artifact,
newest publish last in the file but unordered on screen:

```
041d8974c293  rivet 0.1.88-scratch2 (card, aarch64 rpi4) 100313055 bytes, added 2026-09-14T07:35:34Z
6b6e7b9b3b34  rivet 0.1.89-scratch3 (card, aarch64 rpi4) 100313055 bytes, added 2026-09-14T07:35:53Z
41faa8bd6a3b  rivet 0.1.90-scratch4 (card, aarch64 rpi4) 100313055 bytes, added 2026-09-14T07:35:54Z
```

The digest prefix printed (up to 12 hex characters) is what `install
--artifact` and `fleet resolve` accept back as an unambiguous prefix. **`list`
does not print the schema version** — that field is real (`manifest.toml`
carries `schema_version = 1` per `card` entry) but is read internally by
provisioning's own image-selection step (`ADR-054` §2 step 2), not surfaced by
any human-facing command found in this tree; read `manifest.toml` directly if
you need to see it.

**Pruning to three, proven live** — publishing a fourth `card` of the same
`(product, kind, platform, arch)` group evicted the oldest, and the report
named it (`ADR-052` §7.2.2, `R-177` D4 — the workstation repository never
kept unlimited builds, unlike a board's own store, because nothing would
otherwise bound its growth):

```
pruned 1 of the repository's own retention rule (ADR-052 §7.2.2, R-177 D4) — keeps the 3 newest of each product, kind, platform and arch:
  removed 6123524f3c72f54a3429669e89438d0846ce10949cac79a52fb3d0db49fc9772 rivet 0.1.87-scratch1 (card, rpi4) …
```

**Refusals, exercised live in this scratch repository:**

| Refusal | Cause | Real text |
| :-- | :-- | :-- |
| No image | no `--img` and no `*-card.img` in `out/` | `no --img was given and no *-card.img was found in out/. cargo rivet image NEVER BUILDS one (R-177 D2) — build a card first with tools/container/build-card.sh, then pass --img, or run this from a tree where that script's output is in out/.` |
| Unknown platform | `--platform` names a token with no `BootSlots` implementor | `"uefi" is not a <platform> this build can produce a payload for. The only BootSlots implementor is "rpi4" (…). ADR-027 §3.1 also lists rpizw and uefi; neither has an implementor…` |
| No signing key | neither `--signing-key`, `$RIVET_SYSTEM_SEED` nor the default file exists | names both places looked, in order, and that `cargo rivet key new --out PATH` mints one |
| Wrong key (`fleet publish` by hand) | `--trusted-key` names a key not entitled, or simply a different key | `refused: this package is signed by key 6da9214…, which is not in this machine's trust set` (real, from a scratch `fleet publish` against a second, unrelated key) |
| Not a fleet repository | `--at DIR` (or the default) has no `.git` | `<DIR> is not a fleet repository yet. cargo rivet fleet init --at <DIR> makes one (ADR-052 §2). Nothing was read.` |

`cargo rivet image` never touches a board — a card is decompressed once, on
the workstation, at provisioning time, and never sent over the control plane
(`ADR-052` §7.2.3.5).

## Building a system update — `tools/container/build-update.sh`

The committed recipe takes a built card image, extracts its `/system` and both
boot partitions, and runs `cargo rivet update` on them:

```sh
RIVET_SYSTEM_SEED=<path to the system seed> \
  tools/container/build-update.sh out/rivet-<version>-<describe>-aarch64-rpi4-card.img <describe>
```

* The image is named **relative to the repository** and must exist at
  `<repo>/<that path>`. The host-side check reads the repository's own `out/`
  even when `RIVET_OUT` points elsewhere, while the container reads
  `RIVET_OUT`.
* `RIVET_KEEP_PAYLOADS` (default 3) removes older `*-update.rvt` from the output
  directory. Set it to `0` when you must keep every payload.
* Pass the describe as the update's version. The update is then named
  `rivet-<describe>-aarch64-rpi4-update.rvt`, which is a different version
  string from the card's.

The underlying verb, which the recipe calls:

```sh
cargo rivet update --system PATH --boot-a DIR --boot-b DIR \
    --signing-key PATH --version VERSION --out-dir DIR
```

Unlike a card image, this does **not** take a card image as input — slicing
one would put a GPT parser and a FAT reader inside the one tool that holds a
signing key, exactly what `ADR-029` §5 keeps out of that path. It takes the
pieces directly:

* `--system PATH` — the whole `/system` filesystem for the slot being written,
  streamed past the digest.
* `--boot-a DIR` / `--boot-b DIR` — both boot partitions' files. **Diffed by
  content**, not by a hard-coded list: a name whose bytes agree across both
  becomes one *invariant* member, a name that differs becomes two
  (`SlotA`/`SlotB`, `ADR-029` §3), and a name present on only one side is
  refused by name before anything is written. `ADR-029` §4's reserved names
  (`autoboot.txt`, `rivet.toml`) are dropped and named in the report rather
  than silently skipped.
* `--signing-key PATH` — the **system** seed, 32 raw bytes or 64 hex, no
  default location.
* `--version VERSION` — **required, typed by the caller**, unlike a crate
  build's own `-g<sha>`: this tool did not build the `/system` image it is
  handed, so composing a version would assert `ADR-027` §3.2.1's commit claim
  on somebody else's behalf (`ADR-027` §9.4 item 3). Pass what
  `tools/card/build.sh` was given — Windows `git describe --tags --always
  --dirty`.
* `--out-dir DIR` — required; the tool composes the name into it:
  `<product>-<version>-aarch64-rpi4-update.rvt` (an update **does** carry
  `<platform>`, unlike an application — it is firmware-shaped, `ADR-027`
  §3.1).

The report names every member, which boot files were invariant versus
slot-variant, and any reserved name it dropped. It also names the trap that
matters most before you send one anywhere:

> `/state` on a card built at the layout's minimum is 232 MiB — an update
> payload is roughly 700 MB (`ADR-052` §1.1, §11's own measurement); the
> store keeps only the three newest of each product (`ADR-052` §7.2.2).

**Run 2026-09-14 by the gate rehearsal.** The recipe's own extraction was run in
the container, and `cargo rivet update` then ran on the workstation with a scratch
system seed. The result was a 703463774-byte payload with 9 members, one of them
`cmdline.txt` in two slot variants, and `autoboot.txt` and `rivet.toml` named as
dropped. `cargo rivet fleet publish … --trusted-key <rivet-system.pub>` published
it.

## Publishing an update, and sending it to a machine

**A system update publishes exactly like everything else** (`R-177` D6 ended
the old exception) — through the fleet repository, verified against the
**system** seed rather than the application one, because the header's own
`kind` field decides which public key a publish checks against
(`cmd/cargo-rivet/src/fleet.rs`'s `deploy_system_seed`, sibling to
`deploy_app_seed`).

```sh
# Publish and send in one command — the ordinary route:
cargo rivet install --rvt out/<name>-update.rvt \
    --host <address>:7777 --identity DIR --pin HEX confirm

# Publish without sending:
cargo rivet fleet publish out/<name>-update.rvt --trusted-key PATH [--at DIR]

# Send one already in the repository, selecting it rather than rebuilding it:
cargo rivet install --artifact <digest-or-prefix> \
    --host <address>:7777 --identity DIR --pin HEX confirm
```

`install --rvt` publishes the file into the fleet repository (creating one if
absent), transfers the repository's own copy into the machine's artifact
store over the control plane (`ADR-052` §7.2), gets a BLAKE3 read-back check,
and then runs `rivet apply install <digest> confirm` on the board — the
digest it just sent, never a path (`ADR-052` §7.2.4). **`--artifact`
publishes nothing**; it is the explicit spelling of *select what is already
there*, the shape `R-176`'s own reading of provisioning ("software flows one
direction, built once, selected many times") applies here too. **It then waits
for that install to finish** — a 683 MB write takes minutes — and exits
non-zero if the installer refused the update, so `finished` in `get install`
has already been read, for this install, by the time it returns.
`cargo rivet deploy` is the interview and takes no arguments.

**Installing does not reboot the board, and it arms the trial.** The install
writes the payload into the inactive slot, verifying every member first and
never partially, and its step 10 (`BootSlots::arm_trial_boot`,
`cmd/rivet-deploy/src/install.rs`) arms the next boot as a one-shot trial of
that slot. `get install`'s `means` says so. **Go straight to `apply
trial-restart confirm`** (below); `apply trial-boot` is not needed after an
install. Nothing has rebooted yet.

**Trap — refused while a prior trial is undecided.** `cmd/rivet-deploy`
refuses a system update outright, before any payload check, dry run or real,
when the machine's default boot slot is not its running slot (`ADR-018` §5.4)
— real text:

> `REFUSED: this boot is NOT COMMITTED. This machine is running <slot>, and
> an ordinary reset boots <slot> — which is what an uncommitted one-shot
> trial looks like (ADR-018 §5). This update would write <slot>, the slot
> that reset falls back to, while the trial is undecided: a reset or a power
> cut part-way through that write boots a half-written system, and a card is
> what recovers that. NOTHING WAS WRITTEN. Either let the trial commit — PID
> 1 confirms a healthy boot, and `rivet get slot` then says `committed:
> true` — or reset to roll back to <slot>; then install again.`

### Taking the trial, and confirming it

```sh
cargo rivet run --host <address>:7777 --identity DIR --pin HEX \
    -- apply trial-restart confirm
```

This is the reboot — `SIGUSR1` to PID 1, the same orderly shutdown path as
`apply restart` (stop the supervisor, sync, then reset into the armed slot).
**`apply restart` alone does not consume an armed trial** — it is an ordinary
reset and, per `ADR-018` §5.3, does not take the one-shot flag with it, so
only `trial-restart` actually boots into what the install just armed.

Then, once the board answers again — see `skills/reading-a-board` for the
full picture — the two facts that matter here:

* **`-- get slot`** reports `running`, `committed` and `agree`
  (`rivet-core::operation::slot`). `committed: true` means PID 1 has already
  confirmed the trial healthy on its own; nothing further is needed.
* **`in_trial_boot` says how the boot was entered, not whether it
  committed** — read `committed`, never that field, for the answer
  (`ADR-018` §5.4, `skills/reading-a-board`'s own trap table).

**The rollback is what happens when nothing happens** (`ADR-018` §5.1's own
words): the trial flag is one-shot, so a board that came up unhealthy and is
simply power-cycled returns to the slot it was previously running, with no
operator action beyond providing power. There is no separate "roll back"
command.

## Checking what's published

`cargo rivet fleet list [--at DIR]` — every artifact this repository holds,
one line: digest prefix, product, version, `(kind, arch platform)`, size,
added. Works identically for `card` and `update` entries; nothing in the
output says which kind a line is except the `(kind, …)` field itself.

## Refusals and traps — a table

| Situation | What happens | Where |
| :-- | :-- | :-- |
| No `--img`/no card built | `cargo rivet image` refuses by name, never builds one | `R-177` D2 |
| `out/` retention removed the image you meant | `RIVET_KEEP` (default 2) deletes older `*-card.img` on the next build | `tools/card/build.sh` |
| `--platform` names a token with no `BootSlots` | refused, lists what does exist | `ADR-027` §3.1 |
| A card or update signed with the wrong key | `fleet publish`/`install`'s automatic publish refuses, names the key id | `ADR-028` §5.5 |
| No fleet repository at `--at` | `fleet publish`/`cargo rivet image` refuse and name the `fleet init` to run | `ADR-052` §2, §4 |
| A `.seed` file sits inside the fleet repository tree | `fleet init`/every publish refuses outright — a seed must never enter a git tree that gets pushed | `ADR-052` §4 |
| Sending an update while the current boot is uncommitted | `rivet apply install` refuses with nothing written, naming both ways forward | `ADR-018` §5.4 |
| `/state` too small, or already holding three of this product | the transfer or install refuses; no partial store | `ADR-052` §7.2.2, §1.1 |
| `apply restart` used instead of `apply trial-restart` | does not consume the armed trial boot | `ADR-018` §5.3 |
| Reading `in_trial_boot` instead of `committed` | tells you how the boot was entered, not whether it stuck | `ADR-018` §5.4 |

## What is unproven

* **Run by the gate rehearsal of 2026-09-14, with scratch seeds and a scratch
  repository:** a generic card build, `cargo rivet image`, the update
  extraction, `cargo rivet update`, `fleet publish`, and `cargo rivet deploy
  --artifact <update digest> confirm` against a QEMU guest. The guest took the
  703 MB transfer, verified the payload at the door and read it back
  byte-identical. Its installer then verified the signature and refused, as
  expected under `-M virt`, with `cannot read
  /proc/device-tree/chosen/bootloader/partition`.
* **On a board, this path has run on dev-01**: `install --rvt`, then `apply
  trial-restart confirm` with no `trial-boot`, came up as a trial on the new
  slot and was committed by PID 1 — the installer's own arming, read through
  `get slot` before and after the restart.
* **A card image's boot cannot be exercised under QEMU at all** — `-M virt`
  does not emulate `tryboot` or the device-tree path first boot reads
  (`ADR-026` §4). Only a real card and a test board close either half of
  this skill.
