# Bluetooth on Rivet

* **Status:** Design candidate — not accepted. Every load-bearing claim below
  was measured on 2026-09-05; nothing here is an opinion about what would
  probably happen.
* **Date:** 2026-09-05
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Sandbox:** Docker volume `rivet-br-bt`, cache tag `-bt`. The real
  `tools/buildroot/configs/rivet_aarch64_defconfig` and the two committed
  kernel fragments were **not** modified; §3.2 carries the diff that was used.

---

## 1. Context, and what this document is for

The maintainer asked, on 2026-09-05, whether Rivet can have Bluetooth and what
the road to it is. He asked for a roadmap or a named obstacle and nothing in
between, and he was explicit that a kernel can be recompiled with Bluetooth in
it, that this project already has the cross toolchain, and that it has an
emulator that runs things — so the answer was to be built rather than reasoned
about.

**The answer is yes, and there are two roads rather than one.** Both were
built. Both ran. This document says what each cost, what each settles, and
which of the four capabilities the maintainer named each one reaches.

**Four capabilities, because they differ by orders of magnitude** and a single
verdict about "Bluetooth" would be worthless:

1. **BLE observer** — reading advertisements. No pairing, no GATT. This is what
   the maintainer's Govee sensors need.
2. **BLE central** — connect, discover services, read and write GATT
   characteristics.
3. **Classic HID host** — game controllers.
4. **BLE peripheral** — advertising and serving GATT.

§7 answers each one separately. §8 is what was built and run. §9 is the
roadmap. §10 is the one thing that is a wall.

---

## 2. What the record said, and the part of it that was wrong

Three statements in the record bear on this, and checking them was the first
thing done.

### 2.1 The kernel already had Bluetooth compiled, as modules, and nothing could load them

`tools/buildroot/linux/rivet.config` says nothing about Bluetooth, so the
question "is `CONFIG_BT` on?" reads as "no". It is not no. Read out of the
kernel `.config` that the **currently shipping** kernel was built from —
`/br/build/linux-custom/.config` in the production `rivet-br` volume:

```
CONFIG_BT=m            CONFIG_BT_BREDR=y      CONFIG_BT_LE=y
CONFIG_BT_HCIVHCI=m    CONFIG_BT_HCIUART=m    CONFIG_BT_HCIUART_BCM=y
CONFIG_BT_HIDP=m       CONFIG_UHID=m          CONFIG_BT_RFCOMM=m
CONFIG_SERIAL_DEV_BUS=y                       CONFIG_BT_HCIBTUSB=m
```

All of it is inherited from the `bcm2711` defconfig, and all of it is `=m`.
`ADR-024` §4 rules that everything is built in and there are no modules; a Rivet
image has no `/lib/modules`. Read off the test board over the control plane the
same day, `/proc/modules` is **empty**. So the whole stack has been compiled
into every card this project has ever produced, installed nowhere, and is
unreachable.

**This is `rivet.config`'s own `CONFIG_DRM_VC4` lesson in a second subsystem**,
and the third time this shape has been found — the first was the wireless stack
in the stock Pi OS kernel, the second was VC4 silently downgraded to `m`. A
tristate that lands as `m` on a machine with no module loader is an absent
driver that looks present in a `.config`.

### 2.2 `docs/SPEC-01` §5.2 names a file that does not exist in this kernel, in either direction

`SPEC-01` §5.2 records **BlueZ, and Bluetooth in the kernel at all** as absent,
with the evidence *"`/proc/net/bluetooth` and `/sys/class/bluetooth` are
absent, measured 2026-08-31"*.

**The conclusion is right and half the test is invalid.** There is no
`/proc/net/bluetooth` in Linux 6.12 at all. The kernel's own
`net/bluetooth/hci_sock.c` registers its procfs entry as

```c
err = bt_procfs_init(&init_net, "hci", &hci_sk_list, NULL);
```

so the files are `/proc/net/hci`, `/proc/net/l2cap`, `/proc/net/sco` and
`/proc/net/hidp`. Measured in the guest on a kernel with the **full** stack
built in, `/proc/net/bluetooth` is still `ENOENT`. A test that returns "absent"
on a machine that has the thing cannot tell absence from presence, which is
`AGENTS.md` §7's failure with the polarity that is usually missed.

The half that is sound is `/sys/class/bluetooth`, and it is decisive:

| | test board, 2026-09-05 | QEMU guest, `CONFIG_BT=y` |
| :-- | :-- | :-- |
| `/sys/class/bluetooth` | **absent** | present |
| `/proc/net` entry count | 35, no `hci`/`l2cap`/`sco`/`hidp` | 39, all four present |
| `/sys/class/rfkill` | 1 entry (the wireless) | 2 entries |
| `/dev/vhci` | absent | present |
| `/proc/modules` | empty | — |

§11 says what this document owes `SPEC-01`.

### 2.3 The defconfig's reason for declining the BT firmware is superseded, not wrong

`tools/buildroot/configs/rivet_aarch64_defconfig`, near line 273, declines
`BR2_PACKAGE_BRCMFMAC_SDIO_FIRMWARE_RPI_BT` because *"there is no Bluetooth
requirement anywhere in the record"*. That was true and is the reason it should
be read as **superseded on 2026-09-05** rather than as a finding: the
requirement now exists because the maintainer created it. `ADR-009` §4's rule
about carrying what nothing uses was applied correctly to the facts of the day.

---

## 3. The kernel: measured, not argued

