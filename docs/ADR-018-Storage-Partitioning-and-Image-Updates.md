# ADR-018: Storage, Partitioning and Image Updates

* **Status:** Accepted — ruled 2026-08-22. §5's `tryboot` mechanism was
  **proven on a Pi 4** — the flag round-trips across a reboot. Two things a
  reader must not miss: §5.3's trap, that the image must write `tryboot.txt` as
  well as `config.txt` or a tryboot reboot loses a headless board; and **§3.1,
  where `/system` is `ext4` rather than `erofs` on the first cards** because the
  stock kernel cannot mount erofs as root. **§4.1 was added 2026-08-23**: PID 1
  mounts `p5`, and a failure there degrades the boot rather than rolling it back.
* **§5 IS PROVEN END TO END ON HARDWARE, 2026-08-23 — both halves.** §5.3.2 ran
  the success path: armed slot B remotely, one-shot restarted, came back on
  `system-b`, PID 1 committed it. §5.3.3 ran the **failure** path the same
  evening: a trial boot whose commit was withheld returned to the previous slot
  on the next reset, **with no userspace involvement at all**. The second was the
  half nobody had ever seen, on this hardware or any other, and it is what every
  update in this design falls back on. Both captures are in `evidence/`.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

Nothing in the record had decided what filesystems Rivet uses, how the card is
partitioned, or how an image update actually works. A/B images are referenced
from `ADR-008` §3 onward as though designed, and they were not.

That became load-bearing when `ADR-017` §3.1.4 made "PID 1 never confirms the
boot" the recovery path for a supervisor that will not start. PID 1 is a Stage 0
component and it now depends on a mechanism with no ADR.

**The maintainer's framing, 2026-08-22: the project will have to simply accept
the limitations of the SD card and work *with* them.** *(`.rulings/` R-040.)*

---

## 2. A/B and card wear are different problems

Worth separating precisely, because the maintainer asked whether A/B already
covers the SD limitation and the answer is "half of it, and not the half you
would expect".

* **A/B solves update atomicity and rollback.** A failed or interrupted update
  never leaves a half-written system, and a bad image is recoverable without
  physical access.
* **Read-only `/system` solves wear** — that region receives *zero* writes
  during normal operation.

The useful consequence: **the wear risk is not spread across the card, it is
concentrated entirely in `/state`.** Every mitigation in §7 has one target.

---

## 3. `/system` is read-only by format, not by mount flag

`ADR-008` §3 already made `/system` read-only. This ADR strengthens *how*.

**`/system` is an `erofs` image** (or `squashfs`), which has no writable mode at
all. It is not `ext4` mounted `ro`, which is one `mount -o remount,rw` away from
writable.

This is `ADR-011` applied to storage: the format is the authority, not a flag
somebody can flip. It also means the read-only property survives a bug, an
agent, or a tired operator at 2am, none of which a mount option does.

The maintainer's question — whether a read-write system makes sense given that
even stopping a service requires a configuration change — answers itself. There
is no operation in the entire design that writes to `/system` outside an update.

### 3.1 Suspended for the first cards — `ext4` read-only, ruled 2026-08-22

**The stock Raspberry Pi kernel cannot mount erofs, so the first cards ship
`ext4` mounted `ro` — exactly the weaker property this section argues against.**
Recorded as a stopgap with a retirement condition rather than as a revision:
nothing in the reasoning above has changed.

**The evidence.** `ADR-024` §8.1's audit recorded `EROFS_FS=m`, and **a module
cannot be loaded from the filesystem it is needed to mount.** Confirmed a second
and independent way by decompressing `tools/qemu/kernel8.img` and listing the
filesystem names the kernel registers: `cgroup2 devtmpfs ext2 ext4 msdos proc
sysfs tmpfs vfat`. No erofs.

*One trap worth recording, because it points the wrong way:* that kernel image
contains **38 occurrences of `EROFS`**. They are the errno for "read-only file
system" in error paths, not the filesystem. An agent reported the search as
finding nothing and was right for the wrong string; the registered-filesystem
list is the evidence that settles it.

**Why accept it rather than wait.** The custom kernel is already scheduled
(`ADR-024` §8, forced by Landlock) and this is a second reason for it — but it
is a project of its own, and **nothing else about Stage 0 depends on the root
filesystem's format.** PID 1, the mount plan, reaping, the supervisor and
`tryboot` can all be proven on real hardware with an ext4 root, and proving them
is worth more than holding the first boot for a property that changes nothing
until an attacker or a tired operator is already at the keyboard.

**The retirement condition, testable rather than aspirational:** when
`ADR-024` §8's kernel ships with `EROFS_FS=y`, `/system` becomes erofs and
`cmd/rivet-image`'s `SYSTEM_FSTYPE` constant is deleted. It does **not**
survive on the grounds that ext4 works — the argument above is why erofs was
chosen and it is undisturbed.

**What is genuinely weaker in the meantime**, stated so nobody has to infer it:
a `mount -o remount,rw /system` succeeds on these cards. Every reason §3 gives
for that being unacceptable applies during the whole stopgap.

---

## 4. The partition layout

```
p1  boot-A     FAT32   firmware, kernel A, DTB, config.txt, provisioning file
p2  boot-B     FAT32   firmware, kernel B, DTB, config.txt
p3  system-A   erofs   read-only image  -- ext4 ro on the first cards, §3.1
p4  system-B   erofs   read-only image  -- ext4 ro on the first cards, §3.1
p5  state      ext4    the only continuously-written area
```

**This table describes the SYSTEM DEVICE, and since 2026-08-24 that is a
distinction rather than a tautology.** `ADR-031` adds **volumes** — further
filesystems that hold applications and their data — so `p5`'s *"the only
continuously-written area"* is true of this device and no longer true of the
machine. Nothing in this section changes: `ADR-031` §2.1 draws the line, and
every fact about the system device stays this ADR's.

**Two boot partitions** because the Raspberry Pi firmware loads the kernel from
a FAT filesystem, so an A/B pair needs two of them (§5).

**One `/state`.** The maintainer asked whether configuration wants its own
partition, and the answer is no. The failure it would prevent is real —
application data filling the card until the machine cannot be reconfigured — but
a partition solves it by fixing a size that has to be guessed years in advance,
and the guess will be wrong. **A reserved-space guarantee inside one `/state`
solves the same failure and can be adjusted.**

The "only the system may touch it" property comes from permissions, which
already hold: `/state/config` is root-owned, and applications run as their own
UIDs under `ADR-015` §5.1.

### 4.1 Who mounts `p5` — answered 2026-08-23 in `ADR-017` §3.1.7

**PID 1**, as a non-essential entry in its mount phase. `ADR-017` §3.1.5 item 1
was amended the same day and §3.1.7 carries the argument; it is not repeated
here so that there is one place to change it (`ADR-011`).

Two consequences belong to *this* ADR rather than to that one:

* **`noatime`, `nosuid`, `nodev`, and deliberately not `noexec`.** §7's table
  names `noatime` first and the reason is §2's: all of a Rivet machine's card
  wear is concentrated here, and without it every read becomes a write. `noexec`
  is the flag one reaches for on a writable partition and it is **not** set,
  because §6 loop-mounts extension images out of `/state/ext` and `ADR-015`
  gives applications artifacts that have to run from somewhere. Setting it
  should be a ruling with a stated consequence, not a tidy-up.
* **A failed mount degrades the boot; it never makes it unconfirmable.** That
  follows from §8: a rollback cannot repair a `/state` an update never touches,
  so an unconfirmable verdict would alternate between two healthy images for
  ever without addressing the fault.

**The device is named by its `PARTLABEL`** — `PLAN.md` §62, 2026-08-28. It was
named by number until then, and neither premise behind that had to be given up:
`/dev/disk/by-partlabel/` really is **udev's**, `ADR-005` §2.1 really does rule
that Rivet has no udev, and resolving a label by parsing the GPT out of the raw
disk inside the process that must not fail really would be too much.

