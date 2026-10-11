# ADR-024: Kernel Configuration

* **Status:** Accepted — ruled 2026-08-22. §3 is the **single authority** for
  kernel requirements; other ADRs reference it rather than repeating flags.
  **The audit ran 2026-08-22 and the stock kernel fails §3 on Landlock —
  the custom kernel is scheduled, not optional. See §8.1.**
  **The custom kernel was built 2026-08-23 and boots under QEMU; §8.2 records
  what it closed, and that §8.1's `MEMCG` explanation of item 4 is wrong.**
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`ADR-002` §5.2 deferred a custom kernel until the configuration was understood
well enough to know what was being removed, with a recorded risk that dropping
`CONFIG_BCMGENET` would silently break Stage 1.

Twenty ADRs later, the requirement list has accumulated across seven documents.
That changes the question: Rivet no longer needs to know what to *remove*, it
needs to know what it *depends on* — and once that is written down, "does the
stock kernel suffice" becomes a check rather than a judgement.

---

## 2. The requirements table is the single authority

Every flag Rivet depends on is listed in §3 and **nowhere else**. Other ADRs
name the capability and point here; none carries its own list.

This is `ADR-011` applied to a checklist. Seven separate flag lists, each
partially maintained, is precisely how `CONFIG_BCMGENET` gets dropped by someone
tidying a config who consulted the wrong one.

---

## 3. What Rivet requires of the kernel

| Flag | Required by | For |
| :-- | :-- | :-- |
| `DEVTMPFS`, `DEVTMPFS_MOUNT` | `ADR-005` §2.1 | no udev — the kernel populates `/dev` |
| `BCMGENET` | `ADR-004` §2.3 | `eth0`, the Stage 1 bring-up scaffold |
| `BRCMFMAC`, SDIO support | `ADR-004` §8 | wireless |
| `FW_LOADER` | `ADR-004` §8 | firmware read from `/lib/firmware` in-kernel |
| `EXT4_FS` | `ADR-018` §7 | `/state` |
| `VFAT_FS` | `ADR-004` §6 | the boot partition is FAT32 (`ADR-018` §4), and PID 1 mounts it to read the provisioning file. **Added 2026-08-22** — found by implementing `ADR-017` §3.1.5 item 2, not by review. Satisfied by the stock kernel, so nothing was broken; it was simply missing from the authority |
| `EROFS_FS` | `ADR-018` §3 | `/system`, read-only *by format* |
| `BLK_DEV_LOOP` | `ADR-018` §6 | extensions are loop-mounted image files |
| `TMPFS` | `ADR-016` §3, `ADR-018` §6 | `/run`, and the `/system/ext` mount point |
| `CGROUPS`, cgroup v2 | `ADR-015` §5.1, `ADR-017` §3.1.3 | resource limits, and the authority for what is running |
| `SECURITY_LANDLOCK` | `ADR-015` §5.1 | filesystem restriction with no policy language |
| `SECCOMP`, `SECCOMP_FILTER` | `ADR-015` §6 | a later enforcement stage, declared now |
| `NET_NS`, `PID_NS`, mount namespaces | `ADR-015` §5.1, §5.3 | the sandbox |
| `NF_TABLES`, `NF_TABLES_INET` | `ADR-023` §2, §7 | the generated ruleset, both address families |
| `NF_CONNTRACK` | `ADR-023` §3.1 | without it `egress` never receives replies |
| `VETH` | `ADR-023` §8 | only if group namespaces ever need egress |
| **`HW_RANDOM_IPROC_RNG200`** | entropy at first boot | needed for cryptographic key generation at first boot. **Corrected 2026-08-23** from `HW_RANDOM_BCM2835`: the Pi 4 is a BCM2711 and its device tree says `compatible = "brcm,bcm2711-rng200"`, which is `iproc-rng200.c`. `HW_RANDOM_BCM2835` is the Pi 3's driver and matches nothing on this board |
| MMC / SDHCI | boot | reaching the root filesystem at all |
| `IKCONFIG`, `IKCONFIG_PROC` | §7 | so the running kernel can report its own configuration |

### 3.1 `CONFIG_USER_NS` is deliberately **off**

Unprivileged user namespaces are a well-known privilege-escalation surface and
Rivet does not need them: `rivet-sysd` runs as root and creates
namespaces on applications' behalf (`ADR-017` §3.1).