### 3.1 What it produced

`linux-reconfigure` against a fragment that asks for the whole stack at `=y`.
**Every requested symbol survived the merge** — read back out of
`/br/build/linux-custom/.config` *after* the build, which is what
`tools/buildroot/build.sh --show-config` exists to do:

```
  CONFIG_BT                          CONFIG_BT=y
  CONFIG_BT_BREDR                    CONFIG_BT_BREDR=y
  CONFIG_BT_LE                       CONFIG_BT_LE=y
  CONFIG_BT_LE_L2CAP_ECRED           CONFIG_BT_LE_L2CAP_ECRED=y
  CONFIG_BT_DEBUGFS                  CONFIG_BT_DEBUGFS=y
  CONFIG_BT_HIDP                     CONFIG_BT_HIDP=y
  CONFIG_UHID                        CONFIG_UHID=y
  CONFIG_BT_HCIVHCI                  CONFIG_BT_HCIVHCI=y
  CONFIG_SERIAL_DEV_BUS              CONFIG_SERIAL_DEV_BUS=y
  CONFIG_SERIAL_DEV_CTRL_TTYPORT     CONFIG_SERIAL_DEV_CTRL_TTYPORT=y
  CONFIG_BT_HCIUART                  CONFIG_BT_HCIUART=y
  CONFIG_BT_HCIUART_SERDEV           CONFIG_BT_HCIUART_SERDEV=y
  CONFIG_BT_HCIUART_H4               CONFIG_BT_HCIUART_H4=y
  CONFIG_BT_HCIUART_BCM              CONFIG_BT_HCIUART_BCM=y
  CONFIG_BT_HCIBTUSB                 CONFIG_BT_HCIBTUSB=y
  CONFIG_CRYPTO_USER_API_HASH        CONFIG_CRYPTO_USER_API_HASH=y
  CONFIG_CRYPTO_USER_API_SKCIPHER    CONFIG_CRYPTO_USER_API_SKCIPHER=y
  CONFIG_CRYPTO_CMAC                 CONFIG_CRYPTO_CMAC=y
```

`CONFIG_CRYPTO_ECDH` and `CONFIG_CRC16` were pulled to `=y` by kconfig `select`
without being asked for. `CONFIG_BT_RFCOMM`, `CONFIG_BT_BNEP` and
`CONFIG_BT_6LOWPAN` were deliberately **not** asked for and stayed `=m`, which
is why the guest answers `EPROTONOSUPPORT` for `BTPROTO_RFCOMM` and BlueZ's
network plugin says *"kernel lacks bnep-protocol support"*. That negative is
useful: it proves the probe can tell a present protocol from an absent one.

### 3.2 The diff that was used

Two files, neither committed. First, a copy of the defconfig with exactly one
line changed:

```diff
-BR2_LINUX_KERNEL_CONFIG_FRAGMENT_FILES="$(BR2_EXTERNAL_RIVET_PATH)/linux/rivet.config $(BR2_EXTERNAL_RIVET_PATH)/linux/qemu-virt.config"
+BR2_LINUX_KERNEL_CONFIG_FRAGMENT_FILES="$(BR2_EXTERNAL_RIVET_PATH)/linux/rivet.config $(BR2_EXTERNAL_RIVET_PATH)/linux/qemu-virt.config $(BR2_EXTERNAL_RIVET_PATH)/linux/bt.config"
```

Second, `tools/buildroot/linux/bt.config`:

```
CONFIG_BT=y
CONFIG_BT_BREDR=y
CONFIG_BT_LE=y
CONFIG_BT_LE_L2CAP_ECRED=y
CONFIG_BT_DEBUGFS=y
CONFIG_BT_HIDP=y
CONFIG_UHID=y
CONFIG_BT_HCIVHCI=y
CONFIG_SERIAL_DEV_BUS=y
CONFIG_SERIAL_DEV_CTRL_TTYPORT=y
CONFIG_BT_HCIUART=y
CONFIG_BT_HCIUART_SERDEV=y
CONFIG_BT_HCIUART_H4=y
CONFIG_BT_HCIUART_BCM=y
CONFIG_BT_HCIBTUSB=y
CONFIG_CRYPTO_USER_API_HASH=y
CONFIG_CRYPTO_USER_API_SKCIPHER=y
CONFIG_CRYPTO_CMAC=y
```

For a shipped kernel these belong in `tools/buildroot/linux/rivet.config`, not
in a third fragment — `ADR-024` §5's split is product requirements against the
development loop, and Bluetooth is a product requirement. `CONFIG_BT_HCIVHCI`
is the exception and belongs in `qemu-virt.config`: it is a controller with no
radio and exists so the loop can measure the stack.

### 3.3 What it costs

| | baseline | with Bluetooth | delta |
| :-- | --: | --: | --: |
| `images/Image` | 29 467 136 B | 30 200 320 B | **+733 184 B** (+716 KiB) |
| the same, `xz -9` | 8 742 784 B | 8 992 796 B | **+250 012 B** (+244 KiB) |

