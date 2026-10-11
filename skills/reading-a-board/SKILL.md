---
name: reading-a-board
description: Read the state of a running Rivet machine over the control plane — what is running, the journal, which A/B slot, storage, trust, and how to change something safely. Use when asked to check, diagnose, read, or administer a Rivet board or machine.
---

# Reading a board

**There is one way in, and it is the control plane.** A Rivet card carries no
login daemon and accepts no remote shell (`ADR-035`). Every command below goes
through:

```sh
cargo rivet run --host <address>:7777 --identity <your identity dir> --pin <the MACHINE's 64 hex> -- <rivet command>
```

If you find yourself reaching for SSH, a serial console, or any other way onto
the machine, you are on the wrong path — say so rather than building the
workaround. `--show` proves what a command will do before it touches anything:
it needs no identity, no pin and no machine, and sends nothing.

```sh
cargo rivet run --show -- apply install-secret rivet-fleetd server.crt --hex DEADBEEF confirm
```

## The questions and the commands that answer them

| Question | Command |
| :-- | :-- |
| What is running, and has anything restarted? | `-- get status` |
| The journal, the whole thing | `-- get log latest all` |
| The boot that just ended | `-- get log previous all` |
| Which A/B slot, and is it committed? | `-- get slot` |
| Which signing keys does it accept? | `-- get trust` |
| How full is each filesystem? | `-- get storage` |
| The last install and how it ended | `-- get install` |
| One file's contents | `-- get file <path>` |
| The document it will accept | `-- get schema` |
| What generates a path, and may it be written? | `-- explain <path>` |
| The machine's own catalogue of verbs | `-- get catalogue` |

`get catalogue apply` and `get catalogue apply install-secret` narrow to one
verb's subjects — ask the machine rather than guessing at a command shape.

## Traps, each of which has cost a reading

* **`get log` defaults to 200 records** and hides exactly the thing you were
  looking for. Say `get log latest all` for the whole current boot, or
  `get log previous all` for the one before — the second word is a boot
  selector, not a filter, and there is no per-service filter: take the whole
  answer and select on each record's `svc` yourself.
* **`get install` reports the LATEST install**, full stop. A stale
  correlation, or a bare `finished` with nothing else checked, reads
  somebody else's install as yours. Read the `correlation` an install
  returned and wait for `get install` to report that same value changing
  state, not merely for a field to appear.
* **`in_trial_boot` says how a boot was ENTERED, not whether it committed.**
  Read `committed` from `get slot`. PID 1 commits a healthy trial itself, so a
  board can look armed and already be past it.
* **Which slot is running is read from what the firmware and the kernel each
  state**, never from hashing a binary — two slots holding the same build
  would make a hash comparison a coin toss. `get slot` reports `agree: true`
  when the two independent answers match; do not substitute your own
  derivation for it.
* **A key in `get trust` does not mean an artifact it signed will install.**
  The machine also checks the signature, the kind that key is entitled to
  sign, and the architecture. There is no expiry and no command that removes
  a trusted key deliberately — an expired trust entry does not fail safe on a
  headless board.

## Changing a machine, briefly

Every `apply` confirms: without the literal final word `confirm` it prints the
plan and changes nothing.

```sh
-- apply edit /state/config/machine.toml --expect "<text>" --replace "<text>" confirm   # replace-exactly-once
-- apply edit /state/config/machine.toml --append "<text>" confirm             # begin <text> with a newline
-- apply reload confirm                        # re-read the document — do this before service-restart
-- apply service-restart <name> confirm        # one service, not the machine
-- apply service-stop <name> confirm           # pause one application; stays stopped until service-start or a reboot
-- apply service-start <name> confirm          # release that stop
```

**`apply service-stop` pauses an application and leaves its files and data
alone** — use it instead of `apply uninstall` when the application is coming
back. It is refused for `rivet-mcpd` and `rivet-netd`, which carry the way in,
and for every other image service. A reboot releases it (`ADR-017` §6.5).

**`apply service-restart` does NOT re-deliver settings from a changed
document.** Settings are parsed at spawn; `apply reload` first, every time, or
the service keeps running on stale values with nothing in the refusal to say
so.

## Getting the pin and the identity right

`--pin` is the **machine's** public key, 64 hex characters — the one
asymmetry in this surface that catches everybody. Read it from the board's own
certificate, never from a seed:

```sh
cargo rivet identity show --directory ~/.rivet/identity/<board>
```

For a fleet with a register, `rivet-fleet get device --device <name>` prints
the pin it holds. `--identity` is **your own** directory, from
`cargo rivet identity new` — by convention `~/.rivet/identity/operator`, which
`cargo rivet run` falls back to when `--identity` is not given
(`cmd/cargo-rivet/src/device.rs`'s `default_identity_dir`).

On Git Bash, set `MSYS_NO_PATHCONV=1` for anything carrying a `/state`,
`/proc` or `/sys` path, and pass `--identity` a Windows path (`cygpath -w`) —
otherwise the shell rewrites the argument before the program starts, and the
refusal you see names the mangled path rather than the real problem.

See `docs/COMMANDS.md` for `cargo rivet --help`'s full verb list and
`docs/COMMANDS.md`'s `rivet` entry for the on-board CLI's own catalogue —
the authority for what exists is `rivet --version`'s generated operation
catalogue, not this document or any other prose description of it.