This is a free hardening win, and it is free *because* of how supervision was
structured. Worth recording as a decision rather than an omission, so that
nobody enables it later to make something work without understanding what it
costs.

---

## 4. Everything is built in; there are no modules

**Ruled 2026-08-22: `=y` throughout, never `=m`.**

The consequence is larger than a build preference. **Module loading is the last
remaining BusyBox applet** (`ADR-005` §3, §5.4.2), so building everything in is
what actually takes the scoreboard of `ADR-001` §4.1 to **zero** rather than
merely low.

It also removes `/lib/modules`, a module loader, and a class of boot-ordering
problem. Firmware is unaffected — the kernel reads `/lib/firmware` directly and
has never needed a userspace helper for it (`ADR-005` §2.1).

**The cost, stated plainly:** unanticipated hardware means rebuilding the kernel
rather than loading a module. For a purpose-built system on fixed hardware that
is the correct trade, and it is the same trade `ADR-008` made when it removed
the package manager.

---

## 5. The configuration is a fragment, not a full `.config`

**Ruled 2026-08-22:** a small file listing Rivet's requirements, merged over
the Raspberry Pi `bcm2711_defconfig`.

The reasoning is about reviewability. A full `.config` is several thousand lines
in which a meaningful change and noise are indistinguishable in a diff — which
is exactly how `ADR-002` §5.3's recorded risk materialises, with a Stage 1
dependency dropped by someone tidying. **A fragment is short enough that a
reviewer reads all of it**, and it is the design artifact: it says what Rivet
requires, and §3 says why.

The acknowledged weakness is that the base `defconfig` brings things along
unexamined. That is acceptable while the goal is *correctness*; when the goal
becomes *stripping*, the fragment is still the right place to record what must
survive the stripping.

**Buildroot builds the kernel**, using Rivet's fragment, from the Raspberry Pi
kernel tree rather than mainline. This is consistent with `ADR-009` §6: the
kernel is a Tier C mechanism, cross-building it is exactly the fiddliness
Buildroot exists for, and the *configuration* remains Rivet's own artifact.

---

## 6. Entropy at first boot

A classic embedded trap worth naming: keys are generated on a machine
that has just started, has no stored entropy, and has done almost no I/O.

The Pi's hardware random number generator solves this, but only if it is
**enabled and credited** — an uncredited source fills the pool without raising
the entropy estimate, and generation still blocks or proceeds weakly.

### 6.1 Answered 2026-08-23 — credited by default, and the named driver was wrong

The paragraph above says what to check and does not say what the answer is.
This section is the answer, read out of the two trees that decide it — Linux
6.12.61 as `tools/buildroot` builds it, and the key generator Buildroot
built — rather than from documentation or memory (`AGENTS.md` §5).

**What actually asks for entropy.** The generator's seeding path calls
`getrandom` twice: it tries
`getrandom(buf, len, GRND_NONBLOCK)` and, on `EAGAIN`, falls back to a
**blocking** `getrandom(buf, len, 0)`. A blocking `getrandom` returns when the
kernel's CRNG is *initialised*, and only **credited** entropy initialises it.
That is precisely the distinction §6 names, and it is why "the hwrng is enabled"
was never the question.

**Both crediting paths are on by default on this kernel**, and neither needs a
symbol adding:

| Source | Credited? | Evidence, by file and line |
| :-- | :-- | :-- |
| the firmware's `/chosen/rng-seed` | **yes** | `drivers/of/fdt.c:1038` calls `add_bootloader_randomness`; `drivers/char/random.c:822` is `static bool trust_bootloader __initdata = true` and `:987` credits `len * 8` bits |
| the SoC's hardware RNG | **yes** | `drivers/char/hw_random/core.c:46` `default_quality = 1024`; `:538` `rng->quality = min(min(default_quality, 1024), rng->quality ?: 1024)`; `:503` `entropy = rc * quality * 8` — and neither `bcm2835-rng.c` nor `iproc-rng200.c` sets a `.quality` of its own, so both take the maximum |