**Method:** both kernels were built in the same tree, on the same day, minutes
apart, by `linux-reconfigure` from the two defconfigs; nothing else differs.
The compressed figure is the one that matters for an update payload
(`ADR-018` §10.5's method), and `ADR-018` §4's `/system` slots are 640 MiB, so
the nominal image size does not move.

### 3.4 The Broadcom firmware half

`BR2_PACKAGE_BRCMFMAC_SDIO_FIRMWARE_RPI_BT=y` installs, in 6 s. The file the
Pi 4 Model B's radio wants is reached through a symlink:

```
BCM4345C0.raspberrypi,4-model-b.hcd -> BCM4345C0.hcd     63 806 B
```

**Two things follow, and both are already-known shapes.** The package installs
symlinks rather than files, exactly as the defconfig's comment predicts for the
wireless half, so resolving them into real files under the names the driver
requests is image assembly's job (`ADR-009` §6's boundary). And the filename
above is **documentation-derived**: `AGENTS.md` §5 says firmware filenames are
copied from a running system, and no board has yet asked for this one. The
first board boot with `hci_uart` in it is what confirms it.

---

## 4. Userland: two roads, both built

### 4.1 The BlueZ road — D-Bus builds, BlueZ builds, and both RAN under Rivet's PID 1

This was the crux and it is settled.

**D-Bus builds in 19 seconds.** `package/dbus/Config.in` has no
`!BR2_STATIC_LIBS` dependency; it needs threads and an MMU and selects `expat`,
and the Bootlin musl toolchain satisfies all three. Out came `dbus-daemon`
(278 880 B) and `libdbus-1.so.3` (433 736 B).

**BlueZ 5.79 builds too**, with one failure that is not about Bluetooth. The
first attempt failed at `client/bluetoothctl`:

```
/br/host/.../ld: .../libreadline.a(terminal.o): in function `_rl_init_terminal_io':
terminal.c:(.text+0xa64): undefined reference to `UP'
terminal.c:(.text+0xb24): undefined reference to `tgetent'
...
collect2: error: ld returned 1 exit status
make[2]: *** [Makefile:5793: client/bluetoothctl] Error 1
```

That is `libreadline.a` wanting termcap symbols that nothing put on the link
line — a readline/ncurses packaging fault in this configuration, reached only by
the readline-linked programs (`bluetoothctl`, `bluetooth-player`, `obexctl`,
`mpris-proxy`). `src/bluetoothd` had **already linked** when it fired.
Dropping `BR2_PACKAGE_BLUEZ5_UTILS_CLIENT` and reconfiguring produced a
complete install in 11 s:

| | bytes |
| :-- | --: |
| `bluetoothd` | 1 415 728 |
| `btmon` | 1 117 776 |
| `libbluetooth.so.3` | 229 976 |
| `btattach` | 37 432 |
| `l2ping` | 19 080 |

**And then it ran.** `tools/qemu/run.sh` booted `rivet-initd` as PID 1 with a
probe standing in for the supervisor, the libraries packed into `/lib` by
`--libraries`, and the two daemons staged under `/state`. `bluetoothd` was
started as an ordinary supervised child — no bus activation, no systemd, no
udev, no FHS beyond the directories the probe made:

```
bluetoothd[64]: Bluetooth daemon 5.79
bluetoothd[64]: src/main.c:parse_config() parsing /state/etc/main.conf
bluetoothd[64]: src/adapter.c:adapter_init() sending read version command
bluetoothd[64]: Starting SDP server
bluetoothd[64]: src/plugin.c:add_plugin() Loading input plugin
bluetoothd[64]: src/plugin.c:add_plugin() Plugin input loaded
bluetoothd[64]: src/plugin.c:add_plugin() Loading hog plugin
bluetoothd[64]: src/plugin.c:add_plugin() Plugin hog loaded
bluetoothd[64]: src/main.c:main() Entering main loop
bluetoothd[64]: Bluetooth management interface 1.23 initialized
bluetoothd[64]: src/adapter.c:read_commands_complete() Number of commands: 88
bluetoothd[64]: src/adapter.c:read_commands_complete() Number of events: 44
bluetoothd[64]: src/adapter.c:read_index_list_complete() Number of controllers: 0
```

and answered a real method call over the system bus:

```
BTPROBE bus: dbus-send exit Some(0)
BTPROBE   reply| method return time=... sender=:1.0 -> destination=:1.1 ...
BTPROBE   reply|          object path "/org/bluez"
BTPROBE   reply|                string "org.bluez.AgentManager1"
BTPROBE   reply|                string "org.bluez.ProfileManager1"
```

*"Number of controllers: 0"* is correct and is QEMU's limit, not Rivet's: no
virtual controller ever completed its init sequence, because there is no radio
to answer `HCI_Reset` (§4.2). Everything above it — the bus, the daemon, the
plugin set, the management socket, the object tree — is settled.

**It failed twice first, and both failures are the finding.** They are what a
message bus demands of Rivet's shape:

| What it demanded | The failure, verbatim | Where Rivet stands |
| :-- | :-- | :-- |
| a password database | `dbus-daemon[62]: Could not get password database information for UID of current process: User "???" unknown` — every connection then timed out | **Rivet already has this.** `/etc/passwd` is a projection (`ADR-007` §3.6.2). The harness runs the probe *instead of* `rivet-sysd`, which is the thing that writes it; supplying the two lines by hand made the bus work |
| a machine ID | it refuses without one | `/var/lib/dbus/machine-id` or `/etc/machine-id`. **Rivet's `/etc` is read-only**, so this is a line the projection must carry or a path under `/state` |
| a config file naming a listen address | `--config-file specified but configuration file /usr/share/dbus-1/system.conf already requested` when `--system` is also passed | a Rivet service definition passes `--config-file` and not `--system`; the config is a projection or a `/system` file |
| a writable `/var/lib/bluetooth` | it did not complain here because the probe made the directory | **This one is real.** BlueZ's link-key store is a compile-time `--localstatedir`. `ADR-008` has no `/var`; this needs either a Buildroot `--localstatedir` under `/state` or a bind mount |

### 4.2 The socket road — a Rust binary spoke HCI, with no BlueZ and no bus

The alternative is that Rust talks to the kernel's Bluetooth sockets directly.
The whole point of `CONFIG_BT_HCIVHCI` is that this can be measured with no
radio at all: `hci_vhci.c` creates a controller when userspace opens
`/dev/vhci` and writes `HCI_VENDOR_PKT`, and a process on the other end of that
file descriptor **is** the controller.

A 480 KB static Rust binary, built against `aarch64-unknown-linux-musl` with
`libc` and nothing else, running as the supervisor on the real target kernel.
What it measured:

```
BTPROBE socket BTPROTO_HCI SOCK_RAW: OK fd=3
BTPROBE socket BTPROTO_L2CAP SOCK_SEQPACKET: OK fd=4
BTPROBE socket BTPROTO_SCO SOCK_SEQPACKET: OK fd=5
BTPROBE socket BTPROTO_RFCOMM SOCK_STREAM: REFUSED EPROTONOSUPPORT (93)
BTPROBE socket BTPROTO_HIDP SOCK_RAW: OK fd=6
BTPROBE mgmt: bind HCI_CHANNEL_CONTROL OK
BTPROBE mgmt: read 12 bytes [01, 00, ff, ff, 06, 00, 01, 00, 00, 01, 17, 00]
BTPROBE   mgmt Read Version -> version 1 revision 23
BTPROBE l2cap: bind ATT cid (BDADDR_ANY, LE public) OK
```

**That fourth line is the whole management API.** `HCI_CHANNEL_CONTROL` is the
socket BlueZ itself uses; `bluetoothd` reported the same interface version
minutes later as *"Bluetooth management interface 1.23 initialized"*, from the
other road, on the same kernel. A Rust program that binds that socket has the
same access to the adapter that `bluetoothd` has, with no daemon, no bus and no
C in the picture.

The data path was proven end to end as well, using `HCI_CHANNEL_USER` — the
mode that takes a controller away from the kernel's own stack:

```
BTPROBE user: bind hci0 HCI_CHANNEL_USER OK — the adapter is now this process's
BTPROBE user: wrote 4 bytes, HCI Reset (opcode 0x0c03)
BTPROBE user: the CONTROLLER side received 4 bytes: [01, 03, 0c, 00]
BTPROBE user: answered with Command Complete, status 0x00
BTPROBE user: the RUST side received 7 bytes back: [04, 0e, 04, 01, 03, 0c, 00]
```

A command composed in Rust reached the transport, and an event written to the
transport reached Rust. That is every layer of HCI except the radio.

**Two negatives worth keeping**, because they say where the emulator stops:

* `HCIDEVUP` on a raw controller is `EOPNOTSUPP` — `hci_register_dev` marks a
  `HCI_QUIRK_RAW_DEVICE` controller `HCI_UNCONFIGURED`, and an unconfigured
  controller is opened through `HCI_CHANNEL_USER` rather than through
  `HCIDEVUP`. That is the kernel behaving correctly, not a defect.
* `HCIDEVUP` on a normal controller is `ETIMEDOUT` after 3 537 ms, with
  `Bluetooth: hci1: Opcode 0x0c03 failed: -110` on the console. The kernel sent
  `HCI_Reset` and nothing answered. **A real radio answers.** This is
  `ADR-026` §4's right-hand column: whether the CYW43455 comes up is the
  board's to settle and no emulator's.

### 4.3 The crates, named — and which of them are BlueZ wearing a Rust coat

Every row was cross-compiled for `aarch64-unknown-linux-musl` in the build
container. "Built" means `cargo build --target aarch64-unknown-linux-musl`
returned zero.

| Crate | Result | What it actually is |
| :-- | :-- | :-- |
| `bluer` 0.17.4, default features | **BUILT** | `default = []`. With no features it is types and two socket modules. Nothing named `dbus`, `zbus` or `glib` appears anywhere in the dependency tree |
| `bluer` 0.17.4, `bluetoothd` | **FAILED** — `libdbus-sys` panics on pkg-config | **The Rust coat.** `grep '^#\[cfg(feature = "bluetoothd")\]' src/lib.rs` gates `adapter`, `adv`, `agent`, `device`, `gatt`, `monitor`, `session` — that is *everything* except `l2cap`, `rfcomm` and `id`. Adapters, scanning and GATT are D-Bus calls to `bluetoothd` |
| `bluer::l2cap` | BUILT (part of the above) | Genuine L2CAP sockets: `Stream`, `SeqPacket`, `Datagram`, `SecurityLevel`. Real, and not enough on its own — it can carry ATT once a connection exists, and nothing in it makes a connection |
| `btleplug` 0.11.8 | **FAILED**, then **BUILT** — see below | On Linux it is `dbus` → `dbus-tokio` → `libdbus-sys` → BlueZ. A binding |
| `bluez-async` 0.7 | **FAILED** — same `libdbus-sys` wall | A binding, and the name says so |
| `bluster` (latest) | **FAILED** — same wall | A binding |
| `bt-hci` 0.2.1 | **BUILT** | A complete HCI command/event/parameter codec with a **transport trait**. No Linux transport ships with it. This is the codec the socket road needs |
| `trouble-host` (latest) | **BUILT** | A BLE host — central, peripheral, GATT — built on `bt-hci`. Also transport-agnostic |
| `burble` 0.1.2 | FAILED, then **BUILT** with `tools/toolchain/env.sh` sourced | A pure-Rust BLE stack that reaches a controller through `rusb`, i.e. **a USB dongle**. It cannot drive the Pi 4's UART radio |

**The `libdbus-sys` wall is not what `SPEC-01` §6 thinks it is, and it is
removable.** That section records the TempMon failure — *"pkg-config has not
been configured to support cross-compilation"* — as rung 2 of `ADR-009` §3's
ladder. It is not. With `libdbus-1` now in the Buildroot staging tree and a
twelve-line shell script named `pkg-config` on `PATH` answering for `dbus-1`,
the build completes:

```
#### RESULT btleplug-with-a-pkg-config: BUILT
   Compiling libdbus-sys v0.2.7
   Compiling dbus v0.9.12
   Compiling dbus-tokio v0.7.6
   Compiling btleplug v0.11.8
