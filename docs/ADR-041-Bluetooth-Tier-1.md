# ADR-041: Bluetooth, Tier 1 — a Rivet Reads BLE Advertisements

* **Status:** Accepted — ruled 2026-09-05 (`.rulings/` R-144), built the same
  day, **proven end to end on hardware the same evening**, and reproduced under
  emulation the same night: the radio attaches, the daemon scans, and an
  application read the maintainer's own sensors off the air through the
  endpoint. **§8 is what the board measured, §9 is what the emulator then
  closed, and §8.4 is what neither has.**
* **Date:** 2026-09-05
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Supersedes in part:** `docs/SPEC-01` §5.2's Bluetooth row, corrected
  2026-09-04 in §5.2.2 of that document
* **Preceded by:** `docs/design-candidates/Bluetooth.md` — a study that built
  and booted rather than reasoned, and remains the reference for the two
  deferred tiers

---

## 1. Context

The maintainer runs several Govee temperature sensors. They broadcast; nothing
connects to them. TempMon reads them today on a machine that is not a Rivet, and
the plan is for TempMon to run on one.

Before 2026-09-05 the record said Rivet had no Bluetooth and could not have any
without adopting D-Bus, because BlueZ's entire public API is D-Bus and Rivet has
no bus. That reasoning was never tested. The maintainer declined to accept it
three times in the same evening. A kernel can be recompiled with Bluetooth, so
that was never to be treated as a blocker. The build environment has the
toolchain and could simply try to build D-Bus, since for all anyone knew it
would work. And the emulator was there to actually run what was built.
*(`.rulings/` R-144, R-297.)*

The study that followed built everything the argument had been about.
`docs/design-candidates/Bluetooth.md` §8 has the wall-clocks; the whole thing
took **396 seconds of building**, against an assumption of hours.

**Three of its findings decide this ADR:**

1. **`CONFIG_BT=m` was already in every card this project has ever shipped**,
   inherited from the Broadcom `bcm2711` defconfig — and `ADR-024` §4 rules
   everything built in, so a Rivet has no module loader. The stack was compiled
   into every image, installed nowhere, and unreachable. This is the third time
   that exact shape has been found here, after the stock wireless stack and
   `CONFIG_DRM_VC4`'s silent downgrade to `m`.
2. **D-Bus builds against musl in 19 s and BlueZ in 105 s, and both RAN** under
   `rivet-initd` in QEMU, with `bluetoothd` answering a real bus query.
3. **A 480 KB static Rust binary spoke HCI with no BlueZ and no bus** — opened
   `AF_BLUETOOTH` sockets, read the management interface's version, and
   completed a command/event round trip against a virtual controller.

Finding 2 is the trap and finding 3 is the road. See §6.1.

---

## 2. The ruling, and its boundary

The ruling (`.rulings/` R-144) was to do Bluetooth, starting with the Govee
sensors and deferring the other two tiers, with the maintainer assuming that
things would be named properly, `rivet-bluetoothd` and the rest.

**Tier 1 only: reading advertisements.** Tier 2 (BLE central — connecting, GATT)
and tier 3 (Classic HID — game controllers) are **deferred, not rejected**, and
R-144.1 item 4 forbids designing for them. `Bluetooth.md` §7.2 and §7.3 are
their roadmaps and are unstarted.

**The naming is `ADR-033` §4's rule and not a choice**: directory
`cmd/rivet-bluetoothd/`, `[package] name = "rivetos-bluetoothd"` because
rivet.gg owns the `rivet-*` namespace on crates.io, `[[bin]] name =
"rivet-bluetoothd"` at `/system/bin`. The operator and the journal see the
binary name; only crates.io ever sees the other.

---

## 3. What tier 1 actually requires

| layer | tier 1 | tier 2 | tier 3 |
| :-- | :-- | :-- | :-- |
| kernel subsystem | `CONFIG_BT`, `CONFIG_BT_LE` | same | + `CONFIG_BT_BREDR`, HIDP, UHID |
| transport driver | `BT_HCIUART_BCM` over serdev | same | same |
| firmware | the CYW43455 patchram | same | same |
| userland | **an HCI socket and a parser** | GATT client | a full BR/EDR host |
| pairing | **none** | link keys at rest | link keys at rest |
| a Rust road exists | **yes** | yes | **no** — §5.3 |

The row that decides the order is the last one, and the maintainer's chosen
order matches the dependency order without being told: tier 1 is what the Govee
sensors need, and TempMon on Rivet cannot happen before it.

---

## 4. The daemon, and why the protocol is a pure module

`cmd/rivet-bluetoothd/` is seven files. The split is the same one this project
makes everywhere: **bytes in and bytes out on one side of a line, syscalls on
the other.**

