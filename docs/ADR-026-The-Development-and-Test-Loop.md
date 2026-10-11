# ADR-026: The Development and Test Loop

* **Status:** Accepted — ruled 2026-08-22, and **verified by running it** the
  same day. §4's table is the load-bearing part: it says what this loop can and
  cannot settle. **Extended 2026-08-23: the loop now has two kernels and can
  test networking. §4.1 says what the second one costs — read it with §4.**
  **Extended again 2026-08-23: it can pack the CLI and `rivet-netd`, so the
  `/etc` projection's login-shell probe has two reachable answers here instead
  of one. §4.2 says which link of the login chain that is, and which links are
  still nobody's but the board's.**
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`ADR-002` §4 describes a board whose debug channel is an HDMI console
photographed with a phone, upgraded later to a serial UART. `PLAN.md` §2.1's
Stage 0 exit criterion is a Rust PID 1 that boots, mounts, reaps and does not
exit. Between those two facts sits a question the record never asked: **how is
PID 1 iterated on?**

A PID 1 that is wrong does not produce a stack trace and a prompt. It produces
`Kernel panic - not syncing: Attempted to kill init!` and a board that must be
powered off, have its card removed, rewritten on another machine, and
reinserted. At perhaps three minutes per attempt that is not a development loop,
it is a punishment, and it makes exactly the wrong tradeoff: it discourages the
small experiments that find the problem.

**The maintainer ruled 2026-08-22** that development iterates against an
emulator, and that what reaches hardware should already have a reasonable claim
to work.

---

## 2. Decision: QEMU is the inner loop, the board is the outer loop

**The inner loop is `qemu-system-aarch64 -M virt` booting the *real* Raspberry
Pi kernel with a Rust PID 1 in an initramfs.** It takes about a second.

**The outer loop is the board**, and it is where anything in §4's right-hand
column is decided. Nothing in this ADR reduces the board's authority; it reduces
how often the board is needed.

### 2.1 The kernel is taken from the board, not from a distribution

`kernel8.img` and `bcm2711-rpi-4-b.dtb` are copied off the target's boot
partition. The loop therefore runs **the exact kernel the hardware runs** —
`6.18.34+rpt-rpi-v8` at the time of writing — rather than a generic arm64
kernel that happens to be nearby.

This matters more than it sounds. Most of what Stage 0 does is a conversation
with a specific kernel: which filesystems it will mount, what `/proc` and `/sys`
contain, how it delivers `SIGCHLD`, what it does when init returns. A different
kernel answers some of those questions differently, and the loop would then be
testing the wrong thing while looking identical.

**When Rivet builds its own kernel** (`ADR-024` §8.1, now scheduled), that
kernel becomes the loop's kernel, and the loop becomes the cheapest place to
find out whether the configuration is right — before it is ever written to a
card.

**Built 2026-08-23 — and it does not replace the board's kernel, it joins it.**
`tools/buildroot/` produces a kernel Rivet configures, and it boots here. The
sentence above assumed the two would be the same kernel, and they are not yet:
Rivet's is `6.12.61-v8`, from the Raspberry Pi tree at the revision Buildroot
pins, while the board runs `6.18.34+rpt-rpi-v8`. **That difference is exactly
what this section warns about**, so the loop now has two kernels and §4.1 says
which question each one is for.

---

## 3. The configuration that works, verified 2026-08-22

```
qemu-system-aarch64 -M virt -cpu cortex-a72 -m 2G \
  -kernel kernel8.img \
  -initrd initramfs.cpio.gz \
  -append "earlycon console=ttyAMA0 rdinit=/init" \
  -nographic -no-reboot
```

Observed: the kernel reaches `Run /init as init process` at 1.30 s, and the
Rust binary reports `running as pid 1`, `uid = 0`, and stays alive.

Three details are not obvious and each cost an iteration to find:

* **`-cpu cortex-a72`** matches the Pi 4's core. Not required to boot, but it
  keeps the emulated instruction set honest about the target — and it is the
  core `ADR-020` §6.2 just confirmed has no crypto extensions.
* **`/dev/console` must exist as a device node inside the initramfs**
  (`mknod dev/console c 5 1`), or init runs with no stdio and the loop appears
  to hang while actually working perfectly.
* **`rdinit=/init`, not `init=`.** `init=` names a path in the root filesystem;
  `rdinit=` names one in the initramfs, which is the whole point of a loop that
  needs no disk image, no partition table and no bootloader.

### 3.1 Cross-linking needs `rust-lld`, and this is not emulator-specific

Building `aarch64-unknown-linux-musl` from an x86-64 host fails at the link
step, because Cargo's default linker is the host `cc`:

```
/usr/bin/ld: crt1.o: Relocations in generic ELF (EM: 183)
/usr/bin/ld: crt1.o: error adding symbols: file in wrong format
```

The fix is a two-line `.cargo/config.toml`:

```toml
[target.aarch64-unknown-linux-musl]
linker = "rust-lld"
```

`rust-lld` ships with the toolchain, so this needs **no cross binutils and no
cross GCC** — which is a genuinely useful property for `ADR-003`'s static-musl
posture: Rivet's *own* binaries need no C cross-toolchain at all. Buildroot is
still required, but only for the C mechanisms (`ADR-009` §6), not for Rivet.

This belongs in the workspace from the first commit, not in a runner script.

---

## 4. What this loop can and cannot settle

**This table is the reason the ADR exists.** `AGENTS.md` §6 says automated
checks prove a thing is configured, not that it functions — and an emulator is
the most seductive version of that error, because it looks exactly like the
real thing right up until it doesn't.

