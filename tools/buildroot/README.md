# `tools/buildroot` — the toolchain, the kernel, and the C mechanisms

Buildroot was ruled in by `ADR-009` §6, for **the musl cross-toolchain and the C
mechanisms only**. `ADR-024` §5 added the kernel. Image assembly is explicitly
*not* here — that is `cmd/rivet-image`'s, and `ADR-009` §6 names the failure
that keeping the boundary prevents:

> Buildroot produces a bag of correctly-built binaries and a toolchain; **it does
> not decide what the filesystem looks like.** If Buildroot's own root
> filesystem conventions ever reach the image, the boundary has been crossed —
> those conventions are the FHS, which `ADR-008` §3 rejects.

So this directory contains a defconfig, two kernel fragments and a driver
script. It contains no packages and no build logic.

## What is here

| Path | Is |
| :-- | :-- |
| `configs/rivet_aarch64_defconfig` | the Buildroot configuration, derived from `raspberrypi4_64_defconfig` |
| `linux/rivet.config` | `ADR-024` §3's requirements, as a kernel fragment |
| `linux/qemu-virt.config` | what `ADR-026`'s development loop needs of the kernel |
| `build.sh` | drives Buildroot from WSL2 |
| `external.desc`, `Config.in`, `external.mk` | Buildroot's `br2-external` boilerplate |

## Running it

Buildroot needs a Linux host, so this is WSL2 — the same environment `ADR-009`
§6 and `ADR-026` §6 already assign to it. From the repository root:

```sh
tools/buildroot/build.sh --fetch linux     # toolchain + kernel
tools/buildroot/build.sh all               # + busybox and the other C mechanisms
tools/buildroot/build.sh gdb strace        # the debuggers, ADR-005 §7
```

`--fetch` downloads and sha256-verifies the pinned Buildroot release the first
time. Everything after the options is a Buildroot target; `linux` alone is much
faster than `all` and is what the QEMU loop needs.

### What the host needs

Buildroot's own `support/dependencies/dependencies.sh` is the authority and it
refuses loudly, which is the good case. On the Arch WSL2 distro this was brought
up on, three of its requirements were missing and had to be installed:
`rsync`, `wget`, `unzip`. Everything else it wants — `make`, `gcc`, `g++`,
`cpio`, `bc`, `flex`, `bison`, `perl`, `patch`, `file`, `which` — was present.

**One WSL-specific failure is handled by `build.sh` rather than by you.** WSL
appends the Windows `PATH` to the Linux one, so `PATH` contains entries like
`/mnt/c/Program Files/...`, and Buildroot stops with *"Your PATH contains
spaces, TABs, and/or newline (\n) characters."* The advice is not wrong, but
`AGENTS.md` §1 says adapting to the maintainer's environment is the tooling's
problem and not his, so `build.sh` drops those entries for the life of the
process and says how many it dropped. Nothing in the build lives behind a path
with a space in it.

Nothing is written inside the repository. The Buildroot tree defaults to
`$HOME/buildroot-<version>` and the output to `$HOME/.rivet/buildroot-<version>`,
overridable with `RIVET_BUILDROOT` and `RIVET_BR_OUTPUT`. Both defaults are
outside the tree for two independent reasons — the output is several gigabytes,
and a build on `/mnt/c` over 9p is slow enough to change how often anyone runs
it. A loop nobody runs is `AGENTS.md` §4's "test suite that cannot be run will
rot silently".

**Nothing produced here is committed.** `ADR-026` §6: the kernel is an *input*
to the loop, not an artefact of it. `.gitignore` has an entry for the output
directory name as a backstop, not as the plan.

### After editing a fragment

Buildroot will not notice. `build.sh` compares timestamps and **stops** rather
than either silently rebuilding the old configuration or silently discarding a
`make menuconfig` session; re-run with `--reconfigure` when you meant it.

### Seeing what the kernel configuration actually became

```sh
tools/buildroot/build.sh --show-config linux
```

This is not decoration, and it earned its place on the first build.

`merge_config.sh` applies the fragment and reports success. `olddefconfig` then
runs afterwards and can *undo* it, with no error anywhere. On the first build
that happened four times at once: the fragment asked for `CFG80211`, `MAC80211`,
`BRCMUTIL` and `BRCMFMAC` at `=y` — `ADR-024` §3 requirements, and §4 forbids
`=m` — and all four came out `=m`, because the base defconfig has
`CONFIG_RFKILL=m` and **a tristate cannot be `y` while something it needs is
`m`**. Adding `CONFIG_RFKILL=y` fixed it. Nothing in the build log said a word.

