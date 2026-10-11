# SPEC-01: Rivet as a Build Target

* **Status:** Normative for `0.1.33` and later. `ADR-003` §6.3's document,
  written 2026-08-31.
* **Date:** 2026-08-31
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Triple:** `aarch64-unknown-linux-musl`
* **Answers:** *"how do I build a program for this, and what will it find when
  it gets there?"*

---

## 1. Context: why this document exists, and why it exists NOW

`ADR-003` §6.3 has asked for this since 2026-07-20:

> If software must be built for Rivet, then **Rivet must eventually be
> documentable as a build target** — the triple, the sysroot, and an honest
> statement of which platform facilities exist and which do not. That last part
> is the valuable half and it is the part nobody writes. **Not needed before
> Stage 3.**

**That last clause was wrong, and it was disproved by two agents rather than
argued with.** On 2026-08-30 two agents from different vendors, with no shared
context, were each told to put a database-backed application on a Rivet board.
Both rejected SQLite. Both gave the same reason, unprompted: there was no C
cross-toolchain they could find, so a crate compiling C could not be built for
the target. One of them asked for this document by name.

**A C cross-toolchain was already in the build environment.** Buildroot produces
one, `tools/card/build.sh` has used it since 2026-08-28, and it had never been
exposed to anyone building an application. The friction was not a missing
capability; it was a missing sentence.

`docs/design-candidates/Shared-Libraries-and-the-Build-Target.md` §3 is the
reasoning. This is the artifact.

---

## 2. The one command

```sh
. tools/toolchain/env.sh
```

It sets, and refuses rather than guessing if the toolchain is not there:

| Variable | Is |
| :-- | :-- |
| `RIVET_TARGET` | `aarch64-unknown-linux-musl` |
| `RIVET_CC`, `RIVET_CXX` | Buildroot's `aarch64-linux-gcc` / `-g++`, **GCC 14.3.0, musl** |
| `RIVET_AR`, `RIVET_STRIP` | the matching binutils |
| `RIVET_SYSROOT` | asked of the compiler with `-print-sysroot`, never composed |
| `CC_aarch64_unknown_linux_musl` | the same compiler under the name the `cc` crate reads |
| `CXX_aarch64_unknown_linux_musl` | likewise |
| `AR_aarch64_unknown_linux_musl` | likewise |

The two sets are the same programs. The first is what a person types; the second
is what Cargo's build scripts look for and nobody would guess.

**Run it in the container** — `tools/container/exec.sh <command>` — which is
where the toolchain is (`ADR-026` §7).

### 2.1 What a build then looks like, per language

**Rust, pure.** Nothing is needed beyond the target:

```sh
cargo build --release --target aarch64-unknown-linux-musl
```

`rust-lld` is the linker (`.cargo/config.toml`, `ADR-026` §3.1) and **an
application crate outside this repository does not inherit that setting** —
`cargo rivet package` says so by name when the link fails.

**Rust with a `-sys` crate.** The same command, after `env.sh`. The `cc` crate
reads `CC_aarch64_unknown_linux_musl` and compiles the C with Buildroot's
compiler. Measured 2026-08-31: `libsqlite3-sys 0.30.1` and `rusqlite` with
`bundled` build and **run** on a board — `examples/probes/sqlite-demo` is the artifact.

**C.** `"$RIVET_CC"`. `examples/probes/greeter` is the artifact.

**C++.** `"$RIVET_CXX"`. The toolchain includes a C++ compiler and
`libstdc++.so.6.0.33` built against musl. **Nothing has been built with it yet,
so this row is a capability and not a claim** — see §6.

**Go.** No toolchain from this project is involved:

```sh
GOOS=linux GOARCH=arm64 CGO_ENABLED=0 go build -trimpath -tags timetzdata ...
```

`CGO_ENABLED=0` produces a static binary that needs no libc at all.
`-tags timetzdata` embeds the IANA database, because **a Rivet has no
`/usr/share/zoneinfo`** and `time.LoadLocation` would otherwise silently fall
back to UTC. `deploy/aura/controller` is the artifact, and §6 says exactly how
far it got.

