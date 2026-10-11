# ADR-009: Reuse Posture

* **Status:** Accepted — the preference is the maintainer's, 2026-08-22. The
  ladder in section 3 and the rules in sections 4 and 5 are derived from it.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

**The maintainer's stated preference is to re-use existing projects and Rust
crates whenever that is feasible.** *(`.rulings/` R-023.)*

Taken at face value this appears to sit awkwardly beside `ADR-001`, which makes
replacing C components a tracked goal, and beside `ADR-006` and `ADR-007`, whose
whole argument is that Rivet's surface should be designed rather than
inherited.

It does not, in fact, conflict with either — but the reconciliation has to be
written down, because the naive readings of the two positions genuinely do
conflict and an agent will pick whichever one suits the commit it is making.

---

## 2. The two axes are different questions

`ADR-001`'s tier policy asks **what language a component is written in, and
whether we intend to move it**. This ADR asks **whether Rivet writes a
component at all**. They are orthogonal, and conflating them is the error.

The clarification this produces is worth more than either policy alone:

> **A Tier B component is replaced by *adopting* a Rust project, not by
> *writing* one.**

That reframes the BusyBox scoreboard from an implausible commitment — write
three hundred utilities — into a tractable one: find the Rust project that
already did it. `uutils/coreutils` is the existence proof and the model.
`ADR-001` section 4.1 should be read this way throughout.

---

## 3. The ladder

In descending order of preference. A step down requires a reason, and the
reason belongs in the commit or the ADR that takes it.

| | Option | When it is right |
| :-- | :-- | :-- |
| 1 | **An existing Rust crate or project** | Default. `uutils`, `nushell`, `schemars`. |
| 2 | **An existing C component, wrapped** | When no Rust equivalent is credible, or when the C one carries a currency benefit we cannot reproduce. `wpa_supplicant` — `ADR-004` section 3. |
| 3 | **Write it** | Only when the component *is* Rivet. |

**`rtnetlink` was a rung-1 example in that table until 2026-08-23**, when it was
surveyed and rejected; §3.6 is the reasoning. The point it was making is still
right — netlink *is* the kind of problem rung 1 exists for, and the survey that
rejected this particular crate had to work to get there. Section 3.6 is that
work.

### 3.1 Where the ladder stops

Rung 3 is not a defeat, and the boundary is precise: **Rivet writes the things
that make Rivet a distinct system, and reuses everything else.**

Written here:

* `rivet-initd` — PID 1 and service supervision. This is the project.
* `rivet-core` — the administrative operations and the configuration model
  (`ADR-006`, `ADR-007`). This is the design.
* The image layout and update mechanism (`ADR-008`).
* The wrappers that turn mechanisms into Rivet types.

Reused wherever possible: everything that is a general-purpose utility, a
protocol implementation, or a wire format. A DHCP client, an mDNS responder, a
JSON Schema generator and a netlink binding are all *solved problems that
someone else maintains*, and Rivet's version of any of them would be worse.

### 3.2 Two corrections this makes to the existing plan

`docs/PLAN.md` currently describes writing a DHCP client and an mDNS responder
in Rust. Under this ADR both are **survey-first**: existing crates are evaluated
before either is written, and writing one requires a recorded reason.

**The mDNS survey was carried out 2026-08-26 and the maintainer ruled
`simple-mdns`** — `PLAN.md` §39, and §39.5 is the recorded decision. Two things
in it are worth reading even if the crate never changes:

* **The candidate whose description matched this project's principles word for
  word was the heaviest of the four.** `mdns-proto` advertises *"no_std,
  no_alloc capable, panic-free"* and brings twenty crates. §3.3's *transitive
  weight* row, arriving from the direction nobody watches — and `cargo tree` is
  the only thing that says so.
* **The survey found a prerequisite rather than only a crate.** A responder
  needs a hostname and the configuration schema has no key for one, which no
  amount of crate-choosing fixes. A survey that had only compared libraries
  would have missed it.

**The DHCP survey has NOT been done.** `PLAN.md` §39.6 says so explicitly, so
that §39 is not read as covering both.

**A third survey, 2026-08-26: git's pack protocol**, and it is the one that most
vindicates this section. A Rust git server had been estimated cheap on the
grounds that *"`gix` is real and the receive-pack side is the whole job"*. Both
halves are true; **`gix` does not have a receive-pack side**, which was
established three ways rather than assumed. The verdict had been reached without
the survey §3.2 requires — the same error corrected once already in the
optimistic direction, arriving this time in the pessimistic one.

