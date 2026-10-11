# ADR-002: Target Hardware and Architecture

* **Status:** Accepted
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

Rivet began as an idea aimed at a Raspberry Pi Zero W, which is the board the
name and the logo came from. That board turns out to be a poor place to *build*
an operating system, for reasons that are about development ergonomics rather
than capability.

The choice of first target constrains almost everything downstream: the Rust
target triple, which crates are available, whether a wired link exists for
bring-up, and how the boot partition is laid out.

---

## 2. Candidates

| Board | SoC / core | Arch | RAM | Wired NIC | Video | Status |
| :-- | :-- | :-- | :-- | :-- | :-- | :-- |
| Pi Zero W | BCM2835, ARM1176JZF-S | ARMv6 + VFPv2 | 512 MB | none | limited | Deferred compatibility target |
| Pi Zero 2 W | RP3A0, Cortex-A53 | ARMv8 | 512 MB | none | limited | Unavailable at acceptable cost |
| **Pi 4 Model B** | **BCM2711, Cortex-A72** | **ARMv8-A** | **1–8 GB** | **Gigabit** | **dual micro-HDMI** | **Primary development target** |

### 2.1 Why not the Zero W as primary

* **ARMv6 is a shrinking island.** The correct Rust triple is
  `arm-unknown-linux-musleabihf`, not any `armv7-*` target — an ARMv7 binary
  will not run on it, which is precisely why 32-bit Raspberry Pi OS is still
  built for ARMv6. Crate availability and CI coverage at ARMv6 are thin and
  getting thinner.
* **No wired network interface.** This is the decisive one. It forces every
  network-stack unknown to be debugged simultaneously with wireless
  association. See `ADR-004` §2.
* **No practical attached console.** Debugging PID 1 on a board with no usable
  video output means buying hardware before the first boot attempt.
* **A single 1 GHz core and 512 MB** makes native compilation impractical and
  removes cross-compilation troubleshooting as an escape hatch.

### 2.2 Why the Pi 4

* `aarch64-unknown-linux-musl` is a Tier 2 Rust target with broad ecosystem
  support. The "does this crate build for my board" class of problem largely
  disappears.
* Gigabit Ethernet driven by `bcmgenet` — in-kernel, device-tree described, **no
  firmware blob and no NVRAM file**. It is the least fussy device on the board,
  which is exactly what a bring-up scaffold needs to be.
* Micro-HDMI plus a USB keyboard is a real console, available without buying
  anything, and it renders kernel panics.
* Four USB-A ports and enough RAM to compile Rust natively when a
  cross-compilation problem needs routing around.
* USB boot from an EEPROM-configurable boot order, so the root device can be a
  USB stick rather than a microSD card.

---

## 3. Decision

**The Raspberry Pi 4 Model B, running 64-bit, is the primary development
target. The Rust target triple is `aarch64-unknown-linux-musl`.**

**The Pi Zero W is retained as a declared deferred compatibility target**, not a
build target. It is not built against on any regular cadence. It exists as a
periodic "does this still fold onto ARMv6 and 512 MB" examination.

Provided the codebase remains portable Rust and takes no architecture-specific
dependency, supporting it later is a build-target problem rather than a rewrite.
That proviso is a real constraint and is the reason the target is recorded here
rather than dropped.

---

## 4. The boot chain

The Raspberry Pi boot sequence removes bootloader work from the project
entirely, and this is worth stating explicitly because it is unusual. The
VideoCore GPU boots first, reads firmware from the FAT partition (on the Pi 4,
the first stage lives in an SPI EEPROM rather than on the card), parses
`config.txt`, loads the kernel and device tree, and only then releases the ARM
cores.

**There is no U-Boot, no GRUB, and no bootloader source tree.** The whole of
Rivet's boot configuration is a FAT32 partition and a handful of text files.