**So §6's trap is not live on this kernel, and the reason is a change in Linux
rather than anything Rivet did.** `CONFIG_RANDOM_TRUST_BOOTLOADER` does not
exist in this tree at all — checked two ways per `AGENTS.md` §7, a `grep` over
every `Kconfig*` and a `grep` over the whole tree, both empty — because upstream
removed the option and made trust the default. `default_quality` likewise
defaults to the maximum rather than to zero, which is what made an
enabled-but-uncredited hwrng the classic embedded stall this section was written
against.

**§6 is therefore not withdrawn.** It described real behaviour, it is still real
on older kernels, and the check it asks for is what found the next paragraph.
What changes is that the answer is now recorded rather than outstanding, and
that the actionable form of it is a **negative**:

> Never put `random.trust_bootloader=0` or `rng_core.default_quality=0` in
> `cmdline.txt`. Either reinstates the stall, from a file that reads like
> tuning. `cmd/rivet-image`'s `boot` module writes that file and writes
> neither.

**And the check found a defect it was not looking for.** §3's table and
`tools/buildroot/linux/rivet.config` both named `HW_RANDOM_BCM2835`. The
Raspberry Pi 4 is a **BCM2711**, and the preprocessed `bcm2711-rpi-4-b.dts` in
the kernel tree reads:

```text
rng@7e104000 {
        compatible = "brcm,bcm2711-rng200";
        reg = <0x7e104000 0x28>;
};
```

That string is in `iproc-rng200.c`'s `of_match` table, not `bcm2835-rng.c`'s. So
the required symbol is `HW_RANDOM_IPROC_RNG200`, and a kernel built from §3's
list as written would have had **no hwrng on this board at all** — an absence
that produces no error, just a source that never appears. It is `=y` in the
built `.config` today only because Buildroot's Pi 4 defconfig selects it; §5
makes the fragment Rivet's statement of requirement and §7 has the running
system check that statement, so a requirement naming the wrong symbol checks the
wrong symbol. Corrected in both places, 2026-08-23.

**What is unproven.** None of this has been observed on a board. Whether the
firmware supplies `/chosen/rng-seed` at all, and how long `getrandom` actually
blocks on a cold Pi 4 before the first host key is generated, are `ADR-026` §4's
right-hand column. The two cheap readings that would settle it are
`/proc/sys/kernel/random/entropy_avail` at the console and a `dmesg | grep -i
random` for `crng init done` and its timestamp.

**A first key was generated 2026-08-23, under QEMU, and it settles the
smaller half of this.** Generation ran as a precondition of the
service that needed it, so the invocation this section is about happened on every
first boot. On QEMU `-M virt` with this kernel, an ed25519 key generation ran to
completion in about **80 ms**, measured between the supervisor's own journal
records either side of it — so `getrandom` did not block measurably and the
stall §6 was written against did not occur.

**That is not the measurement §6 asks for**, and the difference is the whole
point of this paragraph. An emulated guest is not a machine with no stored
entropy. QEMU's own device tree carries the seed — read out of it rather than
assumed, with `qemu-system-aarch64 -M virt,dumpdtb=…` and `dtc`:

```text
chosen {
        stdout-path = "/pl011@9000000";
        rng-seed = <0x... 0x... 0x... 0x... 0x... 0x... 0x... 0x...>;
};
```

256 bits, which the table above shows this kernel **credits in full** via
`add_bootloader_randomness`. So the CRNG was initialised before the kernel had
finished booting and the fallback to a blocking `getrandom` was never taken.
That is the *arrangement* being confirmed end to end, and it is not the same as
the stall being absent on hardware. What remains open is specifically **a cold
Pi 4** — first power-on, no stored seed, and whatever the VideoCore firmware
does or does not put in `/chosen`. The two readings above are still the ones
that settle it, and they are still unmade.

---

## 7. Rivet verifies the kernel at boot and reports what is missing

`IKCONFIG_PROC` lets the running kernel report its own configuration. Rivet
checks §3's requirements at startup and **reports any that are absent.**

This is `AGENTS.md` §3's rule — *stubs refuse; they never fake success* — applied
to kernel features, and it matters because the failure mode is otherwise silent.
A Landlock call on a kernel without Landlock does not sandbox anything; an
`nf_tables` ruleset that cannot be installed leaves everything permitted. Both
would appear to work.

A missing requirement therefore surfaces as a stated fact at boot, in the
journal and in `rivet status`, rather than as a security property that quietly
does not exist. This is the same discipline as `ADR-015` §6.1's refusal to
silently accept an unenforceable manifest.