**There is a third resolver that is neither udev nor a GPT parser in PID 1:
the kernel**, which parsed the GPT
already and publishes each partition's label on its sysfs `uevent`
(`block/partitions/efi.c` → `core.c`'s `add_uevent_var(env, "PARTNAME=%s", …)`).
A `uevent` file is generated when it is read, so nothing has to have processed a
hotplug event and no `/dev/disk/` symlink has to exist. PID 1 reads a directory
and a handful of small files, which is cheaper than the composition it replaced.

**The paragraph above was contradicted by this repository before it was
written.** §5.2's `cmdline.txt` says `root=PARTLABEL=system-a`, resolved by
`block/early-lookup.c` before any userspace exists at all — so `/system` was
already named by label and `/state` by device path, on one card, for opposite
reasons. `cmd/rivet-image`'s own table gives the argument that applies to both:
a device path *"names the medium"*, and the same card in a USB enclosure is
`/dev/sda`.

Three answers, not two. Exactly one partition with the label is mounted; none
falls back to the composed `/dev/mmcblk0p5` and says so, so a machine with no
readable `/sys` behaves as every machine did before; **more than one mounts
nothing**, because two cards written by `cmd/rivet-image` on one machine make
either choice another machine's data at `/state`. That last case degrades the
boot rather than failing it, on §8's terms.

**Confirmed on the board, 2026-08-28.** Five partitions carry a label, they are
this section's five, exactly one is `state`, and it is on `/dev/mmcblk0p5` —
the path the old rule assumed. The change is a no-op on this hardware in the
good case and correct where the assumption was not.

**`ADR-031` §4.2 is still the other case, and the distinction has moved rather
than gone.** A volume is resolved by **GUID, parsed out of the GPT by Rivet** —
which is exactly what PID 1 still declines to do, and what the corrected rule
above still avoids: PID 1 does no parsing at all, it reads the kernel's answer.
Volume attachment happens later, in the supervisor, where a failure is a
journalled refusal rather than a machine that will not boot. An agent reading
either paragraph in isolation will conclude one overturns the other; neither
does (`ADR-031` §10 item 4).

**Five partitions requires GPT or an extended partition.** Which of these the
Pi firmware supports on the target hardware is a detail to confirm before
building the first image.

---

## 5. Updates: the Pi firmware already has the A/B primitive

Rivet does not need to implement boot-slot switching. The Raspberry Pi
bootloader provides it.

### 5.1 `tryboot`

* `autoboot.txt` on the boot partition, with `tryboot_a_b=1`, defines a default
  boot partition and a separate `[tryboot]` one.
* `reboot "0 tryboot"` sets a **one-shot** flag: the firmware boots the *other*
  slot exactly once.
* The flag does not persist. If nothing commits the new slot, the next boot
  returns to the previous one **without any userspace involvement.**

That last property is the whole thing. The failure path requires no code, no
watchdog and no timer — a system that fails to commit reverts by default, which
is the correct direction for a failure to point.

**Which `autoboot.txt` — verified on hardware 2026-08-23, and the first bullet
above was silently ambiguous.** It says *"on the boot partition"*, and §4 has
**two**. The firmware reads the **first** FAT partition's file and only that
one. Two boots of one card, with the two files made to disagree,
were run with the console lines read back; the second boot is the decisive
one, because `p1` named a partition that was not itself and was obeyed. That
also rules out *"the firmware ignores `autoboot.txt` and defaults to 1"*, which
would have taken this whole section with it.

`rivet_core::state::AUTOBOOT_PARTITION` is the one place that number lives,
and `cmd/rivet-image`'s `layout` module carries a test tying the table it
writes to it.

**`cmd/rivet-initd` had this wrong**, and it is worth leaving visible rather
than quietly corrected: it rewrote the *booted* partition's file, which is right
in slot A and writes into the void in slot B. Slot B could never have been
committed, every update would have rolled back, and each rollback would have
looked exactly like a healthy machine refusing a bad image. Fixed the same day.

### 5.2 The update flow

1. Write the new image to the inactive `system-` partition and its kernel to the
   inactive `boot-` partition. `/state` is not touched.
2. Verify the signature (`ADR-015` §4 — the same trusted-key set and the same
   verification path as applications and extensions).

   > **This step checks *authenticity* and nothing else, and that is a gap
   > rather than a design — noted 2026-08-23, `ADR-028`.** It does not compare
   > the incoming version against what is running, so **a replayed,
   > correctly-signed, older image installs cleanly** — and A/B makes that
   > easier rather than harder, since an attacker need not disturb the running
   > slot at all: fill the *inactive* one and wait for a reboot. `ADR-028` §6
   > rules the response — report a non-newer version rather than refuse it, with
   > enforcement arriving later as a **fleet policy** (§6.2) — and §8 item 4
   > asks what this flow should do with a slot whose verification failed, which
   > it also does not currently say.
3. `reboot "0 tryboot"`.
4. The new slot boots. **PID 1 confirms** once the system is healthy
   (`ADR-017` §3.1.5 item 5), which makes the new slot the default.
5. If it does not — kernel panic, supervisor that will not start, network that
   never comes up — the next boot is the old slot, unchanged.

> **The steps are inverted, and there is a process that performs them —
> 2026-08-24.** `ADR-029` §10 corrected the order as a **direct contradiction**:
> steps 1 and 2 as written above put attacker-controlled bytes on a partition
> before establishing they were ours, and `ADR-029` §5 makes verification
> **entirely precede any write**. `ADR-029` §9.2 is the corrected sequence and
> is the normative one; this list is left readable because §10's argument is
> about it.
>
> The performer is **`cmd/rivet-deploy`**, a supervised binary of its own —
> the maintainer's 2026-08-24 ruling and `ADR-029` §14. Step 1's *"its kernel"* is also
> too narrow: the boot half is a **file set** (`ADR-029` §6.2), and the
> installer skips every file the target already holds, which is a *safety*
> property rather than an optimisation.

**The artifact step 1 writes is not the artifact `cmd/rivet-image` produces.**
That tool writes a whole card — GPT and all five partitions — and this flow
writes one system filesystem and one kernel into an existing device. `ADR-027`
§4 names the two apart before the fleet server in `PLAN.md` §9.4 has to hold
both; the shape of the payload named there is still this section's to decide,
and is `ADR-027` §9 item 1.

**`/state` is never touched by an update**, which is `ADR-008` §3's guarantee
made concrete: there is no merge, no `.dpkg-new`, and no three-way
reconciliation, because machine content and image content are on different
partitions.

### 5.3 Load-bearing, and mostly confirmed on hardware — 2026-08-22

Checked against a Raspberry Pi 4 Model B Rev 1.1 running stock Raspberry Pi OS,
bootloader `224877da` (release, 2026/05/17), capabilities `0x7f`.

**`tryboot` exists on this board.** Two independent readings agree: the running
kernel exposes `/proc/device-tree/chosen/bootloader/tryboot` (reading `0`, as it
must on a normal boot) alongside `boot-mode`, `partition`, `capabilities` and
`version`; and the bootloader EEPROM image under
`/lib/firmware/raspberrypi/bootloader-2711/` contains the literals `tryboot` and
`TRYBOOT`. The flag is therefore both implemented by the firmware and reported
to the OS, which is exactly what §5's confirmation step needs to read.

**The syntax, from Raspberry Pi's `config.txt` documentation** — flagged as
documentation-derived, because no `autoboot.txt` exists on this system to read:
`autoboot.txt` lives on the **boot partition**, uses `config.txt`-style
conditional sections, and carries `boot_partition=` under `[all]` with an
override under `[tryboot]`; `tryboot_a_b=1` makes the tryboot boot load the
normal `config.txt` and `boot.img` rather than `tryboot.txt` and `tryboot.img`.
A tryboot boot is requested with `sudo reboot "0 tryboot"`, and the flag clears
after one boot.

**The flag mechanism was then proven end to end, 2026-08-22.** The board was
rebooted with a tryboot argument, came back on the network, and
`/proc/device-tree/chosen/bootloader/tryboot` read **`1`** where it had read `0`
before. `/proc/device-tree/chosen/bootloader/partition` read `1`, and `uname -m`
still reported `aarch64`, so the tryboot boot was a correct boot rather than a
degraded one. §5's confirmation step therefore has a real flag to read.

**A trap worth recording, because it would have cost a headless board.** In
tryboot mode the firmware loads `tryboot.txt` **instead of** `config.txt`. On
this system `config.txt` carries `arm_64bit=1`, and there is no 32-bit kernel on
the boot partition — so a tryboot reboot with no `tryboot.txt` present would
have attempted a 32-bit boot and lost the machine. A copy of `config.txt` was
placed at `tryboot.txt` before the test for exactly this reason. **Rivet's
image must write both files, and `ADR-002` §4's boot-partition assembly is where
that belongs.**

**How the reboot is actually requested.** Raspberry Pi's documented
`sudo reboot "0 tryboot"` is rejected by systemd 257 with *"Too many
arguments"*; the form that worked was
`systemctl reboot --reboot-argument="0 tryboot"`. This is incidental to Rivet,
which has no systemd and will call `reboot(2)` with `LINUX_REBOOT_CMD_RESTART2`
and the string directly — but it is worth knowing that the documented CLI
spelling is not portable across systemd versions, in case a bring-up step is
ever scripted against a stock image.

**And "call `reboot(2)` directly" is not one function call — found 2026-08-23
while building it.** There is **no C entry point on this target that can carry
the argument**. The `libc` crate's Linux binding is `reboot(how_to: c_int)`,
which is what the glibc *and* musl wrappers are; musl's is literally
`syscall(SYS_reboot, MAGIC1, MAGIC2, type)` with no fourth word. So
`LINUX_REBOOT_CMD_RESTART2` is reached by making the syscall directly —
`syscall(SYS_reboot, LINUX_REBOOT_MAGIC1, LINUX_REBOOT_MAGIC2,
LINUX_REBOOT_CMD_RESTART2, "0 tryboot")` — which is what every implementation of
`reboot "<arg>"` does. Recorded because the obvious reading of the paragraph
above is that `libc::reboot` grows an argument, and it cannot.
`cmd/rivet-initd/src/kernel.rs` carries it as `RebootHow::TrialRestart`, and
**it has never been executed**: it stops the machine on success, and `ADR-026`
§4 puts `tryboot` in the column QEMU does not emulate at all.

**One of the two below is now proven; the other is not — updated 2026-08-23.**
The second item, the `autoboot.txt` *syntax*, is half closed: `boot_partition=`
under `[all]` was read and obeyed on a two-boot-partition card (§5.1), so
the file, the section and the key are all confirmed. **What is
still untested is the `[tryboot]` override specifically** — neither boot used
tryboot, so that section was only ever observed being left alone. It needs a
board that can *request* a tryboot reboot.

**Rivet's userland can request one since 2026-08-23, and the
experiment is still not run.** `RebootHow::TrialRestart` is on `cmd/rivet-initd`'s
`Kernel` port, and **`rivet-adapter-image`'s** `arm_trial_boot` moves the
`[tryboot]` override
onto a chosen slot and reports which reboot carries the one-shot. Nothing has
run on a board. The item therefore
stays open, and what changed is that it is one *experiment* away rather than one
implementation away: the same distinction §8's closing paragraph draws about
re-provisioning.

**Where the port lives, corrected later on 2026-08-23.** It was cut inside
`cmd/rivet-initd`, which is a binary crate with no lib target, so nothing but
PID 1 could name it; `ADR-014` §2.2 rules the move to `rivet-adapter-image`
and carries the argument, which is `ADR-028` §4's — an authenticated principal
supplies the authority to install an update and the record does not say which
process executes it. **No behaviour changed in the move**, including the
2026-08-23 slot-B correction in §5.1.

**What `arm_trial_boot` hands back is `rivet_adapter_image::ArmedReboot`, not
`RebootHow`.** The two platforms differ in *where the one-shot lives* — in the
reboot call on this hardware, in NVRAM on a UEFI machine — so the port names
that difference and the caller maps it onto its own syscall. `RebootHow` stayed
in `cmd/rivet-initd` because it also carries `PowerOff` and `Restart`, which
are `ADR-017` §3.1.5 item 6's shutdown handling and have nothing to do with
images.

**Two things remain unproven, and both need more than a reboot.** First, whether
the firmware read `tryboot.txt` or fell back to `config.txt` cannot be told from
this run, because the two files were made identical on purpose — distinguishing
them would have meant risking the boot. Second, and more important for §5:
**that a `[tryboot]` section in `autoboot.txt` selects a different
`boot_partition` was not tested**, because this card has one boot partition to
select. That test needs the A/B layout to exist, so it belongs to image assembly
rather than to an audit.

If `tryboot` turns out not to be available, the fallback is rewriting
`config.txt` on a single boot partition to select the active kernel — which
works, but makes the boot partition writable and reintroduces the "interrupted
during a write" failure that `tryboot` avoids entirely. That is a materially
worse design, so this is worth confirming early rather than assuming.

### 5.3.2 CLOSED on hardware, 2026-08-23 — the whole of §5 ran

**The second unproven item above is the longest-standing unknown in this
project.** It was written on 2026-08-22, it is what `ADR-018` §5 rests on
entirely, and it could not be settled without an A/B card, a way to arm a trial
boot, and a way to ask a running board to restart — none of which existed until
today.

**The full cycle ran on a Raspberry Pi 4 Model B on 2026-08-23**, from a card
this repository built, driven entirely from a workstation.

`rivet apply trial-boot b confirm`, armed from slot A:

```
{"subject":"trial-boot","slot":"b","running-slot":"a","armed":true,
 "rebooted":false,"reboot":"one-shot", …}
```

Then `rivet apply trial-restart confirm`, which dropped the connection as
designed — the transport was carrying the answer:

```
{"subject":"trial-restart","requested":true,"rebooted":false, …}
```

**The board came back in slot B.** Two independent readings, quoted rather than
summarised — one the kernel's, one the firmware-composed command line:

```
[    2.825999] EXT4-fs (mmcblk0p4): mounted filesystem
               feedface-0000-4000-8000-f00000000004 ro with ordered data mode.
[    2.827945] VFS: Mounted root (ext4 filesystem) readonly on device 179:4.
  /proc/cmdline: … root=PARTLABEL=system-b rootfstype=ext4
```

`p4` is `system-b` (§4). And PID 1 then committed it:

```
rivet-init: boot confirmed: partition 2 is now the default
```

> **PID 1's console prefix was `rivet-init:` when this and every other capture
> in this file was taken.** It became `rivet-initd:` on 2026-08-24 with the
> crate rename (`PLAN.md` §0.4.7). The captures are left as the board printed
> them (`AGENTS.md` §5); a board today prints `rivet-initd:`. Stated once,
> here, rather than at each block.

**So `[tryboot]` selects a different `boot_partition`, the firmware honours
`reboot(2)` with `LINUX_REBOOT_CMD_RESTART2` and `"0 tryboot"`, the one-shot
clears, and §5.1's confirmation step makes the trial slot the default.** §5 is
proven end to end rather than reasoned about.

**The rollback — a trial boot whose commit is withheld returning to the previous
slot on the next reset — was not exercised by this run**, because the trial boot
was healthy and confirmed. It is the half the whole design rests on, and it was
**CLOSED the same evening — §5.3.3.**

The first unproven item above — whether the firmware read `tryboot.txt` or fell
back to `config.txt` — is **also still open**, and for the same reason as before:
`cmd/rivet-image` writes the two files byte-identically on purpose
(`boot.rs`'s `tryboot_txt`), so this run cannot distinguish them either.

### 5.3.3 The ROLLBACK, observed on hardware — 2026-08-23, later the same evening

**§5.3.2 closed the half where the trial boot succeeds. This closes the other
half**, and the two together are the whole of §5.1's mechanism. A trial boot
whose commit is withheld returns to the previous slot on the next reset, **with
no userspace involvement at all** — that had never been seen on this hardware or
any other, and it is what every update in this design falls back on.

#### How the failure was produced, and why not by breaking a binary

`PLAN.md` §0.0.4 step 11 originally said to make the supervisor unstartable in
slot B. **A cheaper fault produces a strictly better experiment** and changes no
binary: `p2`'s `rivet.toml` carries an unknown key and `p1`'s does not
(`PLAN.md` §0.0.5). Three facts already in the tree make that work — PID 1 reads
the provisioning document from the boot partition the *firmware* booted (§5.3,
confirmed on hardware 2026-08-23), an unknown key is refused at parse time rather than
ignored (`ADR-007` §3.6.5), and a refused document is never stored, so `/state`
keeps the good one and the supervisor reads that.

**The consequence is the point, not a side effect.** The trial boot comes up
completely healthy and answers on the network; the *only* thing wrong with it is a line that
does not appear.

#### What the board did, quoted rather than summarised

The firmware took the one-shot. Two independent readings — one kernel-owned, one
firmware-owned:

```
[    2.657799] Waiting for root device PARTLABEL=system-b...
[    2.856205] EXT4-fs (mmcblk0p4): mounted filesystem
               feedface-0000-4000-8000-f00000000004 ro with ordered data mode.
rivet-init: reading the provisioning file from the boot partition
  mounted /dev/mmcblk0p2 read-only on /run/boot-partition
```

PID 1 then refused the document and **withheld the commit**:

```
  provisioning document REFUSED: unknown configuration key `rollback_experiment`
    at line 15, column 1; this document declares: version, services, network, …
  the machine could not apply (ADR-017 §3.1.6, ADR-018 §5).
rivet-init: boot NOT confirmed: the provisioning document could not be applied
```

**No `boot confirmed` line appears anywhere in that boot**, and the boot is
otherwise complete: `/state` mounted, clock restored from `/state/clock` before
the network existed and then SNTP-stepped, supervisor up, `/etc` projected with
`permits_a_login:true`, and every service started.

Then an **ordinary** restart — not a trial one — `rivet apply restart confirm`:

```
{"subject":"restart","requested":true,"rebooted":false, …}
```

And the board came back on slot A:

```
rivet-init: reading the provisioning file from the boot partition
  mounted /dev/mmcblk0p1 read-only on /run/boot-partition
```

corroborated by the machine's own firmware-owned reading, taken from
the running system rather than off the console — `rivet apply trial-boot a`:

```
{"error":"E_UNSAFE","subject":"a","message":"this machine is running boot
 partition 1, …"}
```

#### Why this is a rollback and not a board that failed to boot

`PLAN.md` §0.0.4 carries the two-column test, and it is the reason the capture
had to run across all four boots: **after the fact the two states are the same
machine in the same state.** This run satisfies the rollback column outright —
the capture contains a *complete* slot-B boot, with every line present except
`boot confirmed`, rather than an absence of any slot-B boot at all.

**Nothing performed the return.** The commit never happened, so `[all]` still
read `boot_partition=1`, and the next reset obeyed it. That is the whole
mechanism: the rollback is what happens when nothing happens.

#### What this run also settled, in passing

* **The pid-1 interlock permits a real board.** `ADR-017` §3.1.10.1 recorded
  that its *accept* path had only ever been driven against a renamed `bash` in a
  throwaway namespace, and that a false refusal on hardware "would look like a
  safety feature while blocking the experiment". `rivet apply trial-restart`
  without `confirm` returned `E_CONFIRMATION_REQUIRED`, not `E_UNSAFE`.
* **`apply restart` does not take an armed trial boot.** The card was still armed
  for slot B when the ordinary restart ran, and it booted slot A. §5.1's
  separation of arming from restarting is now demonstrated rather than asserted.
* **A prediction the CLI makes about itself is wrong, and it is the CLI's to
  fix.** Both `trial-restart` and `restart` say to "expect the client to report a
  dropped connection rather than an exit status". Both returned their JSON and
  exited **0**: PID 1's shutdown — stop the supervisor, wait, sync — is slower
  than the transport is at delivering the answer. Harmless, and the text should say
  the answer *may* arrive rather than that it will not.

**Evidence:** `evidence/2026-08-23-ab-rollback-withheld-commit.mkv`, 546 seconds,
one continuous capture spanning all four boots. Not tracked; see that directory's
`README.md`.

### 5.3.1 The arming has a caller, later on 2026-08-23 — and the reboot still does not

**`arm_trial_boot` is called by `cmd/rivet`**, the CLI, through
`rivet_core::operation::trial` — `rivet apply trial-boot <a|b> [confirm]`.
This section said it had no caller until 2026-08-23, which is a claim that went
stale within a day and is exactly the class `AGENTS.md` §7 is about.

**Why the CLI is the caller rather than the update path.** `ADR-028` §4 rules
that *authority* comes from an authenticated principal *"at the console or over
the control plane"*, and `ADR-006` §5 makes the CLI the administration surface — the
network API being deferred, and nothing connecting the CLI to the supervisor. So
the operator is §4's principal. Arming is separable from the update path, and
separating it is what makes §5.3 item 2 a one-card experiment.

**There are two callers of `arm_trial_boot` since 2026-08-24, and the difference
between them is the whole of the maintainer's 2026-08-24 ruling.** `cmd/rivet` arms a
slot somebody already filled; `cmd/rivet-deploy` fills one and arms it as the
last step of the same operation (`ADR-029` §9.1). The reason it is not the CLI
doing both is not authority — it is that **an update writes 640 MiB, and running
that from the login shell means a remote session owns a long write.** This section
had the update path deliberately unbuilt on the grounds that the payload format
and signature scheme were open; `ADR-029` decided the format on 2026-08-23 and
`ADR-029` §7.1 the scheme, and ruling 1 then made the installer its own
supervised binary (`ADR-029` §14).

**Neither of them reboots**, and that is unchanged: `rivet apply
trial-restart confirm` is still the thing that sends `SIGUSR1`.

**Fetching is still not built.** There is no transport, and `ADR-029` §12
item 1's trust question is open; the installer takes a path to a file that is
already on the machine.

**Two safety interlocks were added with the caller**, both refusing before
anything is written and both reachable as a dry run:

1. **`tryboot_a_b=1` must apply unconditionally in the `autoboot.txt` that will
   be read.** This is the trap recorded three paragraphs below, as a fuse.
   `cmd/rivet-image` writes the setting under `[all]` on every card it builds,
   so a card this repository produced passes — a hand-edited one may not, and
   `ADR-002` §4.1 has the operator editing that partition from Windows.
2. **The running slot may not be trialled.** A trial of the slot already running
   is a restart that comes back identical and is then committed, so an update
   that did nothing is reported as one that worked — the same silent-success
   direction §5.1's slot-B commit bug had.

**A Rivet board can request the one-shot restart.** It could not until
2026-08-23, when this was the thing blocking the experiment: nothing in the
image could send a signal to PID 1 or ask it for anything, so a board could be
restarted only by removing power, and a power cycle does not carry the one-shot.

**Ruled and built later on 2026-08-23 — `ADR-017` §3.1.10, option 2 of
three weighed.** PID 1 accepts a **third signal**: `SIGUSR1` means
*shut down and restart into the armed slot*, and it goes through the same
orderly path as `SIGTERM` and `SIGINT` — stop the supervisor, sync, then
`reboot(2)`. `RebootHow::TrialRestart` therefore has a caller, in the one
process `ADR-017` §3.1.5 item 6 gives shutdown to.

**What that leaves, stated precisely, because the two are easy to conflate.**
The signal is receivable **and it has senders — landed later on 2026-08-23**,
which is the second time in one day this paragraph outlived its subject. The
three subjects exist: `rivet apply power-off |
restart | trial-restart confirm`, in `rivet_core::operation::stop`, refusing
unless `/proc/1/comm` reads `rivet-initd` so that a development host cannot be
stopped by the same command. `ADR-006` §8.5 and `ADR-017` §3.1.10.1 carry it.

**`LINUX_REBOOT_CMD_RESTART2` has executed once, on a development host, in a
throwaway PID namespace** — it had executed nowhere at all until then. That proves the
call is well-formed and accepted by a Linux kernel and **nothing about the
firmware**: `ADR-026` §4 puts `tryboot` in the column QEMU does not emulate at
all, so §5.3 item 2 is still one **experiment on a board** away, and nothing
smaller will close it. A green test suite is not that experiment and must not be
read as one.

**So the whole of §5 is now a sequence one card can demonstrate**, and it is two
commands and a wait:

```
rivet apply trial-boot b confirm      # arm — writes [tryboot]
rivet apply trial-restart confirm     # take it — SIGUSR1 to pid 1
```

What is being tested is the line between them: whether the firmware reads
`[tryboot]` on a `"0 tryboot"` reset and boots the slot it names. Everything on
either side of that line has been observed on hardware already.

### 5.4 No update is written while a trial is undecided — 2026-09-12

**`cmd/rivet-deploy` refuses a system update unless this boot is committed**,
before any payload check and on a dry run as well as a real one, with nothing
written. `PLAN-Feature-Set-2026-09-12` §3.3f records how it was found.

**The hazard is one sentence of §5.1 read against the installer.** An update
writes the slot that is not running. During an uncommitted trial the running slot
is not the default, so the slot that is not running **is** the default — the one
§5.1's rollback returns to on the next reset. Writing it while the trial is
undecided puts a 640 MiB write on the only slot the machine can fall back to, and
a reset or power cut part-way through leaves a machine whose fallback is a
half-written system. §5.1's whole virtue — the failure path needs no userspace —
is exactly what that write removes, and a card is what recovers it.

**The condition is `default slot ≠ running slot`, and not the tryboot flag.**

| Reading | What it says | Why it is or is not the condition |
| :-- | :-- | :-- |
| `in_trial_boot` (the device tree's `tryboot`) | how this boot was **entered** | stays true for the life of a trial PID 1 has confirmed, so refusing on it would refuse every committed trial — and three readers were misled by exactly that before `default_slot` existed |
| `default_slot` (`autoboot.txt`'s unconditional `boot_partition`) | what an **ordinary reset** boots | the update writes the default precisely when the default is not the running slot — **the hazard itself, measured directly** |

It is the same comparison `rivet get slot`'s `committed` field makes
(`SlotRead::committed`), read through the same `BootSlots::default_slot`. A
default that cannot be read is also refused: whether the write lands on the slot a
reset boots is then unknown, and unknown is not safe for the one failure that
needs a card to recover.

**The refusal names both ways forward**: let the trial commit — PID 1 confirms a
healthy boot (`ADR-017` §3.1.5 item 5), after which `get slot` reports
`committed: true` — or reset to roll back, then install again.

**Proven by unit test against a fake card, not on a board.** A card whose firmware
booted slot A while `autoboot.txt`'s default is slot B is refused with no device
write and the slot byte-identical; a committed trial (tryboot set, default equal
to running) still installs. `ADR-026` §4 puts `tryboot` in the column QEMU does
not emulate, so nothing short of a board in an uncommitted trial exercises the
real device tree and `autoboot.txt` together.

### 5.5 The commit, and the one partition several programs mount

**The commit is one decision per boot, of at most three attempts.** PID 1
decides once the supervisor has stayed up and the machine has been reached.
While a trial waits to be reached, it checks once a second. Within that one
decision it makes at most three attempts, five seconds apart, scheduled in its
own loop so that reaping and signals are served between them. It retries only
a refusal by the boot medium: a mount, read, write or rename of `autoboot.txt`,
or the lock below being held. An `autoboot.txt` whose contents it will not
rewrite is refused once, because retrying changes nothing. Each refused attempt
is journalled with its number and a read-only listing of the partition's root,
at most 64 entries with their sizes. After the third refusal the boot is
declined exactly as before, and the trial rolls back at the next reset. A
shutdown between attempts abandons the retry.

**Every program that mounts the autoboot partition at its mount point holds one
lock.** That is PID 1's commit and listing, `rivet`'s arming and default-slot
read, and `rivet-deploy`'s. Each holds an exclusive `flock` on
`/run/rivet/autoboot.lock` from before its mount until after its unmount. The
lock is created mode `0600`. When the boot half being installed is the autoboot
partition itself, the installer holds it across the whole write, including the
selector bytes it captures and restores. **PID 1 never waits on the lock**: a
held lock is a medium refusal, and the retry covers it. A command waits at most
ten seconds, then reports the partition busy without mounting it.

**Why the lock is needed.** The commit writes `autoboot.txt.new` and then
renames it over `autoboot.txt`. A reader that mounted the same partition at the
same point, found PID 1's mount already there, read through it and unmounted
it, would pull the filesystem out from under PID 1 between the write and the
rename. The rename then fails with `ENOENT`, and a healthy trial goes
uncommitted. That interleaving is replayed in
`rivet-adapter-image`'s `slots_lock_tests`, and the lock is what removes it.

**A mount found at the mount point under the lock is orphaned**, because no
other lock holder can be using it. The holder unmounts it, mounts afresh and
records the reclaim. An orphan that will not unmount is a refusal. A program
mounting where the lock does not reach unmounts only a mount it made, and
treats `EBUSY` as "already mounted" only when the mount table shows a
filesystem there.

---

## 6. System extensions are files, not partitions

`ADR-016` extensions are variable in number and size and independently
installable, so partitions are obviously the wrong shape.

**`/state/ext/<name>.img`, loop-mounted read-only at `/system/ext/<name>`.**
They live in `/state` because they must survive a `/system` replacement, and
`/state` is precisely the thing an update never touches.

**One implementation detail worth catching now:** mount points cannot be created
on a read-only filesystem. So `/system/ext` is a **tmpfs** mounted over an empty
directory present in the image, and extensions mount inside that. Without this
the first extension is impossible to mount and the reason is non-obvious.

---

## 7. `/state`, and working with the card rather than against it

**`ext4`, ruled 2026-08-22**, with `f2fs` to be evaluated later.

`f2fs` is designed for flash and would likely do better. `ext4` is chosen for
the same reason static linking was (`ADR-014` §4.2): during bring-up, every
recovery tool understands it and every failure mode is documented, and removing
unfamiliarity from "why will this not mount" is worth more than a wear
improvement nobody has measured yet. Switching later means reformatting
`/state`, which is recoverable — `rivet export`, reflash, `apply`.

Measures, all of which target **the system device's** one writable region —
scoped 2026-08-25, because `ADR-031` makes a volume a second writable filesystem
and none of the four below has been ruled for one. §2's *"the wear risk is not
spread across the card, it is concentrated entirely in `/state`"* stays true of
the card and stops describing the machine. `noatime` is the only measure that
carries over without a decision; an NVMe is not an SD card and the other three
were chosen against one (`ADR-031` §10 item 2):

| Measure | Reason |
| :-- | :-- |
| `noatime` | every read otherwise becomes a write |
| **no swap** | swap on an SD card destroys it. `zram` is the answer if memory pressure ever appears — compressed RAM, no card writes |
| journal batched and size-capped | `ADR-017` §7. An unbounded log is the same hazard as the restart loop in `ADR-017` §3.5 |
| reserved space for system use | §4 — application data must never make the machine unconfigurable |

---

## 8. Recovery when `/state` itself is damaged

A/B rollback does not help here, because `/state` is deliberately never touched
by an update. A corrupt `/state` is a machine that boots into a healthy image
with no configuration.

There is a recovery path already present in the design and worth naming: **the
provisioning file on the FAT boot partition** (`ADR-004` §6). If `/state/config`
is unreadable, PID 1 can re-run provisioning and reach a machine that is
network-reachable and re-configurable, rather than one that is bricked.

Not designed here — it needs care around when re-provisioning is correct versus
when it would silently discard a machine's configuration. Recorded so that the
capability is not lost, and tracked in `docs/PLAN.md`.

**2026-08-23: the code that would do it now exists, and refuses.** `ADR-017`
§3.1.8 has PID 1 seed `/state/config` from the provisioning document on a first
boot. It seeds **only when nothing is stored**; a stored document it cannot read
— unparseable, not UTF-8, or an `EIO` from the card — is refused rather than
overwritten, and the refusal names §9 item 3 as the open question it is refusing
under.

That is deliberately *not* a decision about the recovery path. The direction an
implementer could guess in is the one that destroys a machine's configuration,
and this section already says the care needed is around **when** it is correct.
So the capability is one ruling away rather than one implementation away, and
the code says which ruling on the console rather than in a comment.

Its counterpart is also worth stating here because it belongs to this section's
subject: **a seeding failure degrades the boot and never makes it
unconfirmable**, for §8's own reason — a rollback cannot repair a `/state` an
update never touches, so withholding the confirmation would alternate between two
healthy images for ever without addressing the fault. `ADR-017` §3.1.7 ruled the
same thing for the mount and §3.1.8 for the write.

---

## 9. Open

1. **GPT or an extended partition — ANSWERED 2026-08-22: the firmware reads
   GPT.** The Pi 4 bootloader EEPROM image contains the GPT header signature
   `EFI PART` together with `GPT: No bootable` and `Matched GUID!` — error and
   match strings from a GPT parser, not an incidental mention — and `start4.elf`
   carries `EFI PART` as well. Five partitions can therefore be laid out with
   GPT and the extended-partition fallback is unnecessary. Note the stock
   Raspberry Pi OS card is **MBR** (`fdisk` reports `Disklabel type: dos`), so
   this is a capability of the firmware rather than something the stock image
   demonstrates. **Unproven:** that this board boots from a GPT-labelled card —
   proving that means repartitioning, which belongs to image assembly rather
   than to an audit.
2. **Partition sizing** — **and the GUI ruling of 2026-08-22 changed the
   question.** `/system` slots are fixed at flash time, so this is answered
   before the first card is written or not at all.

   This item previously read: *"`/system` slots must fit the largest anticipated
   image including a GUI variant."* **There is no GUI variant.** `PLAN.md` §4.13
   ruled the GUI a system extension, and `ADR-016` §6's example of a genuine
   base variant was withdrawn with it. So `/system` is sized for **one** base
   image, which is smaller and far more predictable than sizing for an unbuilt
   desktop.

   Extension images live on `/state` (§6 loop-mounts them, and it is the only
   writable filesystem in §4's layout). **This does not create a new sizing
   problem, and an earlier draft of this item wrongly said it did.** §4 already
   ruled the general case: `/state` is one partition taking the remainder of the
   device, with a *reserved-space guarantee* rather than a fixed size, precisely
   because "a partition solves it by fixing a size that has to be guessed years
   in advance, and the guess will be wrong." A desktop extension is one more
   consumer of that guarantee, not an exception to it.

   **So the GUI ruling made this item easier, not harder.** The only genuinely
   fixed sizes on the device are the two `system` slots, and they now hold one
   base image whose size is knowable instead of an unbuilt desktop's. What
   remains is the ordinary question of what the reserved-space guarantee should
   be — answerable from a measured base image, which is `PLAN.md` §6's tracked
   numbers, not a guess to be made in advance.

   `ADR-016` §7 item 4 carries the storage side of this.
3. **When re-provisioning from the boot partition is correct** (§8). **Still
   open, and now reached by running code**: `rivet-store` refuses to overwrite
   a stored document it cannot read and names this item on the console when it
   does. A ruling here turns that refusal into a recovery.
4. **`f2fs` evaluation**, once there is a running system and a measured write
   volume rather than a guess.

---

## 10. `/state` grows on first boot, and the image stops being card-sized

* **Status:** Accepted and **built** — ruled 2026-08-23, built 2026-08-24, and
  §10.3's "unresolved" is resolved in §10.7. It has grown a `/state` from
  254 MiB to 5.7 GiB on a real Linux kernel over a loop device. **IT HAS RUN ON
  A BOARD.** The supervisor performs it as a one-shot before it converges any
  service; §10.7.6 is the honest account of what is and is not proven.
* **This bullet said otherwise until 2026-08-29, and it is `AGENTS.md` §7 in its
  less-watched direction: a stale claim of ABSENCE.** The 2026-08-28 card grew
  its own `/state` to **118 GiB on a Raspberry Pi 4** on first boot, and the
  supervisor said so on the next boot in its own words —
  `"/state already fills the device; nothing was written"` — which proves the
  grow AND §10.3's idempotence on hardware in one line. `PLAN.md` §63.1 carries
  the reading. **`PLAN.md` §94.6 found this exact staleness on 2026-08-29,
  wrote *"that is now stale and should be updated"*, and the update was not
  made** — so the finding sat one file away from the sentence it was about,
  which is the whole failure this correction pass exists for.

### 10.1 The problem, measured rather than estimated

§4 rules that `/state` takes **the remainder of the device** — **the system
device**, per §4's own scoping note and `ADR-031` §2.1 — with a
reserved-space guarantee rather than a fixed size, because "a partition solves
it by fixing a size that has to be guessed years in advance, and the guess will
be wrong."

Until this section, the only mechanism honouring that ruling was **building the
image at the size of the card**. `cmd/rivet-image` defaulted to 7000 MiB — an
8 GB card less the slack such cards lack — and `/state` expanded to fill
whatever was left. The image was correct. Its *cost* was never examined.

Measured on 2026-08-23, from the first image that ever booted a board:

| | |
| :-- | --: |
| Image as built | 7000 MiB |
| `/state`, allocated | 5208 MiB |
| `/state`, occupied | **0 bytes** |
| Whole image, non-zero bytes | **~67 MB** |
| Whole image, `xz -9` | **22.4 MB** |

**The image was 99% zeroes**, and writing and verifying it cost the maintainer
over twenty minutes per iteration — to deliver two binaries totalling about
1 MB, which is all that changes between most iterations.

That is not a rounding error in the development loop. It *is* the development
loop, and `ADR-026` exists precisely because the cost of reaching the board
determines how much gets proven there.

### 10.2 The decision

**The image is built at the layout's minimum (§4's partitions plus a small
`/state`), and `/state` is grown to fill the device on first boot.**

`cmd/rivet-image`'s default device size is therefore `minimum_device_mib()`
— 2048 MiB — rather than a card size. `--device-mib` still accepts a larger
value for anyone who wants the old behaviour.

**This serves §4's ruling better than the thing it replaces, and that is the
argument for it.** A fixed-size image cannot know the remainder of a device it
has never seen; sizing the image to one particular card honoured §4 *for that
card only*, and silently mis-sized `/state` on every other card. Growing at
first boot is the only mechanism that honours §4 on a card of any size — which
is what §4 actually says.

It is also, unremarkably, what Raspberry Pi OS does. That is worth stating
plainly rather than arriving at by invention: a small distributed image with a
first-boot resize is the settled convention for this hardware, and `ADR-009`
§2's reuse posture applies to conventions as much as to code.

### 10.3 What first boot must do

Growing `/state` is two steps, and **only the second is unusual**:

1. **Move the GPT and enlarge partition 5.** The backup GPT header sits at the
   end of the *device*, not the end of the image, so a freshly written card has
   its backup header in the wrong place and partition 5 ending early. Both are
   the same repair.
2. **Grow the ext4 filesystem** inside the enlarged partition.

**Constraints this must respect:**

* **It must be safe to interrupt.** A card pulled mid-resize must still boot.
  This is the same reasoning §5 applies to updates and §8 to a damaged
  `/state`: the failure must land somewhere recoverable, and "the resize did not
  happen" is recoverable in a way "the partition table is half-written" is not.
* **It must be idempotent and self-disarming.** It runs when `/state` is smaller
  than the device allows, and does nothing otherwise. There is no flag file to
  go stale — the condition is observable from the device itself, which is
  `ADR-011`: one fact, one place, and the place is the hardware.
* **It must not be PID 1's job.** `ADR-017` §3.1.5 is a complete and deliberately
  short list, and this is not on it. It belongs above PID 1.
* **`/state` must not be mounted while it is resized**, unless the online path
  is used deliberately and knowingly.

**Unresolved: who does it and with what.** The candidates are Rivet's own Rust
(a GPT rewrite is well within reach; an ext4 resize is emphatically not) or
`resize2fs` and a GPT tool adopted as C mechanisms under `ADR-009` §6, which
Buildroot is being stood up to provide anyway. **`ADR-009` §3.2 requires the
survey before the choice**, and the survey has not been done. Recording the
decision without pretending the mechanism is settled.

### 10.4 The hazard this introduces, and what holds the line meanwhile

`cmd/rivet-image`'s old default carried a comment naming exactly the risk this
section creates:

> Deliberately not the *minimum*: a too-small default would silently produce a
> `/state` of a few megabytes, which boots and then fills.

**The hazard was live while §10.3 was unimplemented and the default was already
the minimum: a card written then had a 254 MiB `/state` and nothing to grow it.**

**Half retired, 2026-08-24, and the half that remains is the important half.**
§10.3's mechanism exists (§10.7) and has been run against a real kernel, so
*"nothing will grow it"* is no longer a property of the design. It remained a
property of **a card written that day**, because nothing called the mechanism
yet (§10.7.3). The two things below still hold the line, and the builder's
warning still names this section — §10.7.6 says exactly what would retire it.

**The hazard also turned out to have a second face that §10.4 did not see**, and
it is worse because it is silent even after the grow works: a 254 MiB `/state`
made `mke2fs` choose 1024-byte blocks, which caps how far the filesystem can
*ever* be grown at about 33 GiB. §10.7.4 has the measurement and the fix.

Two things hold the line until §10.3 exists:

1. **The builder says so on every build.** A card at the layout's minimum
   produces a warning naming this section and stating in terms that it is NOT
   IMPLEMENTED. `AGENTS.md` §3 — warn, never quietly proceed. A test asserts the
   warning is emitted, so it cannot be lost in a refactor.
2. **`--device-mib` still exists.** Anyone who needs a large `/state` before
   §10.3 lands passes the card's size and gets the old behaviour exactly.

**This is a deliberate, recorded trade: a known hazard that announces itself, in
exchange for a development loop that costs a quarter of what it did.** It is not
a defect to be discovered later, and §10.3 retires it.

---

### 10.5 The tracked image size, 2026-08-23 — four binaries rather than two

`ADR-002` §5.1 makes image size one of three numbers this project tracks, on the
argument that *"a property that is not measured on a machine that cannot feel
its loss is a property that is already gone."* §10.1's table is the last
measurement; this is the next one, taken after `cmd/rivet` and
`cmd/rivet-netd` were added to the tree.

> **A tracked measurement needs an artifact it can name.** Every row here and in
> §10.6 measured a file called `rivet.img`, and the numbers are only readable
> because the prose says which build each came from. `ADR-027` fixes that: an
> artifact carries its version, architecture and platform in its name, so a
> future row can cite the file it measured instead of describing it.

| | Before | After |
| :-- | --: | --: |
| Nominal image | 2048 MiB | **2048 MiB — unchanged** |
| `/system` payload, both slots | — | **+1 823 KiB** |
| Whole image, `xz -9` | 23 511 592 B | **23 659 792 B** (+148 200 B) |

**The nominal size does not move, and that is the layout doing its job.** §4's
partitions are fixed sizes and §10.2 builds at the minimum, so two binaries land
inside the 640 MiB `/system` slots without changing what an operator writes to a
card. What moves is the *distributed* artifact, by 145 KiB compressed.

**Method, because the two numbers are not the same measurement.** The `/system`
figure is `debugfs stat`'s `Blockcount` for each file — 736 and 1088 blocks of
512 bytes, so 933 888 B per slot and 1 867 776 B across the two. The compressed
figure is the difference between two images built by the same tool from the same
inputs, one of them with the two new binaries supplied as empty files:
everything else about the two images is identical, so the delta is those two
payloads and nothing else.

**On-card sizes**, from the same build: `rivet` 375 648 B, `rivet-initd`
609 280 B, `rivet-netd` 553 904 B, `rivet-sysd` 491 576 B — all
`0755`, uid 0, gid 0, in both `/system` slots, and byte-identical to the
cross-compiled inputs. Two builds of the same inputs produced the same sha256,
so §10's arrangement and `ADR-015` §4's signing are both undisturbed.

### 10.6 The next measurement, 2026-08-23 — the first C mechanism

Taken after `cmd/rivet-image` began installing the first C mechanism from
Buildroot — a 535 744 B stripped binary. That mechanism is not
in an image today, and the measurement is kept because **what it measures is the
shape of a C mechanism arriving**, which is the recurring question, not that
particular program.
Same method as §10.5, same machine, same day; the "before" column is §10.5's
image rebuilt from the current tree rather than §10.5's own numbers, so the
delta is the mechanism and nothing else.

| | Before | After |
| :-- | --: | --: |
| Nominal image | 2048 MiB | **2048 MiB — unchanged** |
| `/system` payload, both slots | — | **+1 048 KiB** |
| Whole image, `xz -9` | 23 684 284 B | **24 139 120 B** (+454 836 B) |

**The nominal size still does not move**, for §10.5's reason: §4's partitions are
fixed and 640 MiB of `/system` slot absorbs half a megabyte without an operator
noticing. What moves is the distributed artifact, by **444 KiB compressed** —
which is the largest single jump so far and is worth a sentence, because it is
the shape of every C mechanism still to come. `wpa_supplicant` and BusyBox are
both larger than this.

**Method.** The `/system` figure is `debugfs stat`'s `Blockcount` — 1048 blocks
of 512 bytes, so 536 576 B per slot and 1 073 152 B across the two, against a
file size of 535 744 B. The compressed figure is the difference between two
images from the same tool and the same four Rust binaries, one built before the
sixth positional argument existed and one after. That the C binary compresses to
87% of its on-disk size while Rivet's own binaries compress far harder is
expected of a stripped C binary against Rust with `lto` and `panic = "abort"`.

**On-card**: 535 744 B, `0755`, uid 0, gid 0, in
both `/system` slots, sha256-identical to the Buildroot output. `/etc` is still
empty in both slots and `/state` still holds nothing but `lost+found`. Two
builds of the same inputs produced the same sha256, so §10's arrangement and
`ADR-015` §4's signing remain undisturbed by the arrival of a binary this
project did not compile.

---

### 10.7 Built 2026-08-24 — the survey §10.3 asked for, and what it decided

§10.3 recorded the decision and refused to pretend the mechanism was settled:

> **Unresolved: who does it and with what.** The candidates are Rivet's own
> Rust (a GPT rewrite is well within reach; an ext4 resize is emphatically not)
> or `resize2fs` and a GPT tool adopted as C mechanisms under `ADR-009` §6 …
> **`ADR-009` §3.2 requires the survey before the choice**, and the survey has
> not been done.

This is the survey and the choice. It changes the *shape* of the answer rather
than picking one of the two candidates, so the table is worth reading before the
prose.

#### 10.7.1 The survey: three operations, and the kernel already does the hard one

| Step | Candidate mechanisms | Chosen | Cost |
| :-- | :-- | :-- | --: |
| Move the GPT, enlarge partition 5 | Rivet's own Rust; `sgdisk`; `sfdisk` | **Rivet's own Rust** | ~600 lines, no dependency |
| Tell the running kernel the partition changed | `BLKRRPART`; `BLKPG_RESIZE_PARTITION`; `partx -u` | **`BLKPG_RESIZE_PARTITION`** | one `ioctl` |
| Grow the `ext4` | `resize2fs` as a C mechanism; reimplement it in Rust; `EXT4_IOC_RESIZE_FS` | **`EXT4_IOC_RESIZE_FS`** | one `ioctl` |

**The third row is the one §10.3 called "emphatically not" within reach, and the
answer is that it never was Rivet's problem.** `resize2fs` growing a *mounted*
filesystem does exactly one thing: it issues `EXT4_IOC_RESIZE_FS` with a block
count and lets `fs/ext4/resize.c` do the work, journalled, in the kernel. So
Rivet issues the same `ioctl` and adopts no mechanism — no `resize2fs` in
either `/system` slot, no e2fsprogs recipe in Buildroot, no new licence in
`deny.toml`. `ADR-009` §2's reuse posture is honoured by reusing **the kernel**,
which was already on the card.

**Reimplementing an offline `ext4` resize in Rust was considered and is refused
outright**, and the reason is worth keeping: growing an `ext4` offline means
writing group descriptors, block bitmaps and inode tables, consuming reserved
GDT blocks, and getting `flex_bg` and `metadata_csum` right. It is thousands of
lines whose failure mode is a filesystem that mounts and is wrong.

**`BLKRRPART` is refused for a specific reason, not a general one.** Re-reading
the whole partition table requires every partition on the disk to be unused, and
`/system` is mounted from this one. `BLKPG_RESIZE_PARTITION` changes one
partition's length on a busy disk, which is what `parted` and `growpart` use and
what this needs.

#### 10.7.2 The consequence: `/state` is mounted, deliberately and knowingly

§10.3's fourth constraint is *"`/state` must not be mounted while it is resized,
**unless the online path is used deliberately and knowingly**."*

The survey settles that clause rather than working around it. The only mechanism
that does not require writing a filesystem resizer **is** the online one, so
`/state` is mounted while it happens. That is the deliberate part; this section
is the knowing part.

What is being trusted is `ext4`'s own online-resize path: journalled, the same
code `resize2fs` reaches, and what every cloud instance in the world runs on
first boot. What is *not* being trusted is anything of Rivet's own touching
filesystem metadata — this project writes no `ext4` structure at all. It reads
one field of the superblock and issues one `ioctl`.

#### 10.7.3 Who does it: **not PID 1**, and the survey is why

§10.3 ruled it out in as many words, and `cmd/rivet-initd/src/seed.rs` restated
the refusal when `ADR-017` §3.1.9 grew PID 1's list for something else:

> Growing `/state` is a GPT rewrite and an `ext4` resize — a mechanism
> `ADR-018` §10.3 records as not even surveyed yet — and it has no dependency on
> anything PID 1 already holds. The list resists *machinery*, and it resists
> work that could equally be done above it. This is neither.

**That refusal survives the survey, and the survey strengthens it.** The
mechanism is the *online* path (§10.7.2), so the filesystem must already be
mounted — and PID 1 is the process that mounts it (`ADR-017` §3.1.5 item 1,
§3.1.7). Everything this operation needs exists only *after* PID 1's first item
is done, so there is nothing about it that only PID 1 can do. §3.1.8's own test
answers it: *"it resists work that could equally be done above it."* This can.

**So `ADR-017` §3.1.5's list does not grow, and no amendment to `ADR-017` is
written.** That absence is recorded here rather than left to be noticed, for the
reason §3.1.8 gives: *"a line quietly not added is as easy to miss later as a
line quietly added."*

**Ruled: `cmd/rivet-sysd`, as a one-shot before it converges any
service** — the same placement and the same argument as the runtime `/etc`
projection (`ADR-017` §3.1.8). It runs after `/state` is mounted, it has a
journal to report into, and `ADR-017` §3.1 already puts a component's
preconditions in the component. A failure degrades the boot and never withholds
the confirmation (§4.1, §8): a rollback cannot repair a `/state` an update never
touches.

`rivet-adapter-image`'s `grow` module holds every decision behind a port; the
implementor is four syscalls at the supervisor's own `libc` wall
(`src/kernel.rs`), exactly as `rivet-store`'s `Filesystem` and this crate's
`BootMedium` are adapted there and in `cmd/rivet-initd`.

**BUILT 2026-08-25**, ruled in by the maintainer the same day and for a reason
this section did not have: `PLAN.md` §13.4 measured a board whose `/state` is
**232 MiB on a 7.42 GiB card**, and established that such a machine **cannot
receive a system update at all** — `apply install` names a payload *path*, so a
683 MB payload has to fit on `/state`. The grow stopped being a tidiness item and
became the thing that decides whether a machine can ever be updated over the
wire.

**The seam is the operation, not the primitives.** `Kernel::grow_state(&mut
self) -> Grew` — one method, returning a total type. Seven raw block-device
primitives on the supervisor's own port would have obliged its fake to simulate a
GPT and an `ext4` superblock in order to test decisions that are not that crate's,
and `ADR-011` would then have had two answers to what a grow does. `LibcKernel`
builds a `GrowMedium` over two descriptors and calls `grow_state`; the fake
answers with a canned outcome.

**Two descriptors, and §10.7.2's measurement is why.** The whole disk `O_RDWR`
for the GPT, and `/state`'s own partition for the superblock read and the resize
ioctl — because a mounted filesystem's writes go through the *partition* device's
page cache and the whole-disk device has a different one.

**The placement is before the `/etc` projection rather than after.** Both are
one-shots and neither depends on the other; the earlier the GPT rewrite
completes, the smaller the window `grow`'s write order is protecting. A test
asserts the order through the journal, and both halves of it were **watched
failing** — a second call trips the once-per-boot count, and moving the call
after `/etc` trips the order.

**A portability difference the host build could never have found.**
`libc::ioctl`'s request argument is `c_ulong` on glibc x86-64 and `c_int` on
`aarch64-unknown-linux-musl`. A constant typed for either fails to compile
against the other, and this crate builds for both. The request numbers are `u32`
— which is what `linux/ioctl.h` actually encodes — and every call site casts.
`cargo check --target aarch64-unknown-linux-musl` is what caught it.

**What is NOT proven, and it is the whole of it.** *Nothing has run this on a
card.* The decisions have a loop-device run behind them (§10.7); the four ioctls
have **never executed anywhere** — not on a board, not under QEMU, where
`ADR-026` §4 leaves the guest with no `/dev/mmcblk0` at all and this correctly
answers `ENOENT` every boot. **The first board that boots a system carrying this
is the test**, and the honest way to run it is behind `ADR-018` §5: put it in the
inactive slot, trial-boot it, and let a slot that does not come up roll back.
That does not protect the *card* — a GPT is not slot-local — which is why
§10.4's write order and its measured account of what Linux does with a damaged
primary GPT are the things standing behind it, not the A/B mechanism.

#### 10.7.4 A defect §10.2 introduced that nobody had looked for: 1 KiB blocks

**`mke2fs` picks its geometry from the size of the filesystem it is making**,
and its `small` usage type — anything under 512 MB — carries `blocksize = 1024`.
§10.2 made the image 2048 MiB, which makes `/state` **254 MiB at build time**,
so every card built since 2026-08-23 has had a 1 KiB-block `/state`. Nothing
asked for it and nothing announced it.

It matters because the block size fixes how far the filesystem can **ever** be
grown: `mke2fs` reserves group-descriptor blocks for future growth, and what
they buy depends on the block size. Measured on `e2fsprogs 1.47.4`, same
254 MiB, 2026-08-24:

| `mke2fs -b` | reserved GDT blocks | ceiling an online grow can reach |
| --: | --: | --: |
| 1024 | 256 | **~33 GiB** |
| 4096 | 31 | **~256 GiB** |

**A 254 MiB `/state` that can never exceed 33 GiB is §4's own failure
reintroduced by the back door** — *"a partition solves it by fixing a size that
has to be guessed years in advance, and the guess will be wrong"* — except that
the guess is now made by a heuristic reading a number that is an artefact of how
the image is *distributed*. `cmd/rivet-image/src/layout.rs` names the
maintainer's **128 GB working card**; that card is inside the 4 KiB ceiling and
outside the 1 KiB one.

**Fixed: `cmd/rivet-image` passes `-b 4096` for every `ext4` it makes.** It
changes nothing for the 640 MiB `/system` slots, which were already over
`mke2fs`'s threshold and already getting 4096. A test asserts both the flag and
the value, so it cannot drift back to a heuristic.

**Cards already written keep 1 KiB blocks**, and the grow works on them — it was
measured on one, 254 MiB to 5.7 GiB. They keep the 33 GiB ceiling, which is
above every card this project has been run on.

#### 10.7.5 What Linux does with a half-written GPT, and one question it raises

**The claim every reader repeats — that a GPT has two copies and a reader falls
back to the second — is false for Linux**, and this section exists because the
mechanism was first written on the strength of it.

Measured on Linux 6.6, against a whole, already-grown card image on a loop
device, one damaged structure at a time, 2026-08-24:

| The primary GPT | partitions the kernel created | `sgdisk -p` |
| :-- | --: | :-- |
| intact | **5** | the table |
| header zeroed, entry array intact | **0** | falls back to the backup |
| entry array corrupted, header intact | **0** | falls back to the backup |

`block/partitions/efi.c` says why: the alternate GPT is only located by
following *the primary header's* `alternate_lba`, and the fallback that reads
the last LBA directly sits behind `force_gpt`, which is off unless **`gpt` is on
the kernel command line**. A card whose primary GPT is self-inconsistent has no
partitions at all as far as Linux is concerned, and does not boot.

**So the rewrite is ordered to make that state as short-lived as a state can
be**: the entire backup GPT is written and flushed first, then **only the
entry-array sectors whose bytes actually change** — one, because partition 5's
ending LBA is eight bytes — and then the primary header. The exposure is the
interval between two adjacent 512-byte writes, once in a card's life. It is the
same exposure `sgdisk`, `parted` and `growpart` all carry, because a GPT cannot
be committed in fewer than two places.

**It is recoverable, and the recovery was measured rather than assumed.** A card
pulled inside the window, put in a host and given `sgdisk -e`, came back with
five partitions and a clean `e2fsck`, **at the layout it had before the grow** —
so it boots, and the next boot grows it. Not the grown layout: the stale primary
header still points its `alternate_lba` at the image's original backup, and that
is the copy a repair follows. Landing on the old whole layout is the better of
the two outcomes anyway, because it is the one this operation knows how to
finish.

> **Open, for the maintainer: should `gpt` go on the kernel command line?**
> `cmdline.txt` is `cmd/rivet-image`'s (`ADR-002` §4), and adding `gpt` would
> make the kernel fall back to the backup GPT at the last LBA — which this
> operation always writes first, and always writes whole. It would turn the
> window above from *"the card needs a host"* into *"the card boots and finishes
> the job"*. It is **not** taken here, for one reason that cannot be settled on
> this development host: the **Raspberry Pi firmware** reads the GPT before the
> kernel exists, and what it does with an inconsistent primary is unknown and
> untested. A kernel that tolerates the state does not help a board that never
> reaches the kernel. Recorded rather than decided.

#### 10.7.6 What has been proven, and what has not

`AGENTS.md` §6 — *until it runs, it isn't working* — and the two halves of that
are stated separately.

**Proven, against a real Linux 6.6 kernel, a real loop device and a real mounted
`ext4`**, driving the shipped `rivet-adapter-image::grow` code from a scratch
composition root that supplies the four syscalls:

* A 2048 MiB Rivet image on a 7.42 GiB device — the size the board's kernel
  named in `GPT:4194303 != 15564799` — grew `/state` from **254 MiB to 5.7 GiB**
  and its filesystem from 65 024 to 1 486 336 blocks of 4096.
* `sgdisk -v` reported **no problems**; the kernel's own partition table showed
  the grown partition; `e2fsck -fn` was clean; `df` showed the space.
* **The second and third runs wrote nothing** and returned *"already fills the
  device"*, including after a remount — §10.3's idempotence, with no flag file
  anywhere.
* **A power cut before each of the four writes**, five cases, each from a fresh
  copy of the image: cuts one to three leave the *old* whole layout, the kernel
  sees five partitions, `e2fsck` is clean and the next boot completes the grow;
  the fourth is §10.7.5's window and its host recovery; after the last write
  there is nothing left to do.
* The two defects that sequence found are both fixed and both documented where
  they were found: `statvfs` reports *usable* blocks rather than the
  filesystem's size, and the whole-disk device's page cache is not coherent with
  a mounted partition's.

**Not proven, and none of it is close to proven:**

> **Two entries left this list on 2026-08-29**, having been true on 2026-08-24
> and false for days before anybody noticed. A list headed *"not proven"* that
> has stopped being accurate is the most expensive kind of stale sentence in
> this repository: it reads as caution, so nobody re-checks it, and it tells the
> next agent not to trust something that has since been demonstrated.
> `AGENTS.md` §7 binds in both directions and this is the direction nobody
> watches. History has both, with what settled them.
* **The firmware's behaviour on an inconsistent primary GPT is unknown**
  (§10.7.5), so the size of the window's consequence on real hardware is a
  guess.
* **§10.7.4's 33 GiB ceiling is arithmetic, not a measurement.** No filesystem
  has been grown until it refused.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §4.1 — the `/state` device was named by number

**Corrected 2026-08-28 — `PLAN.md` §62.** The superseded paragraph read:

~~**The device is named by number, not by `PARTLABEL`.** `cmd/rivet-image`
writes the label and mounting by it would survive repartitioning — but
`/dev/disk/by-partlabel/` is **udev's**, and `ADR-005` §2.1 rules that Rivet
has no udev, so nothing on a board creates those symlinks. Resolving a label
without udev means parsing the GPT out of the raw disk inside the process that
must not fail. The number is what the image just wrote.~~

Neither premise had to be given up. The kernel is a third resolver, and it
publishes each partition's label on its sysfs `uevent`.

### In §5.3 — the section was headed unverified

**Changed 2026-08-22**, when a board was read. The superseded heading fragment
was ~~Unverified, and load-bearing~~.

### In §5.3 — nothing could request a tryboot reboot

**Superseded 2026-08-23**, and again later the same day. Three superseded
fragments:

~~nothing in Rivet's userland can yet: `reboot(2)` with
`LINUX_REBOOT_CMD_RESTART2` is not on the `Kernel` port. `PLAN.md` §0.0 makes
that the next thing to build.~~

~~`cmd/rivet-initd/src/slots.rs`'s~~ — the port moved to `rivet-adapter-image`.

~~**Nothing calls either** — the caller is the update path, which has no ADR and
is deliberately not built —~~

### In §5.3.2 — the rollback had never been seen

**Closed the same evening, 2026-08-23 — §5.3.3.** The superseded paragraph read:

~~**What this does not close.** The *rollback* — a trial boot whose commit is
withheld returning to the previous slot on the next reset — was **not**
exercised, because the trial boot was healthy and confirmed. It is the half the
whole design rests on and the half nobody has ever seen.~~

### In §5.3.1 — the update path was deliberately not built

**Superseded 2026-08-24**; both premises are gone. The superseded sentence read:

~~The **update path** — fetching, verifying and
writing a payload — is still deliberately not built, because `ADR-027` §9 item 1
leaves the payload format undecided and §8 item 1 of `ADR-028` leaves the
signature scheme open.~~

`ADR-029` decided the format on 2026-08-23 and `ADR-029` §7.1 the scheme, and
the maintainer's 2026-08-24 ruling then made the installer its own supervised binary.
~~Fetching~~ is the one word of it still true, and the body says so.

### In §5.3.1 — a board could be restarted only by removing power

**Ruled and built later on 2026-08-23 — `ADR-017` §3.1.10.** The superseded
paragraphs read:

~~**What is still missing, and it is the thing that blocks the experiment.**
Nothing on a Rivet board can *request* the one-shot restart. `RebootHow::
TrialRestart` is on `cmd/rivet-initd`'s `Kernel` port and **has no caller**;
`ADR-017` §3.1.5 item 6 gives shutdown to PID 1, which restarts on `SIGINT` and
powers off on `SIGTERM`; and no binary in the image could
send a signal to PID 1 or ask it for anything. **A Rivet board can currently be
restarted only by removing power, and a power cycle does not carry the one-shot.**~~

~~So §5.3 item 2 is now blocked on *how an authenticated principal asks PID 1 to
restart*, which no ADR answers. That is a maintainer's decision (`AGENTS.md` §1).~~

### In §5.3.1 — the signal had no sender, and had never executed

**Both changed later on 2026-08-23**, which is the second and third time in one
day that paragraph outlived its subject. The superseded fragments read:

~~The signal is *receivable*; the three `rivet apply` subjects that would send it
— `power-off`, `restart`, `trial-restart` — are a separate change in
`cmd/rivet`.~~

~~And **nothing has executed `LINUX_REBOOT_CMD_RESTART2` on any machine**~~

### In §9 item 1 — GPT or an extended partition was open

**Answered 2026-08-22: the firmware reads GPT.** The superseded question was:

~~GPT or an extended partition~~

### In §10 — the grow was not implemented, then had no caller, then had not run on a board

**Corrected three times: 2026-08-24, 2026-08-25 and 2026-08-29.** The sequence,
in order:

~~**Status:** Accepted, **not implemented.** Ruled 2026-08-23.~~

~~**it has not run on a board and nothing calls it yet.**~~

~~**something calls it since 2026-08-25 — §10.7.3 — and it has still not run on
a board.**~~

~~the four ioctls have never executed anywhere, because the emulator has no
`/dev/mmcblk0`.~~

**`PLAN.md` §94.6 found this exact staleness on 2026-08-29, wrote *"that is now
stale and should be updated"*, and the update was not made** — so the finding
sat one file away from the sentence it was about, which is the whole failure
this correction pass exists for.

### In §10.4 — the hazard was live and nothing would grow a card

**Half retired 2026-08-24.** The superseded paragraph read:

~~**That hazard is real and is now live**, because §10.3 is unimplemented while
the default is already the minimum. A card written today has a 254 MiB `/state`
and nothing will grow it.~~

### In §10.7.3 — the supervisor half was not built

**BUILT 2026-08-25.** The superseded paragraph read:

~~**This half is not built.** `rivet-adapter-image`'s `grow` module holds every
decision behind a port; the implementor is four syscalls at the supervisor's own
`libc` wall (`src/kernel.rs`), exactly as `rivet-store`'s `Filesystem` and
this crate's `BootMedium` are adapted there and in `cmd/rivet-initd`. Until
that is written **a card still does not grow**, and §10.4's warning is still the
thing holding the line.~~

### In §10.7.6 — two entries in the "not proven" list

**Both struck 2026-08-29**, having been false since 2026-08-28 and 2026-08-25
respectively. They read:

~~**Nothing has run on a Raspberry Pi**, and `ADR-026` §4 puts `/state`, the
boot partition and the firmware in the column only a board can answer. A loop
device is not an SD card: it does not wear, does not lie about flush, and does
not go read-only when it is dying.~~

The card built on 2026-08-28 grew its own `/state` to **118 GiB** on a Raspberry
Pi 4, measured on the board rather than assumed — `PLAN.md` §63.1. *The sentence
about loop devices remains true and is why this matters*: what a board settled is
the ordinary grow, not wear, not a flush that lies, and not a card going
read-only as it dies.

~~**Nothing calls it.** §10.7.3's supervisor half is unwritten, so a card
written today still does not grow. This is the single thing standing between
the mechanism and §10.4's retirement.~~

§10.7.3's supervisor half landed on 2026-08-25 — and §10's own status block said
so on that date while this bullet went on saying the opposite for four days.
**Two sections of one document contradicting each other, with nothing to tell a
reader which is current, is the `ADR-015` §3.2 defect exactly.** The supervisor's
message off the 2026-08-28 board is the proof that it calls it:
`"/state already fills the device; nothing was written"`.