**The survey's conclusion is what shipped and is still shipping.** Git on a
Rivet is the C implementation, vendored and exec'd: today by
`cmd/rivet-mcpd/src/git.rs` over smart HTTP, and before 2026-08-30 by a
`rivet-gitd` crate over SSH. What changed was the transport, never the decision
this section records.

**Twice now, a cost verdict reached without a survey has been wrong, and in
opposite directions.** That is the strongest argument this ADR has for §3.2, and
it is stronger than the argument §3.3 makes from first principles.

---

## 3.3 The trade-off, stated honestly

**The maintainer's motivation, clarified 2026-08-22, is not reuse as a
principle: he does not want to waste large amounts of time reinventing other
people's work.** *(`.rulings/` R-024.)*

That is a cost question rather than a preference, and it deserves a cost answer,
because reuse is not uniformly cheaper. It is dramatically cheaper in some
places and quietly more expensive in others.

**Where reuse plainly wins:**

* **Well-specified, solved problems** — protocol implementations, wire formats,
  cryptography. The spec is fixed, someone has already read all of it, and a
  bespoke version is not merely slower to write but *worse*, in ways that surface
  as intermittent failures years later.
* **Breadth problems.** Three hundred coreutils. There is no version of this
  project in which writing those is a good use of a weekend.
* **Fiddly domain knowledge we do not have** — musl cross-toolchains, the WPA
  four-way handshake, device-tree overlays.
* **Anywhere being subtly wrong is expensive**, which is most of the above.

**Where reuse quietly costs more than it saves:**

* **Impedance mismatch.** The crate does eighty per cent of what is wanted and
  the remaining twenty per cent is fought forever. This is the most common way
  reuse loses, and it is invisible at adoption time.
* **Wrong altitude.** A crate written for an application, used in PID 1: it
  assumes an async runtime, a global logger, an allocator that never fails, or
  it panics where an operating system must not. For Rivet this is a live risk
  rather than a theoretical one.
* **Transitive weight.** A small crate that pulls a large runtime behind it.
* **Comprehension cost.** Reading a dependency closely enough to trust it in
  PID 1 can cost as much as writing the thing would have — and leaves less
  understanding of our own system than writing it would have.

### 3.4 The asymmetry that actually decides it

The cost of a dependency is paid **at its boundary**, not in its middle.

A crate whose types never surface is nearly free and is cheap to remove later.
A crate whose types reach Rivet's configuration schema, error model or control
flow is expensive for ever, and removing it is a redesign.

So the operative question is never "should this be reused?" but:

> **Can I put a wall around it?**

| Can be walled cleanly | Resists being walled |
| :-- | :-- |
| protocol implementations, parsers, formats | anything wanting to own the process lifecycle |
| algorithms, codecs, cryptography | anything with opinions about the error model |
| platform bindings (netlink, ioctl) | anything with opinions about configuration or layering |
| self-contained programs driven over a socket or pipe | async runtimes, global state, frameworks |

Take everything in the left column, freely. Treat the right column as a design
decision rather than a dependency choice.

### 3.5 What is not reinvention

The worry behind the maintainer's note deserves a direct answer, because the
ladder can look like it forbids writing anything.

**Writing `rivet-initd` is not reinventing systemd. Writing the configuration
model is not reinventing `/etc`.** Those are the project. Reusing them would not
save time; it would produce somebody else's operating system with a new name.

The test is section 3.1's: *does this make Rivet Rivet?* Rung 3 is narrow by
design, and everything outside it is genuinely someone else's problem to have
solved.

It is also worth noticing how little is left. The highest-effort work in this
project is almost entirely on rungs 1 and 2 — cross-compiling the C mechanisms
against musl, the WPA handshake, three hundred utilities, schema
generation, netlink, DHCP. Reuse-first removes nearly all of the available waste
before the first line is written, and what remains to be written is the part
worth writing.

---

## 3.6 A worked survey, and a gate this ADR was missing — 2026-08-23

Added when Stage 1's netlink adapter was written. It is kept here rather than
only in the crate's source because it changes §5's gate list, and because it is
the first time this ADR's ladder was walked all the way down to rung 3 against
an example the ladder itself named.

**The question:** which crate does `rivet-adapter-net` use for rtnetlink?
**The answer taken:** none. The full reading, with the numbers, is in
`rivet-adapter-net/src/lib.rs`'s module documentation; the condensed form:

| Candidate | Cost | Verdict |
| :-- | :-- | :-- |
| `rtnetlink` 0.23 | `new_connection()` **does not exist** without `tokio_socket` or `smol_socket`, verified by compiling it; `futures-util`/`futures-channel` non-optional regardless | rejected — `ADR-021` §4.1's precedent exactly |
| `neli` 0.7 | 33 crates, `parking_lot`, and `serde` via `neli-proc-macros` — which `rivet-config` went to some length to exclude | rejected |
| `netlink-packet-route` + `netlink-sys` | 13 crates, sync, MIT, no runtime | **the close call** — rejected on §3.6.1 |
| Written here | ~4 requests, 2 reply shapes, fixed-layout structs and 4-byte TLVs | taken |

### 3.6.1 The gate this ADR did not have: it must build on the development host

§5 lists cross-compilation as a gate — *"every dependency must build for
`aarch64-unknown-linux-musl`; a crate that does not is not a candidate, however
good it is."* All three candidates pass that. **The gate that decided it is the
one §5 does not name:**

> **A dependency that does not build for the *development host* makes its
> consumer's test suite unrunnable on the machine the code is written on.**

Measured 2026-08-23 on `x86_64-pc-windows-msvc`: `netlink-packet-route` 0.33
fails to compile with twelve errors (`cannot find value AF_INET in crate libc`
and siblings, in `address_family_fallback.rs`); `neli` 0.7 fails with 485.
`AGENTS.md` §10.5 records that Windows is a development host, and `AGENTS.md`
§4 records the consequence — *"a test suite that cannot be run will rot
silently."*

This gate is **weaker than the cross-compilation one and is not a veto.** It
loses to a dependency that carries enough value, and it does not apply at all to
a crate whose consumer is inherently platform-bound: `cmd/rivet-initd` and
`cmd/rivet-sysd` are Linux-only by nature, because a syscall port has to
name syscalls, and no dependency choice would change that. It applied here
because **netlink is bytes** — the wire format, the decisions and every failure
path are platform-independent if they are written to be, and taking the
dependency would have spent that property to save the smaller half of the work.

### 3.6.2 What would overturn it, so the decision is revisited rather than inherited

The verdict is scoped to Stage 1's four requests and does **not** generalise.
Two things already in this record would flip it, and both are parsing rather
than encoding — which is where §3.3's *"worse in ways that surface as
intermittent failures years later"* actually bites:

* **Decoding messages Rivet did not construct** — `RTMGRP_LINK` broadcasts for
  carrier changes, or reading back what a DHCP client did. That is the whole
  `IFLA_*`/`IFA_*` attribute space.
* **nftables** (`ADR-023` §9, §10 item 1), which is an order of magnitude more
  wire surface than anything Stage 1 sends.

The wall §3.4 asks for is already in the right place for that: no netlink type
reaches `rivet-core` or the configuration schema, so swapping the
implementation is a change to three modules rather than a redesign.

---

---

## 3.7 A second worked survey: the TLS stack — 2026-08-28

`ADR-035` §7.2 ruled the **wire format** — a self-signed certificate pinned by
public key — and deliberately did not name an implementation. This is that
decision, measured rather than argued, on §3.6's pattern.

### 3.7.1 What the candidate has to do

Four requirements, and the fourth is the one that decided it:

1. **Cross-compile to `aarch64-unknown-linux-musl`** — §5's hard gate.
2. **Build on the development host**, `x86_64-pc-windows-msvc` — §3.6.1's
   weaker gate. It applies here: `cmd/rivet-mcpd` is ordinary portable code
   (a socket and a spawned process), so it is not the platform-bound consumer
   §3.6.1 exempts.
3. **Sign and verify Ed25519**, because `ADR-025` §2 pins Ed25519 keys and the
   certificate has to carry the key that is pinned.
4. **Run on a Cortex-A72** — the Raspberry Pi 4's core, and the only CPU Rivet
   has ever run on.

### 3.7.2 The measurements

| Candidate | cross-compiles | dev host | Ed25519 sign | runs on the A72 | C toolchain |
| :-- | :-- | :-- | :-- | :-- | :-- |
| `rustls` + `graviola` 0.4.1 | yes, **no C** | yes | yes | **NO** | none |
| `rustls` + `ring` 0.17.14 | yes, **needs a cross C compiler** | yes | yes | **yes** | required |
| `rustls-rustcrypto` | not tested | — | — | — | none |
| `rustls` + `aws-lc-rs` 1.18 | not tested | — | — | — | C **and** cmake |

**`rustls-rustcrypto` is version `0.0.2-alpha`.** That is the whole entry. A
pre-alpha crypto provider is not a candidate for an operating system's trusted
path, and no other property could compensate.

