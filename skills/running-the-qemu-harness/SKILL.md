---
name: running-the-qemu-harness
description: Run Rivet's QEMU inner development loop — the gate (smoke.sh), the artifact-store proof, and what this loop can and cannot settle before a change goes anywhere near a board. Use when asked to test, prove, or reproduce a Rivet change under QEMU, or before attempting something on real hardware.
---

# Running the QEMU harness

`tools/qemu/` proves PID 1 semantics, the toolchain and syscall behaviour
against the real target kernel. **It proves nothing about the VideoCore boot
chain, `tryboot`, `bcmgenet` or `brcmfmac`** — `ADR-026` is the decision and
the full right-hand-column table of what only a board can settle. A green run
here earns a change the right to be *attempted* on hardware; it is not
evidence that the change works there.

## Run it in the container — the supported way

```sh
tools/container/qemu.sh                 # the gate — two boots, pass or fail
tools/container/qemu.sh --shell         # a shell in the same box, to look around
tools/container/qemu.sh --run -- $T/rivet-initd --supervisor $T/rivet-sysd
```

No port is published and no privilege is asked for — QEMU's `hostfwd` lands on
the container's own loopback, so two runs cannot collide over a host port.

## `smoke.sh` — the yes/no answer

```sh
./smoke.sh                                    # builds everything, both boots
./smoke.sh <init>                             # PID 1 alone
./smoke.sh <init> <supervisor>                # the pair, with NO CLI — the negative case
./smoke.sh <init> <supervisor> <cli>          # all three, BOOT 1 only
./smoke.sh <init> <supervisor> <cli> <mcpd>   # ...and the boot that ASKS
```

The full set is the default rather than a mode: a boot with no supervisor is
the system with its only child amputated, and a boot with no control plane is
a system nothing can reach — so the argument count selects which run happens
and there is no flag to forget.

**Two boots, and the second is the one that asks** (`ADR-026` §4.7). BOOT 1 has
nothing configured and asserts that `rivet-mcpd` refuses to start **by name**,
because that is what a card without it really does. BOOT 2 mints two throwaway
identities per run (never committed — a committed `device.seed` would be a
private key every reader of this repository holds), boots with a real control
plane, and runs `get status`, `get catalogue` and `get file /etc/hosts`
through `cargo rivet run` — the same binary an operator holds — asserting that
a connection pinned to the *wrong* key is refused.

On failure it exits non-zero, names exactly what failed, and dumps the full
console. It also refuses loudly rather than reporting a false pass when it
cannot run at all — no `qemu-system-aarch64` on `PATH`, no kernel image, a
binary that is not static aarch64, or a build failure.

## The artifact store, with real signed artifacts

`tools/qemu/adr-052-proof.sh` is the proof for `ADR-052`'s store: real signed
`.rvt` files transferred over the bulk endpoint into a booted machine's store,
verification at the door, an untrusted-key refusal, a wrong-architecture
refusal, retention, and the already-running-stop case.

```sh
RIVET_APP_SEED=<path to a scratch app seed> tools/container/exec.sh tools/qemu/adr-052-proof.sh
```

`tools/qemu/retention-artifacts.sh <series>` packages four ascending signed
versions for a board retention run — **pass a series the target store has not
seen**, or it tests the re-send path instead of retention.

## Stopping and starting one application

`tools/qemu/adr-017-hold-proof.sh` boots a guest with one installed
application and drives `apply service-stop`, `service-restart` and
`service-start` through `cargo rivet run`, asserting the application stays
stopped past the restart backoff and runs again under a new pid (`ADR-017`
§6.5). Its identities and trust store are minted per run, so it needs no seed:

```sh
tools/container/exec.sh sh tools/qemu/adr-017-hold-proof.sh
```

## Backing up and restoring a board's secrets