| QEMU settles it | Only the board settles it |
| :-- | :-- |
| PID 1 semantics — mounting, `SIGCHLD`, reaping, never returning | The VideoCore boot chain: `config.txt`, `start4.elf`, firmware-provided device tree |
| The `aarch64-unknown-linux-musl` toolchain and static linking | `tryboot` and `/proc/device-tree/chosen/bootloader` (`ADR-018` §5.3) — not emulated at all |
| Syscall behaviour against the real target kernel | `bcmgenet` / `eth0` (`ADR-004` §2.3) — QEMU's `raspi4b` explicitly *disables* genet |
| Whether a kernel configuration boots and what it exposes — including Rivet's own, before it is flashed | `brcmfmac` and the wireless firmware (`ADR-004` §8.1). There is no emulation of a CYW43455 |
| `/proc` and `/sys` contents, cgroup controllers, `/sys/kernel/security/lsm`. **And, since §4.6 on 2026-08-28, `ADR-018` §4's partition layout**: `--card` attaches an image `cmd/rivet-image` wrote, the kernel parses its GPT, `/state` mounts as `ext4` with the ruled flags, and what is written there survives a boot | SD/MMC timing and card wear — and, until §4.6 on 2026-08-28, anything about `ADR-018`'s partition layout on real media. The first two are the medium's and a file behind virtio-blk touches neither |
| Landlock, seccomp and namespace behaviour once the custom kernel exists | Boot *time* — `PLAN.md` §4.25's measurement is meaningless under emulation |
| The configuration document being read, and every parse error in it | Thermal behaviour, throttling, and the A72's actual performance |
| **Added 2026-08-23, §4.2:** the `/etc` projection's login-shell probe, in both directions | `getusershell()` against **the board's own musl**, which is a different build from the one this loop links. This cell said *whether anything can actually log in* until 2026-08-27: §4.3 moved that to the left column on 2026-08-23 and the cell was never updated, so it claimed a limit the harness did not have for four days |

| **Added 2026-08-26, §4.4; widened 2026-08-27, §4.5.** It once showed only that an interlock which identifies pid 1 **fails closed**; since §4.5 it shows **that the interlock RECOGNISES this machine**: `apply reload` is delivered here and the supervisor reports re-reading the document, and **`apply power-off` actually stops the guest** | This cell held *anything an operator asks pid 1 or the supervisor to DO*, on the grounds that `run.sh` packs pid 1 as the initramfs `/init` so `/proc/1/comm` reads `init` and not `rivet-initd`. **That was a fact about `run.sh`, not about emulation — §4.5.** `/init` was never required; `rdinit=` already told the kernel what to run |

| **Added 2026-08-30, §4.7:** **whether a build ANSWERS** — the image default starting `rivet-mcpd`, a `[control-plane]` address being read and bound, mutual TLS over pinned Ed25519 keys, an MCP `tools/call` composed from the catalogue and performed, and a wrong pin being refused | Everything about the board's own reachability: `bcmgenet`, the address a real network hands it, and whether the identity `cmd/rivet-image` wrote onto p5 is the one the daemon reads. This loop places the identity into an initramfs, which is not enrollment |

**The rule that follows:** a change may be called *working* only when it has run
on the board. QEMU earns the right to *attempt* it. That is `AGENTS.md` §6
applied to an emulator, and the phrase to avoid is "it works in QEMU" used as
though it settled anything in the right-hand column.

## 4.4 The row added 2026-08-26, and how long it had been true

**Found by an assertion that expected to pass.** `tools/qemu/smoke.sh` was given
a check that `rivet apply reload confirm` reaches the supervisor — the sending
half of `ADR-032` §3.2, whose receiving half had landed the same day. It
refused, with `pid 1 is `init`, not `rivet-initd``.

**The interlock is right and the emulator is the odd one.** On a board pid 1 is
`/system/bin/rivet-initd` and `/proc/1/comm` reads `rivet-initd`; `run.sh` packs
the same binary as the initramfs `/init`, which is what the kernel requires, and
`comm` is the file name. `crate::shutdown`'s own documentation already states the
constraint — *"thirteen characters, which matters"* — and simply never met a
machine where the name came out differently.