---

## 8. Custom kernel timing: audit first

**Ruled 2026-08-22.** The stock Raspberry Pi kernel is checked against §3, and
the answer decides the schedule rather than a preference doing it:

* **If it satisfies §3** — stay stock through Stage 2 and go custom when
  *stripping* becomes the goal, exactly as `ADR-002` §5.2 intended.
* **If Landlock, `nf_tables` or cgroup v2 are absent** — a custom build arrives
  sooner, because those cannot be worked around in userland.

**Do not strip during bring-up.** Stages 0–2 are spent asking why something does
not work, and debugging and tracing infrastructure is what answers that. Strip
after it works — the same reasoning that chose `ext4` over `f2fs`
(`ADR-018` §7) and static over shared linking (`ADR-014` §4.2).

---

### 8.1 The audit was run, 2026-08-22 — **the stock kernel fails §3**

Run against a Raspberry Pi 4 Model B Rev 1.1 (`raspberrypi,4-model-b`,
`brcm,bcm2711`) on stock Raspberry Pi OS, kernel `6.18.34+rpt-rpi-v8`. Read from
`/boot/config-$(uname -r)` — **not** `/proc/config.gz`, which does not exist,
for the reason given in the failures below.

**Present and satisfactory:** `DEVTMPFS`, `DEVTMPFS_MOUNT`, `BCMGENET`,
`BRCMFMAC_SDIO`, `MMC_SDHCI`, `MMC_SDHCI_IPROC`, `FW_LOADER`, `EXT4_FS`,
`BLK_DEV_LOOP`, `TMPFS`, `CGROUPS`, `SECCOMP`, `SECCOMP_FILTER`, `NET_NS`,
`PID_NS`, `NAMESPACES`, `NF_TABLES_INET`, `HW_RANDOM_BCM2835`.

**One entry of that list was the wrong question, 2026-08-23 — §6.1.** The audit
checked `HW_RANDOM_BCM2835` because §3's table asked for it, and found it
present. That is a true reading of an irrelevant symbol: the Pi 4's RNG is an
RNG200 and `bcm2835-rng.c` matches nothing in this board's device tree, so a
`y` there says nothing about whether the board has a hardware random source.
The symbol that answers the question is `HW_RANDOM_IPROC_RNG200`, and **it was
not checked** — it is neither in this list nor in the failures below, so its
state on the stock kernel is unknown rather than good. It is one more line to
read on the next board pass. The finding is a small instance of `ADR-011`: an
audit is only as good as the authority it audits against.

**Four failures, in descending order of cost:**

1. **`CONFIG_SECURITY_LANDLOCK` is not set.** This is decisive. It is not merely
   absent from the config — `/sys/kernel/security/lsm` reads `capability` and
   nothing else, and `/sys/kernel/security/` contains only `integrity` and
   `lsm`, so Landlock is neither compiled nor active. `ADR-015` §5.1's sandbox
   has no kernel to run on. **This is the flag that decides the schedule**, and
   §8's second branch is therefore the one that applies.
2. **`CONFIG_USER_NS=y`.** §3.1 wants it deliberately off. Not a blocker —
   nothing breaks by having it — but the hardening win §3.1 describes is not
   available from a stock kernel, which is itself an argument the custom build
   makes for itself.
3. **Six requirements are modules, not built in** — `BRCMFMAC`, `EROFS_FS`,
   `NF_TABLES`, `NF_CONNTRACK`, `VETH` and `IKCONFIG` are all `=m`. Each works,
   so this is not a functional failure; it is a direct contradiction of §4, and
   it is why `/proc/config.gz` is missing despite `CONFIG_IKCONFIG_PROC=y` —
   `IKCONFIG=m` and the module is not loaded. §7's self-report would have had
   nothing to read.