---

## 3. Static or dynamic, and how to choose

**Static is the default and should stay the default.** `ADR-003` §5 rules it for
Rivet's own binaries, and for an application it is simply the case with fewer
moving parts: one file, no loader, no closure, nothing to install first.

A program is dynamic when it links a shared object, and then:

* its `PT_INTERP` must be **`/lib/ld-musl-aarch64.so.1`**, which is what the
  toolchain above writes. A glibc cross-compiler writes a glibc loader's path,
  and musl is not ABI-compatible with glibc — `cargo rivet package` refuses that
  binary and says so;
* everything in its `DT_NEEDED` must be either on the platform's public surface
  (§4) or in its **declared closure** (`SPEC-02`).

`cargo rivet package` reads all three out of the binary and reports them, so
this is checked at build time rather than discovered on a board.

---

## 4. The platform's public ABI surface

**One library:**

```text
libc.so
```

That is musl, and on musl it is also the dynamic loader — the same file, at
`/lib/ld-musl-aarch64.so.1`. `rivet_core::package::library::PUBLIC_LIBRARIES` is
the list in the code and a test pins it.

**Everything else an application links arrives in its own closure**, signed and
content-addressed (`SPEC-02`). That is Android's `public.libraries.txt` lesson:
every symbol the platform promises is one it must keep promising across image
updates, and the list is deliberately one entry long.

`libstdc++.so.6` is **not** on it. A C++ program either links it statically
(`-static-libstdc++`) or ships it as a library artifact of its own.

---

## 5. The honest facilities list

**This is the half `ADR-003` §6.3 called valuable and said nobody writes.** Every
row was read off a running board on 2026-08-31 unless it says otherwise.

### 5.1 What is there

| | |
| :-- | :-- |
| kernel | Rivet's own, `6.12.61-v8`, built by Buildroot (`ADR-024`) |
| libc | musl, static or dynamic |
| filesystem | `/` read-only; `/state` the one writable partition (`ADR-018` §4) |
| an application's writable path | **`/state/app/<name>/data`, mode `0700`, owned by the application's own uid — and nothing else** (`ADR-015` §3.2A) |
| `/tmp` | tmpfs, 64 MiB, `1777` |
| `/run`, `/dev/shm` | tmpfs, `0755 root:root` — **see §5.2** |
| `/dev` | devtmpfs, **no udev**, so nodes keep the kernel's default `0600 root:root` |
| `/proc`, `/sys` | mounted |
| cgroup v2 | mounted at `/sys/fs/cgroup` |
| networking | ordinary sockets; DNS through `/etc/resolv.conf`, which is a **projection** (`ADR-007` §3.5) |
| the journal | an application's **stdout and stderr are captured** and appear in `rivet get log`. This is the supported way for an application to say anything |
| time | **no real-time clock** (`ADR-021`). The wall clock is wrong until something corrects it, and then jumps |
| entropy | `/dev/urandom`, `/dev/hwrng` |

### 5.2 What is NOT there, and what breaks because of it