```
boot/  (FAT32, and readable and writable from a Windows host)
  start4.elf   fixup4.dat          Pi 4 firmware; no bootcode.bin on the card
  kernel8.img                      64-bit kernel
  bcm2711-rpi-4-b.dtb              device tree
  config.txt                       arm_64bit=1, enable_uart=1
  tryboot.txt                      read INSTEAD of config.txt in tryboot mode
  autoboot.txt                     boot_partition, and the [tryboot] override
  cmdline.txt                      one line; the kernel contract
  rivet.toml                     first-boot provisioning — see ADR-004 §6
```

**`tryboot.txt` is not optional, and omitting it is a headless-board hazard.**
Verified on hardware 2026-08-22 (`ADR-018` §5.3): in tryboot mode the firmware
reads `tryboot.txt` **instead of** `config.txt`, so every setting the board needs
to boot — `arm_64bit=1` above being the one that bites — must appear in both. A
tryboot reboot with no `tryboot.txt` present attempts a boot with none of them.
Image assembly writes the two together or it has built a rollback mechanism that
bricks the rollback.

`cmdline.txt` is a single line and is the entire interface between the firmware
and Rivet:

```
console=serial0,115200 console=tty1 root=/dev/mmcblk0p2 rootfstype=ext4 rootwait init=/sbin/rivet-initd
```

**That line is stale and is kept only as the shape of the thing.** Flagged
2026-08-22 while implementing `ADR-017` §3.1.5, and it contradicts three later
rulings — which is worth noticing, because it was written before the storage
layout existed:

* **`root=/dev/mmcblk0p2` is wrong.** `ADR-018` §4 makes `p2` **boot-B**, a
  FAT32 partition, because the Pi firmware needs an A/B pair of them. The root
  filesystem is a `system` slot, `p3` or `p4`.
* **`rootfstype=ext4` is wrong.** `ADR-018` §3 makes `/system` **erofs**,
  read-only by format. `ext4` is `/state`.
* **`init=/sbin/rivet-initd` is wrong.** `ADR-008` §3 and `ADR-016` §3 put
  binaries under `/system/bin`.

Worse, the correct line **cannot be written here at all**: which slot is root
differs between the A and B images and is decided at image assembly, which is
the same place `ADR-018` §5.1's `autoboot.txt` is written. This is a note for
whoever builds the image, not a value to copy.

### 4.1 The boot partition is also the transfer mechanism

Because the host is a Windows machine and the FAT partition is natively
readable and writable there, it doubles as the file-transfer channel during
bring-up: a rebuilt binary or a changed configuration is dragged onto it in
Explorer, and PID 1 collects it on the next boot. No ext4 tooling, no WSL disk
mounting, and no additional stack on either side. This is the reason the USB
gadget proposal was rejected — see `ADR-004` §7.

---

## 5. Consequences

### 5.1 Leanness must become a measured property

The Zero W's constraints enforced minimalism whether or not anyone intended it.
A Pi 4 with gigabytes of RAM will absorb any amount of bloat without ever
producing a symptom.

Therefore minimalism is stated as a design property and tracked as numbers in
the repository from the first commit:

* image size
* boot time to a usable login
* BusyBox applet count (`ADR-001` §4.1)

A property that is not measured on a machine that cannot feel its loss is a
property that is already gone.

### 5.2 Kernel sourcing

The stock Raspberry Pi kernel, firmware and device tree are used initially. A
stripped custom kernel is deferred until the configuration is understood well
enough to know what is being removed and why. Building a custom kernel on day
one converts every subsequent failure into a two-variable problem.

**Refined 2026-08-22 by `ADR-024` §8.** The question is no longer what to
remove but what Rivet *depends on*, which turns the timing into an audit
rather than a judgement: stay stock if the stock configuration satisfies
`ADR-024` §3, and go custom sooner only if it does not.

### 5.3 Recorded risk

Building a custom kernel later without `CONFIG_BCMGENET` removes the wired
interface that `ADR-004` depends on for bring-up. Any custom kernel
configuration must be checked against the Stage 1 tripwire in `docs/PLAN.md`.

**Addressed 2026-08-22 by `ADR-024`.** That risk generalised — seven ADRs had
accumulated kernel requirements — so `ADR-024` §3 is now their single authority,
and `ADR-024` §7 has Rivet verify them at boot and report any that are absent
rather than failing silently later.