`tools/qemu/adr-030-secrets-proof.sh` boots a guest whose `/state/secrets` holds
a throwaway identity, `peers` and one extra secret, backs it up with `cargo
rivet backup-secrets`, damages the guest's store and restores it with `cargo
rivet restore-secrets`, then boots a SECOND guest with another identity and
asserts the first guest's backup is refused there (`ADR-030` §5). It boots
twice. Both commands read a passphrase only from a real terminal (R-185 D1), so
the proof runs each under `script(1)` and types a throwaway passphrase into that
pty through a FIFO once the prompt appears — there is no flag or variable to
reach for instead:

```sh
tools/container/exec.sh sh tools/qemu/adr-030-secrets-proof.sh
```

## The proxy as part of the image

`rivet-proxyd` is an image service (`ADR-060`), so a guest gets it the way a
card does: `run.sh --proxyd PATH` packs it at `/system/bin/rivet-proxyd`, and a
`--document` enabling `rivet-proxyd` runs it — as its own uid, under a Landlock
ruleset, from `/state/app/rivet-proxyd/`. **Do not install it with `--app`**:
the image defines the name, so an application under it is the old shape that
the supervisor retires on boot.

`tools/qemu/adr-060-proxy-proof.sh` boots twice. The first boot is a fresh
machine with the proxy's certificate in `/state/staging/rivet-proxyd.txt` and
no `upstream-name`; it asserts the process runs as the non-zero uid
`/etc/passwd` names for it, the `ENFORCED` Landlock record granting
`/system/bin/rivet-proxyd`, the bind on `0.0.0.0:443`, the staged material
delivered and owned and the staging copy gone, the upstream name taken from the
certificate, and the operator's index served. The second boot holds the proxy
APPLICATION the machine used to run (`--app`, with its data owned by that uid);
it asserts the retirement is journalled once at `info` with no error naming the
proxy, `current` is gone, `data` is kept, and the image's proxy serves the index
from the kept certificate. Identities and the fleet authority are minted per
run:

```sh
tools/container/exec.sh sh tools/qemu/adr-060-proxy-proof.sh
```

## Backing up and restoring an application's data, through the proxy

`tools/qemu/adr-051-restore-proof.sh` boots one guest carrying the control
plane, the image's `rivet-proxyd` on 443 (`--proxyd`, forwarded from the
container's 8443) and `tools/qemu/restore-probe` installed twice — once
declaring `restore-path`, once not. It mints a throwaway fleet authority with
`rivet-provision ca` into its scratch directory, installs the TLS files with
`cargo rivet secrets`, then backs the probe up with `cargo rivet backup`,
changes its data, restores it with `cargo rivet restore`, and asserts a third
backup is byte-identical to the first, the probe's pid did not change, and the
refusals (no `restore-path`, wrong format, a truncated file) send nothing
(`ADR-051` §12):

```sh
tools/container/exec.sh sh tools/qemu/adr-051-restore-proof.sh
```

## Recovering a destroyed machine

`tools/qemu/adr-063-recover-proof.sh` installs `restore-probe` by digest from a
scratch fleet repository, changes its data, takes `cargo rivet backup` and
`cargo rivet backup-secrets` (the passphrase typed into a `script(1)` pty),
then boots again with a fresh `/state` holding only the identity and `peers` a
card carries. `cargo rivet recover --after-boot` puts the secrets back,
installs the probe by digest, refuses to write over its fresh data until
`--replace` names it, and the run asserts the same pin, the same artifact and
a byte-identical backup (`ADR-063` §7). The card step is a person's and is not
run:

```sh
tools/container/exec.sh sh tools/qemu/adr-063-recover-proof.sh
```

## Who reaches an application: the gate, the routed port, and a `:direct` one

`tools/qemu/adr-046-backend-port-proof.sh` boots the same kind of guest with
`restore-probe` once, declaring `tcp:8080` (its routed port, forwarded from
18080) and `tcp:8081:direct` (forwarded from 18081) beside the image proxy's 443. It
mints the operator's `browser` pair and a second machine's front-door pair
(`ca server rivet-02`). It asserts:

* the supervisor's journal says `127.0.0.1:8080,0.0.0.0:8081` for the probe and
  `0.0.0.0:443` for `rivet-proxyd` (`ADR-046` §7.1);
* through the proxy, the operator is served the index, mutates and restores the
  data, and the other machine's certificate and a cross-site restore are refused
  `403` with the data unchanged (§4.1, §4.5);
* on 18080 — QEMU's forward to the guest's non-loopback address — nothing
  completes and the data is unchanged;
* on 18081 the same read is answered `200`.

A routed port answering on 18080 means the supervisor bound it on every address.
Read its `OK` and `FAIL` lines, not only the exit code:

```sh
tools/container/exec.sh sh tools/qemu/adr-046-backend-port-proof.sh
```

## A fleet server's git repository, pushed and cloned back

`tools/qemu/adr-035-git-proof.sh` boots one guest carrying the vendored `git`
and `git-http-backend` (run.sh's `--git` and `--git-http-backend`; the second
is a separate binary, and without it no guest serves git at all) and a
document declaring `[repos.fleet]`. It reads `/state/git/fleet.git` off the
machine, runs `cargo rivet fleet push` as a plan and then with `confirm`,
clones the repository back with the container's stock `git` — which verifies
the guest's certificate and pinned key itself — and compares refs, tree and
files; then a fast-forward push, a fetch with a gzipped request body, an
identity not in `peers` refused by both clients, and uncommitted work and a
non-fast-forward push refused (`ADR-035` §4.1). Identities are minted per run:

```sh
tools/container/exec.sh sh tools/qemu/adr-035-git-proof.sh
```

## A fleet server's objects, synced and pruned

`tools/qemu/adr-052-objects-proof.sh` boots the same kind of guest plus the
installer and a trust store holding one throwaway key, installs the real
`rivet-fleetd` (stripped with the cross `strip` — unstripped it is over the
supervisor's program-size bound — and not enabled) so its data directory
exists, then runs `cargo rivet fleet sync`: a plan, two signed objects sent and
listed back with `get file`, a rerun sending nothing, a tampered object and a
mislabelled system update refused at the door, an identity not in `peers`
refused, and `--prune` removing exactly the object the manifest stopped naming
(`ADR-052` §7.4). The wrong kind is sent by mislabelling it in a scratch
manifest, because `fleet sync` sends only what a manifest calls an application
or a library:

```sh
tools/container/exec.sh sh tools/qemu/adr-052-objects-proof.sh
```

## Gantry's half of a fleet server: the socket, an APK, the phones, port 80

`tools/qemu/adr-061-gantry-proof.sh` boots a guest that is its own fleet
server: the real `rivet-fleetd` (stripped, ENABLED, its certificates and a
one-record register staged as a card stages them), the image's proxy, git with
`[repos.fleet]`, and `tools/qemu/gantry-probe` installed twice — as `gantry`
(granted `rivet-fleetd:gantry`, serving `plain:install` and `app:api` behind
`proxy-path = "gantry"`) and as `stranger` (declaring nothing and dialling the
socket's path directly). The probe speaks `SPEC-06` at every start and writes each reply into
its data directory, where the proof reads it with `get file`; restarting it is
how the proof asks again. The trust store holds the fixture key, a throwaway
key with `apk` in its mask and one without (`ADR-061` §3, §6, §7).

It asserts, one `OK`/`FAIL` line each: fleetd serving the socket to exactly
gantry's uid; `fleet`, `artifacts`, `enrol` of a hand-composed P-256 key and
`phones` answered; `stranger` refused by the socket's mode; `fleet publish app.apk --sync confirm` with
`cmd/cargo-rivet/tests/fixtures/apk/rivet-harness-derived.apk` held in the
store and listed by the probe's next `artifacts`; an `apk` from the key without
`apk` refused at the door; `cargo rivet install` of the held apk refused by the
installer by name; hiding a product (`ADR-052` §7) — the same APK published
again under a second product name, `fleet hide` of the first committing a
`[[hidden]]` entry, then `fleet push … confirm` and `fleet sync … --prune
confirm`, with the sync naming the hidden object `hidden` and not sending it,
the prune removing exactly that object read back from the store, the second
product's object surviving, and the probe's next `artifacts` no longer offering
it; `fleet unhide`, push and sync sending it again, held and offered again;
`fleet retire` of the second product, push and `--prune confirm` removing
exactly its object and `artifacts` no longer offering it; `fleet phones list`,
`rename … confirm` and
`revoke … confirm`, each seen by the probe's next `phones`; the proxy bound on
443 and 80 with the plain route `200`, a `POST` `405`, other paths and `/`
`404`, and `Authorization` forwarded unchanged on an `app:` route; then a
second boot with no plain rule, where the proxy is bound on 443 alone and port
80 answers nothing; then a third boot where the probe is installed again as
`not-gantry`, declaring the same `connects` as `gantry`, and the supervisor
refuses the unshared socket to BOTH, each refusal naming both claimants, and
neither runs (`ADR-015` §5.3). It boots three times:

```sh
tools/container/exec.sh sh tools/qemu/adr-061-gantry-proof.sh
```

**The vendored git in the Buildroot tree is dynamic** (it NEEDs `libz.so.1`),
so this proof stages what it NEEDs and passes run.sh's `--libraries`, and
run.sh accepts a dynamic `--git` only then: a proof that packs a dynamic
`--git` without `--libraries` is refused by name before anything boots.

## What this loop cannot reach — check before spending an hour rediscovering it

* **`tryboot`, the boot partition, and `/proc/device-tree/chosen/bootloader`
  are not emulated at all** (`ADR-026` §4). Any check that reads the default
  slot from the device tree — the uncommitted-trial refusal among them —
  cannot be proven here; `run.sh` refuses `--card` together with `--document`
  and `--secrets` for exactly this reason: the one option that gives a guest a
  real boot partition to read also takes away the control plane that would let
  an `apply install` reach it.
* **PID 1's shutdown path (`SIGTERM`/`SIGINT`, the grace period, `SIGKILL`
  escalation) cannot be triggered.** QEMU cannot signal a guest's PID 1, and
  the board's own kernel image has no PCI host bridge and no `virtio` strings
  at all, so neither a USB nor a virtio keyboard can deliver
  `sendkey ctrl-alt-delete` to it either. This stays unit-test-only until
  either `ADR-024`'s own kernel is booted here with virtio input support, or a
  guest-side way to signal PID 1 exists.
* **Networking beyond SLIRP needs `ADR-024`'s own kernel** — the board's
  `kernel8.img` contains no `virtio_net`, so `--net` against it is a dead end
  for anything past what SLIRP itself provides.
* **A single vCPU makes a CPU cap unmeasurable** — `run.sh` needs `--cli` for
  anything that depends on more than one core being visible.

**"It passed `smoke.sh`" is not "it works", and neither is "it compiles"**
(`AGENTS.md` §10.5). State which of the two you have, plainly, in whatever you
report — the harness is where a change earns the *attempt* at hardware, not
where it earns the claim.

See `tools/qemu/README.md` for the full account — the root/user split if you
run the scripts outside the container, fetching a kernel, networking flags,
and reading a failure's decoded `exitcode=`.

## Running nightly and on demand — `.github/workflows/qemu-proofs.yml`

**Built 2026-09-13 (R38, `.rulings/ R-167`).** Every `tools/qemu/*-proof.sh`
and `smoke.sh` also runs as a separate GitHub Actions workflow on the same
self-hosted runner as `ci.yml` — manual trigger and nightly, never on a push.
One step per proof, so a failed run names exactly which one. `.github/
RUNNER.md` §7 has the runner's own requirements (the `rivet-br` kernel volume,
the open question of where the signing seed comes from) and why the nightly
run disables `exec.sh`'s cache sweep while a manual run does not. `adr-052-
proof.sh`'s step is the only one that can be skipped, and only for want of
`RIVET_APP_SEED` — it prints `skipped: no signing seed available to this
runner` rather than failing or running silently when that variable is absent.
No agent may trigger a GitHub Actions run or touch repository settings
(`AGENTS.md` §10's hard boundary on this project's automation), so what is
verified is only that the commands it runs pass directly, one at a time,
outside GitHub Actions — never a run of the workflow itself.