`crate::hci` composes commands and parses events and touches nothing — no
socket, no clock, no allocation it does not own — so the five-command scan
setup is a table a reader can check against the specification and a test can
drive with no kernel at all. `rivet_core::bluetooth` holds the wire vocabulary
that both ends of the endpoint agree on, and it is in `rivet-core` for
`ADR-014` §3's reason: the crate performs no I/O, so a parser there cannot grow
a socket by accident.

### 4.1 The scan is passive and duty-cycled

Passive scanning listens and never transmits. Active scanning would send
`SCAN_REQ` and collect responses, which is how a device's name is usually
learned — tier 1 does not need a name, and transmitting is a power and privacy
cost taken for nothing.

**And it turns out to get names anyway, for the devices that matter here.** A
passive scan on the board (§8.3.2) read `GVH5075` off AD type `0x09`,
*Complete Local Name*, in the sensors' own `ADV_IND` — a device that wants to be
recognised puts its name in the advertisement rather than waiting to be asked.
So the sentence above is right about the mechanism and understates the result:
what active scanning buys is a name from a device that **withholds** one, and
none of the devices tier 1 exists for does.

The window is shorter than the interval, and **that property is a compile-time
assertion** (`_A_DUTY_CYCLE_IS_NOT_CONTINUOUS`) rather than a test. A window
equal to the interval is continuous reception, which is the power posture §6.5
declines to take without a measurement that no emulator can produce. It was a
`#[test]` until clippy correctly observed it was comparing two constants at run
time; two plausible edits — raising the window, lowering the interval — now fail
the build.

### 4.2 The `unsafe` is confined to two files and every block says why

`crate::sockets` holds the `unsafe` that reaches the three descriptors, and
`crate::privilege` the `unsafe` that gives up the daemon's capabilities (§6.6).
Every block carries a SAFETY note naming what it upholds. The rest of the
daemon is safe Rust over a file descriptor.

### 4.3 The consumer that reads it is a probe, and it exists because the daemon journals nothing

`examples/probes/ble-listen`. Added 2026-09-05 to take §8.3's measurements, and
it is here in the design record rather than only in `examples/` because **its
existence follows from §6.3**: a daemon that fans out and deliberately writes
nothing it hears into a rolling 1 MB journal cannot be asked *what did you
hear*. The only reader is a consumer, so the second and third measurements are
one question.

It declares `connects` and nothing else — no device class, no path, no
listening socket — which is the smallest possible demonstration of §6.2. It
decodes no vendor payload; it prints the address, the PDU kind, the RSSI and
the length-tagged AD structures, and renders a Complete Local Name as text
because that structure means a name for every device that sends one.

**It should be disabled when nobody is reading it**, for the reason it exists:
a roll-up a minute displaces whatever else the board's journal is recording.

---

## 5. The kernel fragment, and what was left out

`tools/buildroot/linux/rivet.config`'s Bluetooth block. Measured cost:
`Image` 29,467,136 → 30,001,664 bytes, and **+244 KiB compressed**, which is
what `ADR-018` §4's sized slots actually pay.

### 5.1 `CONFIG_BT_BREDR` is deliberately OFF, and that line is load-bearing

The base defconfig sets it. Leaving it alone would ship Bluetooth Classic — SDP,
BR/EDR L2CAP, and with them the `=m` RFCOMM, BNEP and HIDP symbols that were
already there: **still unreachable, but now attached to a subsystem that is
reachable.** That is the deferred tier 3 arriving as untested surface for a
capability nobody ruled, which is the one thing R-144.1 asks this work not to
do.

Turning it off is not a rejection of tier 3. It is the line `Bluetooth.md` §7.3
changes back, and the boot of 2026-09-05 shows the consequence plainly: the
kernel announced the L2CAP socket layer and **no SCO layer**, because there is
no Classic.

### 5.2 The transport is a UART behind serdev, not USB and not SDIO

The Pi 4's Bluetooth half is on a UART. So `BT_HCIUART_SERDEV` and
`CONFIG_SERIAL_DEV_BUS` are in and `BT_HCIBTUSB` is not. `BT_HCIUART_BCM`
selects H4 and the firmware loader; H4 is named anyway, because it is the frame
format on the wire and a reader should not have to know that one symbol quietly
implies another.

### 5.3 Why no Rust road exists for tier 3, stated once

`Bluetooth.md` §4.3 cross-built every candidate. `bt-hci`, `trouble-host` and
`burble` are LE-only. `bluer` looks like the exception and is not: its entire
adapter, device and GATT surface sits behind a `bluetoothd` feature, so the
Rust crate is a D-Bus client wearing a Rust coat. **There is no Rust Bluetooth
Classic host stack**, which is why tier 3's roadmap goes through BlueZ or
through somebody writing one.

---

## 6. The daemon's decisions

R-144.1 listed what the ruling did not settle, and §6.1 to §6.5 are the
answers. §6.6 is what the daemon may hold while it runs (`.rulings/` R-253).

### 6.1 No BlueZ and no D-Bus — and the study proving they work is the reason to be careful, not the reason to adopt