4. **The memory cgroup controller is disabled at the command line.** This one is
   not a kernel-config failure at all and is the most dangerous of the four,
   because nothing announces it. Raspberry Pi OS ships `cgroup_disable=memory`
   in `cmdline.txt`; `/sys/fs/cgroup` is correctly `cgroup2fs`, but
   `/sys/fs/cgroup/cgroup.controllers` reads `cpuset cpu io pids` — **no
   `memory`**. `ADR-015` §5.1 and `ADR-017` §3.1.3 both want memory limits, and
   they would have been accepted and silently ignored. Rivet owns
   `cmdline.txt` (`ADR-002` §4), so the fix is one deletion — but it has to be
   *known*, and §7's boot-time verification should assert the controller is
   present rather than assume `cgroup2fs` implies it.

   **CORRECTED 2026-08-23 — the command line was not the cause, and the fix is
   not one deletion.** Rivet booted its own image on the same board, with its
   own `cmdline.txt`, which reads in full:
   `console=serial0,115200 console=tty1 root=PARTLABEL=system-a rootfstype=ext4 rootwait init=/system/bin/rivet-initd`.
   There is no `cgroup_disable=memory` anywhere in it. PID 1 read
   `/sys/fs/cgroup/cgroup.controllers` and got **`cpuset cpu io pids`** — the
   same four, still no `memory`. Deleting a parameter that was never there
   cannot be the remedy —

   and the obvious inference, that the cause lies in the kernel with
   `CONFIG_MEMCG` as the leading candidate, **is WRONG. §8.2 disproved it the
   same day.**
   `CONFIG_MEMCG=y` is line 21 of `bcm2711_defconfig`, and decisively: the
   board's *own* `kernel8.img` — the same binary, `6.18.34+rpt-rpi-v8` — was
   booted under `-M virt` and PID 1 printed `cpuset cpu io **memory** pids`.
   One kernel image, controller present under an emulator and absent on the
   board. Nothing compiled in can explain that.
   **The cause is once again open**, and the command line is back in the frame
   rather than out of it: the Pi firmware *composes* the bootargs and appends
   `cmdline.txt` to what it built, so the file Rivet writes is not the whole
   of what the kernel receives.

   ### SETTLED 2026-08-23, on hardware, and item 4 is **closed**

   Both commands were run. `CONFIG_MEMCG=y`. And `/proc/cmdline` on a Pi 4
   running Raspberry Pi OS carries `cgroup_disable=memory` **among the
   firmware-composed parameters** — between `snd_bcm2835.enable_headphones=0`
   and `numa_policy=interleave`, before the `console=` at which `cmdline.txt`'s
   own contents begin. So the original diagnosis was right about *what* and
   wrong about *where*, and the correction above was wrong about *what*.

   **The remedy is an addition, not a deletion**, which is why both earlier
   readings failed to produce one: there is nothing in `cmdline.txt` to remove.
   `cgroup_disable=` can only be **countered**, and `cgroup_enable=memory` is
   what counters it. `cmdline.txt` is appended last, so Rivet's parameter wins
   on order — luck rather than design, and recorded as such in
   `cmd/rivet-image/src/boot.rs`.

   **Confirmed by Rivet's own first boot with the fix**, read off the HDMI
   console:

   ```
   /proc/cmdline: … cgroup_disable=memory numa_policy=interleave …
   /sys/fs/cgroup/cgroup.controllers: cpuset cpu io memory pids
   ```

   The firmware still disables it; the controller is present anyway. **`ADR-015`
   §5.1's memory limits have a kernel to run on, and §8.1's four failures are
   three.**

   **One defect the same boot exposed.** PID 1 capped `/proc/cmdline` at 240
   characters, and the board's firmware-composed half exceeds that on its own —
   so the console truncated *before* reaching the half Rivet writes, and the
   fix could be confirmed only from the controllers line. A cap that hides
   exactly what the line exists to show is worse than no cap, because it reads
   as evidence. Raised to 1024.

   **How this was caught twice is the point, and the second time is the sharper
   lesson.** The original item was found because §7 asked for boot-time
   verification rather than assumption, PID 1 was written to do exactly that,
   and it printed the contradiction unprompted on its first boot. **The
   correction above then reasoned from the file Rivet *wrote* instead of from
   what the kernel *received*** — the same class of error one paragraph after
   diagnosing it, and precisely what `AGENTS.md` §7 means by never concluding
   from a single probe. Reading a file you authored is not reading the system.

**What this changes.** §8's conditional resolves: **the custom kernel arrives
sooner.** Landlock cannot be worked around in userland, so `ADR-015`'s sandbox
is gated on the custom build, and ~~applications (Stage 4) cannot be confined
before it exists~~. Stages 0–2 are unaffected — everything they need is present
in the stock kernel, and §8's "do not strip during bring-up" still holds.