| Absent | What fails, and how it looks |
| :-- | :-- |
| **a POSIX shell** (`ADR-005` §5.4) | anything that shells out. `os/exec`, `std::process::Command("sh")`, `system()`. It compiles perfectly |
| **an environment an application can be given** | there is no `env` key in a manifest, so a program configured by environment variables cannot be configured. Every variable an application gets is **composed by the machine** from that application's own signed declarations and from its name — see §5.2.4 for the list — and an author can influence none of them |
| **a home directory** | `HOME` is `/`, which is read-only. Anything resolving a config path through `$HOME` or an XDG variable lands somewhere it cannot write. The fix is not to resolve one: the supervisor hands an application its data directory in `RIVET_APP_DATA`, and `rivet_sdk::app::data()` reads it (`ADR-036` §3) |
| **`/usr/share/zoneinfo`** | `time.LoadLocation` by name, `chrono-tz` from disk. This row also listed `localtime` — **see §5.2.1: `localtime` works from `0.1.63`** |
| **D-Bus** | any `dbus`/`zbus` client. There is no system bus and no daemon |
| **BlueZ, and Bluetooth in the kernel at all** | **`/sys/class/bluetooth` is absent on this board.** The row cited `/proc/net/bluetooth` as well, measured 2026-08-31 — **the conclusion still holds and that TEST was invalid; corrected 2026-09-04, see §5.2.2.** `btleplug` and everything under it |
| **DRM/KMS — `/sys/class/drm` is absent** | no `/dev/dri`, no GPU, no Wayland, no compositor, no OpenGL. The framebuffer at `/dev/fb0` is `0600 root:root`, so an application cannot open it either (`ADR-015` §6.2) |
| **udev** | anything expecting a device node's mode to have been relaxed |
| **a package manager, and the FHS** (`ADR-008` §3, §4) | `/usr/lib`, `/usr/share`, `/opt`, `/var/lib` are not there |
| **`/etc` as a source of truth** (`ADR-007` §3.6) | `/etc` is a read-only projection; writing to it fails, and editing it would be overwritten |
| **`pkg-config` for the target** | a `-sys` crate that probes with pkg-config refuses. `libdbus-sys` is the measured case — see §6 |

#### 5.2.1 The timezone, from `0.1.63` — one of those rows half moved

`ADR-021` §9, ruled 2026-09-02. A Rivet's configuration document now carries a
`timezone` key, and the supervisor projects `/etc/localtime` from it.

**What this changes for an application, precisely:**

| You call | Before | From `0.1.63` |
| :-- | :-- | :-- |
| `time.Now()` / `time.Local` (Go) | UTC | **the machine's zone**, on a machine whose document names one |
| `chrono::Local` (Rust) | UTC | **the machine's zone**, same condition |
| `localtime(3)` (C) | UTC | **the machine's zone**, same condition |
| `time.LoadLocation("America/Chicago")` (Go) | fails | **still fails** without `-tags timetzdata` |
| `chrono_tz` / any lookup **by name** | fails | **still fails** |

**So `-tags timetzdata` is still not optional**, and every place in this
document that says so is unchanged. The distinction is that
`/usr/share/zoneinfo` is still absent — the database lives at
`/system/share/zoneinfo`, which is the operating system's and not an
application's search path — while `/etc/localtime` is now present when the
operator has named a zone. **A runtime that reads the FILE gets the answer; a
runtime that looks a NAME up in a directory does not.**

**An application must not assume either way.** A machine whose document names no
zone has no `/etc/localtime` and keeps UTC, which is a supported machine and is
every Rivet before `0.1.63`. A clock that says which zone it is showing is
still the right design.

#### 5.2.2 The Bluetooth row's test was invalid — corrected 2026-09-04

The row's **conclusion is unchanged**: there is no Bluetooth on a Rivet today.
Its **test** could not have established that, and is replaced here rather than
left in place, because a measurement anybody can repeat and misread is worse
than no measurement (`AGENTS.md` §7, both directions).

**`/proc/net/bluetooth` does not exist in Linux 6.12 in either direction.**
`hci_sock.c` registers its procfs entries as `bt_procfs_init(&init_net, "hci",
…)`, so the paths are `/proc/net/hci` and friends; nothing has ever created a
file called `bluetooth` there. Measured in a QEMU guest running a kernel with
the whole stack built in: still `ENOENT`. So that half of the 2026-08-31
reading was a true statement about a file that is absent on every Linux,
proving nothing about this one.

**The valid tests**, both taken on the same day against a guest with the stack
and against the board without it:

| test | board (no stack) | guest (stack built in) |
| :-- | :-- | :-- |
| `/sys/class/bluetooth` | absent | present |
| `hci`, `l2cap`, `sco`, `hidp` in `/proc/net` | none of them, of 35 entries | all four, of 39 |