Tier 1 needs neither. The sockets were measured working from plain Rust, and
that is the whole userland requirement for reading advertisements.

Adopting a message bus is a large, permanent change to this system's shape:
`ADR-008` has no FHS, there is no package manager, and services are supervised
rather than bus-activated. **Making it as a side effect of a successful
experiment would be the wrong lesson from a good measurement.** If a bus ever
earns its place here it should be because something needed it, argued on its
own terms, with `Bluetooth.md` §4.1 as evidence that it is possible.

### 6.2 No new device class — an application asks for a socket the way it asks for any socket

An HCI user-channel socket is **exclusive**: one process holds the adapter and
a second is refused. So an SDK module every application called would work for
exactly one application and silently stop working for the second — a defect
discovered in production, months later.

The daemon is therefore the **arbiter**, in the shape `rivet-launcher` already
has for the display (`ADR-015` §6.2, `ADR-036` §3.1): the platform holds the
scarce thing and hands applications what they need.

**And that needed no new mechanism.** An application declares
`connects = ["rivet-bluetoothd:advertisements"]`, the supervisor binds the
socket and hands over the descriptor, and the file is chowned to the consumer
that declared it — `ADR-015` §5.3, unchanged. A `devices = ["bluetooth"]` class
was considered and rejected: an application that never touches the radio has no
business holding a grant that says it does, and the class would have been a
second vocabulary for a relationship `connects` already describes.

`rivet-bluetoothd` is consequently the **first image service that PROVIDES an
endpoint**, and `provides` folds into `listen` exactly as an application
manifest's does.

That change surfaced a message that had quietly become wrong. A consumer whose
provider is absent was refused with *"no such application is installed"*, which
was true of every provider there had ever been and became misleading the moment
a system service was one. It now says *"no such service is enabled"* and names
the document key.

### 6.3 The daemon fans out and does not filter

A board in a house hears a great many advertisements per second and the obvious
economy is a filter in the daemon. Declined, on `ADR-011` rather than laziness:
a filter here is **a second place that has to know what an application wants**,
and the application already knows. What it would filter on — an address, a
manufacturer id, an RSSI floor — is a policy nobody has ruled, and inventing one
would be the operating system deciding which of the maintainer's sensors count.

The volume is bounded at the other end instead. A consumer that cannot keep up
has records **dropped, counted and reported**; it is never blocked on. A
blocking write would let a slow application stall the process reading the radio,
which is the one process on the machine that must not stall.

**And nothing here knows what a Govee is.** Vendor decoding belongs to the
application. The daemon delivers an address, an RSSI, a timestamp, an
advertising type and the raw bytes.

### 6.4 With no adapter it reports and stays up

Every Rivet built before this one has no Bluetooth configured, and the x86
target may have no radio at all. A daemon that crash-looped on a machine with no
controller would be a service permanently in `failed` on hardware where that is
the correct and expected state. It reports what the kernel said and waits,
blocked on the management socket for `Index Added`. It holds `CAP_NET_ADMIN`
while it waits, because the bind that follows needs it (§6.6).

### 6.5 It ships DISABLED, on the merits

Three reasons, and the third is the one that decides it. A machine not reading
advertisements is reachable and functional in every other way. The adapter is an
exclusive claim on a radio whose power cost has not been measured. And **with no
consumer installed it would hold that radio for nobody.**

That has a cost worth naming: an application declaring `connects` on a machine
where the service is off is refused and not started. §6.2's corrected message is
what makes that legible instead of baffling.

### 6.6 It holds a capability only until the controller is bound, and exits when the controller goes away

**Binding `HCI_CHANNEL_USER` is the one thing the daemon does that needs
privilege**: the kernel checks `CAP_NET_ADMIN` at the bind, and reading and
writing a bound user channel check nothing. So the daemon parses the air
holding no capability at all.

* **Its own uid.** It runs as the identity its name derives, under the
  image-service ruleset, with a home on the built-in volume, like the other
  homed image services (`ADR-017` §4). Its own uid fits because nothing it
  hands a consumer is a file it owns: the advertisement stream is the socket
  the supervisor binds, chowns to the consumer and passes as a descriptor
  (§6.2), and accepting on a handed descriptor needs no ownership. The other
  shape, uid 0 holding nothing, would still own root's files; nothing here
  needs them.
* **Started keeping `CAP_NET_ADMIN` and `CAP_SETPCAP`** (`ADR-017` §3.9).
  `CAP_SETPCAP` is the capability that lets a process remove one from its own
  bounding set, so it is what makes the bounding set emptiable from inside.
* **Once the bind succeeds**, it closes the management socket and empties its
  bounding, ambient, permitted, effective and inheritable sets, sets
  `NoNewPrivs` (already set by the supervisor), reads each set back with
  `capget` and `prctl`, and journals the masks in `/proc/<pid>/status`'s
  spelling. A drop the kernel did not honour stops it with exit 5 rather than
  scanning holding what it said it gave up.