**The gate is discharged, 2026-09-05.** The custom kernel was built, it carries
Landlock, and an application confined by it was measured under QEMU reporting
**ABI 6** — every filesystem right `rivet_core::confinement` uses is enforced.
`ADR-042` is the confinement built on top of it, and `ADR-042` §6.2 carries the
console output.

**The struck clause is struck narrowly and the rest of this section stands.**
Everything above about the *stock* Raspberry Pi kernel remains true and remains
the measurement that decided the schedule. What stopped being true is the
forward-looking sentence, because the condition it was waiting on happened. A
reader reaching for a claim about Landlock here should now ask **which
kernel**: the stock one has none, Rivet's own answers 6, and the supervisor asks
the running kernel at every spawn rather than assuming either (`ADR-042` §5).

**Method, and what is unproven.** A configuration file read, plus four runtime
readings (`/sys/kernel/security/lsm`, `/sys/kernel/security/`,
`stat -f /sys/fs/cgroup`, `/sys/fs/cgroup/cgroup.controllers`) that check what
is *active* rather than what is *compiled* — a distinction that matters, since
a compiled-in LSM still has to be in `CONFIG_LSM` or `lsm=` to do anything.
**Unproven:** that a Rivet-configured kernel with these corrections builds and
boots on this board. That is the custom-kernel task itself, not an audit.

---

### 8.2 The custom kernel was built, 2026-08-23 — and §8.1's `MEMCG` hypothesis does not survive it

The build exists: `tools/buildroot/`, Buildroot 2026.02.3, an aarch64 musl
toolchain, and §3's requirements as the fragment §5 ruled for. It boots under
`qemu-system-aarch64 -M virt` (`ADR-026` §4.1). Three of §8.1's four failures
are closed by it and the fourth is not what the record says it is.

**§5's argument for a fragment paid off on the first build**, which is worth
recording because the argument was about reviewability and the payoff was not.
The fragment asked for `CFG80211`, `MAC80211`, `BRCMUTIL` and `BRCMFMAC` at
`=y`, per §3 and §4. `merge_config.sh` applied all four without complaint — and
then `olddefconfig` silently demoted every one of them back to `=m`, because the
base defconfig has `CONFIG_RFKILL=m` and a tristate cannot be `y` while
something it needs is `m`. **Four §3 requirements, contradicted in silence, with
no error anywhere.** Adding `CONFIG_RFKILL=y` fixes it, and the reason it was
caught at all is that `tools/buildroot/build.sh --show-config` reads the
produced `.config` back and compares it to what was asked for, rather than
trusting the merge. That check is now the standing method for §3 compliance at
build time, and it is the build-time half of what §7 does at boot.

**Closed, and confirmed from the source side rather than from a board.**
`merge_config.sh` reports the value each symbol held in the Raspberry Pi
`bcm2711_defconfig` before Rivet's fragment overrode it, which makes it an
independent reading of the same facts §8.1 gathered by running things:

| §8.1 item | The defconfig's own value | Rivet's fragment sets |
| :-- | :-- | :-- |
| 1. Landlock | `# CONFIG_SECURITY_LANDLOCK is not set`, and `CONFIG_LSM=""` | `=y`, and Landlock first in the list |
| 2. `USER_NS` | `CONFIG_USER_NS=y` | not set (§3.1) |
| 3. six requirements as modules | `BRCMFMAC`, `EROFS_FS`, `NF_TABLES`, `NF_CONNTRACK`, `VETH`, `IKCONFIG` all `=m` | all `=y` (§4) |

The empty `CONFIG_LSM` is worth pausing on: it is the *mechanism* behind §8.1's
runtime reading of `capability` and nothing else, and it means that compiling
Landlock in without also listing it would have produced exactly the same
`/sys/kernel/security/lsm` and looked like a failed build.

**Item 4 is the one that changes, and it is not a kernel-configuration failure
after all.** Two readings, taken this session:

1. **`CONFIG_MEMCG=y` is already in `bcm2711_defconfig`** — line 21 of
   `arch/arm64/configs/bcm2711_defconfig` in the Raspberry Pi kernel tree at the
   revision this build pins. Rivet's fragment does not turn it on; it was
   never off. So the symbol §8.1's correction named as the leading candidate is
   not a candidate.
