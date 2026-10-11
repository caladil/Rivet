# Rivet

A Rust-first Linux distribution for the Raspberry Pi.

Rivet takes the Linux kernel, the Raspberry Pi boot firmware, and a small
named set of C daemons as given, and supplies everything above them — PID 1,
service supervision, networking, configuration and administration — in Rust.

It is not an attempt to write an operating system kernel, and it is not hostile
to C. The stated posture is:

> **Rivet is a Rust-first Linux, not a Rust-only one. C programs are
> mechanisms; they are never the interface.**

---

## Status

**Rivet runs on a Raspberry Pi 4 Model B.** A card this repository builds
boots it; PID 1 and everything above the kernel are Rust; it configures its
network from a document on the boot partition; it updates itself A/B, with a
trial boot the firmware rolls back if the new slot never commits; it runs
signed applications under supervision; and it is administered over a mutually
authenticated control plane. For more than one machine there is a fleet server
with a register, a repository of signed artifacts and a web portal, and an
application that offers a backup has it fetched over the control plane
(`docs/ADR-051`).

**There is no stable release.** Commands, formats and interfaces change without
notice, and nothing here promises compatibility from one commit to the next.

**No images are published.** You build your own card from this source, against
your own signing keys (`docs/ADR-010` §6.5). *Getting started* below is that
path.

**Compiled, emulated and run on a board are three different claims here.**
`docs/ADR-026` §4 is the table of what QEMU cannot settle — `tryboot`, the boot
partition, the firmware's device tree, `/state` and a PHY are all in the column
only a board can answer. "It works in QEMU" is not "it works", and neither is
"it compiles".

---

## What is intended to be different about it

Most of the design work has gone into two questions that are usually answered by
accident.

**How much C, and where?** Every non-Rust component sits in one of three tiers
declaring the *intent* to replace it — Rust now, C now with a tracked Rust
replacement, or C indefinitely and wrapped (`docs/ADR-001`). `wpa_supplicant`
is in the last tier deliberately: depending on it is what keeps Rivet current
with WPA3 and whatever follows, where implementing our own would strand the
project on WPA2.

**What does administration look like on a system designed now?** Conventional
Unix administration is fifty years of individually reasonable decisions that
nobody could coordinate, producing several dozen configuration formats under
`/etc`. The Windows Registry is the instructive counter-example: opaque, but
still structurally coherent thirty years on. Its coherence and its opacity are
separable, and the coherence came from centralization, not from the binary
format.

So Rivet aims at: one typed configuration document with a schema generated from
the code; desired state and observed state as distinct things, with observed
state never persisted; and `/etc` as generated output that nothing authors by
hand (`docs/ADR-006`, `docs/ADR-007`). Structured data is the primary output of
every command, with human-readable text rendered from it rather than instead of
it.

That last property is also what makes the system tractable for an AI agent to
administer, which is a stated goal rather than a side effect
(`docs/ADR-012`).

---

## Technology stack