* **When the controller goes away it exits** with exit 4, saying how many
  advertisements it heard. A process holding nothing cannot bind the next
  controller, and the supervisor's `on-failure` already knows how to bring a
  service back: it starts the daemon again holding `CAP_NET_ADMIN`, back at the
  wait of §6.4. An application reading the stream sees end of file and
  connects again, which `rivet_sdk::bluetooth` documents.
* **Ordinary unplugging never ends a machine's Bluetooth.** A run that lasted
  the supervisor's success window is a success (`ADR-017` §3.5), so each such
  exit starts the give-up count again. Five losses, each within that window of
  the daemon's start, is a controller that will not stay up, and the supervisor
  gives up on it as on any service. `cmd/rivet-sysd`'s supervisor tests pin
  both, and `tools/qemu/adr-041-proof.sh`'s stage 3 takes the whole path under
  emulation: the virtual controller is unplugged and plugged back in, the
  daemon exits 4, and its next run binds the new controller, drops every
  capability again and scans. The control plane cannot make a board lose its
  CYW43455, so the exit and the restart are proven under emulation and not on
  a board.

---

## 7. What this ADR does not settle

* **The power cost of a duty-cycled scan, in watts.** The Bluetooth and
  wireless halves are one Broadcom part, and **this board carries no
  instrument that can answer the question** — the only power-adjacent facts it
  will report are an SoC die temperature and a process's CPU time, and neither
  is the radio's consumption. §8.3.4 reports both, and reports the SoC
  temperature as **not distinguishing a scanning board from a quiet one**,
  which is a measurement of the instrument as much as of the scan. A wall meter
  or an inline USB power meter is what would settle it.

  **Amended 2026-09-05.** The original sentence — *"nothing has measured what a
  duty-cycled scan costs in power or in wireless throughput"* — remains true of
  power. It is differently true of throughput: on the board that ran this
  `wlan0` is **down and has moved zero bytes in either direction**, so the
  contention question has no measurement to take here rather than an unmeasured
  one, and a board that used its wireless would be a different experiment.
* **Whether the scan parameters are right.** 30 ms of listening in every 100 ms
  is a starting point taken from common practice, not from this board. What the
  board does say is that they are **sufficient**: §8.3.2's sensors were heard
  repeatedly at this duty cycle. Whether a longer window would hear them more
  often, or a shorter one often enough, is untested.
* **Both deferred tiers**, which is the point of deferring them.

---

## 8. What has run on hardware, and what has not

**Updated twice on 2026-09-05.** In the morning, after a card was built and
booted on the maintainer's Pi 4 — §8.1 and §8.2. In the evening, after the
daemon was enabled on `0.1.69` and an application read the endpoint — §8.3.
Every reading below is quoted from that board.

### 8.1 Proven, in the kernel's own words

```text
Bluetooth: Core ver 2.22
NET: Registered PF_BLUETOOTH protocol family
Bluetooth: HCI device and connection manager initialized
Bluetooth: HCI socket layer initialized
Bluetooth: L2CAP socket layer initialized
serial serial0: tty port ttyAMA1 registered
Bluetooth: hci0: BCM: chip id 107
Bluetooth: hci0: BCM: features 0x2f
Bluetooth: hci0: BCM4345C0 (003.001.025) build 0000
Bluetooth: hci0: BCM4345C0 'brcm/BCM4345C0.raspberrypi,4-model-b.hcd' Patch
Bluetooth: hci0: BCM43455 37.4MHz Raspberry Pi 3+-0190
Bluetooth: hci0: BCM4345C0 (003.001.025) build 0382
```

`/sys/class/bluetooth` contains `hci0`.

So: **the radio attaches over serdev, the driver identifies the chip, and the
patchram loads and takes effect** — the firmware build number moving from 0000
to 0382 is the patch applied, which is a stronger statement than the file having
been opened. **No SCO layer appears**, which is §5.1 visible on hardware.

It also settled a name. `Bluetooth.md` §3.4 recorded
`BCM4345C0.raspberrypi,4-model-b.hcd` as documentation-derived and unverified;
the driver asked for exactly it. The bare `BCM4345C0.hcd` shipped alongside as a
fallback and was **deleted the same day**, because it was never reached —
`cmd/rivet-image`'s `FIRMWARE_BT_COMPATIBLE` carries the journal lines as the
reason.

### 8.2 A defect this found, which no test could have

`cmd/rivet-image`'s `FIRMWARE_FILES` was a hard-coded list of **three**, all
wireless. Enabling Buildroot's Bluetooth firmware package and the kernel driver
did not add the patchram to it, and **the first card shipped a Bluetooth driver
with no firmware.** `hci_bcm` requests its patchram at `hdev->setup`, so nothing
fails at boot; the controller simply never appears, which is indistinguishable
from a dead radio.

