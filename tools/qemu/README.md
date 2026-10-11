# `tools/qemu` — the inner development loop

See `docs/ADR-026` for the decision and, more importantly, for **what this loop
can and cannot settle**. The short version: it proves PID 1 semantics, the
toolchain, and syscall behaviour against the real target kernel. It proves
nothing about the VideoCore boot chain, `tryboot`, `bcmgenet` or `brcmfmac`.

A change is *working* when it has run on the board. This earns it the right to
be attempted there.

## Run it in the container

```sh
tools/container/qemu.sh                 # the gate — two boots, pass or fail
tools/container/qemu.sh --shell         # a shell in the same box, to look around
tools/container/qemu.sh --run -- $T/rivet-initd --supervisor $T/rivet-sysd
```

**That is the supported way, and it is the same box `tools/card/build.sh` runs
in** — `tools/container/Dockerfile`, pinned by digest, with
`qemu-system-aarch64` and `cpio` in it since 2026-08-30. It mounts the
repository at `/rivet` and the Buildroot output read-only at `/br`, and points
`RIVET_KERNEL` at `/br/images/Image`.

Two things come out of that mount and only one of them is obvious. The kernel
is: `--net` needs `ADR-024`'s own, because the Raspberry Pi's `kernel8.img`
contains no `virtio_net` at all. The other is the **cross C compiler** —
`rivetos-mcpd` links `rivetos-tls`, which links `ring`, whose build compiles C
(`ADR-009` §3.7.4) — so `smoke.sh` reads `RIVET_BR_OUTPUT` for
`host/bin/aarch64-linux-gcc` and refuses early by name without it.

**The volume needs `images/Image` in it.** `tools/container/build-card.sh`
documents populating `rivet-br` with `host` and two `target` binaries; this is
the third thing, and `qemu.sh` refuses before building anything if it is absent:

```sh
docker run --rm -v rivet-br:/br -v "<dir holding Image>:/in:ro" \
    rivet-card-build:2 sh -c 'mkdir -p /br/images && cp /in/Image /br/images/'
```

**No port is published and no privilege is asked for.** QEMU's `hostfwd` lands
on the container's own loopback and the client runs in the same container, so
nothing crosses the boundary and two runs cannot collide over a host port. `-M
virt` under TCG needs no `/dev/kvm`.

## Requirements, if you run the scripts directly

A Linux host with `qemu-system-aarch64`, `cpio` and `gzip`. The container is
the answer to this section rather than a wrapper around it — a build nobody
else can reproduce is a build nobody can review (`ADR-026` §6) — but the
scripts take no position on where they run.

### The root/user split, which will bite you once

Booting needs **root** (the initramfs needs a `/dev/console` device node), or
`fakeroot`. Building needs **your user**, because `rustup` installs targets
per-user — as root, `cargo build --target aarch64-unknown-linux-musl` fails with
`can't find crate for 'core'`, which reads like a broken toolchain and is not.

`smoke.sh` builds *and* boots, so run as root it fails at the build step. Build
first, then hand it the binaries:

```sh
cargo build --release --target aarch64-unknown-linux-musl \
    -p rivetos-initd -p rivetos-sysd -p rivetos-cli -p rivetos-mcpd  # as your user
T=../../target/aarch64-unknown-linux-musl/release
sudo ./smoke.sh $T/rivet-initd $T/rivet-sysd $T/rivet $T/rivet-mcpd
```

**In the container neither half of this applies**: it runs as root, so the
device node is made without `fakeroot`, and `rustup`'s target is in the image
rather than in a home directory.

`smoke.sh` with no argument is for the case where your user can also boot —
`fakeroot` present, or a host that lets you make device nodes.

```
qemu-system-aarch64      # Arch: pacman -S qemu-system-aarch64
cpio gzip                # to build the initramfs
coreutils                # `timeout`, for smoke.sh's bounded run
file                     # both scripts refuse a binary they cannot identify
rustup target add aarch64-unknown-linux-musl
```

Cross-linking needs no C toolchain, only this in the workspace's
`.cargo/config.toml` (`ADR-026` §3.1):

```toml
[target.aarch64-unknown-linux-musl]
linker = "rust-lld"
```

## The kernel is an input, not an artefact

`kernel8.img` is **not committed** — it is somebody else's binary and it is
large. Fetch it from a board running the target kernel:

```sh
./fetch-kernel.sh user@10.0.0.2
```

That writes `kernel8.img` and `bcm2711-rpi-4-b.dtb` into this directory, both
`.gitignore`d. Once `ADR-024`'s custom kernel exists, point the runner at that
instead — and then this loop becomes the cheapest place to discover whether the
kernel configuration is right, before anything is flashed.

**It exists, since 2026-08-23**, and the last sentence has already paid for
itself: the build's configuration check found four `ADR-024` §3 requirements
silently demoted from `=y` to `=m`, which on a card would have been a wireless
driver that is present, unloadable, and reported by nothing.

```sh
tools/buildroot/build.sh --fetch linux
RIVET_KERNEL=<that output>/images/Image ./run.sh <init> --net
```

**Both kernels stay.** They answer different questions and neither is strictly
better — Rivet's has Landlock, the memory cgroup controller and virtio; the
board's is the version the hardware actually runs. `ADR-026` §4.1 is the table,
and it is worth reading before quoting a result from either.

## Running

```sh
T=../../target/aarch64-unknown-linux-musl/release

./run.sh $T/rivet-initd                             # PID 1 alone
./run.sh $T/rivet-initd --supervisor $T/rivet-sysd  # PID 1 and its child
./run.sh $T/rivet-initd --supervisor $T/rivet-sysd \
         --cli $T/rivet --netd $T/rivet-netd      # what a card carries
```

The first form packs one binary as `/init`. Each option **also** packs one more,
**at the path the crate that looks for it compiled in** — never a path written
in `run.sh`:

| Option | Destination | Read out of |
| :-- | :-- | :-- |
| `--supervisor` | `/system/bin/rivet-sysd` | `SUPERVISOR_PATH`, `cmd/rivet-initd/src/supervise.rs` — PID 1 execs it |
| `--cli` | `/system/bin/rivet` | `LOGIN_SHELL_PATH`, `rivet-core/src/etc.rs` — the `/etc` projection probes it |
| `--netd` | `/system/bin/rivet-netd` | `NETD_PATH`, `cmd/rivet-image/src/system.rs` — the image installs it there |
| `--mcpd` | `/system/bin/rivet-mcpd` | `MCPD_PATH`, `cmd/rivet-image/src/system.rs` — **and cross-checked against `cmd/rivet-sysd`'s copy**, because that is the definition that starts it |
| `--secrets` | `/state/secrets` | `MCPD_IDENTITY_DIR` and `MCPD_PEERS_PATH`, `cmd/rivet-sysd/src/definitions.rs` — the argument vector the supervisor hands the daemon |