That is a wireless driver that would have been present on the card, unloadable
(there is no module loader — `ADR-024` §4), and reported as fine by everything.
`--show-config` reads the produced `.config` back and prints, for every symbol
either fragment asked for, what is really in it. `AGENTS.md` §6: state what you
verified and by what method — and this is the method.

## The glibc trap, which is the one thing to know

`ADR-009` §6 recorded it on 2026-08-22 and it is still true of Buildroot
2026.02.3. Buildroot's `configs/raspberrypi4_64_defconfig` gets the target
right —

```
BR2_aarch64=y
BR2_cortex_a72=y
```

— and then selects

```
BR2_TOOLCHAIN_EXTERNAL_BOOTLIN_AARCH64_GLIBC_STABLE=y
```

which is **glibc**. `ADR-003` §3 requires musl, statically linked, and the
consequence of missing this is not a build failure. It is a build that succeeds
and produces the wrong thing: `ADR-003` §3.1 explains that glibc's NSS `dlopen`s
its modules at runtime, so a statically linked glibc C mechanism calling
`getpwnam` — which several of them do — either fails or silently needs the
shared objects it was supposed to have eliminated. That surfaces at Stage 1, on
the board, as a mechanism that does not work.

Rivet's defconfig swaps exactly one symbol:

```
BR2_TOOLCHAIN_EXTERNAL_BOOTLIN_AARCH64_MUSL_STABLE=y
```

and adds ~~`BR2_STATIC_LIBS=y`~~ **`BR2_SHARED_STATIC_LIBS=y`**, because
`ADR-003` §3's claim is not "we use musl" but "the image contains no dynamic
loader and no shared libc", and a shared build would satisfy the first and fail
the second.

**This paragraph said `BR2_STATIC_LIBS=y` until 2026-09-03 and it had not been
true since Mesa was added.** `package/mesa3d/Config.in` carries
`depends on !BR2_STATIC_LIBS`, so the display work moved the configuration to
`BR2_SHARED_STATIC_LIBS=y` — Buildroot builds both kinds and each package
chooses — and this file was not changed with it. The audit of 2026-09-03 found
it (`PLAN.md` §123, Wave 4.1).

**What the change costs, and what was done about it.** Nothing now guarantees
that `git`, `git-http-backend`, `wpa_supplicant`, `wpa_cli` or `busybox` link
statically; they simply happen to. Measured on 2026-09-03 against the 2026.02.3
tree with the cross `readelf`, **all five are static**, so no card already
written carries a binary it cannot run. That is a measurement of one tree at one
moment: `tools/card/build.sh` now refuses to assemble an image containing a
binary with a `PT_INTERP`, so the next time it stops being true the build says
so instead of the board.

Verified after configuring, by reading Buildroot's generated `.config`:

```
BR2_TOOLCHAIN_EXTERNAL_BOOTLIN_AARCH64_MUSL_STABLE=y
# BR2_TOOLCHAIN_EXTERNAL_BOOTLIN_AARCH64_GLIBC_STABLE is not set
BR2_TOOLCHAIN_EXTERNAL_MUSL=y
BR2_SHARED_STATIC_LIBS=y
```

## What else was removed from the Pi 4 defconfig, and why

Each of these is `ADR-009` §6's boundary being kept, not a preference:

| Dropped | Because |
| :-- | :-- |
| `BR2_ROOTFS_POST_BUILD_SCRIPT`, `BR2_ROOTFS_POST_IMAGE_SCRIPT` | image assembly (`ADR-008`) |
| `BR2_TARGET_ROOTFS_EXT2*`, `BR2_TARGET_ROOTFS_TAR`, `BR2_PACKAGE_HOST_GENIMAGE`, `BR2_PACKAGE_HOST_DOSFSTOOLS`, `BR2_PACKAGE_HOST_MTOOLS` | ditto — `ADR-018`'s five-partition layout is Rivet's, and `cmd/rivet-image` writes it |
| `BR2_PACKAGE_RPI_FIRMWARE*` | `tools/fetch-firmware.sh` already fetches the firmware, pinned by sha and verified (`ADR-010` §6). Two mechanisms fetching the same blob is two answers |
| `BR2_PACKAGE_KMOD*` | `ADR-024` §4 rules `=y` throughout and no modules; a module loader would be a mechanism for something that cannot happen |
| `BR2_SYSTEM_DHCP="eth0"`, `BR2_INIT_BUSYBOX` | Buildroot writing `/etc` and an inittab. `ADR-007` §3.6.2 makes `/etc` a generated read-only *view*, and `rivet-initd` is PID 1 (`ADR-009` §3.1) |