2. **The stock kernel publishes the `memory` controller when it is booted
   somewhere other than the board.** `tools/qemu/kernel8.img` — the file
   `tools/qemu/fetch-kernel.sh` copied off the target, reporting
   `Linux version 6.18.34+rpt-rpi-v8` — was booted under
   `qemu-system-aarch64 -M virt` with Rivet's PID 1, which printed:

   ```
     /sys/fs/cgroup/cgroup.controllers: cpuset cpu io memory pids
   ```

   **`memory` is there.** The same kernel that omits it on the board offers it
   under an emulator, so whatever removes it is not compiled into the kernel.

That is the reading that settles the direction. §8.1's original diagnosis —
*something is disabling the controller at boot* — was right, and its 2026-08-23
correction was wrong to move the cause into the kernel. What the correction
established is narrower than what it claimed: Rivet's **`cmdline.txt`** does
not carry `cgroup_disable=memory`. That is not the same as the kernel not
receiving it.

**`cmdline.txt` is not the kernel command line.** The Raspberry Pi firmware
composes `/chosen/bootargs` and appends the file's contents to it; the kernel
receives the composition. §8.1 quotes Rivet's `cmdline.txt` "in full" and then
concludes about what the kernel was *told* — but that is the file Rivet
**wrote**, not the line the kernel **received**, and `AGENTS.md` §5 is explicit
about the difference: report what the system said, not what it was expected to
say. Under QEMU the kernel gets its command line directly from `-append`, with
no firmware in between, which is very likely why the same binary behaves
differently.

**The reading that closes it is one command on a booted board:**

```sh
cat /proc/cmdline
```

**It was taken the same afternoon this section was written**, and this section
went on asking for it until 2026-08-28. §8.1's
`SETTLED 2026-08-23, on hardware, and item 4 is closed` block, **earlier in this
file**, records both commands being run on a Pi 4: `CONFIG_MEMCG=y`, and
`/proc/cmdline` carrying `cgroup_disable=memory` among the firmware-composed
parameters. The remedy is the addition this section predicted —
`cgroup_enable=memory`, which `cmd/rivet-image` has written since — and the
first Rivet boot with it read `cpuset cpu io memory pids` back off the board.

**Two sections of one ADR disagreed about whether a question was open for five
days**, and the stale one was written in the imperative, so it is the one an
agent acts on. It cost a session on 2026-08-28 re-deriving a closed finding
before the contradiction was noticed (`PLAN.md` §63.5). The lesson is `PLAN.md`
§61's, arriving in an ADR instead of a table: **when a section is corrected, the
sections that cite the same question are what go stale**, and nothing but
re-reading them catches it.

Re-confirmed 2026-08-28 on a different card, slot and build, which is a
regression check rather than a finding: PID 1 printed `/proc/cmdline` with both
flags on it and `cgroup.controllers` with `memory` in it. §7 makes that reading
— not the flag — the authority for whether the request was granted, so it is
worth repeating and is not worth writing a new section about.

**Method, and what is unproven.** Two config-file reads and one emulated boot of
the board's own kernel image. **Unproven:** that the `kernel8.img` in
`tools/qemu/` is byte-identical to the kernel the board booted when §8.1's
correction was written — the version strings match and both came off that board,
but no hash was compared. **Also unproven:** anything at all about the board,
since neither reading touched it. And Rivet's own kernel publishing `memory`
under QEMU (`ADR-026` §4.1) proves the *fragment* is not wrong; it proves
nothing about the Pi, for the same reason.

**Nothing outside this section has been corrected.** `ADR-015` §5.1, `ADR-023`
§7 and `cmd/rivet-image/src/boot.rs` all still carry §8.1's original
`cgroup_disable=memory` story — which this section makes *more* plausible than
§8.1's correction left it, but for a different reason and with a different
remedy. `cmd/rivet-initd/src/boot.rs` still *prints* the old remedy as advice
at boot: "Rivet owns cmdline.txt, so the fix is one deletion there." **That
line is now known to be wrong** and should be reworded to point at
`/proc/cmdline` instead. It is left alone here only because `AGENTS.md` §7 says
to grep for the claim rather than the paragraph, and doing that properly is a
sweep across five files that should happen once, after the board reading — not
twice, on two different theories.

---

## 9. Open

