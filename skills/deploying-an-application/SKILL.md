---
name: deploying-an-application
description: Deploy a Rivet application (a crate or a built .rvt) to a running Rivet machine over the control plane — build, package, publish into the fleet repository, transfer, install, and start it. Use when asked to deploy, install, update, or ship software onto a Rivet board.
---

# Deploying an application

An **application** is a signed `.rvt` — a program plus its manifest — installed
onto a running Rivet machine over the control plane (`ADR-035`). This is
different from a **card image** (provisions a machine, `cmd/rivet-image`) and a
**system update** (updates the OS itself, `cargo rivet update`); see
`docs/COMMANDS.md`'s `cargo-rivet` entry for the full verb list.

## Two front ends, one planner

* **`cargo rivet deploy`** is the interview, for a person at a terminal. It
  takes no arguments — it picks the artifacts and the hosts itself — and asks
  before each step. Anything typed after `deploy` is refused.
* **`cargo rivet install`** is the scripted verb, for an agent or a script.
  The same planner drives it (`ADR-055` §3): it installs, **waits for that
  install to finish**, and then **prints the steps that remain** as exact
  commands — the same steps the interview would offer for the same artifact on
  the same machine.

## An agent's order

0. **Run the same `cargo rivet install` once WITHOUT `confirm`.** It changes
   nothing, connects to nothing and reads no key, and it ends with **NEEDS**:
   the machine it will reach, the files it will read (the operator identity and
   a signing seed hold private keys), what it will write, and what happens on
   the machine. If your environment does not already allow all of that, ask
   whoever runs you for exactly that list before going on. Nothing in this
   skill states the list itself, because the tool derives it from the run you
   are about to make.

1. **`cargo rivet install`, and let it wait.**

   ```sh
   cargo rivet install <device> --artifact <digest-or-prefix> confirm
   ```

   It reads `get install`'s `correlation` just before its request, waits for
   it to change and then for that install to finish, and **exits non-zero if
   the install failed**, naming the installer's outcome. Do not add
   `--no-wait`; the next steps write into a data directory only the installer
   makes. `--enable` is fine when the application declares no per-machine
   material, or when the machine already holds it — otherwise it is refused,
   naming the commands, and the order below is the one that works.

2. **Run the steps it prints, in the order it prints them, and nothing else.**
   They follow a `next` line and are numbered. Which ones appear depends on the
   artifact and the machine; the full vocabulary is:

   | step | what it prints |
   | :-- | :-- |
   | a dependency first | the enable of a system service the manifest `connects` to and this machine has switched off, or `cargo rivet install <device> --artifact <digest> confirm` for one the fleet repository holds — before the application's own steps, as the interview offers them |
   | stage material | `rivet-provision ca server <hostname> <ca dir>` (once per machine — skip it if the server pair already exists), then `cargo rivet secrets <app> --host … --identity … --pin … --name <hostname>.local confirm` |
   | enable | `cargo rivet run <device> -- apply edit /state/config/machine.toml --append '…' confirm`, the text spanning several lines inside the quotes and beginning with a newline — an append that would continue an unterminated last line is refused |
   | restart a service | `cargo rivet run <device> -- apply service-restart <app> confirm` |
   | restart the proxy | `cargo rivet run <device> -- apply service-restart rivet-proxyd confirm` — a routed application's route is read at the proxy's start |
   | converge | `cargo rivet run <device> -- apply reload confirm` |
   | fleet sync | `cargo rivet fleet sync <device> confirm` — only on a fleet server, so `rivet-fleetd`'s store holds what was installed |

   **Improvise nothing.** A step the planner does not print is not owed, and a
   step it prints is — including the proxy restart, which was the one most
   often forgotten when these steps were typed from memory.

3. **Read the machine back** (see *Verify* under the traps below).

`next NOTHING REMAINS` means exactly that. `THE STEPS THAT REMAIN COULD NOT BE
PLANNED` means this workstation has no application signing seed to read the
artifact's manifest with; the install itself happened.

## The one command, for a crate already on this machine

```sh
cargo rivet install --manifest-path <crate>/Cargo.toml \
    --host <address>:7777 --identity <your identity dir> --pin <the MACHINE's 64 hex> \
    confirm
```

This builds the crate, strips and packages it, **publishes it into the fleet
repository first** (`~/.rivet/fleet`, or `--at DIR` for a scratch one —
`ADR-052` §7), then sends and installs the published copy, and prints what
remains. `--enable` writes `[services.<name>] enabled = true` into the
machine's document **before** the payload is sent; without it the install lands
but stays idle.

**Installing does not start it.** The steps `install` prints are what does.
Among them, `apply reload` comes before any restart — `apply service-restart`
alone does **not** re-deliver settings from a document you just changed; it
restarts the service with whatever the supervisor already parsed at its last
spawn.

## The manual recipe, when you need control over the build

This is what actually shipped `rivet-fleetd` on 2026-09-13
(`docs/PLAN-Feature-Set-2026-09-12.md` §8.2) — cross-building by hand rather
than letting `deploy` build for you, e.g. to control the target profile or to
package a binary that was already built and stripped elsewhere:

```sh
# 1. Cross-build, inside the container — tools/toolchain/env.sh prints what it sets.
tools/container/exec.sh sh -c '. tools/toolchain/env.sh && cd /rivet/<crate> && \
    cargo build --release --target "$RIVET_TARGET"'

# 2. Strip. The release profile does NOT strip (ADR-005 §7.5); an unstripped
#    binary over MAX_PROGRAM_BYTES (16 MiB) is refused by name.
#    Inside the container the build lands in "$CARGO_TARGET_DIR" (/target, a
#    shared volume), NOT in the crate's own target/. A target/ beside the crate
#    is a leftover from some earlier native build, and stripping it packages an
#    old program under today's version without any error.
"$RIVET_STRIP" -o /tmp/program "$CARGO_TARGET_DIR"/aarch64-unknown-linux-musl/release/<bin>

# 3. Package from the already-built, already-stripped binary — no cargo build here.
cargo rivet package --manifest-path <crate>/Cargo.toml --binary /tmp/program \
    --target aarch64-unknown-linux-musl --out-dir out/

# 4. Install the built .rvt. --at DIR for a scratch fleet repository while
#    rehearsing; omit it to publish into the maintainer's real ~/.rivet/fleet.
#    Then run the steps it prints.
cargo rivet install --rvt out/<name>.rvt --at <scratch-dir-or-omit> \
    --host <address>:7777 --identity <dir> --pin <hex> confirm
```

Check the manifest before any of this, if you did not write it yourself —
builds nothing, signs nothing, needs no key:

```sh
cargo rivet package --print-manifest --manifest-path <crate>/Cargo.toml
```

## Traps that have already cost a session

* **`cargo rivet` on your PATH is probably stale.** `cargo rivet --version`
  prints which file answered and which is first on PATH, and says so loudly
  when they differ. `AGENTS.md` §10 has the full account; the short form is
  `cargo install --path cmd/cargo-rivet --force --locked` from the repository
  root — and the full install is that line plus
  `cargo install --path cmd/rivet-provision --force --locked`, which deploying
  does not need and provisioning does.
* **`--pin` is the MACHINE's public key, not yours.** The one asymmetry in
  this whole surface that catches everybody.
  `cargo rivet identity show --directory ~/.rivet/identity/<board>` reads it
  without ever opening the private half.
* **On Git Bash, `MSYS_NO_PATHCONV=1` for anything carrying a `/state` or
  `/proc` path** — the shell rewrites the argument into a Windows path before
  the program starts otherwise, and the machine's own refusal names the
  mangled path, not the shell's mistake.
* **A missing signing key is refused by name**, naming `$RIVET_APP_SEED` and
  `~/.rivet/keys/rivet-app.seed`, both places `install` (and `package`) look —
  the same lookup, one function (`credentials::app_seed`).
* **A missing fleet repository is initialised by the install itself** — no
  separate `fleet init` step is required first.
* **A proxied application's ports are unreachable from the LAN, by design**
  (`ADR-046` §7.1). With a `proxy-path` in the manifest, every `tcp:` and `udp:`
  port binds `127.0.0.1`; reach it through `https://<host>/<proxy-path>/`. A
  port a phone or another machine must dial directly is declared
  `"tcp:<port>:direct"` after the routed (first tcp) port, and the `.rvt` has to
  be rebuilt for it to take effect. The start's journal record says what was
  bound — `get log latest all`, field `bound`. `cargo rivet inspect` prints each
  port's address from the signed manifest.
* **Verify the install landed on the machine, not just that the command
  exited 0** — `AGENTS.md` §6, "until it runs, it isn't working":
  * `apply reload confirm` then `get status` — is the service `started`, and
    is `restarts: 0`?
  * `get file /state/app/<name>/<version>/bin/<bin>` — does the reported size
    match the stripped binary this session built? The transfer's own
    BLAKE3 verification is a second, independent confirmation, not a
    substitute for reading the machine back.
  * A **system update**, not an application, is a different procedure —
    `skills/publishing-a-system-image-and-update`. Since `.rulings/` R-177 D6 an
    update is published first like everything else, and `install --artifact
    <digest>` selects it from the fleet repository.
* **`install --rvt FILE` needs the application SEED on this workstation**, even
  for a file that is already signed, because it publishes before it sends.
  Without one it refuses: `NOTHING WAS PUBLISHED AND NOTHING WAS SENT`. An
  artifact already in the repository installs without one — `cargo rivet
  install --artifact <digest-or-prefix> … confirm` — but the steps that remain
  are planned from the artifact's signed manifest, which is read with the seed,
  so without it `install` says they could not be planned.
* **An application that serves HTTPS behind `rivet-proxyd` needs its TLS files
  before it will stay up.** Its manifest declares `material = ["fleet-tls"]`,
  `install` refuses `--enable` until the machine holds them, and the steps it
  prints stage them before anything enables the application. A machine that
  enables it anyway holds it `waiting-for-material` rather than crash-looping
  (`ADR-017` §3.5.2), and starts it within seconds of the files arriving.
* **`rivet-proxyd` itself is not installed.** It is part of the system image
  (`ADR-060`): `cargo rivet install` of a proxy payload is refused at the
  machine's install door, `fleet publish` refuses one before it is stored, and
  neither front end offers one. A fix to the proxy ships as a system update
  (`skills/publishing-a-system-image-and-update`). Restarting it — `apply
  service-restart rivet-proxyd confirm` — works as for any service, and is
  among the steps `install` prints after a routed application.

See `docs/COMMANDS.md` for what `cargo rivet --help` prints, `cargo rivet
install --help`, `cargo rivet deploy --help` and `cargo rivet secrets --help`
for each verb's own options, and `ADR-052` for why the repository sits between
a build and a board.