Also dropped: `BR2_GLOBAL_PATCH_DIR="board/raspberrypi/patches"`, which carried
patches for the Pi firmware and userland packages that are no longer selected.

## The kernel

`ADR-024` §5 rules a **fragment** merged over the Raspberry Pi `bcm2711_defconfig`,
from the Raspberry Pi kernel tree rather than mainline, and the reasoning is
about reviewability: a full `.config` is several thousand lines in which a
meaningful change and noise are indistinguishable in a diff, which is exactly
how `ADR-002` §5.3's recorded risk materialises with a Stage 1 dependency
dropped by someone tidying.

The tarball is pinned **by commit sha, not by branch** — the same discipline as
`tools/fetch-firmware.sh`, and for the same reason: a branch moves, a sha does
not, and a kernel that changed underneath the build would make every earlier
boot log a claim about a kernel that no longer exists. The sha used is the one
Buildroot's own Pi 4 defconfig pins, so it is a revision Buildroot's CI builds.

### Why there is one kernel and not two

It would be tidier to keep a "board kernel" and a "loop kernel" apart, and it
would be worse. `ADR-026` §2.1's whole claim is that the loop runs *the kernel
the hardware runs*, so that a behavioural difference is a difference in Rivet
rather than in the kernel. Two configurations would quietly reintroduce the
failure §2.1 names: "the loop would then be testing the wrong thing while
looking identical."

So `qemu-virt.config` is merged into the same kernel. The cost is virtio and a
generic PCI host bridge compiled in for hardware a Pi 4 does not have — tens of
kilobytes, and `ADR-024` §8 is explicit that bring-up is not when to strip. The
fragments stay in separate files so that the eventual stripping pass can see
exactly what it would be dropping.

### What this kernel fixes

The loop could not test networking at all. The Raspberry Pi's own `kernel8.img`
binds **nothing** under `-M virt` — verified 2026-08-23 by booting it with three
NIC configurations and reading the kernel log: `virtio-net-device` produced no
virtio messages, `virtio-net-pci` produced nothing because the image has no
generic PCI host bridge driver, and `qemu-xhci`+`usb-net` produced no xhci
because `qemu-xhci` is itself a PCI device. Only `lan78xx` and `smsc95xx`
registered, with no bus to find a device on. This directory's sibling
`tools/qemu/README.md` had recorded the same wall from the other side: `strings`
over the decompressed image finds no `virtio` at all.

That is the QEMU-Stage-1 block, closed 2026-08-23 (`ADR-026` §7 item 2),
and it is what this build removes.

### What it produces, and what it does not

`images/Image` (a raw arm64 kernel, which `qemu-system-aarch64 -kernel` takes
directly) and `images/bcm2711-rpi-4-b.dtb`. `make all` additionally builds the
C mechanisms into `target/`: BusyBox — for `udhcpc` — and git,
built against musl, confirmed with `file` and by `readelf -d`
reporting no dynamic section at all.

**`gdb strace` builds two more** into `target/usr/bin/` — `ADR-005` §7, added
2026-08-24. `BR2_PACKAGE_GDB` here is the *stub*, not a debugger on the target:
`BR2_PACKAGE_GDB_DEBUGGER` is deliberately absent and the host half is the
`aarch64-linux-gdb` the Bootlin toolchain already ships in `host/bin`.

**Do not use `BR2_TOOLCHAIN_EXTERNAL_GDB_SERVER_COPY` to shortcut it.** That
symbol copies the toolchain's prebuilt stub, which is dynamically linked against
`libstdc++.so.6` and requests `/lib/ld-musl-aarch64.so.1` — an image with no
dynamic loader would build cleanly and fail at `exec` on the board naming the
interpreter. `ADR-005` §7.1 has the `readelf` output.

**A targeted build leaves them unstripped**, because Buildroot strips in
`target-finalize`, which belongs to `all`. That is 837 040 bytes of debug
information about the debugger, per slot, on both slots — so
`tools/debug/build.sh` copies them out and strips explicitly rather than leaving
it to whichever make target somebody typed. Use that script rather than these
targets directly.

**It is not the kernel Rivet will ship**, and two things say so plainly:

* The build is `6.12.61-v8`; the board runs `6.18.34+rpt-rpi-v8`. `ADR-026`
  §4.1.2 treats that as the loop's largest remaining fidelity gap and is the
  reason both kernels are kept.
* `ADR-024` §4 rules `=y` throughout and no modules, and this build still
  compiles and installs some four thousand modules inherited from
  `bcm2711_defconfig` that nothing can load. Making §4 true means
  `# CONFIG_MODULES is not set` in the fragment; `ADR-024` §9 item 5 records why
  that was left for the maintainer rather than taken here.