`tools/card/build.sh` now lists `/lib/firmware/brcm` off the built image and
**refuses a card whose firmware directory came out empty** — the same check it
already made for `/system/bin` and `/lib`, extended to the third directory
nobody had covered.

### 8.3 The daemon runs, and this machine hears the sensors it was built for

**Measured 2026-09-05, on `0.1.69` in slot `a` of the maintainer's Pi 4.** This
section is the answer to the four questions the previous one asked; the
question form is kept in `## History` below, because what it asked for is
exactly what a reader should check any future claim of this kind against.

**Addresses are masked here, and the masking is deliberate.** `AGENTS.md` §8.3
forbids real personal data in a tracked file, and a device address plus a model
name is a fact about somebody's house. The vendor half of an address — the OUI
— is published by the IEEE and is not personal, so it is kept; the
device-specific half, and the four hex characters a Govee appends to its own
name, are `xx`. The board's journal carries the unmasked lines.

#### 8.3.1 Enabled, and scanning seventeen milliseconds later

No card and no reboot: `rivet apply edit` added `[services.rivet-bluetoothd]`
with `enabled = true`, and `rivet apply reload` was the whole of it.

```text
864.141  rivet-bluetoothd  started
864.156  rivet-bluetoothd  the management socket is bound, so this kernel HAS a
                           Bluetooth subsystem. Asking it which controllers exist
864.156  rivet-bluetoothd  the Bluetooth controller is held by this daemon alone,
                           and the scan is being set up
864.158  rivet-bluetoothd  SCANNING. This machine is listening for BLE
                           advertisements and every one it hears is handed to the
                           application that declares connects on this service
```

**Every branch that fires here is the good one, and each was a real risk.** The
management socket bound, so §6.4's no-subsystem exit was not taken. The user
channel bound first time, so nothing else on the machine held the adapter —
`CONFIG_BT` is built in (`ADR-024` §4) and the kernel's own stack was not up on
it. And §4.1's five-command scan setup ran against a real controller rather
than QEMU's `vhci`: `hci::Sequence` reached `Progress::Scanning`, which means
the reset, the event masks, the scan parameters and the enable were all
accepted in order.

**It stops and starts cleanly too**, which matters because §6.5 makes this an
operator's switch rather than a boot-time fact. Setting `enabled = false` and
reloading gave `stopping` then `stopped`; setting it back gave `started` and
`SCANNING` again 34 ms later, on a second acquisition of the same controller.

#### 8.3.2 It heard the sensors, and they named themselves

`examples/probes/ble-listen` is the consumer — §8.3.3. Its first minute:

```text
1098.337  ble-listen  reading the advertisement stream at
                      /run/rivet/listen/rivet-bluetoothd/advertisements
1098.473  ble-listen  first 2 - A4:C1:38:xx:xx:xx Public Connectable rssi -67dBm
                      [09:<a name> 03:88ec 01:05 ff:88ec…]
```

and its roll-up, after the first 24 were printed whole:

```text
1248.793  ble-listen  heard 2793 advertisement(s) from 29 device(s),
                      0 line(s) that did not parse
1314.881  ble-listen  device A4:C1:38:xx:xx:xx x6 rssi -47dBm name `GVH5075_xxxx`
1314.881  ble-listen  device A4:C1:38:xx:xx:xx x2 rssi -73dBm name `GVH5075_xxxx`
```

**Two Govee H5075 sensors, identified by the board and not by this project.**
The model name is AD type `0x09`, *Complete Local Name*, which the specification
defines for every device that sends one; `03:88ec` is a 16-bit service UUID and
`ff:88ec…` is manufacturer-specific data under company identifier `0xEC88`.
Nothing in the daemon, the SDK or the probe was told what a Govee is — §6.3 —
and the sensor said so itself.

Three numbers worth keeping:

| what | reading |
| :-- | --: |
| advertisements in the first 150 s | **2,793**, from 29 devices |
| rate | **~18–19 per second** |
| lines the consumer could not parse | **0** |

**That last one buys something a test could not.** `rivet_core::bluetooth`'s
encoder and `rivet_sdk::bluetooth`'s decoder are two implementations of one line
format, deliberately (`ADR-036` §5), and `rivet-sdk/tests/agreement.rs` feeds
one the other's output. That proves they agree about bytes this repository
composed. **These 2,793 lines came off the air**, from 29 devices this project
has never seen, carrying every advertising PDU type and payload length real
traffic produces — and the decoder refused none of them. `AGENTS.md` §4's rule
about testing a format against what the other side actually wrote, with the
other side being a room full of strangers' hardware.

**And nothing was dropped.** §6.3's back-pressure record — *"an application is
not reading advertisements as fast as this machine hears them"* — never fired,
so the consumer kept up with the air at this rate and the counter that would
have said otherwise is in place and silent rather than absent.