| | |
| :-- | :-- |
| Language | Rust 2024, `resolver = "3"`; the toolchain is pinned by `rust-toolchain.toml` |
| Target | `aarch64-unknown-linux-musl`. Rivet's own binaries are static except the compositor, which links Mesa (`docs/ADR-058`); an application may link the shared libraries it declares (`docs/SPEC-02`) |
| Hardware | Raspberry Pi 4 Model B (`docs/ADR-002`) |
| Kernel and firmware | The Raspberry Pi Linux kernel, built by Buildroot with Rivet's own configuration (`tools/buildroot/`), and the Raspberry Pi boot firmware, fetched pinned by `tools/fetch-firmware.sh` |
| C | A small named set of programs Rivet drives as mechanisms and never exposes as the interface. `docs/ADR-001` sets the tiers and `docs/ADR-005` names them |
| Third-party Rust | Kept deliberately few. `rivet-core` has none: its `[dependencies]` table is empty, so the build refuses what policy would only discourage. `cargo tree` is the count |
| Workstation tools | [Ignition](https://github.com/caladil/Ignition) for configuration, logging and the command-line surface. Nothing that runs on a Rivet links it: applications use `rivet-sdk` (`docs/ADR-036` §4) |
| Build environment | A pinned Linux container (`tools/container/`), the same on Windows, macOS and Linux hosts (`docs/ADR-062`) |

---

## Project layout

```
rivet-core/          the domain: every administrative operation as a typed
                       thing. No I/O, no platform types, no dependencies
rivet-config/        the configuration document, its schema and its parser
rivet-render/        JSON and text, both rendered from one structured value
rivet-store/         persistence on /state
rivet-adapter-*/     the platform behind ports: netlink and DHCP, the A/B boot
                       primitive, mounts, HDMI-CEC
rivet-crypto/        BLAKE3 and Ed25519, behind the core's ports
rivet-tls/           TLS with pinned Ed25519 keys, for both ends of the
                       control plane
rivet-webauthn/      the proxy's passkey checks
rivet-remote/        reaching a running Rivet from a workstation
rivet-mdns/          one-shot .local resolution
rivet-register/      the fleet register
rivet-fleet-repo/    the fleet repository's text half
rivet-fleet-root/    a fleet root minted on its fleet server
rivet-filecopy/      the backup copy engine the workstation tools share
rivet-sdk/           what an application on a Rivet links (docs/SPEC-03)
rivet-addon/         what a host application and its addons say to each other

cmd/                 one directory per binary
  rivet                the administration CLI, on the machine
  rivet-initd          PID 1
  rivet-sysd           the supervisor, the journal, the /etc projection
  rivet-netd           network convergence; rivet-dhcp-hook beside it
  rivet-timed          the time client
  rivet-mdnsd          the mDNS responder
  rivet-dnsd           the fleet's name service
  rivet-mcpd           the control plane's listener, port 7777
  rivet-proxyd         the path proxy and authentication gate
  rivet-fleetd         the fleet server and its web portal
  rivet-deploy         the installer: verifies a signed .rvt and installs it
  rivet-displayd       the compositor
  rivet-launcher       the kiosk launcher
  rivet-splash         the boot splash
  rivet-bluetoothd     the Bluetooth LE observer
  rivet-smbd           SMB file sharing
  femto                the editor
  rivet-image          builds a card image
  cargo-rivet          the workstation tool: package, deploy, run, fleet, …
  rivet-fleet          the fleet register, from a workstation
  rivet-provision      the interview that composes a machine's document
  rivet-reset          returns a workstation to the state a first run finds

examples/            templates to copy, probes to read, and Aura (docs/SPEC-04)
deploy/              packaging manifests for programs built outside this
                       workspace
docs/                ADRs, SPECs, design candidates and generated references
skills/              recurring procedures, step by step
keys/                the public keys a card is built to trust — yours
tools/               the build container, card and update builds, Buildroot,
                       the QEMU harness, and the guards on docs/
vendor/              foreign source this project carries and owns
assets/              the fonts and images a board draws with
```

**The dependency direction is enforced by the build, not by convention**
(`docs/ADR-014` §3). `rivet-core` depends on nothing that touches the world, so
a transport or a platform type cannot reach the domain even by accident.
`cargo test -p rivetos-core` needs no network, no device, no root and no
filesystem beyond temporary paths; if it ever does, the boundary has leaked.

---

## Getting started

### What you need installed, by what you are here to do

There are three reasons to be here, and they need very different amounts of
software. **Find yours, install what its column says, then let the tool check
the machine:**

```console
$ cargo rivet doctor                # all three goals
$ cargo rivet doctor --for app      # just one: run, app or image
```

`doctor` reads and changes nothing. It opens with one verdict per goal — ready,
or how many things to fix — then lists each goal's checks: `✓` fine, `!` works
with a caveat, `✗` must be fixed (with the command, after `→`), `·` needed only
in a case it names. `--verbose` adds why each one matters. Run it from the
directory you will build in, because the Rust toolchain active there is one of
the things it checks. `docs/ADR-062` is the reasoning behind every row below.

| | **Run a Rivet** — reach, deploy to, provision and update boards | **Build applications** with the SDK | **Build the operating system** — card images and system updates |
| :-- | :-- | :-- | :-- |
| `doctor --for` | `run` | `app` | `image` |
| Rust | [`rustup`](https://rustup.rs) | the same, plus the `aarch64-unknown-linux-musl` target — automatic inside this checkout, `rustup target add aarch64-unknown-linux-musl` for a crate outside it | the same as *Run* |
| A host C compiler | yes, for building the two tools below: Xcode Command Line Tools on macOS (`xcode-select --install`), Visual Studio Build Tools on Windows, `gcc` or `clang` on Linux | the same | the same |
| `git` | to clone this repository | the same | the same, and `tools/card/bump.sh` mints each version with it |
| This repository's tools | `cargo install --path cmd/cargo-rivet --force --locked` **and** `cargo install --path cmd/rivet-provision --force --locked`, from one checkout | the same | the same |
| Credentials in `~/.rivet` | `cargo rivet setup` for a **new** fleet; for an **existing** fleet, your seeds from the workstation that has them (`cargo rivet key backup` there, `key restore` here) | the application seed | the system seed, **matching `keys/*.pub` in this checkout** — `doctor` compares them |
| Linker | — | `linker = "rust-lld"` for `aarch64-unknown-linux-musl` in the crate's `.cargo/config.toml` (inherited inside this checkout; the Rust templates under `examples/templates/` carry their own) | — |
| A container runtime | — | only if your crate compiles C (`rusqlite`'s `bundled`, `ring`) — then as for *Build the OS* | Docker Desktop (macOS, Windows), or Docker Engine or Podman (`RIVET_DOCKER=podman`) on Linux. On Apple Silicon, turn on Docker Desktop's *Use Rosetta for x86_64/amd64 emulation* |
| The Buildroot tree | — | the same condition | `tools/container/build-buildroot.sh --fetch`, once. **It takes hours**, and longer emulated |
| Boot firmware | — | — | `tools/fetch-firmware.sh`, pinned by commit and sha256 |
| `uv` | — | — | for `tools/docs/*` and the pre-commit hook |
| A shell for `tools/*.sh` | — | — | Git Bash on Windows; the system shell on macOS and Linux |

**Where the platforms differ** (`docs/ADR-062` §5):

* **Writing a card to a device** needs administrator everywhere, and each
  platform asks differently: Windows raises its administrator prompt, macOS
  asks for your password through `sudo` in the terminal, and on Linux you run
  the command with `sudo` yourself. Only the write itself runs elevated on
  Windows and macOS.
* **The build container is `linux/amd64` on every host.** On Apple Silicon it
  runs emulated through Rosetta: Buildroot takes about two hours from nothing
  there, and a card image a few minutes once it exists. *Run* and *Build
  applications* (pure Rust) need no container at all.
* **The browser certificate for the web portal is filed automatically on
  Windows only.** On macOS and Linux, `rivet-provision` names the command to run
  by hand instead. Nothing else needs it: every board is reachable from the
  command line without the portal.
* **A card written on macOS can fail its own read-back check**, because macOS
  mounts the new boot partitions mid-write and writes into them. The card
  works; the check cannot prove it. `docs/ADR-062` §5.1 is the ruled fix:
  refusing those mounts for the length of the write.
* **The workstation crates test natively** on all three; the rest of the
  workspace is Linux-only and tests inside the container.

### The first run, in the order the steps depend on each other

**A Rivet card image is built AGAINST your keys.** That is why this section is
numbered. The image carries a trust store saying which signing keys the machine
will accept, and a `peers` file saying which workstation may reach its control
plane — both composed **when the image is built**, both on media the machine
treats as read-only.

So building a card before you hold the keys does not fail. It succeeds, and
produces a board that boots, takes its address, brings up its control plane and
then refuses you — on hardware with no shell, no login and no serial console.
The only repair is to write the card again.

**The last step exiting zero is not the test.** The test is step 8: `cargo
rivet run … -- get status` answering with the machine's own JSON.

#### 1. What has to be on the workstation

Everything in the *Build the operating system* column above, because a first
run builds a card. `rust-toolchain.toml` pins the exact Rust release and rustup
installs it, with the target, the first time `cargo` runs in this tree. There is
no cross-compiler to install: the container carries it, and
`tools/toolchain/env.sh` prints what it sets.

After step 2 installs the tools, `cargo rivet doctor --for image` is the check.

#### 2. Install this repository's workstation binaries

```console
$ cargo install --path cmd/cargo-rivet --force --locked
$ cargo install --path cmd/rivet-provision --force --locked
$ cargo rivet --version
```

`--locked` builds against the committed `Cargo.lock`, so the tools on the PATH
use the dependency versions this repository was tested with.

**Both lines are the install.** `cargo install --path` copies exactly the binary
the crate it is pointed at declares, and `cargo rivet provision` runs
`rivet-provision` as its interview. Install them from the same checkout:
`provision` refuses a pair whose builds disagree, naming both.

**Check the version line before you believe anything the tool says.**
`--version` prints the build stamp, which file answered, and whether that is the
file `cargo rivet` resolves to. An older `cargo-rivet` on a PATH cannot be fixed
from here, so the presence of that line is the signal.

#### 3. Your credentials — `cargo rivet setup`

```console
$ cargo rivet setup
```

It mints four things, in the order they depend on each other, into `~/.rivet`:

| | | |
| :-- | :-- | :-- |
| **system signing seed** | `~/.rivet/keys/rivet-system.seed` | signs system **update** payloads |
| **application signing seed** | `~/.rivet/keys/rivet-app.seed` | signs **applications** and **libraries** |
| **your control-plane identity** | `~/.rivet/identity/operator/` | your end of the mutually authenticated TLS |
| **the board's identity** | `~/.rivet/identity/board/` | the machine's end, plus a `peers` file naming yours |

**The two seeds are separate on purpose** (`docs/ADR-028` §5.5): a machine that
can sign an application must not be able to ship an operating system, and the
system key is not entitled to sign an application.

**It never overwrites a key.** A seed already on disk is reported and left
alone, so running it twice is a report rather than a disaster. Overwriting one
orphans every board that trusts its public half and every payload signed with
it, and there is no undo. `cargo rivet setup --check` looks and mints nothing.

The public halves go beside the seeds, and its closing report says what to copy
into `keys/` in this repository — the card build mounts nothing else, and a
fresh clone has no keys there until you add yours (`keys/README.md`). A public
key belongs in the repository; a seed never does.

#### 4. The machine's document — `rivet-provision`

```console
$ cargo run -q -p rivetos-provision
```

**The one command-line tool in this repository written for a person.** It asks
what the machine should be — hostname, addressing, headless or kiosk,
stand-alone or fleet — validates each answer as it is given, and runs the whole
document through `rivet-config`'s real parser before writing it, so a refusal
arrives here rather than from the board on first boot.

On a workstation with no **fleet certificate authority**, its first question is
whether you are starting a new fleet or joining an existing one
(`docs/ADR-063`). A new fleet mints the authority and files your browser
identity into the certificate store a browser reads; joining mints nothing and
says what to carry across from a workstation already in the fleet. The
authority is only needed for the web portal; a board is completely reachable
from the command line without it.

Addressing is `static` or `dhcp`. Choose `static` for a first board unless you
have another way to find its lease: you type its address into step 8.

#### 5. The card image

Take the environment `cargo rivet setup` printed, then:

```sh
RIVET_DESCRIBE=$(tools/card/bump.sh | tail -1) \
RIVET_DOCUMENT=<the document from step 4> \
tools/container/build-card.sh
```

`bump.sh` increments the patch version, commits that one change alone and
prints the describe string (`docs/ADR-027`). It refuses a dirty tree. On macOS
and Linux, run it from the ordinary shell; **on Windows, run it from Git Bash,
never WSL** — WSL's `git` reads a Windows working copy's line endings as
modifications, and `-dirty` in an artifact's name is a claim that uncommitted
changes exist.

The build prints what it composed. Read those lines: `--- trust:` names the keys
the card will accept, `--- provisioning:` names the document, and
`--- identity:` says whether anything at all will be able to reach the machine.

#### 6. Writing the card — `cargo rivet write-image`

```console
$ cargo rivet write-image --image out/<image>.img --document <document>.toml \
      --device <device> confirm
```

It injects the document into **both** boot partitions — a card provisioned onto
one of them forgets its configuration at its first update — and then writes the
image to the device and reads it back.

**`cargo rivet provision`, with no `--document`, runs steps 4 and 6 as one
command**: it launches `rivet-provision`'s interview itself, then writes the
card with the document that produced. Neither verb asks anything with no
terminal attached: a script or an agent gets a refusal naming the flag to pass
instead.

**It refuses twice before anything is written**, and neither refusal is a
warning, because a warning is read after the card exists:

* the credentials of step 3 are not on this host, naming each missing piece and
  the command that makes it; and
* the **image's own** `peers` file does not name your operator key, which means
  the card cannot answer you. `--not-mine` is the way past that, for a board
  somebody else operates.

Omit `--device` and it enumerates the removable disks and asks; omit `confirm`
and it prints the plan and writes nothing. **A disk it did not enumerate as
removable is refused.** The cost of the wrong path is this workstation's own
disk.

#### 7. First boot

Insert the card and power the board. PID 1 mounts, reads the document off the
FAT boot partition, brings the network up from Rust over netlink, moves the
identity to `/state/secrets` and deletes the boot-partition copy, and starts the
control plane on port 7777.

#### 8. The step that is actually the test

```console
$ cargo rivet identity show --directory ~/.rivet/identity/board
$ cargo rivet run --host <address>:7777 --identity ~/.rivet/identity/operator \
      --pin <the board's public key> -- get status
```

**The pin is the BOARD's public key, not yours.** It is the one asymmetry in
this whole surface and it catches everybody: `--identity` is your directory,
`--pin` is what you expect the machine to prove it holds.

If that answers, the first run worked. Nothing before it establishes that.

### More than one machine: a fleet

The first run produces **one board you can reach**. A **fleet** adds an
authority that issues every machine's certificate, a fleet server that holds
the register and serves the portal, and the machines that join it.
`docs/SPEC-05-Provisioning-a-Fleet.md` is the normative sequence, and
`skills/provisioning-a-fleet-server/SKILL.md` walks it.

**The role is the one answer that changes every other.** `rivet-provision` asks
it as *Fleet*:

| answer | what it is | what the role turns on |
| :--- | :--- | :--- |
| `stand-alone` | one board, administered from your workstation | control plane on **loopback**; no register, no portal |
| `server` | the machine that holds the register and serves the portal | control plane on **every interface**, `rivet-fleetd`, `rivet-proxyd`, `rivet-mdnsd` |
| `member` | a board that belongs to an existing fleet | control plane on every interface, and `rivet-mdnsd`, because it is reached by name |

Three things in that sequence cost a card or a browser if they are got wrong:

* **Back up the authority before anything replaces it** —
  `rivet-provision ca backup <directory>`. There is no default location for
  secret keys, and `ca restore` is the inverse.
* **Install `root.crt` yourself.** The tool names the file and the store and
  stops: trusting a root certificate is a decision a person takes.
* **The timezone has no default.** A machine provisioned without one keeps UTC
  silently, and every timestamp you would use to find out why is wrong too.

A fleet server has worked when `get status` shows `rivet-fleetd` `started` and
`https://<host>:4443/` shows the fleet with the machine in it — measured with
`get status` and the portal's `/api/fleet`, never by reading its HTML.

---

## Build and test

**Enable the docs pre-commit hook, once per clone:**

```console
$ git config core.hooksPath .githooks
```

It regenerates `docs/INDEX.md` and `docs/index.json` before each commit and
stages them, and skips itself with one line if `uv` is not on the PATH.

**The whole workspace tests under Linux, which on any host means the
container** — several crates are Linux-only:

```sh
tools/container/exec.sh cargo test --workspace
tools/container/exec.sh cargo clippy --workspace --all-targets
tools/container/exec.sh tools/test-all.sh     # formatting, lints, tests and the dependency gate, every workspace
```

**The workstation crates also test natively** on Windows, macOS and Linux, which
is the quick loop when that is what you changed (`docs/ADR-062` §5):

```sh
cargo test -p rivetos-core -p rivetos-config -p rivetos-crypto -p rivetos-tls -p rivetos-remote \
    -p rivetos-fleet-repo -p cargo-rivet -p rivetos-provision -p rivetos-reset -p rivetos-fleet
```

**Building an application for a Rivet** is `docs/SPEC-01-Rivet-as-a-Build-Target.md`:
what the target offers, what it deliberately lacks, and what each kind of
program needs. Start from `examples/templates/` — `template-network-service`
and `template-go-service` for something that serves, `template-kiosk` for
something that draws — rather than deriving the build flags yourself.

**The inner loop is QEMU**: a real PID 1 under emulation, with any combination
of the supervisor, the CLI, `rivet-netd` and a provisioning document.
`skills/running-the-qemu-harness/SKILL.md` and `tools/qemu/README.md` are how,
and `docs/ADR-026` §4 is what a green run does not prove.

---

## Using it

A board is reached over **the control plane** (`docs/ADR-035`): MCP over TLS,
mutually authenticated with pinned Ed25519 keys, on port 7777. There is no
second way in — no remote shell and no login. The CLI is an adapter over
`rivet-core` and contains no logic of its own, so the same catalogue answers on
the machine and over the wire.

From a workstation, every operation is one `cargo rivet run`, with the machine's
answer printed exactly as it composed it:

```console
$ cargo rivet run --host <address>:7777 --identity <dir> --pin <hex> -- get status
$ cargo rivet run <device> -- get storage
$ cargo rivet run <device> -- apply service-restart <name> confirm
$ cargo rivet run --show -- apply edit /state/config/machine.toml --expect a --replace b confirm
```

**A bare leading word before `--` is a device name**, resolved against the
fleet register into `--host`, `--identity` and `--pin`; anything you also write
explicitly wins. `--show` prints the call and sends nothing. `--full` prints an
answer over 64 KiB uncapped — or redirect it to a file, which is never cut.
`cargo rivet run --help` is the whole story.

The words after `--` are the machine's own command:

```console
get catalogue                  what this machine can do — start here
get status                     every service, and the machine's own health
get log [boot] [limit]         the journal; limit defaults to 200, `all` for everything
get storage                    how full each mounted filesystem is
get schema                     the configuration document's JSON Schema
explain <path>                 what generates a path, and what to change instead
apply trial-boot <a|b> confirm arm a one-shot trial of the other A/B slot
apply trial-restart confirm    take it — restart once into the armed slot
apply restart confirm          restart the machine on its default slot
apply power-off confirm        stop the machine and leave it off
apply service-restart <name> confirm
                               restart one service, not the machine
```

**Everything under `apply` changes the machine, so it confirms first:** without
the final word `confirm` you get the plan and nothing happens. Arming a trial
boot and restarting into it are deliberately two commands, and a plain
`restart` does not take an armed trial.

`docs/COMMANDS.md` is every workstation and board binary's own `--help`,
generated rather than transcribed. `rivet get catalogue` on a machine is the
authority for what that machine can do.

### Applications

An application is a signed `.rvt`. `cargo rivet package` builds a Rust crate for
the board and signs it (`docs/ADR-015`); the crate declares what it is, and what
it asks for, in `[package.metadata.rivet]`, and that table travels inside the
signature as the permission request. `cargo rivet inspect FILE.rvt
--trusted-key PATH` verifies one through the same verifier a machine uses and
prints what it asks for.

`cargo rivet install` sends one to a board over the control plane and waits for
the install to finish; `cargo rivet deploy` is the interactive interview that
composes it. **Installing does not run it**: whether an application runs is the
machine's document's decision, and `install --enable` writes that for you.
`skills/deploying-an-application/SKILL.md` is the procedure.

**The signing key** is found at `--signing-key`, then `$RIVET_APP_SEED`, then
`~/.rivet/keys/rivet-app.seed`. **A key you generate yourself will not deploy to
a board somebody else built**: a machine accepts exactly the keys its image was
built with. Ask one with `rivet get trust`.

### The fleet repository

`cargo rivet fleet` keeps a git-tracked manifest and a digest-keyed object store
on the workstation, `~/.rivet/fleet` by default, naming every signed artifact a
fleet is built from — so *"what exactly is on this machine, and can I put the
identical thing on that one?"* has an answer (`docs/ADR-052`). `fleet publish`
verifies an artifact before it stores it, publishing the same artifact twice
changes nothing, and initializing or publishing into a tree that contains a
`.seed` file is refused with no override. `cargo rivet deploy` publishes into it
before it sends.

---

## Reading the design

Most of the record was written *before* the code it governs. In the order it
was decided:

1. `docs/ADR-001` — the language and dependency posture, which frames the rest
2. `docs/ADR-002` — target hardware, and why the boot chain is not a problem
3. `docs/ADR-011` — **the single source of authority**, which every other
   decision turns out to be an instance of. The best entry point to the record
4. `docs/ADR-006` and `docs/ADR-007` — the administration surface and the
   configuration model
5. `docs/ADR-017` and `docs/ADR-018` — supervision, and storage and A/B updates
6. `docs/ADR-026` — how this is iterated on before hardware is involved, and
   the limits of doing so
7. `docs/ADR-035` — the control plane

Beside the ADRs: the **SPECs** (`docs/SPEC-*.md`) are normative — what the
software must do, precise enough to test against. `skills/` holds the recurring
procedures. `docs/COMMANDS.md` is the command reference and `docs/INDEX.md`
maps every heading in `docs/` to its file and line range, both generated.
`docs/design-candidates/` keeps proposals for their reasoning, including the
ones that were rejected.

### How this repository is published

Rivet is developed in a private repository and published here as a copy of its
tree, without its history. A few files stay private: a working journal
(`docs/PLAN.md` and its archives), a status file (`STATE.md`), the maintainer's
brief for AI agents (`AGENTS.md`), and the record of his rulings
(`.rulings/`). The documents published here cite them, and those citations are
left as they are. The ADRs and SPECs are written to read completely without the
rulings; for the journal and the brief, moving what is worth citing into an ADR
is ongoing work.

---

## License

Licensed under either of

* Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
* MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option. Copyright © 2026 Joel Lindell.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this work shall be dual licensed as above, without any
additional terms or conditions.

The reasoning for dual licensing — Apache-2.0's explicit patent grant removes an
adoption barrier MIT leaves in place — is in `docs/ADR-010` §6.4. The image as a
whole is an aggregate of separately licensed components; see `docs/ADR-010`.

---

## Acknowledgements

Rivet is developed in conversation with Anthropic's **Claude Opus** models
(Claude Opus 5 and Claude Opus 5.5), through **Claude Code**. The design
decisions are the maintainer's; the documents in `docs/` were drafted
collaboratively and record the reasoning on both sides, including the proposals
that were rejected and why. The code was written the same way.

The name and the original logo came from **Google Gemini**, and the name is the
reason the project exists in this form rather than another.