1. **Run the audit — DONE 2026-08-22, §8.1.** It fails on Landlock, so §8's
   second branch applies and the custom kernel is scheduled rather than
   optional. Note for anyone repeating it: `/proc/config.gz` is **not** the way
   in on a stock image — `IKCONFIG=m` and the module is unloaded, so read
   `/boot/config-$(uname -r)` instead.
2. **Device tree and overlays** come from the kernel build. Which overlays
   Rivet's image ships — and whether `config.txt` references any beyond the
   base device tree — is unexamined. The build now produces
   `bcm2711-rpi-4-b.dtb` (§8.2), so the raw material exists.
3. **A hardening pass** — `STRICT_KERNEL_RWX`, `FORTIFY_SOURCE`,
   `SLAB_FREELIST_RANDOM`, restricting `/dev/mem` and `dmesg` — is deferred
   until after §8's "strip when it works" point, and is recorded here so it is
   not forgotten.
4. **The kernel pin lags the board by six release cycles.** §8.2's build is
   `6.12.61-v8`; the board runs `6.18.34+rpt-rpi-v8`. That is `ADR-026` §4.1.2's
   largest caveat and the reason the loop still keeps both kernels. Moving
   `BR2_LINUX_KERNEL_CUSTOM_TARBALL_LOCATION` to a matching Raspberry Pi
   revision would close it; the sha in use is the one Buildroot's own CI builds,
   and a newer one has not been tried.
5. **§4 is not yet satisfied, and the build says so loudly.** "`=y` throughout,
   never `=m`" applies to Rivet's requirements and they are all `=y` — but the
   base `bcm2711_defconfig` still carries some four thousand modules, which the
   build compiles, strips, compresses and installs into `target/lib/modules`.
   Nothing loads them: §4 removed the module loader. Setting
   `# CONFIG_MODULES is not set` in the fragment would make §4 true rather than
   aspirational and would cut most of the build time, at the cost of turning
   every remaining `=m` into an absence — which is the right outcome and also a
   large, reviewable change. Deferred here rather than taken, because §8 says
   not to strip during bring-up and this is close enough to stripping to be the
   maintainer's call.
6. **Read `/proc/cmdline` on a booted board** (§8.2). One command, and it
   decides between two live explanations for the missing memory controller.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §3 — the wrong hardware RNG driver

**Corrected 2026-08-23.** The superseded table entry was
~~`HW_RANDOM_BCM2835`~~, which is the Pi 3's driver and matches nothing on a
BCM2711. See §6.1.

### In §8.1 — the missing memory controller was blamed on the kernel

**Disproved the same day by §8.2.** The superseded inference read:

~~so the cause lies in the kernel rather than the command line, and
`CONFIG_MEMCG` is the leading candidate. **This makes item 4 a genuine
kernel-config failure rather than the configuration accident it was filed
as**: a fourth §3 requirement now needs the custom build, and unlike the
command line Rivet cannot reach it by owning a file on the boot
partition.~~

`CONFIG_MEMCG=y` is line 21 of `bcm2711_defconfig`, and the board's own
`kernel8.img` printed the controller as present under an emulator. The cause is
the firmware's composition of the bootargs.

### In §8.1 — two commands were said to be untaken

**Both were run on 2026-08-23.** The superseded paragraph read:

~~**Unproven, and the two commands that settle it**, on a booted board, in
this order: `cat /proc/cmdline` — which has never once been read — and only
then `grep -E '^CONFIG_MEMCG' /boot/config-$(uname -r)`.~~

### In §8.2 — the closing reading was said to be untaken

**Struck 2026-08-28 — it was taken the same afternoon this was written.** The
superseded text read:

~~**The reading that closes it is one command on a booted board**, and nobody has
taken it:~~

~~If `cgroup_disable=memory` is in there, the cause is the firmware's composition
and the remedy is `config.txt` or the firmware's own defaults — **not** a
deletion from `cmdline.txt`, because there is nothing there to delete. If it is
*not* in there, both hypotheses are dead and the next reading is
`/boot/config-$(uname -r)` for `MEMCG` (§9 item 1), against the possibility that
the `rpt` Debian packaging differs from `bcm2711_defconfig`.~~

### In §9 item 1 — the audit was outstanding

**Done 2026-08-22, §8.1.** The superseded item title read:

~~Run the audit~~