All three are `/system/bin` per `ADR-008` §3 and `ADR-016` §3, and none of them
is written down here: a ruling that moves a path moves this harness with it,
instead of leaving a runner that packs a file where nothing looks for it.
(`ADR-002` §4's `cmdline.txt` still says `/sbin/rivet-initd`; that conflict is
already flagged in `supervise.rs` and is image assembly's to settle.)

`--cli`'s constant is `rivet-core`'s and not `cmd/rivet`'s, because the
destination that matters is the one the **probe** uses — and since that fact has
a second copy in `cmd/rivet/src/invocation.rs` which nothing pins against the
first, `run.sh` reads both and **refuses when they disagree**:

```
run.sh: the two copies of the CLI's install path disagree:
    rivet_core::etc::LOGIN_SHELL_PATH  = /system/bin/rivet-typo
    cmd/rivet's invocation::INSTALL_PATH = /system/bin/rivet
    Refusing to pack at one of them. Whichever is stale, a run
    made now would report a login shell found at a path the CLI
    does not answer to.
```

Exit QEMU with `Ctrl-A` then `x`.

### Networking — `--net`, `--net-pci`, `--forward-port`

```sh
RIVET_KERNEL=<buildroot output>/images/Image \
  ./run.sh $T/rivet-initd --supervisor $T/rivet-sysd \
           --net --forward-port 7777:7777
```

`--net` attaches a virtio NIC on virtio-mmio behind QEMU's user-mode stack; the
guest gets `10.0.2.15/24` with a gateway and DHCP server at `10.0.2.2`, and
needs no root, no bridge and no host configuration. `--net-pci` puts the same
NIC on the generic PCI host bridge instead. `--forward-port H:G` forwards a host
port to a guest port, implies `--net`, and may be given more than once — **SLIRP
has no route into the guest without one**, so this is how the control plane is
reached.

**Two transports, because the kernel has to have a driver for whichever one is
used**, and a kernel with one and not the other produces a QEMU that starts
cleanly and a guest with no interface. That silence is what makes this
expensive, so `--net` reads the kernel image first and **refuses** when it can
prove there is no `virtio_net` in it — naming the kernel as the cause instead of
letting the next hour go to Rivet's netlink code. It distinguishes three
answers, not two: found, absent, and *could not tell*, and only refuses on the
middle one (`AGENTS.md` §7 — an error and a true negative must not be
confused).

`ADR-026` §4.1 is where the trade is written down. The short form: this is
virtio, not `bcmgenet`; it tests Rivet's Stage 1 code and settles nothing
about whether the board's NIC comes up.

### Why the supervisor is optional

Without it, PID 1 finds nothing to spawn and runs its **failure** path: ten
retries with doubling backoff, then `GIVING UP`, then the boot confirmation
withheld for ever. That is correct behaviour, it is worth being able to
reproduce, and for a long time it was the only thing this loop could show. So
the one-binary invocation is unchanged and still supported.

With it, PID 1's **success** path runs instead — the child starts, stays up, and
the pair share one console:

```
rivet-initd: started /system/bin/rivet-sysd as pid 55
rivet-initd: reaped pid 53 (exit 7) — direct child
{"ts":"1970-01-01T00:00:01.370Z","boot":"568449a2-…","sev":"info",
 "svc":"rivet-sysd","msg":"supervisor starting","services":0}
...
rivet-initd: boot could NOT be confirmed: cannot read
              /proc/device-tree/chosen/bootloader/tryboot: No such file …
```

That last line is the *whole* difference in the confirmation path, and it is
worth reading carefully. `boot NOT confirmed:` is the **gate** refusing — the
mount phase, the provisioning document or the supervisor was unacceptable, so
PID 1 never looked at the flag. `boot could NOT be confirmed:` means the gate
**passed**, the supervisor had been up for `CONFIRM_SETTLE_MS`, and PID 1 went
looking for `tryboot` and did not find it. Under QEMU it never will:
`ADR-026` §4 puts `tryboot` and `/proc/device-tree/chosen/bootloader` in the
board's column — "not emulated at all". Reaching the read is what this loop can
settle. What the read *returns* is the board's.

Nothing else is created in the initramfs for the supervisor. `/run` is a tmpfs
PID 1 mounts, and the `sd_notify` sockets under `/run/rivet/notify` are made
by the supervisor's own `open_notify()` with `create_dir_all`. Pre-creating
either here would be this harness making a claim about the image's shape that
image assembly has to make. (That sentence used to end "image assembly — which
does not exist yet"; `cmd/rivet-image` exists, so it is corrected here rather
than left standing, 2026-08-23.)

### Why the CLI is optional too — and what packing it settles

The runtime `/etc` projection has exactly one input it reads off the machine
rather than out of a constant: it asks the kernel whether anything is at
`rivet_core::etc::LOGIN_SHELL_PATH`, and writes an `/etc/shells` naming that
path **only if the answer is yes**. Measured on real
hardware, that is why it matters: an account whose shell is not listed is rejected
*before* anything will look at a key.

Until `--cli` existed that probe had one reachable answer here, so every run
reported

```
"outcome":"Rendered { files: 5, read_only: true, names_a_login_shell: false }","permits_a_login":false
```

which was correct, and meant the loop could not see the interaction between the
image installing a CLI and the projection finding it. With `--cli` it reports

```
"outcome":"Rendered { files: 5, read_only: true, names_a_login_shell: true }","permits_a_login":true
```

and `/etc/shells` grows from 488 bytes of "this machine has no login shell" to
231 bytes naming one, with `/etc/passwd`'s shell field following it.

**Both are kept, because both are states a card can be in.** An image built
without a CLI, or one whose install path moved, produces the first — so `--cli`
is optional in exactly the way `--supervisor` is, and `smoke.sh` asserts
whichever one the run selected.

**What packing the CLI settles, and what it does not.** Until 2026-08-30
nothing exec'd it: PID 1 spawned the supervisor and nothing else. With `--mcpd`
something *does* exec the CLI: `rivet-mcpd` hands each caller the stdin and stdout of
`/system/bin/rivet mcp`, so BOOT 2 runs it and asserts what it answered.

**What is still true**, narrowed: BOOT 1 execs nothing but the supervisor and
the services it starts, and the `/etc` assertions there are about the projection
probing a path and finding what is at it — not about anything running.
`AGENTS.md` §6: a binary that is present is not a binary that works, and the
boot that settles the second is the one with the control plane in it.

`--netd` settles less again, and deliberately. It puts `rivet-netd` where
`cmd/rivet-image` puts it on a card, matching what a card carries; the
supervisor's `system_services()` has started it unconditionally since
2026-08-23. `smoke.sh` asserts nothing about its console output; it is here so
that a run of the loop carries the same binary a card does.

### Watching the supervisor actually supervise something

`stage0_services()` is empty by design (`ADR-017` §4 has not said what a service
definition contains), so a supervisor PID 1 spawns supervises nothing and says
so. The `--dev-service` harness flag exists to exercise the machinery by hand —
and **PID 1 cannot pass it on**: `Kernel::spawn` in `cmd/rivet-initd` takes a
path and no arguments at all, so a normally-spawned supervisor always gets an
empty argv. `run.sh` refuses `--supervisor-args` on its own rather than
accepting a flag that would silently do nothing.

To use those flags the supervisor has to be `rdinit` itself:

```sh
./run.sh $T/rivet-initd --supervisor $T/rivet-sysd \
    --supervisor-as-init \
    --supervisor-args "--text --dev-service up=/system/bin/rivet-sysd \
                       --dev-service ghost=/system/bin/not-installed --dev-notify up"
```

`run.sh` prints a banner in this mode, because the console it produces is **not**
the pid1-spawns-supervisor path and must never be read as though it were. The
supervisor is pid 1 here, so nothing mounts `/proc`, `/sys` or `/run`, and its
degraded branches (`boot="unknown"`, cgroups unreadable) are part of what you
are watching. What it does show, on the real kernel:

```
[  1.388] info  up: started pid=53 program="/system/bin/rivet-sysd" attempt=1
[  1.394] error ghost: could not be started detail="No such file or directory …"
[  1.395] info  ghost: will retry after backoff attempt=1 delay_ms=250
[  5.128] error ghost: giving up; the service is marked failed and will not be restarted
[ 31.393] warn  up: running, but has not declared readiness; it is reported as
                running and not as ready after_ms=30022
```

### This guest can have a Bluetooth controller — `--vhci`

Added 2026-09-05, `ADR-041` §9. `CONFIG_BT_HCIVHCI` is in
`tools/buildroot/linux/qemu-virt.config` and `drivers/bluetooth/hci_vhci.c`
registers an HCI controller when a process opens `/dev/vhci` and writes
`HCI_VENDOR_PKT` — so **a process can be the controller**, and
`tools/qemu/vhci-fixture` is one.

```sh
./run.sh $T/rivet-initd --supervisor $T/rivet-sysd \
    --bluetoothd $T/rivet-bluetoothd \
    --vhci $T/rivet-vhci-fixture --vhci-plan count=3000,gap_us=25000 \
    --document <a document with [services.rivet-bluetoothd] enabled = true>
```

**It is the one binary here that the KERNEL execs.** `rdinit=` is the fixture;
it starts one child of itself and then execs the real PID 1, which is therefore
still PID 1 and still reached with the argv it would have had. There was nowhere
else to put it: PID 1 spawns the supervisor and nothing else, the supervisor's
service list is product code, and an application runs under a uid that cannot
open a `crw------- root root` node.

Two things follow that a reader should not have to discover:

* **`--bluetoothd` packs the daemon and does not start it.** `ADR-041` §6.5
  ships the service disabled, so a `--document` enabling it is what makes it
  run. Without `--vhci` it finds no adapter and waits for one, for ever, which
  is correct (§6.4) and is a state worth being able to boot.
* **The default plan injects nothing.** `--vhci-plan` says what the controller
  does: `count=`, `gap_us=`, `delay_ms=`, `wait_s=`. It travels on the kernel
  command line, so it must not contain spaces, and `run.sh` refuses one that
  does rather than letting the guest come up without a PID 1.

`tools/qemu/adr-041-proof.sh` is the pass/fail form of it, on three boots. It
settles nothing about a radio: every byte it delivers was composed by the
fixture.

### The artifact store, with real signed artifacts — `adr-052-proof.sh`

```sh
RIVET_APP_SEED=$HOME/.rivet/keys/rivet-app.seed \
    tools/container/exec.sh tools/qemu/adr-052-proof.sh
```

**`run.sh --deploy-request` packs a PLACEHOLDER object** — a line of text whose
contents are not the digest that names it — and that is the right shape for the
question that flag asks, which is what the *supervisor* does with a request. It
is not evidence about the installer, and until 2026-09-12 nothing was: the
commits that built `ADR-052` §7.2's store had never verified a signature out of
one.

This script is the other half, and it uses no placeholder anywhere. It packages
six real `.rvt` files with `cargo rivet package`, boots a machine with
`cmd/rivet-deploy` packed and a trust store carrying `keys/rivet-app.pub`, and
lets `cargo rivet deploy` put them in the store the way a machine really gets
them — over `ADR-035` §3.2's bulk endpoint, keyed by the digest that endpoint
computed. It asserts that one is verified and installed and the supervisor
starts it, that one signed by an untrusted key is stored and **refused**, and
that retention leaves the three newest of a product with the newest arrival
among them.

**`/state` is the initramfs tmpfs here**, because `--card` gives a guest real
ext4 and refuses `--document` and `--secrets` with it — so a machine with a card
cannot receive a transfer, and no single run can have both. The syscalls are
real; the medium is not.

## Getting a yes/no answer: `smoke.sh`

`run.sh` is for a human watching the console. `smoke.sh` is the one a human
or CI runs to get a pass/fail without watching anything:

```sh
tools/container/qemu.sh                       # the whole thing, in the box
./smoke.sh                                    # builds everything, makes BOTH boots
./smoke.sh <init>                             # skips the build; PID 1 alone
./smoke.sh <init> <supervisor>                # that pair, with NO CLI — the negative
./smoke.sh <init> <supervisor> <cli>          # all three, BOOT 1 only
./smoke.sh <init> <supervisor> <cli> <mcpd>   # ...and the boot that ASKS
```

**The full set is the default rather than a mode**, because a boot with no
supervisor is not the system — it is the system with its only child amputated,
and everything it can demonstrate is a failure path — and a boot with no control
plane is a system nothing can reach. The argument count selects which run
happens, so there is no flag to forget.

**Two boots, and the second one is the one that asks** (`ADR-026` §4.7). BOOT 1
is the machine with nothing configured: no network, no document, no control
plane, and the assertion that `rivet-mcpd`'s spawn must **refuse by name**,
because that is what a card without it really does. BOOT 2 mints two throwaway
identities with `cargo rivet identity new`, places one at `/state/secrets` and
pins the other, boots with a `[control-plane] listen` of `0.0.0.0:7777` and that
port forwarded, and runs `get status`, `get catalogue` and `get file /etc/hosts`
through **`cargo rivet run`** — the binary an operator holds. A connection
pinned to the *wrong* key is asserted to be refused, because otherwise none of
the three answers would mean anything. `apply power-off confirm` is the guest's
last act.

It asserts, for every run:

* the kernel reaches `Run /init as init process`;
* the console contains `rivet-initd: starting as pid 1`;
* the console does **not** contain `Kernel panic`;
* QEMU was still running — not exited — when the timeout stopped it, i.e.
  PID 1 stayed parked rather than returning.

and, when a supervisor was packed, also:

* PID 1 reported `started <SUPERVISOR_PATH> as pid N`, and never `cannot spawn`
  it;
* the supervisor announced itself on the same console;
* the supervisor never exited, so PID 1 never retried, backed off or gave up;
* PID 1 reached a boot-confirmation **verdict** rather than withholding one —
  see the two spellings above. It does not assert `Confirmed`, because
  `Confirmed` is not reachable under an emulator and asserting it would be
  asserting a lie.

and, in **whichever direction the login path went** (added 2026-08-23):

* with a CLI packed: the projection reported `/etc/shells names a login shell`,
  the `"shell"` field is the path this harness packed to, and the run reported
  `"permits_a_login":true`;
* with none packed: it reported `/etc/shells names NO login shell`, the
  `"would_be"` field is that same path, and the run reported
  `"permits_a_login":false`;
* either way, the probe **answered**. A `could not tell whether a login shell is
  present` is a third outcome and fails both directions — without that check, an
  `EIO` from `file_exists` would satisfy the negative run for entirely the wrong
  reason.

and, in **whichever direction the control plane went** (added 2026-08-30,
`ADR-035` phase 1, `ADR-026` §4.7):

* **BOOT 1 packs none**, so `cmd/rivet-sysd`'s `rivet-mcpd` definition — which
  is `enabled: true` — must try to start it and refuse permanently, naming what
  the machine has lost (`ADR-017` §3.1.6). That value said `false` for three
  whole phases while every board ran a document that turned it on, so **a card
  built from the defaults booted with no control plane at all** and nothing
  could see it (`PLAN.md` §94.7). Flipping that one line back fails this run in
  seconds;
* **BOOT 2 is the other direction.** The document's `[control-plane] listen` is
  `0.0.0.0`, not the built-in `127.0.0.1:7777`, because SLIRP cannot route to a
  loopback listener inside the guest — and it deliberately does **not** name the
  service, so what is asserted is the image default. The daemon must report the
  address it bound *and* that it came from the document (a flag that recorded
  whether `--document` was *passed* rather than whether it *said* anything is
  `PLAN.md` §82's defect), and it must report `MUTUAL TLS` — without which the
  whole exchange could have run over the plaintext loopback transport and every
  assertion would be identical;
* the identities are minted **per run** and never committed. There is no key
  fixture in this repository and there cannot be (`AGENTS.md` §8.3): a committed
  `device.seed` is a private key every reader of this repository holds, so any
  of them could impersonate any machine that pinned it.

Both directions are asserted because an assertion that can only ever pass is not
a test (`AGENTS.md` §4). Each direction's greps were run against the *other*
direction's console and every one of them gave the opposite answer, and moving
the packed path by one character was watched failing:

```
smoke.sh: FAIL: a CLI was packed at /system/bin/rivet-typo and the projection
did not report finding a login shell. The probe is a file_exists on that exact
path (cmd/rivet-sysd's etc.rs), so either run.sh packed it somewhere
else or the binary is not there.
```

The timeout (`RIVET_SMOKE_TIMEOUT`) defaults to 10s for a lone PID 1 and to
`CONFIRM_SETTLE_MS + 10s` for a pair — read out of `supervise.rs`, because a
pair bounded to ten seconds would never reach the confirmation and the most
interesting assertion would quietly never run.

On failure it exits non-zero and names exactly what failed, then dumps the
full captured console for a human to read. On the panic case specifically it
decodes the `exitcode=` field named in `docs/ADR-026` §1: `0x0000000b` is
SIGSEGV, `0x00000000` means `main()` simply returned — both fatal, and they
need different fixes.

It also refuses loudly, rather than reporting a false pass, when it cannot
run at all: no `qemu-system-aarch64` on `PATH`, no kernel image, no binary,
a binary that is not static aarch64, or the build itself fails. A test that
reports success without running is worse than no test (`AGENTS.md` §3).

**A green `smoke.sh` run earns the right to try hardware. It settles nothing
in `docs/ADR-026` §4's right-hand column** — the VideoCore boot chain,
`tryboot`, `bcmgenet`, `brcmfmac`, boot time, thermal behaviour. Only the
board settles those. "It passed `smoke.sh`" is not "it works."

## What this loop still cannot reach

Recorded so the next session does not spend the same hour rediscovering it.

**PID 1's shutdown path — `SIGTERM`/`SIGINT`, the 5s grace and the `SIGKILL`
escalation — cannot be triggered here.** QEMU cannot signal a guest's pid 1,
and the usual way round that is the monitor's `sendkey ctrl-alt-delete`: the
kernel turns Ctrl-Alt-Del into `SIGINT` to pid 1, which `rivet-initd` handles
as reboot. It does not work with **this** kernel, and the reason is a property
of the image rather than of QEMU:

* the monitor accepts the key — `-monitor pipe:PREFIX` (make `PREFIX.in` and
  `PREFIX.out` with `mkfifo` first; QEMU opens existing FIFOs and will not
  create them) plus `echo sendkey ctrl-alt-delete > PREFIX.in` is reported as
  successful;
* but there is no keyboard for it to arrive on. `-device qemu-xhci -device
  usb-kbd` is PCI, and the board's kernel image contains no generic PCI host
  bridge driver, so the controller is never probed. `-device
  virtio-keyboard-device` is virtio-mmio, and `strings` over the decompressed
  `kernel8.img` finds **no `virtio` strings at all**. `usbhid` is built in;
  nothing it could bind to is.

Two consequences worth carrying forward. The shutdown path stays unit-test-only
until either `ADR-024`'s own kernel is booted here with `CONFIG_VIRTIO_MMIO` +
`CONFIG_VIRTIO_INPUT` (or `CONFIG_PCI_HOST_GENERIC`), or a guest-side way to
signal pid 1 exists. And `ADR-026` §7's open item 2 — virtio-net for Stage 1 —
runs into the same wall: **the board's kernel image cannot drive virtio
either**, so networking in this loop needs Rivet's own kernel first.

**Both halves of that paragraph were resolved on 2026-08-23, and only one of
them by being fixed.** `tools/buildroot/` now builds Rivet's own kernel, with
all three of the symbols named above. Everything above is still true of
`kernel8.img` and is left standing for that reason — the wall is real, it is a
property of the board's image, and the loop still runs that image by default.

* **Networking works.** `./run.sh --net` (virtio-mmio) or `--net-pci`, against
  `RIVET_KERNEL=<buildroot output>/images/Image`. Read from inside the guest:
  `eth0 -> /sys/bus/virtio/drivers/virtio_net`, and `udhcpc` takes a lease from
  QEMU's user-net gateway. `ADR-026` §4.1 records what that does and does not
  settle, and `--net` refuses outright on a kernel it can prove has no driver,
  rather than producing a normal-looking console with no interface in it.
* **The shutdown path is still unreached.** `CONFIG_VIRTIO_INPUT=y` is in the
  new kernel, so the keyboard half of the obstacle is gone — but *that a
  keyboard now exists* has not been confirmed by sending a key and watching PID
  1 receive `SIGINT`. Nothing here has tested it, so nothing here claims it.

**What execs the CLI is `rivet-mcpd`.** Until 2026-08-23 nothing in a guest ran
any binary except PID 1 and the supervisor: `--cli` and `--netd` put files in
the initramfs and no process ever exec'd either, so whether the CLI *behaves* as
a login shell — `argv[0]`'s leading dash, `-c`, silence on stdout — was settled only by `cmd/rivet`'s own unit tests, which run on any
host.

`ADR-035` closed that gap from the other side, and the answer is better than
the one that was reached for first: **no shell, no account, no key**, and the
caller controls one tool call rather than one command line.

**What is still true**, narrowed: nothing here runs the CLI in an *interactive*
session, and nothing will — `ADR-005` §5.1 rules there is no interactive
prompt, so `argv[0]`'s leading dash is settled only by `cmd/rivet`'s own unit
tests. `--netd` is still never exec'd by `run.sh` alone, but the supervisor
starts it (`ADR-017` §4.3), and BOOT 2 is the run that needs it to.

**PID 1's healthy-run reset** (`HEALTHY_RUN_MS`: a run of ≥ 60s clearing an
earlier failure count) is also not reachable. It needs a supervisor that fails
several times and *then* succeeds, and this harness cannot produce one without
building a purpose-made binary that lies about being the supervisor.

## Reading the result

A working boot ends with the kernel line `Run /init as init process` followed by
whatever the binary prints. The failure you will see most is:

```
Kernel panic - not syncing: Attempted to kill init!
```

which means PID 1 returned or died. `exitcode=0x0000000b` is SIGSEGV;
`exitcode=0x00000000` means it simply returned, which is equally fatal — PID 1
must never return (`ADR-017` §3.1.5).

If you see no output at all, the usual cause is a missing `/dev/console` device
node in the initramfs, not a hang. `run.sh` creates it, which is why it needs
`fakeroot` or root.

If a run ends much earlier than its timeout for no visible reason, check what is
on **stdin**. `-nographic` puts the guest console on stdio, so QEMU reads it —
a script piped into `sh` will be eaten from under you. `smoke.sh` passes
`</dev/null` for exactly this; do the same when driving `run.sh` from a script.