**It has been true of the three stop subjects since they were built on
2026-08-23, and nobody had noticed**, because nothing in this harness had ever
asserted one: a smoke test that powered the guest off would have ended the run it
was in the middle of. So `apply power-off`, `apply restart` and
`apply trial-restart` have never been exercised under emulation, and their
board evidence (`ADR-018` §5.3.3's four stops on a Pi 4) is the only evidence
there is. That is not a defect and it is not a gap that wants closing here — it
is this section's whole point, arriving in a new place.

**What the harness asserts instead** is the half it can: the interlock **fails
closed** and its refusal **names the pid 1 it found**. That is what protects a
development host from a command meant for a board, and it is worth a test in its
own right — the failure mode it guards against is a build that stopped checking,
which would look identical to a passing run until somebody ran `apply power-off`
on their workstation.

---

## 4.5 §4.4's row was this script's doing — struck 2026-08-27

**§4.4 is a good section that reached the wrong conclusion**, and it is left
above with its reasoning intact because the mistake is the instructive part.

Everything it observed was true. `run.sh` packed pid 1 at `/init`;
`/proc/1/comm` therefore read `init`; `cmd/rivet`'s interlock requires exactly
`rivet-initd` (`crate::shutdown::INIT_COMM`) and refused. What did not follow is
the conclusion: *an emulator cannot settle what an operator asks pid 1 to do.*

**The clause that carried it is the one to look at:** *"`run.sh` packs the same
binary as the initramfs `/init`, **which is what the kernel requires**"*. The
kernel requires no such thing. It runs what `rdinit=` names, and this script has
always passed `rdinit=` — its own comment says so, four lines above the
assignment. `/init` was a convention nobody had questioned, and `comm` is the
file name of whatever was exec'd.

So pid 1 is now packed at `cmd/rivet-image`'s `INIT_PATH` — **where a board puts
it** — and `rdinit=` points there. That is *more* faithful than what it replaced,
not a workaround: the emulator now answers what the hardware answers.

**Measured the same day.** `rivet apply reload confirm` succeeds under QEMU and
`cmd/rivet-sysd/src/reload.rs` writes *"SIGHUP: the document has been re-read and
applied"* to the journal. `tools/qemu/smoke.sh` asserts both — the exit status
**and** the record — because a command that exits 0 and delivers nothing is a
reload that was accepted and ignored.

### What is still the board's, and it is narrower than the row said

**`apply power-off` now runs here, and it is the first of `ADR-017` §3.1.5's
stops ever to run under emulation.** §4.4's practical objection — *a smoke test
that powered the guest off would have ended the run it was in the middle of* —
was the real obstacle, and the answer was ordering rather than emulation: it is
the **last thing done to the login guest**, after every other reading has been
taken.

**What is asserted is that the machine STOPS**, not that a command exited 0. pid
1 calls `reboot(RB_POWER_OFF)`, which on `-M virt` is a PSCI call QEMU honours by
exiting, so *the process ending on its own* is the observation. A machine that
accepted the command and stayed up is the failure worth catching, and an exit
status cannot tell the two apart.

**`apply restart` is still unexercised**, and it is a harness gap rather than a
limit: it would need this loop to watch a second boot come up on the same guest.

**`apply trial-restart`'s effect remains hardware's in full.** `tryboot` and
`/proc/device-tree/chosen/bootloader` are not emulated at all, so a trial boot
can be *asked for* here and never *observed* — that one is genuinely in the
right-hand column and stays there.

### The general lesson, which is §58's arriving twice

The question to ask of anything in the right-hand column is **does this touch
something only a board has** — not *is this important enough to need a board*,
and not *has it only ever been done on a board*. Two rows have now failed that
question in two days: the git path (§58) and this one. Both had been treated as
hardware work, and neither was.

---

## 4.6 The layout clause falls — 2026-08-28, and it cost one option

**`PLAN.md` §62 is the account; this is what changed in this table.**

The `/state` row had been read for months as *"the emulator has no disk"*. It
had a disk available the whole time: `-M virt` provides virtio-mmio slots
unconditionally and `ADR-024`'s own kernel fragment has carried
`CONFIG_VIRTIO_BLK=y` since it was written — with a comment saying what it was
for. `tools/qemu/run.sh` simply never attached one.

With `--card <image>`, and before any change to Rivet at all:

```
virtio_blk virtio0: [vda] 4194304 512-byte logical blocks (2.15 GB/2.00 GiB)
 vda: vda1 vda2 vda3 vda4 vda5
```

**What that settles**, and each of these was in the right-hand column the day
before:

* the GPT `cmd/rivet-image` writes is parsed by a kernel, and `ADR-018` §4's
  five partitions enumerate;
* the `ext4` that tool makes on `p5` is mountable by the machine that boots on
  it, with `ADR-018` §7's flags, read back from the kernel's own mount table;
* **`/state` persists across a boot** — `--card-rw`, two boots, and the second
  keeps the first's journal (`ADR-022` §7) and restores its clock floor
  (`ADR-021` §3). Every `/state` result before this landed on an initramfs
  tmpfs and died with the guest.

**What it does not settle, and the row is now exactly this:** SD/MMC timing and
card wear. A file behind virtio-blk has no wear and no timing, and §2.1's
warning applies with full force — this is the emulator looking like the real
thing, and the two things it cannot look like are named above.

**The method note is the reusable part.** §4.5 struck a row by re-reading the
harness; this one falls by re-reading the *kernel configuration*. Both rows had
been true when written and both had been made false by work done for another
reason — and in both cases the table was the last thing anyone re-read. The
cheap standing check is the one §4.5 asked for: at every ADR pass, take each
cell in the right-hand column and ask *what exactly would have to be true for
this still to hold*, rather than whether it sounds right.

---

## 4.7 The gate could boot a Rivet and not speak to it — 2026-08-30

**For one day this loop had no route into a guest at all, and the gate that runs
before every deploy was red without anybody noticing.** The interactive login
path left the image on 2026-08-29, and `tools/qemu/smoke.sh` had reached a
running guest through it and through nothing else; neither that script nor
`run.sh` mentioned `rivet-mcpd`, the control plane or its port at all.
`PLAN.md` §102.5 recorded it as a finding rather than
leaving it to be rediscovered. This section is what replaced it.

### 4.7.1 What this gate is FOR, said plainly

§4's table splits what an emulator can settle from what a board must. **This
section is about a third thing the table does not name: what the gate is worth
paying for.** In the twenty-four hours before it was written, four card builds
and four trial boots were spent establishing facts that were all of this shape —
*does this build come up, converge, and answer* — and every one of them was
answerable in a minute without touching hardware.

So the gate's job is exactly that sentence, and it is now the whole of what a
green run claims:

1. **It boots.** PID 1 reaches its handoff, mounts what it mounts, spawns the
   supervisor and never returns.
2. **It converges.** `/etc` is projected and read back off the kernel, the
   journal opens on `/state` and flushes what it buffered, the services the
   image defines start or refuse *by name*, and PID 1 reaches a boot-confirmation
   verdict instead of withholding one.
3. **It answers.** A workstation asks it a question over `ADR-035`'s control
   plane and gets the machine's own JSON back.

Nothing in that list is a claim about hardware, and the run says so at length
before it exits. What it earns is the right to spend a card build.

### 4.7.2 What the run actually does now

**Two boots, and the second one is the one that asks.**

**Boot 1 — the machine with nothing configured.** No network, no document, no
control plane. Everything §4.2, §4.3 and `ADR-022` §7 put here is asserted on it,
and one thing is new: `cmd/rivet-sysd`'s `rivet-mcpd` definition is
`enabled: true`, so a machine whose image lacks the binary must try to start it
and **refuse permanently, naming what the machine has lost** (`ADR-017` §3.1.6).
That is what a card without it really does, and it is the negative direction of
the assertion below.

**Boot 2 — the machine answering.** In order: two throwaway Ed25519 identities
are minted with `cargo rivet identity new`; the guest's seed, certificate and a
`peers` file naming the operator's key are placed at `/state/secrets`, where the
supervisor's own definition points `--identity` and `--peers`; a document is
composed from `qemu-network.toml` plus a `hostname` and a `[control-plane]
listen` of `0.0.0.0:7777`; the guest boots with the port forwarded; and
`cargo rivet run` — **the binary an operator holds**, not a probe written in the
harness — carries three operations:

```text
get status     {"subject":"status","phase":"converging","services":[{"name":"rivet-netd",…
get catalogue  {"subject":"catalogue","shape":"A command is a VERB, then a SUBJECT, …
get file       {"subject":"file","path":"/etc/hosts","present":true,"type":"file",…
```

with the daemon's own account of what it did, off the guest console:

```text
rivet-mcpd: listening on 0.0.0.0:7777 (from the document's [control-plane] table)
rivet-mcpd: MUTUAL TLS — ADR-035 §7.2 over ADR-025 §2. Both ends are pinned by
```

Then `apply power-off confirm`, over the same control plane, as the guest's last
act — which is §4.5's ordering argument unchanged, over the control plane.

### 4.7.3 Three choices worth stating, because each had a cheaper wrong answer

**The address in the document is `0.0.0.0` and not the built-in default.**
`rivet_config::control_plane::DEFAULT_LISTEN` is `127.0.0.1:7777`, and SLIRP has
no route from the host to a loopback listener inside the guest — so a forwarded
port connects to nothing and the failure reads as a control plane that never
started. Proven by injection: with `127.0.0.1` the daemon reports listening, the
interface comes up, and the client gets `Connection reset by peer`.

**The document does NOT enable the service, and that is the point.** The
definition's `enabled` said `false` for three whole phases while every board ran
a document that turned it on, so **a card built from the defaults booted with no
control plane at all** and nothing could see it (`PLAN.md` §94.7). The harness
refuses a document that names `rivet-mcpd`, so what this boot asserts is the
image default. Proven by injection: flipping that one line back to `false` fails
boot 1 in seconds, with a message naming the three-phase history.

**A wrong pin is asserted to be REFUSED**, and without it none of the three
answers above would mean anything: they would pass equally well against a control
plane that authenticated nobody. The wrong pin is a real Ed25519 key — the
operator's own — so what is exercised is the trust decision and not the hex
parser. Proven by injection: pointing the probe at the *right* pin fails the run
with *"a connection pinned to the WRONG key exited 0 and was answered"*.

### 4.7.4 The identity is minted per run, and a fixed one is refused

Two identities, both by `cargo rivet identity new` into a temporary directory
removed on exit. The alternative — a committed pair — was considered and is
refused on more than `AGENTS.md` §8.3's rule:

* a committed `device.seed` is a **private key every reader of this repository
  holds**, so any of them could impersonate any machine that pinned it;
* the first person to copy the fixture onto a board because it was convenient
  would not be doing anything the tree told them not to;
* and `ADR-025` §3 has generation live in the fleet tool precisely so that a
  device's identity has a provenance. A fixture has none.

The cost is two Ed25519 keypairs per run, which is microseconds beside a QEMU
boot. **The rule this follows is `AGENTS.md` §4's:** the harness mints, places,
pins and then refuses a wrong pin, so the exchange is shown to fail as well as to
pass.

### 4.7.5 A gate that cannot tell a broken guest from a broken harness is not one

`timeout`'s exit 124 says only that something did not happen, and every reason
it did not happen looks identical — which this project has paid for before. So
the wait for the control plane ends on **either** the daemon's own `listening on`
line **or** the supervisor's give-up record (`ADR-017` §3.5), and the failure
prints the daemon's stderr first and the supervisor's actions second.

That distinction earned itself immediately. The first run against a real guest
failed with *"the control plane started, exited, and the supervisor GAVE UP"*,
carrying `rivet-mcpd`'s own sentence: `/state/config/machine.toml is not a
document this build can read: duplicate key`. **The harness had composed a
document with two `version` keys**, because it filtered the fixture's own line
with `grep -v '^version = 1$'` and the fixture is a tracked file on a Windows
checkout — `$` does not match before a `\r`. Neither the control plane nor the
supervisor was at fault, and a bare timeout would have said nothing about either.
`.gitattributes` now pins `tools/qemu/*.toml` to LF and the composition counts
its own keys before booting.

### 4.7.6 It runs in the container, not in a hand-built distro

`tools/container/qemu.sh` is the runner and `tools/container/build-card.sh` is
its pattern: the repository at `/rivet`, the Buildroot output read-only at `/br`,
and the cargo caches in named volumes. `qemu-system-aarch64` and `cpio` joined
the image, which is why `IMAGE_VERSION` is `2`.

**Two inputs come out of `/br` and only one of them is obvious.** The kernel is:
`RIVET_KERNEL` points at `/br/images/Image`, because `--net` needs `ADR-024`'s
own kernel and §4.1 is the trade. The other is the **cross C compiler** —
`rivetos-mcpd` links `rivetos-tls`, which links `ring`, whose build compiles C
(`ADR-009` §3.7.4) — so the guest build needs `host/bin/aarch64-linux-gcc`
exactly as the card build does. Without it `cc-rs` fails minutes in naming a
compiler nobody asked for, so `smoke.sh` refuses early by name.

**No port is published and no privilege is asked for.** QEMU's `hostfwd` lands
on the container's own loopback and the client runs in the same container, so
nothing crosses the boundary and two runs cannot collide over a host port. `-M
virt` under TCG needs no `/dev/kvm`, which is what makes this identical on a host
that has none.

---

## 4.1 Two kernels, 2026-08-23 — and the trade between them is real

The loop now boots either of two kernels, and **neither is strictly better**.
The table above splits QEMU from the board; this one splits QEMU from itself,
and the reason it exists is that the new kernel makes the loop able to do more
while being *less* representative — which is precisely the shape of mistake §4
was written to prevent.

| | `kernel8.img`, the board's | `Image`, Rivet's |
| :-- | :-- | :-- |
| Comes from | `tools/qemu/fetch-kernel.sh`, copied off the target | `tools/buildroot/build.sh`, built from `ADR-024` §3's fragment |
| Version | `6.18.34+rpt-rpi-v8` — **the kernel the hardware runs** | `6.12.61-v8` — Raspberry Pi's tree at the revision Buildroot 2026.02.3 pins |
| Its configuration is | Raspberry Pi's, and not this project's to know | Rivet's, reviewable, and `ADR-024` §5's whole point |
| Landlock | absent (`ADR-024` §8.1 item 1) | `LSM: initializing lsm=capability,landlock` |
| memory cgroup controller | absent (`ADR-024` §8.1 item 4) | present — see below |
| virtio, generic PCI host | neither, so no NIC of any kind | both, on both transports |
| Selected with | the default | `RIVET_KERNEL=<output>/images/Image` |

### 4.1.1 What Rivet's kernel settles that the board's cannot

**Networking, which is the whole reason it was built.** `-M virt` with a virtio
NIC now produces an interface with a driver bound to it, verified by reading
sysfs from a BusyBox shell inside the guest rather than by inferring it from the
absence of an error:

```
== driver bound to each interface (readlink of device/driver) ==
  eth0 -> /sys/bus/virtio/drivers/virtio_net
== udhcpc on eth0 ==
udhcpc: lease of 10.0.2.15 obtained from 10.0.2.2, lease time 86400
== ping the user-net gateway ==
2 packets transmitted, 2 packets received, 0% packet loss
```

Both transports work — `virtio-net-device` on virtio-mmio and `virtio-net-pci`
behind `pci-host-generic`, which enumerates `[1af4:1000]` where the board's
kernel found nothing at all. `tools/qemu/run.sh --net` and `--net-pci` select
them; a forwarded port is the route to *attempting*
Stage 1's exit criterion without a board.

**Also, and for free, two of `ADR-024` §8.1's findings.** PID 1 prints both on
every boot, so no new instrumentation was needed:

```
[    0.008267] LSM: initializing lsm=capability,landlock
  /sys/fs/cgroup/cgroup.controllers: cpuset cpu io memory pids
```

Landlock is active rather than merely compiled, which is what §8.1 item 1 said
the stock kernel could not offer. And `memory` is in the controller list, which
is what item 4 said was missing — though **not for the reason the record gives**;
see `ADR-024` §8.2, which is where that finding is written down.

### 4.1.2 What Rivet's kernel does NOT settle — and the board's does

This is the load-bearing half, and it is a genuine loss rather than a caveat.

* **It is not the kernel the hardware runs.** `6.12.61` against `6.18.34` is six
  release cycles. §2.1's argument applies with full force and in the other
  direction: most of what PID 1 does is a conversation with a specific kernel —
  which filesystems it will mount, what `/proc` and `/sys` contain, how it
  delivers `SIGCHLD`, what it does when init returns — and a different kernel
  answers some of those differently *while the console looks identical*. **Any
  question of the form "does the target kernel do X" is still `kernel8.img`'s,
  and only `kernel8.img`'s.**
* **Its own configuration has never booted the board.** Everything it proves
  about Landlock, cgroups and `nf_tables` is proof about a kernel running under
  emulation. `ADR-024` §8.1's closing sentence still stands unaltered: that a
  Rivet-configured kernel builds and boots *on this board* is unproven.
* **`virtio_net` is not `bcmgenet`.** §5 already ruled this the right trade —
  the netlink code under test is the same and only the driver differs — and §4's
  third row is unchanged: `bcmgenet` and `eth0` are the board's. What the loop
  can now settle is Rivet's Stage 1 code; what it still cannot settle is
  whether the board's NIC comes up.
* **It is not the kernel Rivet will ship either**, yet. `ADR-024` §4 rules
  `=y` throughout and no modules; this build still compiles and installs some
  four thousand modules inherited from `bcm2711_defconfig`, because §8 says not
  to strip during bring-up. So it is neither the board's kernel nor the final
  one — it is the configuration under test.

**The rule that follows, and it is §4's rule sharpened:** pick the board's
kernel by default and reach for Rivet's when the question is about Rivet's
*kernel configuration* or about *networking*. A result obtained under
`Image` and reported as though it had been obtained under `kernel8.img` is the
same error as "it works in QEMU", one level in.

---

## 4.2 The login path, 2026-08-23 — the loop can now see one link of it

`tools/qemu/run.sh` packed two binaries: PID 1 and the supervisor. The card
carries four. The two it did not pack are `cmd/rivet` — the CLI, which is the
login shell — and `cmd/rivet-netd`, and the first of
those had a visible cost.

The runtime `/etc` projection (`ADR-007` §3.5, `ADR-017` §3.1.8) reads exactly
one fact off the machine rather than out of a constant: it asks the kernel
whether anything is at `rivet_core::etc::LOGIN_SHELL_PATH`, and names that
path in `/etc/shells` only if the answer is yes. Naming a binary that is not
there would fail at `exec` **after** authentication, which is the worse of the
two failures, so the probe is the design. Under this loop it had **one reachable
answer**, and every run reported:

```
Rendered { files: 5, read_only: true, names_a_login_shell: false }
"permits_a_login":false
```

That was a true statement about an initramfs with no CLI in it — and it meant
the interaction between *the image installing the CLI* and *the projection
finding it* could not be exercised anywhere but on a card.

### 4.2.1 What was added, and how the paths are derived

`run.sh` gains `--cli` and `--netd`, alongside the `--supervisor` it had. Every
destination is **read out of the crate that looks for the binary**, never
written in the script — the precedent `--supervisor` set, and the reason is that
a harness which restates a path packs a file where nothing looks for it the day
a ruling moves the path, with the failure path running quietly in a run that was
meant to demonstrate the success path.

| Option | Destination | The constant, and who uses it |
| :-- | :-- | :-- |
| `--supervisor` | `/system/bin/rivet-sysd` | `SUPERVISOR_PATH` in `cmd/rivet-initd/src/supervise.rs` — PID 1 `exec`s it |
| `--cli` | `/system/bin/rivet` | `LOGIN_SHELL_PATH` in `rivet-core/src/etc.rs` — the projection **probes** it |
| `--netd` | `/system/bin/rivet-netd` | `NETD_PATH` in `cmd/rivet-image/src/system.rs` — the image installs it |

`--cli` reads `rivet-core`'s constant rather than `cmd/rivet`'s because the
destination that matters is the one the probe uses. That fact has a second copy
in `cmd/rivet/src/invocation.rs` (`INSTALL_PATH`) which `ADR-011` forbids and
which nothing pins against the first — `cmd/rivet-image`'s tripwire compares
its own `CLI_PATH` to a literal — so `run.sh` reads both and refuses when they
disagree. That refusal was watched firing.

### 4.2.2 What it settles, read off the console

Both directions were run, against `kernel8.img`, on 2026-08-23. With the CLI
packed:

```
"msg":"/etc/shells names a login shell, so a login program will not reject an account
 for having it","shell":"/system/bin/rivet"
"outcome":"Rendered { files: 5, read_only: true, names_a_login_shell: true }","permits_a_login":true
```

and without it, in the same harness:

```
"msg":"/etc/shells names NO login shell …","would_be":"/system/bin/rivet"
"outcome":"Rendered { files: 5, read_only: true, names_a_login_shell: false }","permits_a_login":false
```

The rendered files differ accordingly — `/etc/shells` 231 bytes against 488,
`/etc/passwd` 53 against 34 — and each was read back off the kernel by the
supervisor rather than asserted.

**`smoke.sh` asserts both**, selected by argument count as the supervisor
already was, because an assertion that can only ever pass is not a test
(`AGENTS.md` §4). A probe that *fails* is a third outcome and fails both
directions, so the negative run proves the probe answered "no" rather than
merely not answering. Each direction's checks were run against the other
direction's console and every one gave the opposite answer, and moving the
packed path by one character was watched failing the positive run.

## 5. `-M raspi4b` was tried and does not work yet

QEMU 11.1 provides a `raspi4b` machine type, which would be more representative
than `virt`, so it was tried first. Recorded so the next session does not spend
the same hour:

* **It boots the kernel.** With the real device tree and the console on the
  **second** serial (`-serial null -serial mon:stdio` — the mini-UART
  `bcm2835aux0` at `0xfe215040`, not the PL011), the kernel runs to
  `Run /init as init process` at 1.50 s.
* **Then userspace dies.** `Kernel panic - not syncing: Attempted to kill init!
  exitcode=0x0000000b` — SIGSEGV, in a binary that runs correctly under `virt`.
  The fault is `el0_da`, a data abort in userspace. **Cause not identified.**
* **RAM is fixed at exactly 2 GiB.** `-m 1G` is rejected outright with
  `Invalid RAM size, should be 2 GiB`, so the theory that the real dtb's memory
  node disagreed with QEMU's RAM could not be tested by shrinking it.
* **Without `-dtb` it produces no output at all.**
* **It disables four peripherals** when given the real device tree —
  `brcm,bcm2711-pcie`, `brcm,bcm2711-rng200`, `brcm,bcm2711-thermal` and
  **`brcm,bcm2711-genet-v5`**.

That last point is the one that settles it: **`raspi4b` has no ethernet**, so
even working perfectly it could not test Stage 1, which is the first stage where
a more representative machine would have paid for itself. `virt` with virtio-net
is *more* useful for Stage 1, not less, despite being less faithful — the
netlink code under test is the same either way, and only the driver differs.

**Worth revisiting** if `raspi4b` gains genet and the userspace fault is fixed,
because a faithful BCM2711 would make §4's right-hand column shorter. It is not
worth blocking on.

---

## 6. Consequences

* **A runner belongs in the repository**, not in a scratch directory, or the
  loop is one person's shell history. `tools/qemu/` holds it. This is the first
  non-documentation content in the tree, and it is deliberate that a *test
  harness* precedes the first crate.
* **The loop needs a Linux host**, so it lives in WSL2 — the same environment
  `ADR-009` §6 already assigned to Buildroot. One Linux environment builds the C
  mechanisms and runs the emulator.
* **The kernel and device tree are inputs, not artefacts.** They are copied from
  a board or produced by `ADR-024`'s build, and they are **not committed** —
  they are large, they are somebody else's binaries, and `ADR-010` would have
  opinions. The runner fetches or is pointed at them.
* **Stage 0's exit criterion gains a predecessor.** Before "the board reaches a
  state where a shell can be entered at the HDMI console", there is now "the
  same PID 1 reaches the same state under QEMU". The board criterion is
  unchanged and still decides.

---

## 7. Open

1. **The `raspi4b` userspace fault** (§5). Not blocking; recorded so it is not
   rediscovered.
2. **Networking in the loop — DONE 2026-08-23, §4.1.** Stage 1 wanted
   `-netdev`/virtio-net so the `rtnetlink` code has an interface to configure.
   It has one: `tools/qemu/run.sh --net` (or `--net-pci`), against the kernel
   `tools/buildroot/` builds. The driver differs from `bcmgenet`, so this tests
   Rivet's code and not the board's bring-up — §4.1.2 keeps that distinction
   in front of the reader rather than leaving it here in an item marked done.
3. **A disk image loop for Stage 3.** A/B images, partitions and `tryboot`
   confirmation need a virtual disk rather than an initramfs — and `tryboot`
   itself is not emulated, so the boot-confirmation path will need a seam that
   can be driven from a test rather than only by firmware.
4. **Whether the custom kernel is built and booted here first — DONE
   2026-08-23, §4.1.** It was, it does, and it was worth it: the build's own
   configuration check caught four `ADR-024` §3 requirements silently demoted
   from `=y` to `=m` before anything reached a card.
5. **Which kernel `smoke.sh` asserts against, and whether it grows a
   network assertion — BOTH ANSWERED 2026-08-30, §4.7.** `smoke.sh`'s second boot needs
   a network, so it asserts one: `rivet-netd` converges `eth0` and a forwarded
   port reaches the control plane on it, which is *the NIC bound* as a pass/fail
   with no console to read. And the kernel question answers itself — the
   control-plane boot needs `ADR-024`'s own, so `tools/container/qemu.sh` points
   `RIVET_KERNEL` at `/br/images/Image` and §4.1's trade is a choice a caller
   makes rather than a default nobody stated.
6. **The kernel pin lags the board by six release cycles** (§4.1). Moving
   `BR2_LINUX_KERNEL_CUSTOM_TARBALL_LOCATION` to a Raspberry Pi revision
   matching `6.18.34` would collapse §4.1.2's first bullet, which is the largest
   of the four. It was not attempted here; the sha Buildroot pins is one its own
   CI builds, and a newer one is not.
7. **Nothing in the guest can exec anything** — added 2026-08-23 with §4.2. The
   CLI can now be packed and *found*; it cannot be *run*, because PID 1 spawns
   the supervisor and nothing else and there is no shell on the machine. The
   same wall stops a write to `/etc` being attempted and watched to fail, which
   `ADR-007` §3.6.1 item 1 calls the non-negotiable mechanism, and it is a
   near neighbour of the unreachable shutdown path `tools/qemu/README.md`
   records. Two routes out: a supervisor started as `rdinit` with `--dev-service` can already start
   an arbitrary program, and a login program with an account would settle it
   directly — which is Stage 1's work regardless.

   **NARROWED 2026-08-30, §4.7.** The first clause is false now and the second is
   not. Something in the guest *does* exec something: `rivet-mcpd` hands each
   caller the stdin and stdout of `/system/bin/rivet mcp`, so the CLI is run
   here and its answers are asserted. What is still true is the `/etc` half —
   nothing attempts a write to that read-only mount and watches it fail, and
   the control plane cannot: `apply edit` refuses a path under `/etc` at
   `rivet_core::operation::filesystem`'s gate rather than at the kernel's, so it
   would exercise the gate and not the mount.

---

## The git path is the emulator's, and nobody had noticed — 2026-08-27

§4 divides what this loop settles from what only a board can. **Every part of
serving a git push is on this side of that line, and it took until 2026-08-27
for anything to try.**

**A git daemon was expected to be** a static aarch64 binary like every other in
this workspace — it was never built, and `ADR-032` §4.2's git shell is
`cmd/rivet` handling its own internal verbs instead. What that reasoning turned
on is unchanged by which binary serves:
the vendored git is one too, and both already run under the emulator. Nothing
in the chain from *a request arrives* to *`receive-pack` writes an object*
touches the card, the
VideoCore boot chain, `bcmgenet`, `brcmfmac` or `tryboot` — which is §4's entire
right-hand column.

`tools/qemu/run.sh` therefore gains `--gitd` and `--git`, and the smoke run
performs a **real push from a real client** and reads the reference back with
`upload-pack`. `PLAN.md` §58 is the account, including the ruling it corrected
on the way — `ADR-008` §7.3.3's amendment exists because this loop ran and a
board could not.

**What stays the board's, and the list did not shrink:** none of it is on ext4.
QEMU `-M virt` has no `/dev/mmcblk0`, PID 1's `/state` mount degrades, and every
byte of that push landed on the initramfs tmpfs. **A push surviving a reboot is
hardware's**, and so is `/state/git`'s ownership on a filesystem that stores
uids and modes the way a card's does.

**The general lesson is worth more than the row.** The question §4 answers is
not *is this feature important enough to need a board* — it is *does this
feature touch anything only a board has*. `[repos]`, `[accounts]` and
`cmd/rivet-gitd` were all treated as hardware work for two days because the
thing they enable is reached over a network, and none of them was.

---

## 8. RULED 2026-08-30: this project is Docker, and WSL leaves the loop

**The question was put as a question and it should not have been.** *(`.rulings/`
R-121.)* Two things still required a hand-configured WSL `archlinux` distro — the
signing seed for `tools/update/build.sh`, and QEMU for `tools/qemu/` — and each
had been parked as *"a decision about a signing key"* or *"out of scope here"*
rather than answered.

> **Docker. Platform-independent. WSL is removed from the build and test loop
> rather than worked around.**

### 8.1 It was never actually a close call, and pretending otherwise cost time

`ADR-026` §6 already ruled the general form: **a build nobody else can reproduce
is a build nobody can review.** `tools/container/` was built on 2026-08-29 to
that argument and proved it — three builds, one with completely fresh volumes,
produced the same 2 GiB image, and the container is *more* reproducible than the
host because `/rivet` and `/cargo` are fixed constants where a host's paths are
that host's.

**Everything after that was the same argument arriving one component at a time
and being treated as a new question each time.** It is not a new question. The
answer was ruled in §6 and the only thing left was applying it.

### 8.2 What kept WSL alive, and why neither reason survives

**The signing seed.** `tools/update/build.sh` reads `RIVET_SYSTEM_SEED`, and
mounting it into a container was called *"a decision about a signing key rather
than a plumbing detail."* **It is a plumbing detail.** The seed is already read
by a process on this host; a read-only bind mount at run time gives it to a
process on the same host, under the same user, for the length of one command. It
is not baked into an image, it is not written into a layer, and nothing about it
reaches a registry. `ADR-015` §4's rule is that the signing key **never leaves
the build machine** — and a container on the build machine has not left it.

The thing that would genuinely violate that rule is `COPY`ing a seed into an
image, and no design here proposes it.

**QEMU.** Simply absent from the image, and adding a package is not a decision.

### 8.3 The cost, stated rather than glossed

**The image gets materially larger.** `qemu-system-aarch64` pulls a wide
dependency closure and this image was already carrying a Rust toolchain. That is
the price of one inner loop that behaves the same on every machine.

**Weigh it against what it replaces.** In one day this project spent **four card
builds and four trial boots** on questions a smoke test answers in seconds,
because the harness could not reach a guest at all — the channel it used was
removed on 2026-08-29 and nobody noticed the gate had gone blind. **(Closed the
same day — §4.7. The gate had also been FAILING, not merely blind: it asserted
a precondition that refused on every boot, and a disabled service runs no
precondition.)** And every WSL invocation started an agent on the maintainer's
private key from his login profile, which hangs with no stdin, has to be killed
by hand, and once sat for 6h40m. **A platform dependency that also trips over
the thing this project spent a day deleting is not a dependency worth
keeping.**

### 8.4 What this does NOT change

* **Buildroot is still not built in the container** and is still mounted. §6's
  own reasoning stands: it is hours of work, a separate question, and the
  Dockerfile records what baking it in would cost and what shape a
  self-contained version should take.
* **`git describe` still comes from the host.** `ADR-027` §10 makes the version
  and the commit a 1:1 mapping, and a `git describe` run inside the container
  would stamp `-dirty` on every image for ever.
* **`ADR-026` §4's table is untouched.** The emulator still cannot settle
  `tryboot`, the boot partition, `/state` or a PHY, and moving it into a
  container does not change what an emulator is. **What the gate is for is
  narrower and worth stating: does this build boot, converge, and answer?**
  That is exactly what four card builds were spent proving this week.

  **§4.7 is that sentence, built** — the same day and by a change already in
  flight when this was ruled. The third clause was the missing one: the harness
  could boot a Rivet and could not ask it anything (`PLAN.md` §102.5). It now
  runs two boots, the second of which reaches the guest over `ADR-035`'s control
  plane with `cargo rivet run`, and `tools/container/qemu.sh` is the runner that
  makes §8's ruling true of it. `cpio` joined `qemu-system-aarch64` in the image
  for the same reason: the emulator boots an initramfs and `run.sh` is what
  makes one.

### 8.5 Built, and the half that is not

**`tools/container/build-update.sh` exists and has produced a payload with no
WSL anywhere in it** — `rivet-0cc26ec-…-update.rvt`, 683,631,164 bytes, verified
through `cargo rivet inspect --trusted-key`: **9 members, all verified**, signed
by the system key. The seed reached the build as a **read-only bind mount for
the length of one command** and nothing else changed.

`rivet-card-build:2` carries `qemu-system-aarch64` (QEMU 11.1.0) beside the
existing toolchain, so the emulator has a home that is not a hand-built distro.

**Still in flight:** `tools/qemu/run.sh` and `smoke.sh` running inside it. The
image is ready for them; the harness is not yet written to use it.

#### One defect, found by running it

The wrapper's own `docker build` passed `$HERE` without `hostpath`, and with
MSYS path conversion off Docker Desktop refused `/c/Users/…` with **"unable to
prepare context: path not found"** — a message about a missing directory, for a
directory that is there. `build-card.sh` had already solved it and the fix is
its line, taken verbatim. **The same class as everything else this week: the
knowledge existed one file away and was not consulted.**

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

The pattern in this document is specific and worth naming: §4's table has a
right-hand column headed *what the emulator cannot settle*, and three of its
cells went on claiming a limit the harness no longer had — one of them for four
days.

### In §4 — three cells of the capability table

**All three moved to the left-hand column and the right-hand cell was not
updated.** The superseded text:

~~and anything about `ADR-018`'s partition layout on real media~~ — struck
2026-08-28, §4.6.

~~Whether anything can actually **log in**~~ — struck 2026-08-27. §4.3 moved this
on 2026-08-23 and the cell was never updated, so it claimed a limit the harness
did not have for four days.

~~that an interlock which identifies pid 1 **fails closed**~~ — struck
2026-08-27, §4.5, when the left-hand cell widened.

~~**Anything an operator asks pid 1 or the supervisor to DO.** `run.sh` packs pid
1 as the initramfs `/init`, so `/proc/1/comm` reads `init` and not
`rivet-initd`.~~ — this was a fact about `run.sh`, not about emulation (§4.5).
`/init` was never required; `rdinit=` already told the kernel what to run.

### In §4.2 — `--netd` settled nothing

**Corrected the same day, 2026-08-23.** The superseded bullet read:

~~**`--netd` settles nothing at all today**, and is honest about it: the binary
is on a card and started by nothing, so packing it
reproduces that state and produces no console output.~~

### In §9 — four open items that closed

**Items 2, 4 and 5.** The superseded titles and text:

~~Networking in the loop~~ — done 2026-08-23, §4.1.

~~Whether the custom kernel is built and booted here first~~ — done 2026-08-23,
§4.1.

~~**Which kernel `smoke.sh` should assert against, and whether it grows a
network assertion.** `run.sh` has `--net`; `smoke.sh` does not, so nothing
yet turns "the NIC bound" into a pass/fail a human can run without reading a
console.~~ — both answered 2026-08-30, §4.7.