**`aws-lc-rs` was not measured**, and this record says so rather than implying
a comparison that did not happen. It was not pursued because it is strictly
heavier on the axis that decided the survey — it needs a C toolchain *and*
cmake, where `ring` needs only the first — so the only way it could have won is
by beating `ring` on an axis `ring` already passes. That is a judgement, not a
measurement, and it is the one place in this section where the two differ.

### 3.7.3 `graviola` compiled perfectly and cannot run on the hardware

It is the better dependency on every axis this project usually cares about:
pure Rust with no C at all, twelve crates, and written by `rustls`'s own
author. It cross-compiled in 4.4 seconds with no toolchain beyond `rust-lld`,
built on Windows, and loaded an Ed25519 private key.

**Its aarch64 support requires the ARMv8 crypto extensions** — `aes`, `sha2`,
`pmull` and `neon` — and its own README says the requirement *"notably excludes
Raspberry Pi 4 and earlier models"*. The board agrees, and the board is the
authority:

```
$ open --raw /proc/cpuinfo   # asked of the board
Features    : fp asimd evtstrm crc32 cpuid
CPU part    : 0xd08
```

`0xd08` is Cortex-A72. No `aes`, no `sha2`, no `pmull`.

**This is the finding worth keeping from the whole survey.** A dependency that
compiles for a target is not a dependency that runs on a machine, and every
gate this ADR had before today — §5's cross-compilation, §3.6.1's development
host — is a *compilation* gate. Both would have passed `graviola` cleanly, and
the failure would have arrived as a `rivet-mcpd` that died at startup on a
headless board in another room. `AGENTS.md`'s "until it runs, it isn't working"
is normally about features; here it is about a dependency, and §5 gains a gate
because of it:

> **A dependency that ships CPU-specific code must be RUN on the target CPU
> before it is adopted, not merely compiled for it.** The card's own hardware
> is the authority, and `/proc/cpuinfo` is where it answers.

### 3.7.4 What `ring` costs, stated plainly

**It needs a C cross-compiler, and Rivet did not have one in this path.**
`.cargo/config.toml` said so in as many words — *"Rivet's own binaries
therefore need no C cross-toolchain at all; Buildroot is required only for the
C mechanisms (`ADR-009` §6)"* — and adopting `ring` makes that false. Without
one the build fails at `cc-rs`:

```
error occurred in cc-rs: failed to find tool "aarch64-linux-musl-gcc":
program not found
```

The compiler already exists. `tools/card/build.sh` runs the cross build **in
WSL**, where Buildroot's `aarch64-linux-gcc` has been sitting since Stage 0, so
what this costs is one environment variable in one script rather than a new
tool anybody has to install. That is a smaller cost than it first looks — but
it is a real one, and it moves `ring` from *"no C anywhere in Rivet's own
binaries"* to *"one C dependency, in the newest and most security-sensitive
binary"*.

**It is eleven crates** — `rustls`, `ring`, `rustls-webpki`,
`rustls-pki-types`, `once_cell`, `untrusted`, `subtle`, `zeroize`, `getrandom`,
`cfg-if`, `libc`. `getrandom` is admitted by `ADR-001` §7.2 and is the correct
answer rather than a concession: it asks the kernel, which is exactly where
§7.3 says Rivet's entropy comes from.

### 3.7.5 The proof

A static `aarch64-unknown-linux-musl` binary built with the Buildroot compiler,
pushed to the board and run there — no emulation, because an emulator would
have modelled a CPU the project does not own and could have reported a pass
`graviola` did not deserve:

```
$ /state/probe/tlsprobe   # run on the board
provider ok: 9 cipher suites
ed25519 self-signed leaf: 285 bytes
HANDSHAKE OK
  suite:   TLS13_AES_256_GCM_SHA384
  version: TLSv1_3
  payload: "hello from the client"
ALL OK
```

A complete TLS 1.3 handshake between a server holding an **Ed25519 self-signed
leaf** and a client that trusted it **by pinning the certificate and refusing
any chain** — which is `ADR-035` §7.2's design, end to end, on the hardware.
The same binary produces the same output on the Windows host.

**Adopted: `rustls` with the `ring` provider.**

## 3.8 A third worked survey: the `card` image's zstd codec — 2026-09-13

`R-177.1` ruled zstd over xz on `ADR-029` §6.1's own numbers, measured again
for a 2 GiB card rather than a ~700 MB update payload — much faster to
compress and decompress, within a few percent of xz's size. This is the
crate choice underneath that ruling, on §3.6's pattern.

### 3.8.1 What the candidate has to do

