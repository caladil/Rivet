# ADR-062: Workstation Platforms, and What Each Goal Needs

* **Status:** Accepted
* **Date:** 2026-09-28
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

A Rivet is built and operated from a workstation, and three kinds of person
arrive at one: somebody who wants to **reach and run** a board, somebody who
wants to **build an application** for one, and somebody who wants to **build the
operating system** — a card image or a system update. What each needs installed
is different, and the difference is large: the first needs two Rust binaries
and a set of credentials, the third needs a container runtime and a Buildroot
tree that takes hours to produce.

The development host has been Windows, with the container carrying everything
Linux-shaped (`ADR-026` §8). Nothing prevented another host in principle, and a
macOS workstation on Apple Silicon is the first to try. What trying it found is
the substance of this record: the container recipe assumes an x86-64 daemon, the
host-side scripts assume GNU tools, the repository records most scripts as not
executable because Windows never reads the bit, and card writing enumerates
disks on Windows and Linux only.

The second problem is how to state requirements at all. A table of prerequisites
in a README is a claim about every reader's machine, and it rots the way every
present-tense sentence in this repository has: nothing fails when it goes wrong.
What a workstation needs is a fact about **that** workstation — which toolchain
is active in the directory you stand in, whether the daemon is up, whether the
Buildroot volume was ever populated, whether the committed public keys agree
with the seeds on this disk.

---

## 2. Decision

1. **Three workstation platforms are supported: Windows, Linux and macOS**, on
   x86-64 and on arm64. What differs between them is stated in §5 and nowhere
   else in prose.
2. **The build container is `linux/amd64` on every host** (§4). An arm64 host
   runs it emulated.
3. **`cargo rivet doctor` is the authority on what a workstation has and
   lacks**, per goal (§3). The README names the goals and points at it; it does
   not transcribe the checks.
4. **A script a person runs on the host is POSIX `sh` against POSIX utilities**,
   and is recorded executable in git (§7).

---

## 3. The three goals, and `cargo rivet doctor`

| goal | `--for` | what it is | what it needs |
| :-- | :-- | :-- | :-- |
| Operate a Rivet | `run` | `run`, `deploy`, `install`, `provision`, `write-image` | `cargo-rivet` and `rivet-provision`, installed from one checkout; the credentials `cargo rivet setup` mints, or your existing seeds restored |
| Build an application | `app` | `cargo rivet package` | the above, plus the `aarch64-unknown-linux-musl` target on the toolchain that builds the crate, `rust-lld` selected as its linker, and the application seed. A crate that compiles C also needs the container (§5.3) |
| Build a card or an update | `image` | `tools/container/*` | a checkout, `git`, a container runtime, the Buildroot volume, the boot firmware, the system seed, and committed public keys that agree with your seeds |

Each goal is judged on its own, because a person who only wants to reach a board
must not be told they are broken for lacking Docker. `--for` narrows the report;
the exit status is non-zero when anything a reported goal **requires** is
missing, and a caveat — something that costs a feature or is produced on first
use — does not change it.

**It is read in a glance and explains itself on request.** One verdict per goal
comes first; under each goal, everything that is fine is one line, and anything
that is not is a headline, at most a sentence or two of consequence, and the
commands that fix it. The reasoning behind each finding is printed only with
`--verbose`, because a paragraph under every line is what makes a checklist
unreadable, and the person who needs the paragraph is the one who asks for it.
A test holds every line within a terminal's width.

**It reads and changes nothing.** A doctor that fixed things would be `setup`,
which already exists for the half of this that is safe to automate. Every
finding names the command that fixes it.

**It is run from the directory you will build in**, because two of its answers
depend on that directory: the active Rust toolchain (inside a checkout,
`rust-toolchain.toml`'s pin, which installs the musl target itself; outside it,
usually `stable`, whose target list is separate), and whether a
`.cargo/config.toml` above it selects `rust-lld`.

**Its sharpest check is the keys.** A card trusts `keys/*.pub` when it is built,
and every board built from that card refuses anything signed by another key for
the rest of its life. A workstation whose seeds do not match the committed keys
builds a card that refuses its own builder, and nothing fails at build time.
There are two ways out and they are not equivalent: bring the seed that matches
the committed key onto this workstation (`cargo rivet key backup` there,
`key restore` here), or deliberately rebuild the committed key from this seed,
which changes what every future card trusts and what no existing board trusts.

---

## 4. The container is `linux/amd64` everywhere

Every `docker build` and `docker run` under `tools/container/` passes
`--platform ${RIVET_DOCKER_PLATFORM:-linux/amd64}`.