`docs/design-candidates/Bluetooth.md` §2.2 carries the readings. That study also
found that **`CONFIG_BT=m` is in every card this project has shipped**,
inherited from the Broadcom defconfig — and a Rivet has no module loader, so the
subsystem is compiled and unreachable. The row above is still true of the
running machine; it is true for a different reason than "the kernel was built
without it".

#### 5.2.3 `libdbus-sys` is a missing host TOOL, not a cross-compilation wall

Also corrected 2026-09-04, and it changes what the last row of §5.2 means.

§6 reads the `libdbus-sys` failure as something about cross-compiling to this
target. It is not: the crate's build script runs `pkg-config` on the **host**,
and the build container has none. With a twelve-line `pkg-config` shim on
`PATH`, `libdbus-sys`, `dbus`, `dbus-tokio` and `btleplug` all cross-compile for
`aarch64-unknown-linux-musl` — measured, `Bluetooth.md` §4.3.

**The row stays in §5.2 regardless**, because what it describes is still true of
this build environment as it ships, and an application author hitting it will
hit it. What changes is the repair: it is a tool to provide, not a limit to
design around.

#### 5.2.4 Every variable an application is handed

`rivet_core::service` names each of these and carries the argument for it; the
supervisor composes them, the first group when it derives an application's
definition and the descriptor group when it spawns the process. **A manifest
sets none of them**, which is §5.2's row restated as a list rather than as a
count that has been wrong since the day after it was written.

| Variable | Composed from | Present when |
| :-- | :-- | :-- |
| `HOME=/`, `TERM=linux` | nothing | always |
| `LD_LIBRARY_PATH` | the manifest's declared `library` closure, then every installed library whose class the manifest accepts | either list is non-empty |
| `RIVET_ADDONS` | the same class-matched libraries, as `<class>=<path to the object>` — `SPEC-02` §11 | a class matched |
| `RIVET_APP_DIR` | the application's name | always |
| `RIVET_APP_DATA` | the application's name | always |
| `RIVET_APP_SETTINGS` | the application's name | always |
| `RIVET_APP_TMP`, `TMPDIR` | the application's name, one value under two names | always |
| `RIVET_CONNECT` | the endpoints granted to this application | something was granted |
| `LISTEN_FDS`, `LISTEN_PID`, `LISTEN_FDNAMES` | the manifest's declared sockets, plus the reserved health entry | the application declares a socket or answers health |

**An absent variable is never an empty one.** Several of these are composed as
`None` rather than as an empty string, because a program that reads an empty
value reads *something* — an empty `LD_LIBRARY_PATH` is the current directory
to the loader, and an empty list is one nameless entry to a careless split.

### 5.3 What an application is granted, and what is only asked for

`ADR-015` §3.3: the manifest **is** the permission request. As of `0.1.33` this
build enforces exactly two of the things in it, and reports the rest as
unenforced every time an artifact is built or started:

| | |
| :-- | :-- |
| **enforced** | the **identity** — an application runs as its own uid, derived from its name, and never as root (`ADR-015` §5.1) |
| **enforced** | **launch-time verification** — the signed header is re-checked against the trust store and the program is hashed before every start (`ADR-015` §4.2) |
| **enforced** | the **declared closure** — `LD_LIBRARY_PATH` is composed from it and from nothing else (`SPEC-02`) |
| *not* enforced | `restart`, `ready`, `state`, `network`, `memory`, `listen`, `provides`, `connects` |

There is no mount namespace, no network namespace, no Landlock (the kernel does
not have it — `ADR-024` §8) and no seccomp. **Unix permissions on its own data
directory are the whole of what separates one application from another.**

---

## 6. Which languages are PROVEN, and what that word means

`docs/design-candidates/Shared-Libraries-and-the-Build-Target.md` §5.8 is exact
about the difference and it governs this section:

> *Supported* is open-ended: it commits to software nobody has tried. **Proven**
> names an artifact.