Unlike §3.6 and §3.7's ports and cryptography, this dependency is
**workstation-only by construction**: `ADR-029` §13/R-107 keeps every
board-installed kind uncompressed on purpose, so the codec that compresses a
`card` image runs in `cmd/cargo-rivet` alone and never crosses into
`rivet-core` or any board binary. That removes §5's hard gate — cross-compiling
to `aarch64-unknown-linux-musl` — from consideration entirely: nothing here is
ever built for that target, which `cargo tree --target aarch64-unknown-linux-musl
-p rivetos-sysd | grep zstd` (and the same for every other board binary)
confirms finds nothing, the same check this ADR already runs for `getrandom`.

What is left is ordinary: it must build on the development host
(`x86_64-pc-windows-msvc`, §3.6.1) and it must actually compress and
decompress correctly at the scale a card image is — 2 GiB.

### 3.8.2 The choice

| Candidate | Cost | Verdict |
| :-- | :-- | :-- |
| `zstd` (the `zstd`/`zstd-safe`/`zstd-sys` family, wrapping Facebook's C
  library) | needs a C toolchain via `cc`, which this host already has —
  `rcgen`'s `ring` feature already exercises it (`ADR-009` §3.7.4) | **taken** |
| A pure-Rust decoder only (`ruzstd` and similar) | decode-only; this tool
  also has to *encode* | rejected — does not do the job |

**Taken: `zstd` 0.13, `default-features = false`.** The multi-thread
(`zstdmt`) feature is dropped: a card publish happens once per release, not on
a hot path, and single-threaded compression of a 2 GiB image measured under
two seconds wall-clock in the proof this ruling shipped with — the axis
`R-177.1` was measured on is transfer and device-side decompression time, not
workstation compression time, and that axis was already won.

### 3.8.3 The proof