| reason | |
| :-- | :-- |
| **The base exists only for x86-64** | the pinned `archlinux:base` digest publishes an `amd64` image and nothing else |
| **So does the cross toolchain** | Buildroot's external Bootlin toolchain is shipped as x86-64 Linux host binaries |
| **One platform is one set of bytes** | `tools/container/Dockerfile` pins the base by digest and the packages by archive date so that a build on one host produces the same image as a build on another. A second, arm64-native recipe would be a second answer to the same question (`ADR-011`) |

On an x86-64 host the flag names what the daemon would have chosen anyway, so
nothing changes there. On an arm64 host the daemon emulates: Docker Desktop on
Apple Silicon uses Rosetta when "Use Rosetta for x86_64/amd64 emulation" is on
(the container's `/proc/cpuinfo` reports `VirtualApple`), and QEMU otherwise.
`cargo rivet doctor --for image` reports which platform the daemon is and says
when it is emulating.

**`RIVET_DOCKER_PLATFORM` exists for the day an arm64 recipe is proven**, not
as an invitation to set it now: nothing in this repository has built the images
for another platform.

---

## 5. What differs by platform

### 5.1 Card writing

`write-image --device` refuses a device it cannot identify as removable. Each
platform enumerates disks its own way and gets root its own way:

| | enumeration | how the write gets root |
| :-- | :-- | :-- |
| **Windows** | CIM | a child relaunched through the administrator prompt |
| **Linux** | `/sys/block` | the operator runs the command with `sudo` |
| **macOS** | `diskutil`, converted to JSON by `plutil` | a child started by `sudo`, which asks for the password in the terminal |

**macOS takes Windows's shape rather than Linux's.** The interview, the plan
and every file the command writes stay the operator's; only the raw write runs
as root, in a child that reads a task file bound by digest, re-enumerates on its
own account, and refuses a disk it will not vouch for — the same child Windows
runs. Running the whole command under `sudo` instead would leave files under
`~/.rivet` and the output directory owned by root. `sudo -v` asks for the
password first and the child is started with `sudo -n`, so the prompt and the
progress line never share the terminal. The disk is freed with `diskutil
unmountDisk` and written through its raw node, `/dev/rdiskN`, which is many
times faster than `/dev/diskN`.

**What macOS calls removable:** `RemovableMedia`, which is true for a card in a
built-in SD slot, or `RemovableMediaOrExternalDevice` with `Internal` false,
which is a USB reader. An absent flag reads as the safer answer. An empty
reader presents no disk at all, so it is never offered.

**macOS mounts a card while it is being written, and the write must refuse
those mounts.** *(`.rulings/` R-227.)* Writing a new partition table makes
macOS probe the disk and mount its FAT boot partitions, and mounting a FAT
volume writes `.Spotlight-V100` and `.fseventsd` into it. The write's own
read-back then finds bytes it did not write and refuses the card, correctly.
`diskutil unmountDisk` before the write does not prevent it, because the
mount follows the new partition table. The same reader and card verify cleanly
from Windows.

The fix is the one Raspberry Pi Imager uses: for the length of the write and
its read-back, the root child registers a **DiskArbitration mount-approval
callback that dissents** for that disk, then removes it. That is `unsafe`
calls into `DiskArbitration.framework` and `CoreFoundation`, with a run loop
on a thread, and **every one of them lives in a macOS-only adapter crate**, a
`cfg(target_os = "macos")` dependency of `cargo-rivet` whose safe interface is
*"hold mounts off this disk until dropped"*. `cargo-rivet` itself stays free of
`unsafe`, as it is on Windows, where the same job is PowerShell. Without that
adapter, a Mac-written card works and fails its read-back.

Without `--device`, `write-image` injects the document into both boot
partitions and leaves a finished image, which any imaging tool can write.

### 5.2 The test suite

The crates a workstation runs — `rivet-core`, `rivet-config`, `rivet-crypto`,
`rivet-tls`, `rivet-remote`, `rivet-fleet-repo`, `cargo-rivet`,
`rivet-provision`, `rivet-reset`, `rivet-fleet` — test natively on macOS. Tests
whose fixture needs two filenames differing only in case are gated to Linux,
because default APFS and NTFS both fold case and cannot hold the fixture.

The rest of the workspace is Linux-only and is tested inside the container
(`tools/container/exec.sh cargo test --workspace`, or `tools/test-all.sh`).

### 5.3 Crates that compile C

The SDK itself is pure Rust, so an application that adds no C links with
`rust-lld` alone and packages natively on every host. A crate whose
dependencies compile C (`rusqlite`'s `bundled`, `ring`) needs Buildroot's cross
compiler, which is an x86-64 Linux program: on Linux x86-64 it can be named
directly (`tools/toolchain/env.sh`); everywhere else such a crate is built inside
the container, with `RIVET_APP_SRC` mounting it beside this repository.

### 5.4 Shells

Host-side scripts run under Git Bash on Windows, and under the system shell on
macOS and Linux. The Windows-only hazards — CRLF, `git describe` disagreeing
between Git Bash and WSL, MSYS path rewriting — are `AGENTS.md` §10.5's and do
not arise on the other two.

### 5.5 The browser certificate

`rivet-provision` files the operator's browser identity, which only the web
portal needs, into the Windows certificate store automatically. On macOS and
Linux it names the command to run by hand instead: neither has one store every
browser reads, and a tool that silently did nothing there would be worse than
one that says it cannot.

### 5.6 A workstation's state lives in two places

Everything a workstation holds for Rivet is under `~/.rivet` **except
`rivet-fleet`'s register**, which the configuration library places beside that
tool's configuration file in the platform's own directory:

| platform | the register |
| :-- | :-- |
| Windows | `%APPDATA%\rivet-fleet\register` |
| macOS | `~/Library/Application Support/rivet-fleet/register` |
| Linux | `~/.config/rivet-fleet/register` — the library's rule; read it off a Linux workstation before relying on it |

Moving to a new workstation therefore means both. `cargo rivet key backup`
carries the signing seeds only; the certificate authority, the identities, the
Armada registry, the artifact repository and the provisioning documents are
the rest of `~/.rivet`. **On a workstation with none of it, the first `cargo
rivet provision` asks whether this is a new fleet or an existing one before it
mints an authority** (`ADR-063` §8): a new fleet gets a new certificate
authority (`ADR-059` §3), and a workstation joining an existing fleet answers
*join*, mints nothing, and carries both of the above across before
provisioning anything.

---

## 6. The package manager's syscall sandbox is off in both build images

**Both Dockerfiles uncomment `pacman`'s own `DisableSandboxSyscalls`, on every
host.** *(`.rulings/` R-226.)*

`pacman` 7.1 isolates its downloader with a seccomp filter. Rosetta does not
support seccomp filters, so inside an emulated container `pacman -Syu` refuses
with *"error restricting syscalls via seccomp: 22"* and *"switching to sandbox
user 'alpm' failed"*, and without the switch neither image builds on an arm64
host.

The sandbox protects the host from a compromised downloader, during a build
that verifies every package it installs against the archive's signatures and
that already runs inside a container. That container is the isolation that
matters here. The switch is set on **every** host rather than only when the
build detects emulation, because a recipe that differs by host is what §4
exists to prevent, and the difference would not reach the finished image
anyway. Each Dockerfile fails its own build if `pacman.conf` stops carrying
the line.

---

## 7. Host-side scripts are portable, and executable in git

A script a person runs on the host is POSIX `sh` using only what macOS, Linux
and Git Bash all ship:

| GNU spelling | portable spelling |
| :-- | :-- |
| `sha256sum FILE` | `sha256sum` when present, else `shasum -a 256` |
| `stat -c %s FILE` | `wc -c <FILE` |
| `sed -i 'expr' FILE` | `sed -i.bak 'expr' FILE` and remove the `.bak` — BSD `sed` reads the word after a bare `-i` as a backup suffix |

Scripts that run **inside** the container (`tools/card/build.sh`,
`tools/update/build.sh`) may use GNU tools freely; the container is Arch.

**Every script with a shebang is recorded `100755` in git**, except one that is
only ever sourced (`tools/toolchain/env.sh`). Windows ignores the bit and macOS
and Linux enforce it — including through a bind mount into the container, where
`tools/card/build.sh` is executed by path.

---

## History

The body above states only what is true now. This section keeps what explains
why a decision changed.

### §6 was an open question, and these were the options

~~Open: the package manager's sandbox under emulation~~ — ruled 2026-09-28,
`.rulings/` R-226, choosing A.

| option | what it costs |
| :-- | :-- |
| **A. Uncomment `DisableSandboxSyscalls` in both Dockerfiles** | the downloader loses its syscall filter while the image is built; packages are still verified, and the finished image is unchanged. Both image tags are bumped, so every workstation rebuilds once |
| **B. The same, only when the build detects emulation** | a recipe that differs by host, for a difference that does not reach the image |
| **C. Turn Rosetta off and let QEMU emulate** | untested whether QEMU's user-mode emulation honours the filter, and several times slower for a Buildroot build measured in hours |
| **D. Build the images on an x86-64 host and move them** | a Mac that cannot build its own images, which is the dependency `ADR-026` §8 removed |
