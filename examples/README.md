# `examples/`

**The directory says which is which** — `ADR-036` §6.1.

```text
examples/
├─ templates/   copy one of these
└─ probes/      evidence. Read them; do not start from them.
```

| | |
| :-- | :-- |
| **A template** | exists to be **copied**. Everything in it is either something your service will also need, or a comment saying why. |
| **A probe** | exists to have **answered a question**. Its value is that the answer is on the record — not that it is a good starting point. |

Every directory here was a probe until 2026-09-01, and each is shaped by the
thing it was settling rather than by what an author needs on day one. **Copying a
probe means inheriting its question.**

This was a `README` heading until the maintainer pointed out that a heading is
documentation and a directory is structure. It is structure now, which is what
§6.1 asked for in the first place.

---

## `templates/` — copy one of these

| Directory | For | State |
| :-- | :-- | :-- |
| `template-network-service/` | **a network service in Rust** — handed a socket, told where to write, talks to the journal, and reads another application's endpoint | built, running on hardware |
| `template-go-service/` | **the same in Go** — and the proof that the contract is language-neutral rather than a Rust crate wearing a specification's clothes | built, running on hardware |
| `template-kiosk/` | **an application that DRAWS** — handed a display, draws with the SDK, and takes its own screenshot so you can see it from somewhere else | built, running on hardware |
| *(screensaver controller, Go + Rust)* | the shipped demo pair | **not yet** — `ADR-036` §6.2B |

### `template-kiosk` solves three things you would otherwise have to find

**The display is handed to you.** `rivet_sdk::screen::handed()` waits on the
channel the supervisor named in `RIVET_DISPLAY` and takes the lease the launcher
cut for it. **It does not arrive on `LISTEN_FDS`** — that is for sockets — and
nothing opens `/dev/dri`. The SDK's first article (`ADR-036` §2): an application declares what it
needs and is HANDED it.

**You do not write a renderer.** `rivet_sdk`'s `draw` feature gives you a
canvas, rounded rectangles, and Geist through `text::Face` — the launcher's own
drawing, extracted rather than reimplemented (`ADR-036` §5.4A).

**The linker flags are already in its `.cargo/config.toml`,** with the reasoning.
An application that links a system shared library needs three of them and the
failure names the library rather than the cause. Start from the template and you
never meet it.

The network templates are deliberately **ordinary** — `tokio`, `axum`, `serde`,
`rusqlite` on one side; the Go standard library on the other (`ADR-036` §6.3).
Nothing should look like it was written for an unusual operating system, except:

* they **do not bind a port** — the supervisor opens the socket and passes the
  descriptor (`ADR-015` §5.2);
* they **do not choose where to write** — `RIVET_APP_DATA` is the one writable
  directory they have;
* they **do not guess another application's socket path** — it arrives in
  `RIVET_CONNECT`, granted by the operator.

---

## `probes/` — read these, do not start from them

| Directory | The question it answered |
| :-- | :-- |
| `hello-rivet/` | the smallest thing that runs at all |
| `socket-demo/` | `LISTEN_FDS`. It also **found** the `LISTEN_FDNAMES` encoding defect — a manifest's `tcp:80` arrived as two names, because the variable is colon-separated |
| `sqlite-demo/` | **`rusqlite` with `bundled`** — SQLite compiled from C for `aarch64-musl`, WAL on `/state`. Two cold agents had each concluded this was impossible; the toolchain was there and simply unexposed |
| `libgreet/` + `greeter/` | a **dynamic** C library and a program linking it, through the declared closure (`SPEC-02`) |
| `mesa-demo/` | the GPU |
| `kiosk-demo/` | taking the display with KMS, and then **accepting a leased descriptor** — the first application a launcher handed a display to (`PLAN.md` §106). **Start from `templates/template-kiosk/` instead**; this one predates the SDK and carries its own bitmap font |
| `slow-stop-probe/` | **is a slow service actually waited for at shutdown** — it catches `SIGTERM`, takes eight seconds to go, and writes down every second of it to both the journal and a file. Built because the shutdown-grace ordering was fixed (`.rulings/` R-141) and then only ever exercised by shutdowns in which nothing was slow, which is consistent-with rather than proof |

---

## `aura/` — the maintainer's own application, in Rust