#### 8.3.3 An application consumed the endpoint, through `connects` and nothing else

`examples/probes/ble-listen` declares one thing:

```toml
connects = ["rivet-bluetoothd:advertisements"]
```

It has no `devices`, no `listen`, no `state`, and it names no path. The
supervisor bound the socket from the **service definition's** `provides`,
chowned the file to the application's own uid, and put the path in
`RIVET_CONNECT`; the SDK read it back and opened it. From the two sides of the
same moment:

```text
1254.864  rivet-bluetoothd  an application connected and will receive every
                            advertisement this machine hears
1254.864  ble-listen        reading the advertisement stream at
                            /run/rivet/listen/rivet-bluetoothd/advertisements
```

**So §6.2's whole argument is now a thing that happened**: a system service
provided an endpoint to an application, `provides` folded into `listen` exactly
as an application manifest's does, and **no new mechanism and no device class
was needed.** `rivet-bluetoothd` is the first image service to have done it.

The application was also **confined** while doing it — `ADR-042`'s ruleset is
built from its manifest and applied between fork and exec — so the granted
socket is reached through the grant rather than around it.

#### 8.3.4 Power: what was measured, and why it is not an answer

**No watts were measured and none are reported.** This board has no current
sensor and nothing in `ADR-035`'s catalogue reads one. What it will answer is an
SoC die temperature and a process's CPU time, and both were taken.

**CPU, which is a real cost and a small one.** Read off `rivet get process`
against the uptime the daemon had been running:

| process | CPU | over | share of one core |
| :-- | --: | --: | --: |
| `rivet-bluetoothd`, scanning | 1,990 ms | 752 s | **0.26 %** |
| `ble-listen`, consuming it | 810 ms | 326 s | **0.25 %** |

At ~18 advertisements a second that is the whole software cost of tier 1 on a
Pi 4: about half a percent of one of four cores for the platform and its
consumer together.

**Temperature, which turned out to measure the thermometer.** Sixteen readings
twenty seconds apart with the scan running, then the daemon and the consumer
disabled and sixteen more, with `aura-renderer` drawing throughout both:

| | scanning | quiet |
| :-- | --: | --: |
| mean | 64.79 °C | 64.70 °C |
| median | 64.76 °C | 64.76 °C |
| range | 63.78 – 66.22 | 64.27 – 65.73 |
| standard deviation | 0.55 | 0.39 |

The difference between the means is **0.09 °C**, against a per-phase standard
deviation five times that and a sensor whose readings quantise in steps of about
0.49 °C. **The honest reading is that the SoC thermal zone cannot tell a
scanning board from a quiet one**, not that a scan is free: the CYW43455 is a
separate part on the other side of the board from the die this sensor measures,
and a milliwatt-scale receiver would not show up here even if it were the whole
answer.