| Language | Status | The artifact |
| :-- | :-- | :-- |
| **Rust, static** | **proven** | every `cmd/rivet*` in this repository, and `examples/probes/hello-rivet` |
| **Rust with C compiled in** | **proven** | `examples/probes/sqlite-demo` — `rusqlite` with `bundled`, SQLite 3.46.0, WAL mode, writing a real database on the board |
| **C, dynamic** | **proven** | `examples/probes/greeter` and `examples/probes/libgreet` — the first Shared Rivet |
| **C, static** | not tried | — |
| **Go, static** | **proven** | `deploy/aura/controller` — Aura's own controller, running on a board and answering its HTTP API. See §6.1 for what it took, because the answer is not "nothing" |
| **C++** | **not tried** | the toolchain and a musl `libstdc++` are present. Nothing has been built |

### 6.1 The Go case, in full, because a partial claim is the thing to avoid

> **Updated 2026-08-31, later the same day.** The table above said *"proven to
> start; not proven to work"* and now says **proven**. What changed is not the
> claim's standard — it is that the one thing blocking the program was built.
> The account below is left as it was written, because a language being proven
> by a program that failed first is more useful than one that says it worked.

`aura-controller` is Aura's Go half, built `CGO_ENABLED=0` from that project's
own tree with no modification. On a board it:

* **ran** — a static aarch64 Go binary executing under its own uid;
* **loaded its configuration**, falling back to defaults because the file did
  not exist, and resolved `America/Chicago` correctly from the embedded tzdata;
* **failed** on one thing, and only one: it binds a unix socket at
  `/run/aura/kiosk.sock`, `/run/aura` does not exist, and an application running
  as its own uid cannot create a directory under a `0755 root:root` `/run`.

So **the language is not the problem and neither is the toolchain.** What was
missing is `ADR-015` §5.3 — the mechanism by which Rivet gives an application a
socket.

**Built the same day** (`PLAN.md` §103.10). `rivet-sysd` opens the socket and
passes the descriptor, Aura adopts it in about ten lines, and the controller now
reports

```text
ipc: listening on /run/rivet/listen/aura-controller/kiosk.sock (passed in by the supervisor, not bound here)
```

with its HTTP API answering from another machine. **What a Go program needs to
honour is `LISTEN_FDS`** — `ADR-015` §5.2.1 is the protocol, and writing for
both paths is what lets one binary run here and under systemd.

**The table above says "proven to start" rather than "proven" for exactly this
reason.** `ADR-003` §6.2's distinction is the whole point of this document:
*loading* and *working* are different questions, and a row that conflated them
would be the partial promise `ADR-005` §5.4 refuses.

---

## 7. What is deliberately not here

**A list of libraries Rivet provides.** There is one (§4). Implying more would
invite reliance the platform has not earned, and `ADR-005` §5.4 is the standing
argument against exactly that.

**A blueprint collection.** `ADR-009` §5's gate — every dependency must build
for `aarch64-unknown-linux-musl` — is currently enforced by the absence of a
mechanism rather than by judgement, and the design candidate's §5 proposes the
format that fixes it. **The format is not built.** `libdbus-sys` is the measured
case that wants it: TempMon's daemon cannot be cross-compiled because
`libdbus-1` is not in the sysroot and `pkg-config` cannot cross. That is rung 2
of `ADR-009` §3's ladder, and rung 2 still cannot be stood on.

**Anything about running a binary somebody else compiled.** `ADR-003` §6.2
stands unamended. Bring the *source*.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §5.2 — `localtime` was listed as absent

**Corrected — `localtime` works from `0.1.63`, see §5.2.1.** The superseded
entry in that row was ~~`localtime`~~.

### In §5.2 — the Bluetooth row's test

**Corrected 2026-09-04, see §5.2.2: the conclusion holds and the TEST was
invalid.** The superseded measurement read:

~~`/proc/net/bluetooth` and `/sys/class/bluetooth` are absent, measured
2026-08-31~~

`/sys/class/bluetooth` is absent on this board and that half stands.