```

The container has **no `pkg-config` binary at all** — with `PKG_CONFIG_SYSROOT_DIR`
and `PKG_CONFIG_LIBDIR` set the error changes to *"Try `apt install
pkg-config`"*. So the blocker is one missing host tool plus a cross wrapper, not
anything about musl, the toolchain, or Rivet. A real fix is a
`aarch64-linux-pkg-config` wrapper in `tools/container/Dockerfile`, which is a
build-environment change and not a Rivet one.

---

## 5. What Rivet's architecture forbids, and what it merely makes work

| Rivet property | BlueZ road | Socket road |
| :-- | :-- | :-- |
| No FHS (`ADR-008` §3) | `bluetoothd` is a single binary plus `main.conf`; `--localstatedir` is compile-time and points at `/var/lib/bluetooth` — **the one real problem**, see §10.2 | nothing to place |
| `/etc` is a read-only projection (`ADR-007` §3.6.2) | needs `passwd`, `group` and a machine ID. The first two already exist; the third is a new line in the projection | nothing |
| No package manager | both arrive as build-time image content, which is what `ADR-008` §4's add-ons are for | — |
| Services supervised, never bus-activated (`ADR-017` §4) | proven: both daemons started as ordinary children and neither noticed | — |
| No udev | BlueZ does not need it; it uses the management socket and rfkill, both of which answered | — |
| Static musl, `BR2_SHARED_STATIC_LIBS` | `bluetoothd` is dynamic and needs five `.so` files. Shared Rivets already put the musl loader on the image, so this is a closure to declare, not a new mechanism | a static Rust binary |
| `ADR-001` §4 tiering | `dbus`, `glib2`, `expat`, `pcre2` and BlueZ are **five new Tier C members**, and `ADR-001` §4.2's argument applies to BlueZ exactly as it does to `wpa_supplicant`: depending on it is what keeps Rivet current with BLE profiles nobody here will implement | zero new C |
| The standing position on buses | the maintainer is not opposed to a bus on principle and does not want one adopted before other options have been explored *(`.rulings/` R-043.)*. **This document is that exploration**, and it found a second option that works | this is the other option |

**One sentence on the trade, because it is the decision and it is the
maintainer's.** The BlueZ road buys every profile that exists — A2DP, HID, HoG,
mesh, everything — for 1.5 MB compressed and five Tier C dependencies including
a message bus. The socket road buys exactly what gets written, for zero new C,
and costs Rust that nobody else maintains.

---

## 6. Pairing, and what it means with no shell and often no keyboard

**Framed, not solved.** Three facts and one shape:

1. **Bonding needs a human moment.** Classic pairing wants a PIN or a
   "yes, that number matches"; BLE Secure Connections wants Just Works, numeric
   comparison or passkey entry. In BlueZ that is an *agent* — a D-Bus object
   implementing `org.bluez.Agent1` that the daemon calls back. `AgentManager1`
   appeared in the object tree in §4.1, so the mechanism is present and
   unclaimed. On the socket road it is the management socket's
   `User Confirmation Request` and `User Passkey Request` events.
2. **Link keys are secrets at rest.** They are the credential that lets a device
   reconnect without asking again, and they belong under `ADR-020` — a named
   entry in the store, not a directory of files BlueZ owns. BlueZ's own store is
   plain files under `/var/lib/bluetooth`, which is the opposite of `ADR-020` §5
   and is the second half of §10.2's problem.
3. **Rivet already has a human, and it is not a keyboard.** `ADR-012` makes an
   agent a first-class operator and §9 of it rules that pressing a button is an
   operation; `ADR-015` §6.2 puts a kiosk launcher on the screen. So the natural
   shape is **an `apply` operation that opens a bonding window** — the operator
   (human at a console, agent over the control plane, or a launcher screen)
   authorises pairing for N seconds, and the confirmation is answered by that
   same operator surface rather than by a `bluetoothctl` prompt that has nowhere
   to run.

That shape is a decision about Rivet's administrative surface and is the
maintainer's (`AGENTS.md` §1). It is not a technical obstacle: both roads
expose the callback.

---

## 7. Capability by capability

### 7.1 BLE observer — reading advertisements

**Roadmap.** Kernel: `CONFIG_BT` + `CONFIG_BT_LE`, both proven `=y`, plus
`CONFIG_BT_HCIUART_BCM` for the radio. Userland: none required. The management
socket answered from Rust with no daemon; passive scanning is `Start Discovery`
on that socket and `Device Found` events back, or `LE Set Scan Enable` down an
`HCI_CHANNEL_USER` socket whose round trip §4.2 measured.

The unknown left is whether the CYW43455 attaches and scans, and only the board
settles it.

### 7.2 BLE central — connect, GATT, characteristics

**Roadmap, and the two roads separate here.**

* **With BlueZ:** free. `bluetoothd` builds, runs, and publishes
  `org.bluez.Device1` and `GattCharacteristic1`. An application reaches it with
  `btleplug` or `bluer`'s `bluetoothd` feature, both of which cross-build once
  the container has a cross `pkg-config`.
* **Without BlueZ:** `bt-hci` gives the codec and `trouble-host` gives a host
  stack, both cross-build today, and neither ships a Linux transport. The
  missing piece is an adapter between `trouble-host`'s transport trait and an
  `HCI_CHANNEL_USER` socket — and §4.2 proved that socket carries commands and
  events in both directions. That is the single most useful piece of Rust this
  study identifies.

### 7.3 Classic HID host — game controllers

**Roadmap, and it is the shortest of the four.** Most of it is already in the
kernel and was already `=y` before this study: `HID`, `HID_GENERIC`, `HIDRAW`,
`USB_HID`, `INPUT_EVDEV`, and `INPUT_JOYDEV`/`JOYSTICK_XPAD` since 2026-08-31.
What Bluetooth adds is `CONFIG_BT_BREDR`, `CONFIG_BT_HIDP` and `CONFIG_UHID`,
all now proven `=y`.

Userland: `bluetoothd`'s `input` plugin, which **loaded** in §4.1. It creates a
kernel HID device through `/dev/uhid` — present in the guest, mode `0600
root:root` — and the controller then appears at `/dev/input/eventN` like any
other gamepad, which is what a Raylib kiosk application already reads.

This tier is the one where the BlueZ road is clearly worth its weight: HID over
Bluetooth Classic is SDP, L2CAP on two fixed PSMs, and a bonding dance, and
re-implementing it in Rust buys nothing.

### 7.4 BLE peripheral — advertising and serving GATT

**Roadmap, materially the same as §7.2 and one step behind it.** With BlueZ it
is `org.bluez.LEAdvertisingManager1` and `GattManager1`, both compiled in.
Without it, `trouble-host` is a peripheral stack as well as a central one, so
the same transport adapter serves both. Nothing about it is separately blocked;
it is simply the last thing to want.

---

## 8. What was BUILT and RUN, and what happened

**Environment:** Docker on the Windows workstation, 28 CPUs and 15.5 GiB of
RAM reported by `docker info`. `JLEVEL=28` throughout. `rivet-buildroot:1` for
Buildroot, `rivet-card-build:4` for cargo and QEMU. Output volume
`rivet-br-bt`, a **warm** tree with the Bootlin toolchain and a previously
built kernel already in it — every figure below is an incremental build and
none of them prices a from-scratch bootstrap.

### 8.1 Buildroot

| Run | Target | **Wall clock** | Outcome |
| :-- | :-- | --: | :-- |
| baseline | `linux`, unmodified defconfig | **80 s** | `Image` 29 467 136 B — byte-for-byte the size the tree already held |
| bt1 | defconfig regenerated, then `linux` | **6 s** | **SILENT NO-OP.** See §8.2 |
| bt2 | `linux-reconfigure` | **80 s** | `Image` 30 200 320 B, every symbol `=y` |
| dbus | `dbus` | **19 s** | built, staged and installed. Downloaded `dbus-1.14.10` in the same 19 s |
| bluez | `bluez5_utils` | **105 s** | **FAILED** at `client/bluetoothctl`; `src/bluetoothd` had already linked |
| bluez2 | same target after editing the defconfig | **6 s** | **SILENT NO-OP again.** §8.2 |
| bluez3 | `bluez5_utils-reconfigure`, no CLIENT | **11 s** | built and installed |
| fw | `brcmfmac_sdio-firmware-rpi-rebuild` | **6 s** | BT firmware installed |
| baseline2 | `linux-reconfigure` back to baseline | **83 s** | `Image` 29 467 136 B, `CONFIG_BT=m` — the tree returned to exactly where it started |

**Total Buildroot wall clock for the whole study: 396 s — six minutes and
thirty-six seconds.** These appear to be the first Buildroot timings this
repository has recorded; `tools/buildroot/build.sh`'s own header and
`tools/container/build-buildroot.sh`'s *"it takes hours"* are about the
cold case — a `--fetch`, a toolchain and every package — and they are right
about that and say nothing about this one. **A kernel experiment against a warm
tree is a ninety-second edit-and-measure loop, and this study did nine of
them.** Nothing was declined on cost.

Disk: the sandbox tree grew from 6.4 GB to about 17 GB across the study; the
host had 127 GB free and never came close.

### 8.2 The trap that cost two runs, and it is not the one the script warns about

`tools/buildroot/build.sh --reconfigure` regenerates **Buildroot's** `.config`.
It does not regenerate the **kernel's**. Run bt1 did exactly what the header
promises — regenerated from the defconfig, printed the new
`BR2_LINUX_KERNEL_CONFIG_FRAGMENT_FILES` line into `/br/.config`, ran `make
linux`, and exited 0 in six seconds with the kernel `.config` still saying
`CONFIG_BT=m`.

The mechanism is `package/pkg-kconfig.mk`:

```
$$($(2)_DIR)/$$($(2)_KCONFIG_STAMP_DOTCONFIG): $$($(2)_KCONFIG_FILE) $$($(2)_KCONFIG_FRAGMENT_FILES)
```

The kernel is reconfigured when a fragment's **mtime** is newer than the stamp.
Editing `rivet.config` in place triggers it — which is why the `CONFIG_DRM_VC4`
work on 2026-08-31 never met this. **Adding a fragment that is older than the
last configure does not**, and neither does changing which fragments are
listed. Run bluez2 is the same failure in a package: changing
`BR2_PACKAGE_BLUEZ5_UTILS_CLIENT` and re-running the target resumed the old
`configure`.

`make <pkg>-reconfigure` is the answer in both cases, and a build reported
green after six seconds is the tell.

### 8.3 QEMU

Four boots, `-M virt` under TCG, `RIVET_KERNEL` pointed at the sandbox tree's
`images/Image`, `rivet-initd` as PID 1 with the probe at `SUPERVISOR_PATH`.
Each was bounded by `timeout`; the probe finished its work by roughly 20 s of
guest time and then parked so PID 1 would not reap it.

| Boot | What it added | Result |
| :-- | :-- | :-- |
| 1 | sockets, ioctls, mgmt, vhci creation | everything in §4.2 except the user channel |
| 2 | `HCI_CHANNEL_USER`, `/proc/net` listing | the full HCI round trip; `/proc/net/bluetooth` shown to be the wrong test |
| 3 | staged D-Bus and BlueZ | `dbus-daemon` refused its own arguments; then refused every connection for want of a password database |
| 4 | `/etc/passwd` and `/etc/group` supplied | **bluetoothd 5.79 on the bus, answering `GetManagedObjects`** |

Boot 3's two failures are in §4.1's table verbatim. Neither is a Rivet
limitation and one of them is a Rivet *feature* — Rivet projects an
`/etc/passwd`; the harness had replaced the process that writes it.

### 8.4 Cross-compiled Rust

Three rounds in `tools/container/exec.sh`. Not individually timed — each round
completed in single-digit minutes including crates.io downloads, and the
outcomes are §4.3's table. The one methodological note worth keeping: the first
round reported every crate as BUILT because the status being tested was
`tail`'s and not `cargo`'s. It was re-run with the exit code captured properly
and four of the nine then failed. **A check that cannot fail proves nothing**
(`AGENTS.md` §4), and it very nearly put four false positives into this
document.

### 8.5 The board — read only

Read over the control plane, on the machine running the `aura-renderer` soak.
Nothing was deployed, rebooted or changed. Kernel `6.12.61-v8`, which is the
same version the QEMU experiments ran, so the two readings are comparable.
Results are §2.2's table.

---

## 9. The roadmap

Ordered. Each increment is provable, each names the unknown it settles, and
the first one runs on the maintainer's Pi 4.

**Step 1 — put the stack in the kernel and read the board.**
Move §3.2's symbols into `tools/buildroot/linux/rivet.config` (all but
`CONFIG_BT_HCIVHCI`, which goes in `qemu-virt.config`), turn on
`BR2_PACKAGE_BRCMFMAC_SDIO_FIRMWARE_RPI_BT`, build a card, boot the Pi 4, and
read `/sys/class/bluetooth`, `/proc/net/hci` and the kernel log.
*Cost:* 80 s of kernel build, one card build, one boot. +244 KiB compressed.
*Settles:* whether `hci_uart` binds the CYW43455 over serdev, and **what
firmware filename the driver actually asks for** — the one claim in §3.4 that is
documentation-derived. This is `ADR-026` §4's right-hand column and it is the
only way to close it.
*Ends in:* an `hci0` on real hardware, or a named driver failure.

**Step 2 — a Rust management client, and scan.**
A small crate that binds `HCI_CHANNEL_CONTROL` and speaks the six management
commands an observer needs. Every socket call it makes was measured in §4.2;
what is unmeasured is the protocol above them.
*Cost:* Rust only. No new C, no image growth beyond step 1.
*Settles:* capability tier 1 — whether a Govee advertisement reaches a Rivet
application with no BlueZ and no bus.
*Ends in:* advertisements in the journal, or a named management-protocol gap.

**Step 3 — decide the bus, on evidence rather than in the abstract.**
Steps 1 and 2 are the "other options" that R-043's position asks to be explored
before a bus is adopted. Put the measured trade in front of the maintainer:
1.5 MB compressed and five Tier C dependencies for every profile that exists,
against Rust nobody else maintains for the ones actually wanted.
*This is a decision, not an implementation, and it is the maintainer's
(`AGENTS.md` §1).*

**Step 4a — if BlueZ is ruled in: game controllers.**
`bluetoothd` as a supervised service with its `input` plugin, `--localstatedir`
pointed under `/state` (§10.2), a `[bluetooth]` block in the configuration
document, and a pairing operation on the administrative surface (§6). Prove it
by pairing a controller and reading `/dev/input/eventN` from a kiosk
application.
*Settles:* capability tier 3, which is the maintainer's stated hobbyist case.

**Step 4b — if BlueZ is ruled out: a Rust host stack.**
A transport adapter between `trouble-host`'s trait and an `HCI_CHANNEL_USER`
socket. Both halves exist; nothing joins them.
*Settles:* capability tiers 2 and 4, and leaves tier 3 unreachable — see §10.1.

**Step 5 — link keys under `ADR-020`.**
Whichever road was taken, bonding produces a secret that must survive a reboot
and must not be a plain file. This is a store entry and a `KeyProvider`
question, and it is the last thing that makes reconnection work without a human.

---

## 10. Blockers, named

### 10.1 Classic HID without BlueZ is a wall, not a step

There is no Rust implementation of the Bluetooth Classic host side. Measured:
`bt-hci` and `trouble-host` are **BLE** stacks — LE only, no SDP, no BR/EDR
L2CAP, no HIDP. `bluer`'s classic support is entirely behind the `bluetoothd`
feature. `burble` is BLE over USB.

*Evidence:* §4.3's table, and `grep '#\[cfg(feature = "bluetoothd")\]'` over
`bluer`'s `lib.rs`, which gates every module except `l2cap`, `rfcomm` and `id`.

*What would remove it:* writing SDP, BR/EDR L2CAP and HIDP in Rust — which is
`ADR-001` §4.2's argument in its purest form, and the answer there is that you
implement the minimum, declare victory, and never touch it again.

**So: capability tier 3 requires BlueZ.** If the maintainer wants game
controllers, the bus question is not optional, and pretending otherwise would
be the study telling him what he wants to hear.

### 10.2 BlueZ's link-key store is at a path Rivet does not have

`bluetoothd` keeps bonding state in plain files under `/var/lib/bluetooth`,
which is a compile-time `--localstatedir`. `ADR-008` §3 rejects the FHS and
there is no `/var`; `ADR-020` §5 says a secret is a named entry in a store, not
a file somebody else owns.

*Evidence:* it ran in QEMU only because the probe created `/var/lib/bluetooth`
first, and the directory was still empty afterwards because no bonding
happened.

*What would remove it:* a Buildroot `--localstatedir` under `/state`, which is
configuration handed to somebody else's recipe and passes `ADR-009` §6.1's
test — or a bind mount from the supervisor, which `ADR-015` §5.3's socket
mechanism already establishes the shape for. **Neither is hard; both are
decisions, and shipping BlueZ without making one puts pairing state somewhere
that does not survive a reboot.**

### 10.3 `bluetoothctl` does not link, and the workaround is to not ship it

*Evidence:* §4.1's quoted `ld` output. `libreadline.a` needs termcap symbols
that nothing supplies.

*What would remove it:* linking readline against ncurses in this configuration,
or a `bluez5_utils` link-flag fix. **Neither is worth doing.** `SPEC-01` §5.2
records that a Rivet has no POSIX shell and no interactive session, so a
readline prompt has nowhere to run; `btmon` and `btattach` build and are the
two that would earn their place. Recorded because the failure looks alarming
and is not.

### 10.4 Nothing here has run on hardware

*Evidence:* every measurement above is QEMU or a build. `ADR-026` §4 puts
`brcmfmac`, the radio and the firmware in the board's column, and the CYW43455's
Bluetooth half shares the combo part with its wireless — so **wireless working
today is not evidence that Bluetooth will**. The `HCIDEVUP` timeout in §4.2 is
precisely the shape of what a board answers differently.

*What would remove it:* step 1 of §9. It is one card build and one boot.

### 10.5 Experiments considered and not run, with the numbers

* **A card build and an image-size delta.** Not run. The marginal figures were
  measured directly instead — `xz -9` over the exact file set, §3.3 and §4.1 —
  which is `ADR-018` §10.5's own method, and `ADR-018` §4's 640 MiB `/system`
  slots make the nominal size question uninteresting at this scale.
* **Emulating a controller well enough to bring an adapter UP.** Not run. The
  kernel's init sequence is roughly two dozen commands with per-opcode
  parameter lengths, and getting one wrong is a ten-second timeout per attempt.
  It would prove a property of the *kernel*, not of Rivet, and §4.2 already
  proved the transport in both directions for 3 537 ms of measured cost.
* **A from-scratch Buildroot bootstrap.** Not run, and no number is offered for
  it. Every figure in §8.1 is against a warm tree and is labelled as such.

---

## 11. Corrections this study owes other documents

Not made here, because this is a design candidate and the change belongs with
the ruling. Listed so the next pass does not have to re-derive them.

1. **`SPEC-01` §5.2's Bluetooth row cites the wrong file.** `/proc/net/bluetooth`
   does not exist in Linux 6.12 with or without the stack. The valid tests are
   `/sys/class/bluetooth` and the presence of `hci`, `l2cap`, `sco` and `hidp`
   in `/proc/net`. The row's *conclusion* was correct and is still correct
   today; the *test* cannot distinguish presence from absence and should be
   replaced rather than left as a measurement anyone could repeat and misread.
2. **`SPEC-01` §5.2 and §6 read the `libdbus-sys` failure as a
   cross-compilation wall.** It is a missing host tool. §4.3 shows the build
   completing with a twelve-line `pkg-config` on `PATH`.
3. **`tools/buildroot/configs/rivet_aarch64_defconfig`'s reason for declining
   the BT firmware is superseded**, not wrong (§2.3).
4. **`tools/buildroot/build.sh`'s `--reconfigure` documentation is incomplete.**
   It says a reconfigure is needed after editing a defconfig or a kernel
   fragment, which is true; it does not say that the kernel's own `.config` is
   regenerated on fragment **mtime** and that `make linux-reconfigure` is what
   forces it. §8.2 cost two runs to that gap.