**One thing the board was already measuring said the same.** `aura-renderer`
journals a frame cost every five minutes (`AGENTS.md` §9.1's soak), and across
the same two phases it reported `DRAWING 22.38ms`, `22.41`, `22.42` and `22.38`
of its 33.3 ms budget. **The scan is not visible in the cost of the thing this
machine exists to do**, which is a different and more useful statement than the
thermometer's, because it is the load anybody would actually notice.

**And the wireless-contention half cannot be asked here at all.** `rivet get
network` reports `wlan0` **down, zero bytes received and zero transmitted** —
this board is on ethernet. §7's throughput question needs a board that uses its
wireless, and this is not one.

### 8.4 What is still not proven

**Narrowed by §9 the same night**, and what is left is written as what a board
would have to do rather than as what nothing has done.

* **Power, in watts.** §7 and §8.3.4. An inline meter is the instrument; nothing
  the board can be asked substitutes for one, and no emulator can be asked at
  all — the part that would draw the current is not there.
* **Whether a scan survives a reboot into a scan ON THE BOARD.** Every start in
  §8.3 was a document reload on a running machine. §9's two boots come up
  scanning from a document that was written before anything ran, so the
  *mechanism* — document, definition, service, five commands, scanning — is
  proven at boot. What that cannot speak to is the half that is the board's:
  whether `hci_uart` attaches the CYW43455 and takes its patchram **before** the
  supervisor starts the daemon, and what the daemon does if it does not. §6.4
  says it waits for `Index Added`, and §9.2 saw that path taken under emulation,
  which is the closest an emulator gets.
* **A consumer that cannot keep up, at the rate of real air.** §6.3's
  drop-and-count path was taken under emulation by a real daemon over a real
  granted socket (§9.3) and has still never been taken **on hardware**, because
  the one consumer that has run on the board kept up. The emulated consumer is
  also slow by refusing to read rather than by being busy, which is a fixture
  and not an application.
* **A second consumer on a board.** The refusal-by-name was seen under emulation
  (§9.4) and not on hardware.
* **Anything about tiers 2 and 3**, which is the point of deferring them.

**What changed on the board:** this is no longer a Bluetooth stack that has been
shown to attach to a radio. It is one that has been shown to hear, to hand what
it hears to an application that asked for it by name, and to cost about a
quarter of one percent of a core doing so.

---

## 9. What the emulator closed, and how

**Measured 2026-09-05 in the evening**, by `tools/qemu/adr-041-proof.sh`. Before
it, everything in §8 had happened on the board and **nothing had happened under
emulation**: no advertisement had ever crossed the endpoint in QEMU, and three
of §8.4's items were open because the only reachable evidence for them was a
unit test.

### 9.1 The fixture, and why a controller has to be a process

`tools/buildroot/linux/qemu-virt.config` has carried `CONFIG_BT_HCIVHCI` since
this ADR was written, for the reason its own block gives:
`drivers/bluetooth/hci_vhci.c` registers an HCI controller when a process opens
`/dev/vhci` and writes `HCI_VENDOR_PKT`, and **the process on the other end of
that descriptor is the controller**. It had no caller.

`tools/qemu/vhci-fixture` is that process, and `tools/qemu/run.sh --vhci` boots
it. Three things about it were found by running rather than by reading, and each
is a way for this to fail while looking like a machine with no radio:

1. **The create opcode must be zero.** Bit 7 is `HCI_QUIRK_RAW_DEVICE` and bit 6
   is `HCI_QUIRK_EXTERNAL_CONFIG`; either marks the controller
   `HCI_UNCONFIGURED`, `mgmt`'s Read Index List skips those, and
   `mgmt_index_added` returns early for a raw one. The daemon would then
   correctly report no adapter on a machine that has one.
   `docs/design-candidates/Bluetooth.md` §4.2 met the same wall from the other
   side and recorded it as `HCIDEVUP` answering `EOPNOTSUPP`.
2. **The kernel's own start-up interrogation must SUCCEED, not merely be
   answered.** `hci_cc_func` fails a request whose completion is shorter than
   the handler's `min_len`; `hci_dev_open_sync` then fails, `hci_power_on`
   returns early, and `HCI_SETUP` is never cleared — leaving a registered
   controller no index list will ever mention. The ten commands and the seven
   that need a body were **measured**, by padding every reply to 65 bytes and
   reading back the lengths the kernel named on the console.
3. **Read BD_ADDR answers `BDADDR_ANY`, and that is load-bearing.**
   `hci_power_on` closes a controller with no public and no static address
   immediately, and **before** it emits Index Added — so the adapter is already
   down when the daemon first sees it. With an address the kernel leaves it up
   for `HCI_AUTO_OFF_TIMEOUT` instead, and a daemon looking in that window would
   be refused the controller and exit, spending its restart budget on a
   controller the kernel had not yet let go of. It is also the truthful answer:
   a controller with no radio has no address.

**It is also PID 1 for about a millisecond.** A controller has to be a process
and this harness has nowhere to put one: `cmd/rivet-initd` spawns the supervisor
and nothing else, `cmd/rivet-sysd/src/definitions.rs` is product code that must
not learn about a fixture, and an application runs confined under a uid that
cannot open a `crw------- root root` node on `ADR-024`'s devtmpfs. So the
fixture is `rdinit=`, starts one child of itself, and **execs the real PID 1** —
which is therefore still PID 1, still reached with the argv it would have had.
The guest gains exactly one process and nothing about Rivet's start-up is
wrapped or reordered.

### 9.2 An advertisement crossed the endpoint

`examples/probes/ble-listen` — **the same program that took §8.3's
measurements**, not one written for the harness — printed twenty-four
advertisements in full and then rolled up:

```text
ble-listen: first 1 - AA:BB:CC:00:00:01 Public Connectable rssi -42dBm
            [01:06 09:52495645542d46495854555245]
ble-listen: heard 2261 advertisement(s) from 3 device(s), 0 line(s) that did not parse
ble-listen: device AA:BB:CC:00:00:01 x754 rssi -42dBm name `RIVET-FIXTURE`
```

**Every number in §9 is one run's, and re-running moves them by a few.** The
roll-up is written sixty seconds into a stream that lasts about seventy-six, and
the bounded run is cut short while the daemon is still draining — so 2,265 and
2,261 are two readings of the same machine, not a change in it. What does not
move is the third number: **0 that did not parse**.

The whole path ran: controller, kernel, user channel, daemon, granted socket,
application. `hci::Sequence` reached `Progress::Scanning` against a real kernel
and a real controller rather than against its own test, and the fixture prints
each command as it answers it, so the order in `cmd/rivet-bluetoothd/src/hci.rs`
is checked against something outside that file.

**Both boots came up scanning from the document**, with no reload: `[services.
rivet-bluetoothd] enabled = true` was written before anything ran. That is the
gesture §8.3.1 could not make on the board, where every start was an `apply
edit` and an `apply reload` on a running machine.

**What this is not.** Every byte was composed by the fixture, so *nothing here is
evidence about the air*. The three synthetic advertisers are `AA:BB:CC:…`, an
address no manufacturer holds, and the one that names itself says
`RIVET-FIXTURE` rather than imitating a sensor — a proof that produced a
plausible `GVH5075` line would be one somebody could mistake for the board's.

### 9.3 A consumer that cannot keep up, taken by a real process

`tools/qemu/ble-backpressure` reads five advertisements, prints them, and then
holds the endpoint and reads no more. Against twenty thousand injected, the
daemon reported

```text
"msg":"an application is not reading advertisements as fast as this machine hears
       them, so records were DROPPED rather than waited for"
"consumer":0,"dropped":1119
```

and **carried on reading**, which is the property §6.3 exists for.

**The five reads are the control and are not decoration.** Back-pressure and a
socket that never carried anything report identically, so both halves are
asserted about one socket, one consumer and one boot (`AGENTS.md` §7). The drop
count is a snapshot at the moment the bounded run was cut short, not a tally.

### 9.4 A second consumer, refused by name

Two applications declaring `connects = ["rivet-bluetoothd:advertisements"]` were
enabled on one boot — **the same binary under two names**, so the only
difference between them is which one `endpoints::resolve` reached first. The
first was granted the socket and read it; the second was told

> ble-second-consumer wants rivet-bluetoothd:advertisements, which is already
> granted to ble-listen

and **was not started**. §6.2's exclusivity and `ADR-015` §5.3's one-consumer
limit are now a thing that happened rather than a list of names in a unit test.

**Removing the refusal is worse than it looks, and that was measured too.** With
the `AlreadyGranted` branch deleted, both applications are granted the endpoint,
both chowns run, and the loser meets `EACCES` on a socket its manifest says it
may open — an intermittent permission error instead of a named refusal, with the
first consumer restarting under it. That is the failure the refusal exists to
convert into a sentence.

**The rule this run observed was first-come, and it has changed** (`ADR-015`
§5.3, `.rulings/` R-210 D3): an unshared endpoint two applications claim is now
refused to both, and each refusal names both. `tools/qemu/adr-041-proof.sh`'s
stage 1b asserts that rule on a boot of its own, with `ble-listen` alone as the
uncontested control beforehand.

### 9.5 Every assertion above was broken on purpose first

`AGENTS.md` §4: a test that cannot fail is not a test. Three properties were
broken, watched to fail, and restored.

| what was broken | what the proof then said |
| :-- | :-- |
| the fixture writes the `BD_ADDR` in the order a person writes it, not least significant byte first | the consumer printed `01:00:00:CC:BB:AA` — a plausible address belonging to nobody, which is the failure mode that does not error |
| `stream.rs`'s `WouldBlock` arm returns `Delivery::Sent` and counts nothing | the five reads still passed and **no drop record appeared at all** — the exact shape where a control alone would look fine |
| `endpoints::resolve` no longer refuses a second consumer | neither *"already granted to ble-listen"* nor *"was NOT started"* appeared |

---

## History

### 2026-10-04: the controller is re-acquired by a restart, not in-process

Until R-253 the daemon ran as uid 0 with every capability and, when its
controller went away, returned to the management socket inside the same
process to wait for another. `cmd/rivet-bluetoothd/src/run.rs` stated the rule:

> ~~If that controller goes away — a dongle unplugged, a driver unbound — the
> socket reads end and the daemon returns to waiting on the management socket
> rather than dying.~~

**Why it changed.** Re-binding needs `CAP_NET_ADMIN`, so a daemon that
re-acquired in-process had to hold it for its whole life, including while it
parsed every advertisement in range. R-253 had it hold the capability only until
the bind and leave the re-acquire to the supervisor's restart, which §6.6
records. The wait for `Index Added` (§6.4) is unchanged; it now happens at the
start of each run rather than in a loop within one.

### The 2026-09-05 morning form of §8.3, before the daemon had run

Superseded the same day by §8.3 above, and kept because it is the better half
of the record: it named four measurements and what would close each, and every
one of them was closed by taking exactly the reading it asked for.

> ### 8.3 NOT proven, and exactly what closes each
>
> * **`rivet-bluetoothd` has never run on the board.** It ships disabled and was
>   not enabled on the boot above. Enabling it and reading its journal is the next
>   step, and it is small.
> * **No advertisement has been received from a real radio.** Every parse this
>   daemon has done was against QEMU's virtual controller or a byte array. **The
>   maintainer's Govee sensors are the test**, and one enabled boot within range
>   of them settles it.
> * **No application has consumed the endpoint.** The `connects` path is asserted
>   by tests and has not carried a real descriptor to a real consumer.
> * **Nothing is known about power.** §7.
>
> **Until those four, this is a Bluetooth stack that has been shown to attach to a
> radio — not one that has been shown to hear anything.**