**Added to this file on 2026-09-03**, having been on disk since 2026-09-02 with
nothing here mentioning it — the audit of that day found the gap
(`PLAN.md` §123).

These are not templates and not probes. They are the Rust port of Aura, the
maintainer's own display application, kept in this repository because the port
is a **Rivet** program even though Aura is not a Rivet project: the C++ original
stays in Aura's own tree and this is the half that uses the SDK.

| Directory | What it is |
| :-- | :-- |
| `aura-card/` | **the vocabulary** — `SPEC-04`'s card wire format, and the only part of Aura a third party links. Zero dependencies, and the one directory here that is a member of the ROOT workspace rather than its own |
| `aura-controller/` | the other half, added 2026-09-05. Holds every fact the display shows and pushes it to the renderer over a socket the supervisor opened, and serves the portal on a `tcp:8080` the supervisor also opened. **The first application here to declare `provides`, `listen` and `proxy-path` together**, which is what makes `/aura/` a route the gate protects |
| `aura-renderer/` | the port. A kiosk application that draws through `rivet_sdk::screen` and takes cards from four slot sockets. **The SDK's most demanding consumer**, and the first program to hold the re-issue contract (`SPEC-03` §3.7) |
| `aura-vitals-test/` | an addon: a producer that connects to one slot and publishes a card, which is what `provides`/`connects` is for |
| `aura-probe/` | measurement, not a program to copy — where a frame's time goes on a real panel (`SPEC-04` §10) |
| `vitals-fixture/` | not a program. The `/proc` and `/sys` tree `aura-vitals-test` is read against and the card it must produce from it. **Hand-composed, and its own `README` says so and says why each number was picked** — the provenance is on the file, per `AGENTS.md` §4 |

**`aura-controller` serves HTTPS, so its `tls-cert`, `tls-key` and `tls-root`
are installed per machine by the operator** — no payload carries them, and
without them it crash-loops. Two commands, one per machine and one per
application, in the order `skills/bringing-up-a-new-machine` puts them:

```sh
rivet-provision ca server <hostname> <ca dir>
cargo rivet secrets aura-controller --host <address>:7777 \
    --identity <your identity dir> --pin <the MACHINE's 64 hex> \
    --name <hostname>.<suffix> confirm
```

**Do not start from these either.** They carry Aura's own layout decisions and
its schema. `templates/template-kiosk/` is the one to copy.

---

## What is NOT here any more

**Aura's PACKAGING moved to `deploy/aura/`** on 2026-09-01 — the Go controller
and the C++ renderer's build. Those two directories were never examples: they
are scaffolding that gets the maintainer's own application onto a board, and
they teach patterns this platform no longer allows — opening `/dev/dri/card0` by
path, and hardcoding a peer's socket path. `ADR-036` §6.2B is the ruling; Aura
stays private, with real data, and is not an example.

(The Rust port under `aura/` above is a different thing and stays: it is written
*to* the SDK rather than around it.)

A **9.2 MB compiled binary** went with it. It was a build artifact of that
private application, tracked in git, and larger than every source file in this
repository combined.

---

## Before you choose a dependency

**`SPEC-01` §5.2 is the list of what a Rivet does not have** — no shell, no
`$HOME`, no environment a manifest can set, no D-Bus, no `/usr/share/zoneinfo`,
no `pkg-config` for the target. A crate that reaches for any of them compiles
perfectly and fails on a board.

What is *known to work* is the table in `ADR-036` §6.3, and its rule for entry is
this project's rule everywhere else: **a row goes in when something ran on the
board**, not when it compiled.

---

## Building one

Each has its own empty `[workspace]` table, so `cargo build` at the repository
root never reaches them and `examples/.gitignore` catches their output.

**Rust** — needs the container, because `rusqlite` compiles C for the target:

```sh
tools/container/exec.sh sh -c '. tools/toolchain/env.sh && \
  cd examples/templates/template-network-service && \
  cargo build --release --target "$RIVET_TARGET"'
```

**Go** — needs neither the container nor a C toolchain. `CGO_ENABLED=0` makes
the cross-compile two environment variables, so this runs on Windows, macOS or
Linux with nothing installed but Go:

```sh
examples/templates/template-go-service/build.sh
```

That asymmetry is about **dependencies, not languages**: a pure-Go program has
no C to cross-compile, and a Rust program without a `-sys` crate would not need
the container either.
