# tools/nushell — the login shell, and how to get one

`ADR-005` §5.1 ruled **nushell** Rivet's interactive login shell on
2026-08-22. §5.6 proved it builds and runs. The ruling made it `root`'s shell on
2026-08-24. `build.sh` in this directory is the recipe that produces the binary
those rulings talk about.

Nothing here is committed. The binary is an **input** to `cmd/rivet-image`,
exactly as the kernel and the Raspberry Pi firmware are (`ADR-010` §6.1).

---

## Build it

```sh
tools/nushell/build.sh
```

It needs Buildroot's cross-toolchain and **refuses rather than downloading
one** — see `build.sh`'s own header for why a second musl would be the wrong
kind of cheap. If it complains, build the toolchain first:

```sh
tools/buildroot/build.sh --fetch toolchain
```

The output is `$HOME/.rivet/nushell/bin/nu`. Roughly **4 minutes** on the
development host, and the binary is **42.4 MiB** stripped.

## Put it on a card

```sh
cargo run -p rivetos-image -- out/card.img \
    <init> <supervisor> <cli> <netd> <mcpd> <timed> \
    --nushell "$HOME/.rivet/nushell/bin/nu"
```

`--nushell` is **optional**, and that is the ruling that makes `nu` optional rather than a
convenience: a card built without it has `cmd/rivet` as its login shell, which
is what every card built before 2026-08-24 had, and the runtime projection is
required to keep producing a working `/etc/passwd` for it.

## Watch it in the emulator

```sh
RIVET_NUSHELL=$HOME/.rivet/nushell/bin/nu \
RIVET_KERNEL=<buildroot>/images/Image \
tools/qemu/smoke.sh
```

`tools/qemu/run.sh --nushell PATH` is the interactive form.

---

## What has actually been proved, and by what method

`AGENTS.md` §6. Everything below was **run**, on 2026-08-24, under
`qemu-system-aarch64 -M virt` on `ADR-024`'s own kernel.

| Claim | How |
| :-- | :-- |
| `nu` 0.115.1 cross-compiles static aarch64 with Buildroot's toolchain | `build.sh`, first attempt, no patches |
| `ADR-003` §3's three properties hold | `file`, `readelf -l`, `readelf -d`, read off the produced ELF by `build.sh` and refused if any fails |
| The projection prefers it | `smoke.sh` asserts `"shell":"/system/bin/nu"`, `"listed":2` and `"also_permitted":"/system/bin/rivet"` off the guest's own console |
| **A login lands in nushell** | `SHELL=/system/bin/nu` read off the running machine, 2026-08-24, by a `smoke.sh` stage that has since been deleted |
| **`ADR-005` §5.1's actual promise** | `rivet --version \| from json \| get name` → `rivet`, on the guest — by the **bare word** since 2026-08-24. The same string is refused by `cmd/rivet`'s `-c` contract with `E_NOT_A_SHELL` |
| **An interactive session, over a real pty** | the same bare command was run over a pty session and its answer asserted; a full REPL — banner, `reedline`, bracketed paste — has been driven by hand on the same guest, with no terminfo on the machine |
| A card without `nu` is unchanged | the same suite passes with `RIVET_NUSHELL` unset, in all three of its other shapes |

### What is NOT proved

* **Nothing here has run on a board.** `ADR-026` §4's right-hand column applies
  in full.
* **An interactive session HAS started**, 2026-08-24. A pty was
  allocated and `reedline` drew its prompt over it. **What is still not proved
  is a HUMAN at that prompt**: the client's stdin has only ever been a FIFO or
  `script(1)`, so nothing has typed a key, resized a window, or interrupted a
  running command.
* **The size cost is measured and not ruled.** `ADR-005` §5.6.2 puts the
  distributed artifact at roughly +42 % for this binary; nobody has re-measured
  it against a card that actually contains one.

### A login session's `PATH` — ruled and built 2026-08-24

**the session-`PATH` ruling, `PLAN.md` §0.11.** The session's `PATH` is `/system/bin`,
set in `tools/buildroot/configs/rivet_aarch64_defconfig`.

The failure it fixed is worth keeping in view, because the value is compiled in
rather than read at runtime. Before the ruling a bare `rivet` was "command not
found", and this was read off the running machine:

```
PATH  = /usr/sbin:/usr/bin:/sbin:/bin
SHELL = /system/bin/nu
```

**None of those four directories exists on a Rivet.** It was invisible while
`cmd/rivet` was the login shell, because a login shell is exec'd by the absolute
path in `/etc/passwd`'s seventh field and no lookup happens. The moment nushell
is the shell, `PATH` is what has to find everything.

`smoke.sh`'s tripwire fails when `/system/bin` is *absent*, in either direction.