Run against a real, `tools/container/build-card.sh`-built 2 GiB card image,
`cargo rivet image` (`cmd/cargo-rivet/src/image.rs`) compressed it to roughly
5% of its plain size, wrote the signed `.rvt`, and published it into a scratch
fleet repository — all in under two seconds wall-clock and under 15 MiB of
peak process memory, because the encoder streams a `CHUNK`-sized buffer at a
time rather than holding the 2 GiB input or its compressed copy resident. The
same image was then materialised back through the read side
(`cmd/cargo-rivet/src/fleet.rs`'s `materialise_image`, streaming the decoder the same way)
and its SHA-256 matched the original byte for byte. See
`cmd/cargo-rivet/src/image_tests.rs` for the same round trip at unit-test
scale, with real zstd and real Ed25519/BLAKE3 throughout.

**Adopted: `zstd` 0.13, workstation-only, `cmd/cargo-rivet`'s own
dependency.**

## 3.9 A fourth worked survey: driving the interview under a real pty — 2026-09-14

`R-178` P6 rules the interactive interview (`cmd/rivet-provision`) proven by a
Rust test harness that types into the real menu and checks what it prints —
not a piped-stdin fake. A piped `Stdin` is not a terminal: `write_image.rs`'s
own `refuse_unless_terminal` (`std::io::IsTerminal`) would see it as none, and
the menu code a real operator drives — line editing, an ANSI cursor, the
`bare Return takes the default` behaviour this project relies on throughout —
is never exercised by feeding bytes into a pipe.

### 3.9.1 What the candidate has to do

Open a REAL pseudo-terminal, spawn `cmd/rivet-provision` attached to it (not
piped), write keystrokes to the master side, and read back exactly what a
person watching a terminal would see — on **Windows**, `SPEC-05`'s own
platform (`ADR-009` §3.6.1's host-buildability gate, but sharper: this
crate's whole job IS the host, `x86_64-pc-windows-msvc`, so unlike §3.6–§3.8
there is no board-side alternative to fall back to if it does not build
there).

### 3.9.2 The choice

| Candidate | Cost | Verdict |
| :-- | :-- | :-- |
| `portable-pty` (part of the `wezterm` project) | one dev-only dependency tree (`nix`, `winreg`, `serial2` — Unix/Windows-conditional, `default-features = false` drops its own `serde` support) | **taken** |
| Hand-rolled `CreatePseudoConsole`/`openpty` FFI, one branch per platform | this project's own C-free posture (`ADR-003`) does not bind `unsafe extern` FFI written for a test harness — every existing `unsafe` boundary in this workspace wraps a *shipped* mechanism, never a dev-only one — and two platforms' worth of pty plumbing is exactly the "solved problem someone else maintains" §3.1 describes | rejected — rung 1 applies, not rung 3 |
| A piped `std::process::Stdio` (no pty at all) | does not do the job — `IsTerminal` sees a pipe, and P6's own point is that a fake stdin proves nothing about the menu a terminal exercises | rejected — fails the requirement, not a cost trade |

**Taken: `portable-pty` 0.9.0, `default-features = false`, dev-only.** It is
dev-only for the reason zstd (§3.8) is workstation-only: nothing under
`cmd/cargo-rivet/tests`/`src/*_tests.rs` is ever built for
`aarch64-unknown-linux-musl`, so §5's cross-compilation gate does not apply —
`cargo tree --target aarch64-unknown-linux-musl -p rivetos-sysd | grep -i pty`
finds nothing, the same check this ADR already runs for `getrandom` and
`zstd`.

### 3.9.3 The proof

`cmd/cargo-rivet/src/provision_tests.rs`'s pty harness (R-178 P6) — see that
file for what it actually drove and what it asserted, and this ADR's own
posture: build once, prove on the platform that matters, name what is
unproven elsewhere rather than claiming it.

**Adopted: `portable-pty` 0.9.0, dev-only, `cmd/cargo-rivet`'s own
dependency.**

## 3.10 A fifth worked survey: the seed-backup passphrase envelope — 2026-09-14

`.rulings/` R-183 D3 rules `cargo rivet key backup`'s seed backup
passphrase-encrypted, and R-185 D1 rules `cargo rivet backup-secrets` into the
same format and typed prompt. This is the crate choice underneath both, on
§3.6's pattern: a key-stretching function and an authenticated cipher.

**One envelope, in one module**: `cmd/cargo-rivet/src/envelope.rs` holds the
byte layout, the Argon2id and XChaCha20-Poly1305 calls, the no-echo prompt and
the forgotten-passphrase sentence. A caller chooses only the magic — `RVTSEED`
for a seed backup (`key_backup.rs`), `RVTSECR` for a board's secrets
(`machine_secrets.rs`). The layout is the one the seed backup wrote from the
start; it moved out of `key_backup.rs` unchanged, and a test opens an envelope
the earlier code wrote.

### 3.10.1 What the candidates have to do

**Workstation-only, exactly as zstd (§3.8) and `portable-pty` (§3.9) are** —
`cmd/cargo-rivet` is a host tool that never ships on a card, so §5's
cross-compilation gate does not apply; `cargo tree --target
aarch64-unknown-linux-musl -p rivetos-sysd` finds neither dependency, the
same check this ADR already runs for the others. What is left: build on the
development host (`x86_64-pc-windows-msvc`, §3.6.1), and be a **widely
reviewed** implementation rather than a bespoke one — §3.3's rule that
cryptography is the case where a bespoke version is worse, applied here for
the second time (§3.7's TLS survey is the first).

### 3.10.2 Key stretching: Argon2id

| Candidate | Cost | Verdict |
| :-- | :-- | :-- |
| `argon2` (RustCrypto) | pure Rust, no C, already the family `pbkdf2`/`hmac`/`sha1` (`ADR-004` §5, WPA2's own key derivation) come from — no new *family* of dependency, only a new member of one already trusted here | **taken** |
| Hand-rolled PBKDF2 (reusing the `pbkdf2` crate already in the graph) | RFC 9106 argues PBKDF2's lack of a memory-hardness parameter is a real weakness against a GPU/ASIC attacker for a secret this valuable — using it here would be reaching for the weaker tool this project already happens to have, not the right one | rejected |

**Taken: `argon2` 0.5, `default-features = false, features = ["alloc"]`.**
RFC 9106 §4's second recommended parameter set (m=64 MiB, t=3, p=4 — chosen
over the first, 2 GiB, because this runs once by hand on whatever workstation
the operator is holding, and 64 MiB costs nothing there either way).

### 3.10.3 The authenticated cipher: XChaCha20-Poly1305

| Candidate | Cost | Verdict |
| :-- | :-- | :-- |
| `chacha20poly1305` (RustCrypto), `XChaCha20Poly1305` | pure Rust, no C; **192-bit nonce** removes any birthday-bound concern from a nonce this tool mints fresh (`getrandom`) on every backup rather than manages a counter for | **taken** |
| `aes-gcm` (RustCrypto) | AES-GCM's 96-bit nonce is the sharper tool for a protocol that can guarantee a counter never repeats; this is a one-shot file format with no session to count within, so a birthday-bound nonce space is a hazard this format has no mechanism to rule out the way a protocol would | rejected — right tool, wrong shape for a file format |

**Taken: `chacha20poly1305` 0.10, `default-features = false, features =
["alloc"]`**, used as `XChaCha20Poly1305`.

### 3.10.4 The passphrase prompt: `rpassword`

Not cryptography, but surveyed the same way: the smallest widely used crate
that reads a terminal's no-echo password mode on both Windows (`ReadConsoleW`)
and Unix (`termios`) without pulling a TUI framework for one prompt. No
serious alternative was found smaller — the two platform code paths are
exactly the kind of "solved problem someone else maintains" §3.1 describes,
and `portable-pty`'s own precedent (§3.9) already established that this
project does not hand-roll a platform console API for a workstation-only
tool. **Taken: `rpassword` 7.**

### 3.10.5 The proof

`cmd/cargo-rivet/src/envelope.rs`'s and `key_backup.rs`'s own unit tests: an
envelope written before the hoist opens, and this build writes the same header;
a real seed and plaintexts of several lengths round-trip; two encryptions of the
same bytes differ (fresh salt and nonce, checked rather than assumed); a wrong
passphrase and a single flipped ciphertext byte are refused identically (the
AEAD's own security property); a flipped byte of the (authenticated,
unencrypted) salt is refused too, proving the associated-data binding actually
holds; another kind's magic, trailing bytes, and every truncation length of a
real envelope are refused as a format error, never a panic.
`cmd/cargo-rivet/tests/pty_key_backup.rs` — the real-terminal half,
`portable-pty` again — typed a real passphrase twice through a real ConPTY,
backed a fixture seed up, restored it into a fresh `HOME`, and got the
identical 32 bytes back; a second run typed the wrong passphrase at restore
and proved nothing was written. `tests/pty_secrets_backup.rs` does the same for
a board's secrets against a TLS server standing in for the board. Those run on
Windows; the Unix prompt has run under `script(1)` in the Linux build container,
through `tools/qemu/adr-030-secrets-proof.sh`.

**Adopted: `argon2` 0.5, `chacha20poly1305` 0.10 and `rpassword` 7, all
workstation-only, `cmd/cargo-rivet`'s own dependencies.**

---

## 4. Reuse implementations, never interfaces

This is the rule that lets reuse be aggressive without the system becoming a
crate salad, and it is `ADR-001` section 5 extended from C mechanisms to Rust
dependencies.

> **A dependency's types must not appear in Rivet's own surface.** Not in the
> configuration schema, not in a CLI output structure, not in a public error
> variant, not in a `rivet-core` signature.

The test is the same as the one in `ADR-001` section 2: if this crate were
swapped tomorrow, how much would have to change? If a crate's error enum is
serialised into Rivet's structured output, the answer is "the interface
contract", and that is a defect rather than a convenience.

This is also what preserves the answer to the `ADR-006` question about feeling
designed. A system assembled from twenty crates still feels designed if the
seams are Rivet's own types. It feels like a pile if the crates' vocabularies
reach the surface — which is exactly how `/etc` happened, one library's file
format at a time.

---

## 5. Costs that reuse-first creates, and the gates for them

Naming these because reuse-first has real failure modes and "prefer reuse" on
its own does not address any of them.

* **Dependency weight.** An operating system that pulls four hundred transitive
  crates has a real size and audit surface. **Gate:** image size is already a
  tracked number (`ADR-002` section 5.1); `cargo tree` and `cargo bloat` make
  the cause attributable.
* **Supply chain.** More dependencies is more trust. **Gate:** `cargo-deny` in
  CI for advisories, and a license allowlist — see `ADR-010`.
* **Cross-compilation.** Every dependency must build for
  `aarch64-unknown-linux-musl` (`ADR-003`). A crate that does not is not a
  candidate, however good it is. This must be checked before adoption, not
  discovered during it.
* **Host-buildability — added 2026-08-23, §3.6.1.** Whether it builds for the
  *development host* as well. Not a veto, unlike the gate above: it is a cost to
  weigh, and it is only a cost at all where the consuming crate could otherwise
  have been platform-independent. Check it in the same pass as the one above,
  because it costs one extra `cargo check`.
* **Abandonment.** A reused project can stop being maintained. The mitigation is
  section 4 — if the seams are Rivet's types, replacing an abandoned
  dependency is a rewrite of one adapter, not of the system.

---

## 6. The largest open reuse question: image building

Every C mechanism in the image must be built statically against musl for
aarch64 (`ADR-003` section 4). That is a genuinely fiddly cross-build problem,
it is **not** the interesting part of Rivet, and it is exactly the kind of
solved problem this ADR says to reuse.

| Option | For | Against |
| :-- | :-- | :-- |
| Hand-rolled build scripts | full control; nothing to learn but our own | re-solves a solved problem; fragile; the fiddliest work in the project |
| **Buildroot** | designed for precisely this; existing Raspberry Pi configurations; handles the musl cross-toolchain | large and opinionated; risk that Rivet becomes "a Buildroot configuration" |
| Yocto | more capable | far heavier than this project warrants |

**Ruled 2026-08-22: Buildroot, mechanisms and toolchain only.** Rivet's own
image assembly — the layout of `ADR-008`, the generated `/etc`, the A/B
structure — stays in Rivet's own Rust tooling. That draws the boundary where
section 3.1 draws it: reuse the solved cross-build problem, own the thing that
makes Rivet distinct.

The boundary is what prevents the failure the "against" column named. Buildroot
produces a bag of correctly-built binaries and a toolchain; **it does not decide
what the filesystem looks like.** If Buildroot's own root filesystem conventions
ever reach the image, the boundary has been crossed — those conventions are the
FHS, which `ADR-008` §3 rejects.

It also supplies the GPL source-archive machinery that `ADR-010` §6.1 says the
build must eventually have, which would otherwise be written by hand.

**CONFIRMED 2026-08-22, and it is a one-line change.** Buildroot's
`package/musl/Config.in` sets
`BR2_PACKAGE_MUSL_ARCH_SUPPORTS` to `default y if BR2_aarch64`, so the internal
toolchain can build musl for this architecture. Better, an external option
exists — `BR2_TOOLCHAIN_EXTERNAL_BOOTLIN_AARCH64_MUSL_STABLE` — so no toolchain
needs building at all.

**The catch is in the shipped configuration, not in the capability.**
`configs/raspberrypi4_64_defconfig` sets `BR2_aarch64=y` and `BR2_cortex_a72=y`
(the right target) but selects
`BR2_TOOLCHAIN_EXTERNAL_BOOTLIN_AARCH64_GLIBC_STABLE` — **glibc**. Rivet's
derived configuration therefore has to swap that one symbol for the `_MUSL_`
variant. Taking the Pi 4 defconfig unmodified would silently produce a
glibc image, which `ADR-003` forbids. **Unproven:** that the resulting
toolchain actually builds Rivet's C mechanisms — that is the build itself.

The original expectation was correct on capability and wrong on convenience:
it is supported, but not by the configuration Buildroot ships for this board.

Buildroot runs on Linux, so this lives in WSL2 — as the hand-rolled option would
have as well.

### 6.1 Where "configuration, never build logic" actually falls — 2026-08-24

§6 says Rivet contributes configuration to Buildroot and never build logic, and
`tools/buildroot/external.mk` carries no package recipes for that reason. The
first change that made the line worth drawing precisely was a
**compile-time constant inside a C mechanism** — the lookup path a session gets.

**The test that was applied**, and which should be applied next time:

> Is this a **value handed to somebody else's recipe**, through a mechanism that
> recipe already provides for it? Or is it a recipe of our own?

Two kconfig symbols carried it, and both were Buildroot's own: the Buildroot-wide
`BR2_SYSTEM_DEFAULT_PATH` (`"/system/bin"` in
`tools/buildroot/configs/rivet_aarch64_defconfig`), and a per-package symbol
whose entire purpose is to name a file Buildroot appends to that package's own
`localoptions.h`. Contributing through both is on the configuration side of the
line, by the same test the kernel fragments in `tools/buildroot/linux/` pass.

**What would have crossed it:** a package `.mk` override, a `*.patch` under a
`BR2_GLOBAL_PATCH_DIR`, or a Rivet package wrapping the upstream one. Each is a
recipe, each would have had to track the upstream version, and each would have
put Rivet's build logic inside Buildroot — the failure §6's "against" column
named.

**A consequence worth stating, because it is the reason the boundary holds
here.** A configuration symbol can be set correctly and mean nothing. In that
case Buildroot's symbol reached the program's `DEFAULT_PATH` and **not** the
separate constant it used for `root`, for which Buildroot had no symbol at all.
A build that set only the symbol would succeed, produce a `.config` that reads
correctly, and change nothing on the machine. **Configuration is not verified by
the build**, so anything contributed this way needs an assertion on a running
system. Contributing *only* configuration does not make a contribution safe; it
moves where the proof has to come from.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §3 — `rtnetlink` was a rung-1 example

**Changed 2026-08-23**, when it was surveyed and rejected; §3.6 is the
reasoning. The superseded entry in the ladder's rung-1 row was ~~`rtnetlink`~~.
The point the example was making is still right and is in the body: netlink is
the kind of problem rung 1 exists for.

### In §6 — Buildroot's musl support was unverified

**Changed 2026-08-22**, when `package/musl/Config.in` was read. The superseded
phrase was ~~Unverified and worth checking first~~.
