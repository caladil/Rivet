# ADR-005: Userland Composition

* **Status:** Accepted
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`ADR-001` fixed the posture and the tier policy. This ADR applies it to the
concrete set of components that must exist for a Rivet host to boot and be
usable, and records what each one is, what tier it sits in, and what replaces it
if anything does.

The organising question throughout: **what does the kernel actually hand us, and
what must we therefore supply?**

---

## 2. What the kernel provides, and what it does not

The kernel mounts the root filesystem and executes exactly one binary as PID 1.
Then it stops helping.

| Provided | Not provided |
| :-- | :-- |
| `/` mounted, per `root=` | `/proc`, `/sys`, `/run`, `/dev/pts`, `/dev/shm` |
| `/dev` populated, if `CONFIG_DEVTMPFS_MOUNT=y` | any process supervision |
| direct firmware loading from `/lib/firmware` | zombie reaping |
| | any expectation that PID 1 may exit |

### 2.1 There is no udev, and this is deliberate

With `CONFIG_DEVTMPFS_MOUNT=y` the kernel populates `/dev` itself before init
runs. udev exists for persistent naming, permission rules, and firmware loading
on kernels old enough to have needed a userspace helper; modern kernels load
firmware directly from `/lib/firmware` in-kernel.

**Rivet ships no device manager.** This removes an entire component and its
rule language from the image. If persistent naming is later required, it is a
new ADR, not a quiet reintroduction.

### 2.2 PID 1's non-negotiable contract

* Every orphaned process on the system re-parents to PID 1. Without a
  `waitpid` loop the process table fills with zombies.
* PID 1 returning causes a kernel panic. This is the enforcement mechanism for
  the fact that PID 1 *is* the operating system.

`rivet-initd` therefore must, at minimum: mount the pseudo-filesystems, handle
`SIGCHLD` and `SIGTERM`, reap indefinitely, and never return.

**Amended 2026-08-22:** it does *not* supervise services. `ADR-017` §3.1 ruled
PID 1 minimal, spawning `rivet-sysd` as its single child. `ADR-017`
§3.1.5 is the complete list of what PID 1 does.

---

## 3. The component table

| Component | Choice at adoption | Tier | Replacement intent |
| :-- | :-- | :-- | :-- |
| PID 1 | `rivet-initd` | A | — it is ours from the first commit |
| Device nodes | devtmpfs | — | kernel-provided; no component |
| Service supervision | `rivet-sysd`, spawned by PID 1 — `ADR-017` §3.1 | A | — |
| Link, address, route | `rtnetlink` from Rust | A | — |
| DHCP client | BusyBox `udhcpc` | **B** | Rust client; `dhcproto` for wire format |
| mDNS responder | Rust — **`cmd/rivet-mdnsd`**, a responder written over the **`simple-dns`** wire format (`ADR-017` §3.7.1) | A | — |
| WPA association | `wpa_supplicant` | **C** | none intended — `ADR-004` section 3 |
| coreutils | `uutils/coreutils` | A | — already Rust |
| findutils, diffutils | `uutils` equivalents | A | — already Rust |
| `grep` / `sed` / `awk` | BusyBox applets | **B** | the least-served category |
| Interactive shell | see section 5 | **B** | — |
| Kernel module loading | BusyBox / kmod | **B** | Rust, or avoided by building in |
| libc | musl, static | **C** | none intended — `ADR-003` |

**Superseded in part, 2026-08-22 — see section 5.4.** The `grep`/`sed`/`awk`
and interactive shell rows are overtaken by the ruling that no POSIX shell ships
in the base image. The rows are left readable because the reasoning that
produced them is what the ruling had to answer.

---

## 4. coreutils is already solved

> **Superseded 2026-08-24 — `uutils` is not adopted. See §4.1.** This section
> was written before §5.1 ruled nushell in as the login shell, and it reads as
> though the two decisions were independent. They are not: a shell with
> builtins named `ls`, `cp`, `mv`, `rm` and `mkdir` shadows a coreutils binary
> of the same name, so adopting `uutils` would ship ~100 binaries to be
> shadowed by the shell that was ruled in afterwards. The section is left
> readable because §4.1's answer is shaped by the argument here.

`uutils/coreutils` is adopted directly and is not a "working towards" item. It
is a mature reimplementation with a high GNU test-suite pass rate, it
cross-compiles to `aarch64-unknown-linux-musl`, and it has been shipped as the
default coreutils in a mainstream distribution release. `findutils` and
`diffutils` from the same project are less battle-tested but adequate.

The genuinely unserved category is `grep`, `sed` and `awk`. `ripgrep` is better
than `grep` but is not `grep`, and there is no POSIX-complete `awk` in Rust worth
depending on. **This is where BusyBox will linger longest**, and pretending
otherwise in the scoreboard would be dishonest.

---

## 4.1 Ruled 2026-08-24: read `uutils`, do not link it — and there is no text-output tier

The maintainer — thirty years of Unix behind him — asked whether the familiar
commands existed. Shown the recommendation to decline `uutils`, he asked back
whether there was **anything in it worth porting**, and then, given the survey
below, made the argument that decides this section: **Rivet does not actually
want the `uutils` version of these commands, because what it needs is something
that works with nushell's structured output. The `uutils` code is worth reading
to learn how a thing is done; what ships has to be wrapped in an I/O surface
that suits nushell better.** *(`.rulings/` R-010.)*

**Ruled: take the algorithms, with attribution. Take no dependency.**

### 4.1.1 The argument, which is stronger than the one §4 makes

§4 argues about `uutils`' *quality*, and by that measure it wins — it is mature,
it cross-compiles, it has shipped as a mainstream distribution's default. That
was the wrong axis. The question is what **shape** an answer has to have here.

**Rivet has two surfaces, and deliberately no third:**

| Surface | Shape |
| :-- | :-- |
| `rivet <verb> <subject>` | structured by default, catalogued (`ADR-006`), reachable from the CLI *and* the network API *and* a dashboard, from one implementation |
| nushell builtins | structured in, structured out, pipeline-shaped |

**Nothing in the userland emits text that another program has to parse**, and
that is a property this project built on purpose rather than one that happened
to survive: §5.4 removed the POSIX shell, and `ADR-006` made every
administrative action an operation. A coreutils binary would be the first thing
in the userland whose output is a parsing problem — `nu` would have to
`detect columns` its way back to structure that existed inside the tool and was
thrown away at its exit.

**The one place text-output foreign binaries are accepted is §7's debugging
tier** — `gdbserver`, `strace`. Those are quarantined on purpose: they are
driven by hand, in an emergency, and nothing consumes their output
programmatically. That exception is what makes the rule legible rather than
absolute.

**And the "wrap it in a better I/O surface" option does not exist**, which is
worth recording because it is the obvious next idea. `uutils`' per-utility
crates (`uu_chmod`, `uu_df`, …) expose essentially `uumain(args) -> i32`: argv
in, bytes to stdout, integer out. Wrapping that yields a subprocess being parsed
in-process — the same problem with more steps.

### 4.1.2 The survey: what `nu` actually lacks

**Asked of a running machine rather than of a package list** (`AGENTS.md` §7 — a
claim of absence needs re-checking as hard as a claim of presence). Under QEMU,
over the control plane, each name checked individually against `help commands`:

| | |
| :-- | :-- |
| **`nu` has none of** | `chmod`, `chown`, `ln`, `df`, `dd`, `truncate`, `mknod`, `sync`, `stat`, `readlink` |
| **`nu` already has** | `date`, `join`, `kill`, `seq`, `sort`, `split`, `tee`, `uniq`; and `ls`, `cp`, `mv`, `rm`, `mkdir`, `cd`, `du`, `open`, `save`, `touch`, `glob`, `mktemp` as builtins |
| **and** | `sys disks` returned `[]`, so there is no second route to a filesystem's free space |

### 4.1.3 Where each gap lands, and the line that decides it

The tempting answer is "put it in the catalogue" twice, and it is right once.
**The line is desired state:**

* **`rivet` operations model desired state** — something to `set`, `plan`,
  `export`, converge toward — with `get` additionally serving facts (which is
  what `get log` is).
* **A file manipulation with no desired state behind it is a shell act**, not an
  operation. `rivet set mode /some/path 0600` would be a `set` with nothing to
  plan, nothing to export and nothing to reconcile: it would break `ADR-006`'s
  model rather than extend it.

| Gap | Lands | Why |
| :-- | :-- | :-- |
| **`df`** | **a Rivet operation** — a `get` subject beside `log` | free space is a **fact**, and the catalogue is already the structured surface: one implementation serves the nu table, the network API and a dashboard. A `df` binary needs a parser per consumer. **It matters now**: `/state` does not grow to fill its partition (`ADR-018` §10, unbuilt) and `ADR-022` §8 caps the journal at 8 MiB against a card whose free space nothing on the machine can measure |
| **`chmod`, `chown`, `ln`** | **a Rivet operation** — `apply set-mode` and `apply set-owner` | imperative acts on files, no desired state. They are the surface rather than a shell builtin because the surface is what an operator reaches when something is already wrong, and `ADR-006` §8.10 is where the refusals live |
| **`sync`** | noted, not urgent | `ADR-022` §8 never `fsync`s and nothing can force a flush — but `rivet apply power-off` syncs through PID 1, so this bites only an operator pulling power *without* using the surface, which is what the surface exists to prevent |
| **`dd`, `mknod`, `truncate`** | **refused** | raw block writes onto an A/B slot are the installer's job, in Rust, behind a verified payload (`ADR-029`; the maintainer's 2026-08-24 ruling). An operator doing it by hand is the failure mode the update design exists to prevent, and shipping `dd` puts it one typo away |

### 4.1.4 What is worth reading in `uucore`, module by module

Better than expected: four modules are genuine structured APIs — no printing, no
argv, values returned rather than exit codes.

| Module | Public surface | Answers |
| :-- | :-- | :-- |
| `mode` | `parse(mode_string, considering_dir, umask) -> Result<u32, String>`, `parse_symbolic`, `parse_numeric`, `parse_chmod`, `get_umask` | `chmod` |
| `fsext` | `MountInfo`, `read_fs_list() -> UResult<Vec<MountInfo>>`, `FsUsage::new(path)`, `statfs`, `pretty_fstype` | **`df`**, and it is the exact shape a `get` subject needs |
| `proc_info` | `walk_process() -> impl Iterator<Item = ProcessInformation>`, `RunState`, `ppid`, `uid`, `start_time`, `tty` | `ps` / `top` |
| `uptime` | `get_uptime() -> UResult<i64>`, `get_loadavg() -> UResult<(f64,f64,f64)>`, `get_nusers()` | `top`'s header |

**`utmpx` is the one that looks relevant and is not.** It answers `w`/`who`/
`last` on a conventional system, and Rivet has no `utmp` or `wtmp` — confirmed
two ways: nothing in `docs/`, `rivet-core/`, `cmd/` or `tools/` names them, and
the Buildroot target rootfs contains no such file. It does not need one:
`ADR-022` §9.2 captures every service's own output as records, so the journal
already holds every login, and `PLAN.md` §0.5 records why that is **better than
`utmp` on three counts** — it survives a reboot, it carries the key that was
used, and a login cannot write to it.

The parts worth reading are narrow and specific: **symbolic mode parsing** (the
capital `X` in `u+rwX` has a rule most reimplementations get wrong), and
`fsext`'s **`statfs` field handling and `pretty_fstype` mapping**, which is
accumulated knowledge about what real filesystems report rather than logic
anyone would derive.

### 4.1.5 Why it cannot be a dependency, with the numbers

`cargo add uucore --no-default-features --features mode` — the most minimal ask
available, four pure functions returning `u32`:

**45 distinct crates.** Among them `fluent`, `fluent-bundle`, `fluent-syntax`,
`fluent-langneg`, `intl-memoizer`, `intl_pluralrules`, `unic-langid`,
`self_cell`, `type-map` — **the Mozilla Fluent localization engine, with a
bundle parser.**

Checked against the manifest rather than inferred from the feature selection:
`fluent` and `fluent-syntax` are declared with **no `optional = true`**, and
neither are `rustix`, `thiserror`, `os_display`, `unic-langid`,
`unicode-width`, `uucore_procs`, nor `nix` under `cfg(unix)`. `uucore`
translates its own error messages, so the i18n runtime is load-bearing for the
crate and **no feature flag removes it**. There is no route to `parse_mode` that
does not ship a localization framework.

That is disqualifying for a distribution whose argument is a small auditable
image, before `ADR-009` §5's gates get a turn.

**One hazard dodged rather than absent:** `openssl` with
`features = ["vendored"]` appears in that manifest under `cfg(unix)`. It **is**
optional and stays out — but vendored `openssl` compiles C, which `ADR-001` §4.4
rules out outright, and it sits one transitively-enabled feature away.

**Method, stated plainly** (`AGENTS.md` §6): this is manifest and
dependency-graph analysis, **not a build**. `uucore` was not compiled for
`aarch64-unknown-linux-musl`. The verdict does not turn on it — 45 crates and a
translation engine to obtain four pure functions is refused whether or not it
compiles cleanly.

### 4.1.6 Attribution

`uucore` is **MIT**; Rivet is MIT. Lifting an algorithm with attribution is
clean, and **it is required**: where Rivet's implementation is derived from
`uutils`, the source file says so and names the module it came from. Written
natively, `mode::parse` is a couple of hundred lines with no dependencies at
all, and `get storage` returns records where `df` returned columns.

### 4.1.7 The consequence for §3's table

The `coreutils` and `findutils, diffutils` rows are **superseded**. Their tier
`A` never meant Rivet *ships* them — it recorded that a Rust implementation
existed — but read alongside §4's "adopted directly" it says otherwise, and it
should not.

---

## 5. The shell, and why it is not infrastructure

The reframing that makes this decision easy:

> **If PID 1 and the service manager are Rust, nothing in Rivet's boot path
> runs a shell script.**

No `/etc/init.d`, no `rc.local`, no sourcing anything, no shell in the critical
path at all. The shell therefore stops being system infrastructure and becomes
purely a human interactive tool.

The consequence is liberating: **POSIX compatibility is a convenience, not a
requirement**, and the choice becomes a genuine one rather than an inherited
obligation.

| Candidate | Language | POSIX | Notes |
| :-- | :-- | :-- | :-- |
| BusyBox `ash` | C | yes | the stopgap; already present |
| `nushell` | Rust | no | structured pipelines; tens of MB; irrelevant on a Pi 4 |
| `brush` | Rust | bash-compatible target | worth evaluating if `#!/bin/sh` must keep working |

**Direction:** `nushell` as the interactive login shell, because a structured
data pipeline is a better fit for an operating system being designed fresh than
untyped byte streams are. BusyBox `ash` remains as `/bin/sh` until the last
consumer of a POSIX script is gone. Not yet ruled — tracked in `docs/PLAN.md`.

### 5.1 Maintainer's position, 2026-08-22

Recorded rather than ruled: the maintainer holds that POSIX compliance for the
Rivet system shell is neither a requirement nor something he wants.
*(`.rulings/` R-262.)* That is a stronger position than the paragraph above
assumed, and it has two consequences worth writing down before Stage 2 reaches
this decision.

**First, `/bin/sh` may be droppable entirely rather than retained.** The
paragraph above treats BusyBox `ash` as a residual obligation. If POSIX is not
wanted, the question becomes whether anything in the image genuinely needs a
`#!/bin/sh` interpreter at all — and per the framing at the head of this
section, Rivet's own boot path needs none by construction. That would remove
a component rather than shrink one, which is a materially better outcome for the
scoreboard of `ADR-001` section 4.1. The remaining consumers are third-party
scripts the operator brings, which is a real cost but a *bounded and visible*
one.

**Second, it composes with `ADR-006` section 4 far better than POSIX would.**
`ADR-006` makes structured output the primary form of every Rivet command. A
structured shell consumes that directly:

```
rivet net status | where state == "associated" | get iface
```

No `jq`, no `awk`, no field-splitting, no quoting hazard — because the command
never serialised to text in the first place. Under a POSIX shell the same
pipeline is a parse, and every parse is a place to be wrong.

The two decisions therefore reinforce each other, and choosing a POSIX shell
would partly waste `ADR-006`. This is an argument *for* the direction, and it
was not available when the direction was first written.

**A human sat at a board and typed at it — 2026-08-24.** The ruling above was
made on 2026-08-22, the shell was proven interactive under emulation on
2026-08-24, and on the evening of the same day the
maintainer reached an interactive prompt on a Raspberry Pi 4:

```
Environment config file created at: /state/home/root/.config/nushell/env.nu
Config file created at: /state/home/root/.config/nushell/config.nu
Version: 0.115.1 (aarch64-unknown-linux-musl)
Startup Time: 67ms 224µs 722ns
~>
```

Three things arrive with that prompt. The **dotfiles rule below is observable**:
`nu` wrote its own configuration under `/state/home/root`, which is the
maintainer's directory on the only writable filesystem, and nothing in Rivet
had an opinion about it. **Startup is 67 ms on the metal**, against ~280 ms under
emulation. And `ADR-006` §4's argument — the one this section makes in the
abstract two paragraphs up — became something a person does with their hands:

```
~> ps | where name == rivet-mcpd | select pid ppid status
~> sys host | get uptime
~> open /etc/hosts
```

`ps`, `open` and `sys` are builtins; the card ships no `cat`, no `grep`, no
`ps(1)` and no `uptime(1)`, and needs none. This is also `PLAN.md` §2.1's
**shell clause**, deferred since Stage 0 was first assessed, and it closes here.

**Ruled 2026-08-22: `nushell`.** The maintainer's reasoning is that it is the
correct choice *for this project* despite its size, and that it will not be the
largest thing Rivet imports — see `docs/PLAN.md` §5 on the graphical stack.

Two consequences worth writing down:

* **`nushell`'s dotfiles are the user's, not the system's.** They live under
  `/state/home` and Rivet's configuration has no opinion about them. Without
  that line a shell's configuration language becomes a second system-config
  surface, which is exactly what `ADR-007` exists to prevent.
* **Build it with features trimmed.** It cannot be made small, but it can be
  made smaller, and image size is a tracked number (`ADR-002` §5.1).

Considered and not taken: making `rivet` itself the login shell, with no
general shell at all — the Talos model, and the most coherent possible
expression of "administration goes through the tools". Rejected because it
leaves an operator able to do *nothing* the admin surface does not already
expose, which is precisely the situation in which a general shell earns its
place.

### 5.2 PowerShell, and what it corroborates

The maintainer raised PowerShell as an illustration of how wide the field is —
that a shell need not be POSIX or Unix-descended to be acceptable, and that if
it were Rust it would be a candidate.

**It is ruled out, and for the reason he suspected.** PowerShell requires the
.NET runtime — on Linux, a self-contained deployment on the order of a hundred
megabytes, with a JIT, a garbage collector, and dynamic loading throughout. That
is not a size objection; it is a direct contradiction of `ADR-003`, which puts
no dynamic loader in the image at all. It is also not Rust and will not become
Rust.

**But the model it introduced is the right one, and that matters more than the
implementation.** PowerShell's thesis — objects through the pipeline rather than
text — is exactly the position this ADR reached from a different direction, and
three of its design choices independently corroborate decisions taken in
`ADR-006`:

| PowerShell | The same idea in Rivet |
| :-- | :-- |
| Objects, not text, through the pipeline | `ADR-006` section 4 — structured output is the primary form |
| An approved verb list (`Get-`, `Set-`, `New-`, `Remove-`) enforcing one vocabulary | `AGENTS.md` section 10.2, and `ADR-006` section 6.4 |
| `-WhatIf` / `-Confirm` supplied by the framework to every command | `ADR-006` section 6.2 — every mutating operation supports a dry run |
| `Get-Help`, `Get-Command`, `Get-Member` — a runtime-introspectable surface | `ADR-006` section 7 — the surface describes itself |

That is the second time in this design record that a system solving the same
problem at very large scale has arrived at the same conclusion independently —
Android's property service (`ADR-008` section 2.1) being the first. Neither is
evidence that the conclusions are correct, but a design that keeps meeting
itself in the work of people who could afford to be wrong expensively is
better-supported than one that does not.

**The practical conclusion:** `nushell` is PowerShell's thesis rewritten by
people who learned from it — structured pipelines, but terse, fast, a single
static binary, and Rust. The choice is not between the model and the
constraints; the model is available inside them.

### 5.3 Not proposed: making the shell the administration interface

`nushell` has a plugin protocol, and Rivet's commands could be implemented as
native plugins returning nushell values directly, with no serialisation at any
point. It is the most complete possible expression of "structured output
primary" and it is tempting.

**It is not proposed**, because it would make the shell a required component of
the administration path and contradict `ADR-006` section 3 — the CLI is one
adapter among several, and none of them may be privileged. A nushell plugin is
legitimate as an *additional* adapter over `rivet-core`, and it should not be
the only way in.

---

### 5.4 Ruling, 2026-08-22: no POSIX environment in the base image

**The maintainer ruled that `/bin/sh` may perhaps be installed at a later date as
an add-on, but that building out an *official POSIX environment* is something he
would wait to do until the Rivet environment itself was completed first.**
*(`.rulings/` R-011.)*

This supersedes the retained-`ash` position in section 5 and the parts of
section 3's table noted there. Three things follow.

**1. The base image contains no POSIX shell.** Not a reduced one, not a
fallback. `nushell` is the login shell and there is no `/bin/sh`.

**2. Complete or absent — never partial.** The reason to defer a POSIX
environment wholesale rather than approximate one is the same reason
`AGENTS.md` section 3 says stubs must refuse rather than fake success: **a
compatibility environment that is incomplete is worse than none.** BusyBox
`ash` plus a dozen applets looks like POSIX, is not, and fails scripts in ways
that are attributed to the script rather than to the environment. An absent
shell fails immediately and unambiguously.

**3. The `grep`/`sed`/`awk` gap dissolves.** Section 4 called that category the
least-served and predicted BusyBox would linger there longest. That was only
true under an assumption this ruling removes — that the tools had to be
*script-compatible*. With no scripts in the system, they need only serve a human
at a prompt, and `ripgrep` and `sd` do that better than their POSIX
predecessors. They are adopted as Rivet's tools, explicitly **not** as
`grep` and `sed` replacements, and the naming should not pretend otherwise.

#### 5.4.1 The one real blocker, and its way round

Before this can be called achievable rather than aspirational, the mechanisms
have to be checked for shell dependencies. One has a genuine one:

**BusyBox `udhcpc` requires a script.** It execs a configuration script on every
lease event, and the stock one is shell. That is a hard dependency and it is in
the Stage 1 plan.

**It is not a shell dependency in fact.** `udhcpc -s <path>` execs *a program*;
nothing requires that program to be a shell script. A small Rust binary reading
the environment variables `udhcpc` sets and applying the lease over netlink
satisfies it exactly, and is a better hook than the script regardless — it is
typed, it is testable off-target, and it shares code with the Rust DHCP client
that eventually replaces `udhcpc` entirely.

The others are clear, though each needs confirming on hardware rather than from
this document:

* **`wpa_supplicant`** is driven over its control socket (`ADR-004` section 4)
  and needs no script.
* **The kernel** needs no `/sbin/hotplug` helper; uevents are delivered over
  netlink and there is no udev (section 2.1).

#### 5.4.1.3 Single-quoted strings are accepted — ruled 2026-08-25

**The finding is `ADR-012` §8.3 and it is a gap between the ruled surface and
the machine:** a remote invocation of `rivet apply edit … --expect '…'` arrives
as one string, that string contains quotes, and the tokeniser's refusal of any
quoted string rejects it. So
the maintainer's own worked example could not be typed remotely, and neither
could most edits worth making — a value with a space in it needs quoting, and
quoting was refused.

**Ruled: `rivet` accepts POSIX single-quoted strings, and nothing else.**

This is item 2's rule admitting a construct rather than being relaxed. A
single-quoted string in `sh` has **no escapes and no expansion** — every byte
between the quotes is literal, and the only thing that ends it is the next `'`.
So splitting one is *provably* the answer a shell would give, which is the exact
test item 2 states. Nothing is guessed and nothing is approximated.

**What stays refused, and why the list did not get shorter:** double quotes
(expansion), `$`, backtick, `\`, `*`, `~`, `#`, `!`, `;` and `&`. Each is
refused for §5.4's original reason — a partial shell fails in ways attributed to
the caller — and each would require *reproducing* a shell's behaviour rather
than reproducing a construct that has none.

**A base64 spelling was considered twice and is not taken.** `ADR-012` §8.3
option 3 carries it: it is a second spelling of one input, against `ADR-006`
§6.4's one-spelling rule, and it is what to add if a trailing newline turns out
to matter in practice.

**Not built as of 2026-08-25** — `PLAN.md` §22.1.

#### 5.4.2 Consequence: BusyBox may leave entirely

With `ash` gone, `grep`/`sed`/`awk` reframed, and `udhcpc`'s hook written in
Rust, the remaining BusyBox applet is kernel module loading — and section 3
already notes that can be avoided by building drivers in.

**The scoreboard of `ADR-001` section 4.1 therefore has a reachable target of
zero**, which was not true when it was written. That is a materially better
outcome than a shrinking count, and it should be stated as the goal for Stage 2.

**Reached 2026-08-22 by `ADR-024` §4.** Building every driver into the kernel
removes module loading — the last applet — so the target is not merely reachable
but decided.

### 5.5 The gap the ruling opened: there is no editor

The maintainer raised on 2026-08-22 that the system might be left with no `vi`
at all. *(`.rulings/` R-263.)*

Correct, and it is not a small point. `vi` was a BusyBox applet, BusyBox is
leaving (section 5.4.2), and `uutils` ships no editor. **As the design stands,
a Rivet system has no way to edit a text file at all.**

Two things depend on this, and they pull in opposite directions:

* **`ADR-007` section 3.6.3 proposes `rivet config edit`**, on the `visudo`
  model. That requires an editor to exist.
* **Nothing else does.** Configuration changes through `rivet config set`
  need no editor, `/etc` is a read-only view, and there are no scripts to edit.

So an editor is a convenience, not infrastructure — but it is the convenience
whose absence is felt hardest at 2am, and a system that cannot be edited at all
is hostile in a way that is difficult to argue is principled.

| Option | Size | Notes |
| :-- | :-- | :-- |
| **None** | — | Consistent, and genuinely unpleasant. `rivet config edit` would have to degrade to `set` only. |
| `kibi` | very small | A ~1000-line Rust editor. Enough to fix a file; not enough to enjoy. |
| `helix` | large | Excellent, Rust, modal. Brings its own configuration surface, which is ironic in this project. |

**Ruled 2026-08-22: a small Rust editor in the base image, with `helix`
available as an add-on image variant** (`ADR-008` §4.1). Chosen for size rather
than capability — `kibi` is the archetype rather than necessarily the choice,
and the specific project is an implementation-time decision under `ADR-009`'s
survey-first rule.

`helix` is deliberately *not* in the base image. It is large, and it brings its
own configuration surface — a faintly absurd thing to ship permanently in a
project whose central argument is about configuration sprawl.

**Amended 2026-08-22:** `helix` is a **system extension** (`ADR-016`), not an
image variant. Taking "image variant" literally would have meant rebuilding and
reflashing the operating system to obtain a text editor, which is the absurdity
that produced `ADR-016`.

**Two uses justify it**, and the second was underweighted when this section was
first written:

* `rivet config edit` (`ADR-007` §3.6.3) needs an editor to exist.
* **The operator's own files in `/state/home`.** `nushell` is ruled in (§5.1)
  and has its own configuration; editing it, or writing a script, is an ordinary
  thing to want on a machine one is logged into. With no editor there would be
  no way to do it at all.

**Whatever is chosen, it is Rivet's editor and not `vi`.** Naming it `vi`
would be the same dishonesty section 5.4 rejected for `grep` and `sed` — a
familiar name promising behaviour the thing does not have.

### 5.5.1 BUILT 2026-08-24 — the implementation-time decision is `femto`, written here

§5.5 left the specific project as *"an implementation-time decision under
`ADR-009`'s survey-first rule"*. This is that decision, taken and built:
**`cmd/femto`, written in this workspace rather than imported.**

The gap that forced it is worth recording because it is the concrete form of
§5.5's abstract argument: an operator logged into a board, wanted to suppress
`nushell`'s welcome banner, and was told by the machine to edit a configuration
file — on a system with nothing to edit it with.

#### 5.5.1.1 Why written and not imported

`kibi` is §5.5's archetype and importing it would have been cheaper. It was
rejected for one reason, and the reason is the whole justification for spending
the effort:

> **On this machine most files are not editable, and no imported editor knows
> that.**

`/etc` is a projection rendered fresh every boot onto a tmpfs that is remounted
read-only (`ADR-007` §3.5.1, §3.6.1 item 1). Open `/etc/hosts` in any imported
editor, change it, save it, and the result is one of the two rows `ADR-007`
§3.6's table calls dishonest — a bare `EROFS` with no explanation, or, for any
path under `/etc` the projection does not render, a write that succeeds and is
gone at the next boot.

Rivet already knows the answer and has since 2026-08-23: `rivet explain
/etc/hosts` names the generator, `writable: false`, the filesystem, and the
`change-with`. `cmd/femto/src/view.rs` reads **the same
`rivet_core::etc::ETC_FILES`** that `rivet-core::operation::explain` reads,
so the editor's refusal and the CLI's explanation cannot disagree — `ADR-011`,
one fact, one owner. A hand-written list of generated paths in the editor would
have been the second authority, consulted at exactly the moment somebody is
already confused.

Two things fell out of doing it here rather than importing, and neither could
have been patched into an imported editor without owning it anyway:

* **Any path under `/etc` is refused, not only the five rendered files.**
  `/etc/resolv.conf` is the example `ADR-007` §3.6.1 item 3 chose *because*
  Rivet deliberately does not render it; an editor that knew only the rendered
  five would have called it writable.
* **`/system` is refused too**, naming `ADR-018` §5's slot replacement. It is
  not a view — it is a whole read-only filesystem — but §3.6.5's rule generalises
  and the refusal costs three lines.

#### 5.5.1.2 RULED — a view opens **read-only**; it is not refused outright

The obvious implementation is to refuse the open: print the explanation and
exit. It is simpler and it is wrong, for three reasons in increasing weight.

1. **An operator opening `/etc/passwd` usually wants to read it.** Refusing the
   open turns one honest answer into two commands, on a machine whose session
   `PATH` is `/system/bin` and nothing else.
2. **`ADR-007` §3.6.1 item 2 calls the provenance header the highest-value
   mechanism in its list** — *"because it is read by exactly the person about to
   make the mistake: they have the file open."* Refusing the open is precisely
   taking that away. Opening read-only with the generator and the `change-with`
   on screen is the same mechanism delivered by the tool instead of by a comment
   in the file.
3. **Three of the five generated files cannot carry item 2's header at all.**
   `/etc/passwd`, `/etc/group` and `/etc/shadow` are colon-separated databases
   whose parsers are not known to skip a comment (`rivet-core::etc`), so for
   those three there is no provenance in the file — and a read-only open with a
   banner is the only place an operator with the file in front of them can ever
   see one.

So the refusal lands on the **write**, and it lands twice: the title bar says
`VIEW — read only` from the moment the file is on screen, and `^O` answers with
the generator and the `change-with` rather than with `EROFS`.

**The write is classified at the moment it is attempted, not at the open**,
which has a consequence worth stating because it is the half that makes
read-only better than a refusal rather than merely gentler: the contents of
`/etc/passwd` can be written to `/state/home/root/passwd.txt`. A copy of a view
is not a view, so the copy is editable, and the buffer becomes the copy. Driven
on a real pty, 2026-08-24.

#### 5.5.1.3 The name, ruled by the maintainer 2026-08-24

**`femto`, and no `rivet-` prefix.**

`nano` descends from `pico`, and the SI prefixes run downward as the editors get
smaller: nano → pico → femto. This one is aimed at roughly a thousand lines,
smaller than either, so the name says *tiny editor of that family* with nothing
to explain. `Fe` is also iron, which is where Rivet's own name comes from.

The absent prefix is the load-bearing half. **`rivet-` means Rivet's own
machinery**; `femto` is a general tool that ships on the card exactly as `nu`
does (§5.1), and `nu` correctly wears no prefix either. There is a separate,
agent-facing `rivet apply edit` — structured, non-interactive,
plan-then-confirm — and *that* is Rivet machinery. Two things both called
"edit" would be a question an operator has to answer at 2am.

It satisfies §5.5's naming rule by construction: it is not called `vi`, and it
promises nothing it does not do.

#### 5.5.1.4 What it costs, measured

| | |
| :-- | :-- |
| Dependencies | **two**: `rivet-core` (the view table) and `libc` (three termios/ioctl calls). Nothing else, and no transitive third-party crate. |
| `aarch64-unknown-linux-musl`, release, stripped | **417,208 bytes** — stripped with the Buildroot cross `strip` per §7.5, because the host's cannot read an aarch64 ELF |
| Unstripped, as `[profile.release]` produces it | 3,259,640 bytes, `statically linked`, no dynamic section, no `INTERP` |
| Tests | 120, none of which needs a terminal |
| `unsafe` blocks | **6**, all in `src/syscall.rs`, one SAFETY comment each |
| Size, counted rather than estimated | 4,333 lines over nine modules: **1,823 of code**, 934 of comment, 1,347 of tests |

**§5.5 aimed at `kibi`'s "~1000 lines" and this is 1,823**, which is worth
saying rather than rounding. About 200 of them are things `kibi` does not have —
`src/view.rs`'s refusal, `src/port.rs`'s test doubles, and the prompt state
machine that makes `^O`, `^W` and `^_` behave the way a `nano` user expects —
and the rest is the same editor with its rendering separated from its buffer so
that both can be tested without a pty. It is the right order of magnitude and it
is not `helix`.

**`crossterm` was surveyed and rejected**, and the number is a reading rather
than an impression: `cargo tree --target aarch64-unknown-linux-musl -e normal`
for `crossterm 0.29.0` resolves 27 crates, 26 of which `femto` does not already
have. Two of them decide it on their own — `rustix` and `linux-raw-sys` are a
second syscall vocabulary beside `libc`, which is what `cmd/rivet-initd`'s
survey already rejected, and `signal-hook` installs signal handlers in a binary
that ships in the base image of a system where signal disposition is PID 1's and
the supervisor's (`ADR-017` §3.1.10). What `femto` wanted from it — raw mode,
the window size and an escape-sequence decoder — is about 250 lines of
`src/syscall.rs` and `src/key.rs`, both of which are driven by tests on a
machine with no terminal. The full argument and the full list are at the
dependency in `cmd/femto/Cargo.toml`, per `ADR-001` §4.4.

**`helix` is unaffected.** It remains a system extension (`ADR-016`) rather than
a base-image component, for the reasons §5.5 gives.

#### 5.5.1.5 What is unproven

**It has not run on a Raspberry Pi.** It was built for the board and the
artifact was inspected; it was *run* on x86-64 Linux through a real pty. What a
board would settle that a pty does not: the serial console's own idea of the
window size, and whether the colours are legible on it.

### 5.5.2 RULED 2026-08-29: `femto` is a CLIENT, not a card binary

**Moved here from `PLAN.md` §96 on 2026-08-29, unaltered.** Only the heading
numbers changed. **Bare `§N` references in the body are `PLAN.md` sections** and
resolve through that file's pointer table — `§89` is the open question this
answers, and `§93.2` the ruling that built the surface it depends on.

It lands in §5.5 because §5.5 is where this project's editor is ruled, and this
changes *where the editor runs* rather than what it is: §5.5.1 built `femto` for
the card on the assumption of *"a human at a prompt"*, and `ADR-035` phase 5
removes the prompt. §5.5.2 rules that it becomes a client of `ADR-006` §8.10's
file surface on the operator's own machine. **Nothing here commits to reviving
it** — §5.5.2.7 says so in the maintainer's own terms.


§89 left an open question: what "remote editing" means once the login shell and `nushell`
are gone, with `femto` archived at `Femto-Dev` and leaving the card at phase 5.
**The maintainer answered it: preserving `femto` turns out to have been worth
doing, because it could still serve as the TUI front end to the MCP editor tool.
That would leave the project with an editor a human can use and an editor an
agent can use — the co-equality he has wanted throughout.** *(`RULINGS.md`
R-012.)*

**Accepted as the answer to §89's question**, and it is a better answer than the
one §89 reached, because it dissolves that section's own objection rather than
overriding it.

#### 5.5.2.1 Why this is not the thing `PLAN.md` §89.2 rejected

§89.2 refused a remote editor in these terms:

> An interactive editor reached over MCP would be a second and weaker answer to
> a question `apply edit` already settles — `ADR-011`, which is the reason most
> of the rest of this file exists.

**That objection stands, and this proposal does not meet it**, because `femto`
as a client is not a second answer — **it composes the first one.** It does not
write files. It calls `get file` to open and `apply edit` to save, so there is
still exactly one write path, one set of refusals, one confirmation, and one
journal record with one principal on it. What `femto` supplies is the part a
human needs and an operation does not: somewhere to *compose* the
`--expect`/`--replace` pair rather than assembling it by hand.

A UI over the authority is not a rival to it. **A second implementation of the
write would have been**, and that is what §89.2 was refusing.

#### 5.5.2.2 `--expect` was already compare-and-swap, which is the hard part

`ADR-006` §8.9's `--expect <text> --replace <text>` replaces text that must
occur **exactly once**, refusing at zero matches and at more than one. For an
editor saving a whole file, the old body is that text, and the pair is therefore
a **compare-and-swap**: if anything changed the file between open and save, the
save refuses instead of clobbering.

**Concurrent-edit detection is the hard problem in a remote editor, and it is
not merely available here — it is the mechanism `edit` was built around.** It
was designed as a safety property for an agent that states what it believes is
there (`PLAN.md` §90.3), and it turns out to be exactly what a human editor
needs from a network file API. Nothing has to be added for it.

#### 5.5.2.3 It RELOCATES `femto` rather than removing it, and that is the win

§89 had `femto` leaving the card at phase 5 as a straight loss of capability
accepted for a 417 KiB saving. **As a client it does not run on the card at
all** — it runs on the operator's machine and edits the board over TLS.

* **Phase 5 costs no editing.** Dropping `nushell` (44 MB) and `femto` stops
  being a trade against usability and becomes only a saving.
* **The memory bound moves off the board.** A machine with 926 MB of usable RAM
  buffering a file whole is a real constraint; the workstation editing it is
  not. `ADR-005` §5.5.1's *"a human at a prompt"* was the assumption that made
  `femto` card-resident, and the prompt is what phase 5 removes.
* **§89.3's extraction cost changes character, and downwards.** That section
  tabulated `femto`'s coupling to `rivetos-core` — `view.rs` on `operation::explain`
  and `operation::storage`, `journal.rs` on the record *type*, `main.rs` on
  `deploy::Principal`. A client reaches those **over the wire as operations**
  rather than by linkage, so what was a porting problem becomes protocol. The
  three files §89.3 named are the three that get simpler.

#### 5.5.2.4 The co-equal surface, which is `ADR-012` arriving at its conclusion

`ADR-012` is titled *The Agent as a First-Class Operator*. **This is the other
half of it**, and the maintainer named it: an editor he can use and an editor an
agent can use, over one surface, with neither privileged.

Worth stating plainly because it inverts how this usually goes: the human is
normally the privileged operator — they get the shell — and the agent is given a
narrower API. Phase 5 removes the shell, so **the human joins the agent on the
API rather than the agent being admitted to the human's channel.** Same
operations, same catalogue, same refusals, same journal, same principal
recorded. The difference between them is only the front end.

#### 5.5.2.5 What it needs, all of which is phase 4's file surface

Nothing new. Every call `femto` would make is a subject §93.2 already ruled in
and which is being built as this is written:

| Editor action | Operation |
| :-- | :-- |
| browse | `get file` on a directory — the listing |
| open | `get file` on a path |
| show size, mode, type before opening | the same subject's meta-info |
| preview the change | `diff` |
| save | `apply edit --expect <old> --replace <new> confirm` |

**So `femto` is not blocked on anything and adds no requirement to phase 4.** It
is a client of a surface that exists when phase 4 lands.

#### 5.5.2.6 Application files: the constraint is not the sandbox

**The maintainer's question, and it needed a real answer rather than a
reassurance: because this is an ADMIN tool, do the sandboxes have to be worried
about at all — could he dip into the files of an application he has written and
hand-edit a config file if he had to?** *(`.rulings/` R-013.)*

**Yes — and the sandbox is not what governs it.** `ADR-015` §5's sandbox is a
mount namespace the supervisor puts an *application* in. The admin surface is
root and outside it. Refusing to touch application paths would recreate exactly
the gap §93.2 closed, since the control plane is the only channel to fall back on.

**What actually governs it is trust and persistence, and they differ by zone:**

| Zone | What is true |
| :-- | :-- |
| `/state/app/<name>/<version>/` — the **program** | Digest-covered by a signed manifest kept beside it and re-verified **at every launch** (`ADR-015` §4.2.1). Editing it means the application stops starting — and shows as **drift** in `rivet plan`/`rivet status`, which §4.2 calls the diagnosis as against the symptom |
| `/state/app/<name>/<version>/` — a **config file** | **No signature breaks** — the manifest digest covers the program. But §7 installs each version into its own directory and swaps a `current` symlink, so **a hand edit here is silently discarded by the next update.** The trap worth naming, because nothing fails at the time |
| `/state/app/<name>/data/` | Mode 0700, owned by the application's identity (§5.1.1). Its own writable state, survives version swaps. **The place a hand edit persists** |
| the configuration document | Where anything *declarative* belongs — `ADR-011`, via `apply edit` and `apply reload` |

**Ruled: permit, and inform — do not refuse.** The plan a write shows before
confirmation names the zone and what follows from it. That is `ADR-006` §8.9's
plan-before-apply doing real work rather than ceremony: an operator about to
lose an edit at the next update should learn it **from the plan**, not six weeks
later when it vanishes.

#### 5.5.2.7 What is NOT decided here

* **Whether `femto` is actually revived.** This section rules what it *would
  be* and that §89's question is answered; committing the work is the
  maintainer's and is not implied by writing this down.
* **Whether editing a program binary specifically should refuse more loudly
  than other writes.** It is the one case where the machine actively stops
  working, and a louder refusal may be right. Not ruled.
* **§89.3's extraction arithmetic has not been redone** under the client
  framing. The direction is argued in §5.5.2.3; the number is not.




---

## 5.6 The feasibility survey, 2026-08-23 — it builds, and it runs

§5.1 ruled `nushell` the interactive login shell on 2026-08-22. **Nobody had
checked that it could be built for this target at all**, which is a larger risk
than the scheduling argument for placing `nushell` after the catalogue worries
about: a ruled decision with
an unbuildable dependency is a hole that only opens at Stage 3.

Surveyed on the maintainer's instruction. **Everything below was verified by
building or running unless it says otherwise.**

### 5.6.1 The verdict

**`nu` 0.115.1 cross-compiles to `aarch64-unknown-linux-musl`, statically, with
default features, on the first attempt, with no patches and no feature
trimming.** Build time 2 m 24 s on the WSL Arch host.

`ADR-003` §3's three checkable properties, all met: `file` reports *statically
linked*, `readelf -l` shows no `PT_INTERP`, `readelf -d` reports no dynamic
section. There is no "if you trim X it works" caveat.

**It also runs**, under `qemu-system-aarch64 -M virt` on **Rivet's own kernel**
(`tools/qemu/kernel8.img`), in a reconstruction of Rivet's actual environment —
`rivet_core::mount::stage0_plan`'s mounts and no others, `/etc` as the five
files `rivet_core::etc` renders remounted read-only, `/state` writable,
`PATH=/system/bin`, no `/bin/sh`, no terminfo:

| Question | Answer |
| :-- | :-- |
| Starts at all? | **yes** — `nu --version` → `0.115.1`, exit 0 |
| Needs terminfo? | **no** — ran with `TERM` unset; no terminfo crate in the lock |
| Needs `$HOME` in the environment? | **no** — with `HOME` unset, `dirs` fell back to musl's `getpwuid()` reading the `/etc/passwd` Rivet generates |
| Needs a writable config dir? | **wants one, degrades without** — read-only `$HOME` warns and runs anyway, exit 0 |
| Needs a `PATH` of externals? | no — `^ls` fails cleanly |
| Startup | 114–290 ms under TCG emulation, so faster on silicon |

And §5.1's actual promise, with the real `cmd/rivet` binary at
`/system/bin/rivet`:

```
nu -c "rivet --version | from json | get operations | length"   → 1
```

`ADR-006` §7's catalogue is reachable from a nushell pipeline with no parsing,
because the value was never text.

### 5.6.2 Size, and the constraint it actually lands on

Stripped, `aarch64`:

| Build | Binary | `xz -9` |
| :-- | --: | --: |
| default features | 42.4 MiB | 10 251 028 B |
| `--no-default-features` | 29.9 MiB | 7 292 036 B |

Trimming buys about 29 %, and costs SQLite, HTTP/TLS, the LSP, the MCP server and
the helix edit mode. §5.1's "build it with features trimmed" is sound and pays
roughly a third.

**It cannot be a system extension, and the reason is structural rather than a
size question.** `/etc/shells` and `/etc/passwd` are rendered by the supervisor's
projection **before it converges any service** (`ADR-007` §3.5.1, `ADR-017`
§3.1.8), and `rivet_core::etc`'s renderer names a login shell only if a binary
is actually at that path. `ADR-016` §5 mounts extensions from the desired-state
document — necessarily *after* the projection. So at projection time a nushell
extension is not mounted, `/etc/shells` correctly names nothing, and **every
login is rejected before authentication** — the same failure an unlisted shell
produces, arriving from a new direction.

**So it lands on the base image**, and the number that moves is `ADR-018` §10's
distributed artifact: roughly 24 MB → 34 MB, **about +42 %**, several times
the largest single jump this project will have taken.
Nominal image size does not move — 42.4 MiB × 2 slots is 6.6 % of the 640 MiB
`/system` partitions. *(The +42 % is an estimate; measuring it needs
`cmd/rivet-image`, which the survey was not permitted to touch.)*

### 5.6.3 It brings C into the image, in no tier

**All of it arrives through one crate.** `fff-search` pulls `libgit2-sys` (with
`libz-sys`), `lmdb-master-sys` and `blake3`. It is declared `optional`, but
`nu-command`'s `os` feature hard-includes it and `nu`'s manifest pins that
feature unconditionally — and `os` is also what brings `crossterm`, `reedline`
and `which`, so it cannot simply be dropped. **There is no feature combination
that yields a pure-Rust `nu`.** Verified: with the cross `gcc` off `PATH`, `nu`
fails to build even at `--no-default-features`.

Two consequences, and **neither is a reason to reverse §5.1**:

* **A C cross-toolchain becomes a prerequisite for a *Rust* component.**
  `.cargo/config.toml`'s comment stays true — Rivet's *own* binaries need no C
  toolchain — but stops being the whole story. This is not a new cost: `PLAN.md`
  §4.6 already ruled Buildroot in for the C mechanisms and the toolchain. It
  gains a second consumer.
* **`ADR-001` §4 says every non-Rust component is in exactly one tier**, and
  `libgit2`, LMDB, zlib and blake3 would be in none — statically linked inside a
  Rust binary, on no scoreboard. **This is an accounting gap, not a violation**,
  and §5.4.2's BusyBox-applet count is undisturbed because these are not applets.
  But *"the image contains exactly one C mechanism"* becomes misleading the
  day `nu` ships. `ADR-001` §4.4 now carries it.

**Why this is not the wrong path.** The C sits *inside a leaf binary that nothing
links against*. PID 1, the supervisor and the CLI are untouched, so the audit
question is "does the shell misbehave", not "does the boot path misbehave" —
`ADR-009` §3.3's wrong-altitude hazard is the one that does not apply here. The
gap is one paragraph of bookkeeping, and the dependency is plausibly removable
upstream by splitting `fff-search` out of `os`.

### 5.6.4 Licences — ruled 2026-08-23

`cargo deny check licenses`, run with **Rivet's own `deny.toml`**, rejected
**47 of 572 crates** across 15 licence expressions. **Nothing GPL, LGPL or AGPL
anywhere** — `ADR-010` §2.1's actual red line is not approached.

Thirteen were bookkeeping. **Two were a ruling:** `option-ext` (via `dirs`, which
is how nushell finds `$HOME`) and `nucleo-matcher` (reedline's completer) are
**MPL-2.0**, which `deny.toml` named as never allowed without the maintainer.
Trimming does not help — `--no-default-features` still carries **both**, because
the licence problem is in nushell's core rather than its options.

**Ruled by the maintainer: MPL-2.0 is allowed.** `ADR-010` §7 carries the
reasoning; the short version is that MPL-2.0 is *file*-level copyleft and does
not infect a statically linked larger work, which is precisely the property
`deny.toml`'s old comment got backwards.

### 5.6.5 What it found that had nothing to do with nushell

* **Rivet had no `/tmp`.** Not an empty one — no such directory. `mktemp`
  failed under QEMU, and `std::env::temp_dir()` is `/tmp` for every crate in the
  tree including ours. Ruled and added the same day;
  `rivet_core::mount::stage0_plan` carries it.
* **The nushell ruling is cited from the wrong section in 18 places.** §5.4.2 is
  *"Consequence: BusyBox may leave entirely"*. The ruling is **§5.1**. Corrected
  across 12 files the same day, including a user-visible refusal message in
  `cmd/rivet`.
## 7. The debugging mechanisms — 2026-08-24

`gdbserver` and `strace`. Both are Buildroot packages that already existed in
the tree, both build static against this project's musl toolchain on the first
attempt, and neither is a subsystem Rivet writes. `ADR-009` §6 already ruled
Buildroot in for the C mechanisms, so this is **a build recipe and an install
path**, on precisely the pattern the other vendored C mechanisms set.

`tools/debug/build.sh` is the recipe, `tools/debug/session.sh` is the evidence,
and `tools/debug/README.md` carries the transcript.

| | Stripped | Both slots (`ADR-018` §4) |
| :-- | --: | --: |
| `gdbserver` | 1 010 936 B | 1 974 KiB |
| `strace` | 1 521 600 B | 2 972 KiB |
| **together** | **2 532 536 B** | **4 946 KiB** |

Measured by `stat` on the produced files on 2026-08-24, Buildroot 2026.02.3,
GDB 15.2, strace 6.19. Unstripped they are 1 519 792 and 1 849 784 bytes —
which matters, because **Buildroot strips in `target-finalize`, which is part
of `make all` and not part of `make gdb strace`**. Another binary in the same
tree reports `stripped` only because it was built by a run that reached
`target-finalize`. `tools/debug/build.sh` strips explicitly for that reason.

### 7.1 `BR2_TOOLCHAIN_EXTERNAL_GDB_SERVER_COPY` is the one-symbol mistake here

The Bootlin external toolchain **already ships a prebuilt `gdbserver`**, at
`<output>/host/opt/ext-toolchain/aarch64-buildroot-linux-musl/debug-root/usr/bin/gdbserver`,
and one kconfig symbol copies it to the target instead of building anything.
Read off that binary rather than assumed:

```text
ELF 64-bit LSB pie executable, ARM aarch64, version 1 (SYSV), dynamically
linked, interpreter /lib/ld-musl-aarch64.so.1, with debug_info, not stripped
 0x0000000000000001 (NEEDED)  Shared library: [libstdc++.so.6]
 0x0000000000000001 (NEEDED)  Shared library: [libgcc_s.so.1]
```

`ADR-003` §4 forbids that and `ADR-008` ships no dynamic loader, so it would
build cleanly, install cleanly, and fail at `exec` on the board naming an
*interpreter* rather than the binary. The defconfig leaves the symbol off and
says why at the line; `tools/debug/build.sh` checks the **outcome** rather than
trusting the setting.

The two halves match without either being pinned: the toolchain's own
`aarch64-linux-gdb` is GDB 15.2, and `package/gdb/Config.in.host` defaults
`BR2_GDB_VERSION` to `"15.2" if … (!BR2_PACKAGE_HOST_GDB &&
BR2_TOOLCHAIN_GCC_AT_LEAST_9)`, which this toolchain (gcc 14.3.0) satisfies.

### 7.2 What `strace` answers that the journal cannot — with a correction it
### produced within a minute of existing

`ADR-022` makes Rivet's own account of a failure a structured record. That
account is Rivet's: an exit status, and whatever the mechanism chose to print.
**Which syscall a C mechanism failed on, against which path, with which
`errno`, is a fact only the kernel has**, and Rivet drives programs it did not
write (`ADR-009` §3).

**The first thing it was pointed at corrected this document.** A refusal this
record described as happening at `open` with `O_CREAT | O_EXCL` was traced on a
running machine and turned out to happen one step later, at the *rename* of a
`.tmp<pid>` file — so the property held, the exit status was non-zero, and the
operator-visible message named *moving* rather than *creating*. The mechanism
written down was not the mechanism running.

That is the whole case for `strace` in one paragraph: the record was derived
from reading somebody else's source, it was subtly wrong, and no amount of
Rivet logging could have shown it.

### 7.3 A debugger has to be *started*, and on this system only `nu` can

**The finding of 2026-08-24, and it was found by running a session rather than
by reading anything.**

Nothing on a card starts `gdbserver`, and nothing should: a debug stub started
by the supervisor is a listening socket on every board, which is `ADR-028`'s
question. It is started by an operator who has logged in — which means it is
started by the **login shell**. The dual-login-shell ruling makes root's shell `nu` when the image
has one and `cmd/rivet` when it does not, and `cmd/rivet` refuses to
execute any program but itself, by ruling (§5.1). So on a card
without `nu`:

```text
$ /system/bin/gdbserver :1234 /system/bin/rivet --version
{"error":"E_NOT_A_SHELL","subject":"/system/bin/gdbserver","message":"rivet
 cannot execute programs. It is the login shell and the only
 command it answers is `rivet` (or `/system/bin/rivet`)."}
$ echo $?
127
```

The refusal is correct — it is §5.1 working exactly as ruled. What follows from
it is not a defect either, but it is a **dependency nobody had written down**:
`--gdbserver` without `--nushell` produces a card whose debugger is
unreachable. `tools/debug/session.sh` refuses rather than reproducing it.

**This bears on §7.4 below and on `ADR-008` §4.1 if either is ever ruled a
variant**: "the debug image" and "the image with a shell on it" are the same
image, or the first one does not work.

### 7.4 UNRULED: whether a card carries them at all

**The maintainer's, and deliberately not answered here.** Both
`cmd/rivet-image --gdbserver` and `--strace` are **optional**, exactly as
`--nushell` is, so a card can be built either way and the question can be
decided against the measured cost rather than against an estimate.

**Both cards were built on 2026-08-24** and the artifact measured rather than
reasoned about. Same six positional inputs, same 2048 MiB layout, the only
difference being the two flags:

**Two of the file names below are no longer any binary's.** PID 1 was renamed
`rivet-init` → `rivet-initd` and the supervisor `rivet-supervisor` →
`rivet-sysd` later on 2026-08-24, after these cards were built and read. The
listing is what `debugfs` printed, and it is left that way — `AGENTS.md` §5.
**Every other quoted machine output in this file is from before the rename
too**; the prose around them is current.

```text
=== debug card, /system/bin ===          === plain card, /system/bin ===
    544600  rivet                          544600  rivet
    654064  rivet-init                     654064  rivet-init
    561568  rivet-netd                     561568  rivet-netd
    774448  rivet-supervisor               774448  rivet-supervisor
    427488  rivet-timed                    427488  rivet-timed
   1010936  gdbserver
   1521600  strace
```

read back out of the `system-a` filesystem with `debugfs -R "ls -l
/system/bin"`, root-owned and `0755`, so the install path is observed rather
than asserted.

**`ADR-018` §10's distributed artifact**, both images gzipped `-9`:

| | Compressed | Delta |
| :-- | --: | --: |
| no debugger | 28 879 370 B | — |
| with both | 31 103 389 B | **+2 224 019 B, +7.7 %** |

Less than the 4 946 KiB of raw bytes, because the two slots hold identical
copies and the compressor finds them. `ADR-005` §5.6.2 put `nu` at roughly
+42 % by comparison.

| Option | For | Against |
| :-- | :-- | :-- |
| **On every card** | the board that is misbehaving is the one you did not plan to debug; a reflash to get a debugger is the loop `ADR-016` was written against | +4 946 KiB on `ADR-018` §10's distributed artifact, on every deployed board, for a tool most of them never run |
| **A `--debug` build only** | costs nothing on a normal card; the flags already exist, so this needs no mechanism at all | the debugger is absent at the moment it is wanted, and getting it means writing a card (`ADR-027` §4) rather than debugging |
| **A system extension** (`ADR-016`) | mounted at `/system/ext/debug`, signed and versioned, added without replacing `/system` — which is the mechanism `ADR-008` §4.1 was narrowed *to* | `ADR-016`'s machinery, for two files. And §7.3 means the extension has to carry `nu` too, or it is inert |

**RULED 2026-08-24 by the maintainer: the second — a `--debug` build only.**
The flags exist and cost nothing; `ADR-016`'s extension is the right shape and
is more work than the problem currently justifies.

**What the ruling costs, said rather than left to be met:** the debugger is
absent at the moment it is wanted. A board that starts misbehaving is one you
have to write a card for before you can attach to it, and `ADR-027` §4 is clear
that writing a card **provisions** a machine rather than updating one — so the
`/state` you wanted to look at goes with it unless you are careful. That is the
price of the option, it was on the table when it was taken, and the mitigation
is `ADR-016`'s extension whenever that becomes cheap.

**What it does not change:** nothing on a card starts a debugger, so a debug
card is a card with two extra files on it and no extra behaviour. `ADR-028`'s
concern — a `ptrace`-capable stub reachable by anything that can log in — now
applies only to a card somebody deliberately built that way, which is the
whole of what this ruling buys on the security side.

**`ADR-028` is the file that would care**, and the security question is not the
size one. `gdbserver` on a deployed board is `ptrace` reachable by anything that
can log in, and a stub bound to a port is a stub anything on the network can
talk to if something starts it. Nothing starts it today (§7.3) and that is the
whole of the current mitigation.

### 7.5 UNRULED: where debug information lives

**`Cargo.toml`'s `[profile.release]` sets `strip = true`**, so the binary a card
carries has no symbol table and no DWARF. Three cases, all three run on
2026-08-24 under `tools/qemu/run.sh`:

| The CLI was built | gdb can | Locals |
| :-- | :-- | :-- |
| `--release` — **what a card carries** | nothing: `0x22d0d0 in ?? ()`, `Function "rivet::render" not defined.` | — |
| `--release`, `strip=false`, `debug=2` | break, stop, name the source line, walk the stack to `rivet::main` | every one `<optimized out>` |
| the dev profile | all of that | **readable** |

**The debug information does not have to be on the card.** `gdb` resolves
symbols locally, from a copy of the same build on the development host; the
target needs only the stripped executable. What Rivet does not have is
anywhere to *keep* that copy, because `strip = true` strips the only artifact
`cargo` produces. The options, none ruled:

1. **Leave it.** A shipped board is debuggable only at the level of addresses
   and `strace`. Cheapest, and it is the state today.
2. **`strip = "debuginfo"`.** Keeps `.symtab`, drops DWARF: breakpoints on
   function names work, source lines and variables do not. Costs a measurable
   amount on every binary in both slots and nobody has measured it.
3. **Keep an unstripped copy off the card**, produced by the same build that
   produced the shipped one, retained wherever `ADR-027`'s artifacts are named
   and signed. Strictly the most useful and the only one that is real work —
   it makes debug information a build *output*, which nothing in `ADR-027`
   currently is.

**RULED 2026-08-24 by the maintainer: the third.**

#### 7.5.1 What the ruling actually buys, and what it does not

The table above is the honest bound and it is worth reading before anyone
expects too much: on a **release** build with the debug information retained,
`gdb` can break, stop, name the source line and walk the stack to
`rivet::main` — and **every local reads `<optimized out>`.** Readable
variables need the dev profile, which is not what a card carries and is not
what this ruling changes.

So what is bought is *function-level* debugging of a shipped board: where it
is, how it got there, and which line. Not what it was holding. That is the
difference between `0x22d0d0 in ?? ()` and a stack trace, which is most of the
distance, and it is not the whole of it.

#### 7.5.2 What it requires, and why it is not one line

Three things, and only the first is small:

1. **`[profile.release]` stops stripping**, so `cargo` leaves an artifact that
   has symbols at all. Today `strip = true` strips the only thing it produces.
2. **Something else strips on the way onto the card**, because `ADR-018` §4's
   slots and `ADR-018` §10's distributed artifact are sized for stripped
   binaries and this ruling must not quietly grow either.

   **`cmd/rivet-image` is not the home, and checking is what settled it.**
   Stripping an `aarch64` ELF needs the **cross**
   toolchain: the host's GNU `strip` 2.47 answers *"Unable to recognise the
   architecture of the input file"*, measured rather than assumed. `ADR-009` §6
   keeps the toolchain on Buildroot's side of the line and has the image
   builder take **finished binaries** as inputs — *"nothing about how the
   binary was built is the image builder's business"* — so teaching it to strip
   would put a cross-toolchain dependency inside the one crate that is
   deliberately ignorant of how its inputs were made.

   **The home is `tools/card/build.sh`**, which is where that knowledge already
   lives (`tools/nushell/build.sh` strips with the same
   `aarch64-linux-strip`). `cmd/rivet-image` is untouched, so its
   **byte-reproducibility** property (`ADR-015` §4, asserted on every build) is
   undisturbed rather than needing to be re-established.
3. **`ADR-027` gains a build output it does not currently have.** The
   unstripped binaries are an artifact that must be *named*, so that the copy
   on a workstation can be matched to the card in front of you — and `ADR-027`
   §3.2.1 already makes a name a claim. An unstripped copy that cannot be tied
   to the exact build on the board is worse than none, because it will resolve
   symbols to the wrong addresses and say so to nobody.

**Item 3 is the one that makes this a change to the record and not only to a
profile**, and it is not attempted in the same pass as items 1 and 2.

#### 7.5.3 Built 2026-08-24 — items 1 and 2, and what item 3 turned out to need

`[profile.release]` keeps symbols and DWARF; **`tools/card/build.sh`** — the
first committed recipe for building a card at all — retains a copy per build
under `out/debug/<describe>/` and strips what goes onto the image.

Measured on the build it produced, with `readelf -S` rather than by grepping
`file`'s prose:

| | Retained | On the card |
| :-- | --: | --: |
| `rivet` | 5 294 808 B, has `.debug_info` | 544 960 B, stripped |
| `rivet-sysd` | 9 061 816 B | 774 904 B |
| `rivet-initd` | 6 391 928 B | 654 400 B |

**The card did not grow.** Compressed, against a build from an earlier commit,
61 736 190 → 61 740 807 bytes — and even that difference is source drift
between the two commits rather than this change. `cargo test --workspace`,
`cargo clippy --all-targets` and the QEMU smoke run all pass with the
unstripped binaries in the emulator's initramfs.

**Item 3 needs less than this section feared, and the reason is worth keeping.**
The check that a retained copy belongs to a given card does not need a naming
scheme at all: **strip the retained copy and compare it byte-for-byte with what
the card carries.** That was run — `cmp` reports the retained `rivet` strips
to exactly the 544 960 bytes in `system-a`. So the tie is *verifiable* today
even though it is not *recorded*, which reduces item 3 from "you cannot know"
to "you have to check". `tools/card/build.sh` says so where an operator will
read it, and until `ADR-027` names the output the standing advice is to keep
one card and one `out/debug/` in play at a time.

### 7.6 UNRULED: which tier

`ADR-001` §4 requires every non-Rust component in the image to be in exactly one
tier, and the tier is a statement about **replacement intent**. §4.4 already
recorded an accounting gap when C arrived through a Rust crate, so the record is
sensitive about this and the answer is not being assigned here.

**The recommendation is Tier C for both, and the argument is §4.2's.** Nobody is
going to rewrite `gdb`'s remote serial protocol or `strace`'s syscall tables in
Rust, and a Tier B entry that nobody intends to act on is exactly the thing
§4.1 exists to prevent. `strace` in particular is a wrapper around the kernel's
own `ptrace` and `PTRACE_GET_SYSCALL_INFO`, and its value is that it tracks the
kernel's syscall set — which is `wpa_supplicant`'s argument in §4.2, in a
different domain.

**RULED 2026-08-24 by the maintainer: Tier C for both.** The counter-argument is kept
because it names a real gap in `ADR-001` §4's table, and the ruling does not
close it:

> neither is on the boot path or beside it, both are absent from a default
> card, and a tier is also a claim about what the image *is*. If §7.4 rules
> them onto a variant only, a reader of §4's table would reasonably want to
> know that — and §4's table has no column for "present on some images".

**§7.4 did rule them onto a variant only**, so that gap is now live rather than
hypothetical: `ADR-001` §4's table says what tier a component is in and cannot
say that two of its entries are on no card by default. The tier is recorded
there with that stated inline; adding a column is a change to §4's shape and
was not part of this ruling.

### 7.7 `ltrace` is refused by kconfig, not declined

`package/ltrace/Config.in` carries two independent hard dependencies Rivet
fails, both inherited from elfutils:

```text
depends on !BR2_STATIC_LIBS
depends on BR2_TOOLCHAIN_USES_UCLIBC || BR2_TOOLCHAIN_USES_GLIBC
```

Rivet is static musl by `ADR-003` §4, so the symbol is not offered at all. It
would also have nothing to do: `ltrace` traces calls into *dynamic* libraries,
and a fully static image has none. Recorded rather than left absent, because
`AGENTS.md` §7 makes an absence a claim.

### 7.8 What is proven, and what is not

**Proven, by running it** — `tools/debug/session.sh`, under
`tools/qemu/run.sh --net --forward-port`, on `ADR-024`'s kernel:

* both binaries build static `aarch64` musl and pass `ADR-003` §3's three checks;
* both **run** on a Rivet machine, started by `nu`;
* `gdbserver` binds and listens — read out of the guest's own `/proc/net/tcp`,
  not inferred from an absence of errors;
* a cross `gdb` on the development host attaches across a forwarded port, sets
  a breakpoint in `rivet::render`, **hits it**, names the source line, walks
  the stack four frames to `rivet::main`, and **prints `output` as
  `rivet::cli::Output::Json`** — a variable read, with its Rust enum variant
  named;
* the inferior then runs to completion and its output arrives over the session, so the
  session did not merely stop the program, it debugged it;
* `strace` traces a C mechanism and reports the `openat` flags and the failure.

**Not proven, and the list is the useful half:**

* **Nothing has run on a board.** `ADR-026` §4's right-hand column owns
  `ptrace` timing on real hardware, and §4.1.2 owns the six release cycles
  between this kernel and the board's. "It debugs in QEMU" is not "it debugs".
* **No card has been *booted*.** Two image files were built and read back with
  `debugfs` (§7.4), which settles that the install path puts the binaries where
  it says. Nothing has written one to a card and nothing has started a debugger
  from `/system/bin` on a real filesystem — the QEMU session runs off an
  initramfs.
* **Nothing has been debugged that was actually broken.** The session was run
  against a program that works. Attaching to a *running* process
  (`gdbserver --attach`), and a core file, are both untried.
* **The `/dev` observation is QEMU's, not a card's.** Under this loop `/dev`
  holds only `console`, `pts`, `shm` and `urandom` — and `/dev/urandom` is a
  32-byte regular *file* rather than the character device a program expecting
  the kernel's would find. Neither `gdbserver` nor `strace` needed anything that
  was missing, which is a fact about these two programs rather than a general one.

---

## 8. Consequences

**Renumbered from §7 on 2026-08-24**, when §7 above was added. Nothing in the
repository referenced `ADR-005` §7 at the time — checked with `git grep` and
with a search of `docs/` — so the renumber breaks no link. The heading is
recorded here rather than silently moved, per `AGENTS.md` §2.

* **The BusyBox applet list is a tracked, explicit allowlist**, never a
  convenience default. It is the scoreboard defined in `ADR-001` section 4.1 and
  it is reported in `README.md`.
* **Every C component is built static against musl** (`ADR-003` section 4).
  Each one is a build recipe this project owns, not a package it installs.
* **Stubs refuse; they never fake success.** Per `AGENTS.md` section 3, an
  unimplemented path returns an error. A network bring-up that reports success
  without a route is worse than one that fails.
* **No component in this table may be invoked directly from more than one place
  in the tree.** `ADR-001` section 5 — a component with scattered call sites
  cannot be replaced, which quietly converts Tier B into Tier C.

---

#### 5.5.2.8 RULED 2026-08-29: it IS revived, and the card binary is the thing that goes

§5.5.2.7's first bullet — *"whether `femto` is actually revived… committing the
work is the maintainer's and is not implied by writing this down"* — is
answered. **`femto` is how the maintainer edits a file in an application's state,
over the control plane.** *(`.rulings/` R-113.)*

**Both halves of §5.5.2.7's caveat were accurate and it is worth saying so
before correcting it**: the shape was ruled and the build was deliberately left
open. It is not open now.

##### The contradiction this exposes, found the same day

**`femto` is still installed on the card.** `cmd/rivet-image/src/system.rs`
pushes `FEMTO_PATH` when the build is given one, and the `0.1.10` card built on
2026-08-29 carries it — verified by reading `/system/bin` out of partition 3 with
`debugfs`, where it sits beside `git`.

**That directly contradicts this section's own heading.** §5.5.2 rules `femto` a
*client, not a card binary*, and the image has been installing it as a card
binary throughout. It is a ruling made, and the code that predates it left in
place because nothing connected the two.

**So reviving `femto` and removing it from the card are one change, not two.**
The client is the new thing; the card binary is the old thing the ruling already
retired. Shipping the first without the second would leave two editors — one on
the operator's machine reaching the file surface, one on a board reached by a
prompt that `ADR-035` has deleted — and the second would be unreachable,
which is worse than absent because `get status` and a `/system/bin` listing would
both still show it.

##### What is not settled

* **§5.5.2.7's other two bullets stand.** Whether editing a program binary
  refuses more loudly is still unruled, and §89.3's extraction arithmetic has
  still not been redone under the client framing.
* **Nothing is built**, and the card binary is not yet removed.

#### 5.5.2.9 CORRECTED 2026-08-29: it is a pane in `rivet-fleet`, and it is the maintainer's editor

§5.5.2 said `femto` becomes *"a client of `ADR-006` §8.10's file surface on the
operator's own machine"* and §5.5.2.8 repeated it. **That was vague in the one
place it needed to be specific, and the maintainer supplied the missing half: it
is part of `rivet-fleet` — his TUI.** *(`.rulings/` R-114.)*

**Not a separate binary.** `rivet-fleet` is already the operator's application,
is already becoming the MCP/TLS client (`ADR-034` §11.5.1), and already renders
panes over subjects the catalogue publishes — the Monitor is one. **An editor
over `ADR-006` §8.10's file surface is another pane over another operation**, and
a standalone `femto` binary would be a second program holding a second copy of
the identity, the pins and the transport to do what the first already does.

##### The objection he raised, and it is worth answering in the record

He objected that proposing to remove the card binary sounded like taking his
editor away merely because the agent had been given a file editor of its own.
*(`.rulings/` R-114.)*

**No, and the distinction is `ADR-006` §3.1's.** `apply edit` is an *operation*
in the catalogue; every adapter reaches it, and an agent using it is one adapter
doing so. `femto` is a *human interface* over that same operation. §3.1's
co-equality is precisely that these do not compete — one catalogue, many
surfaces — and an editor for the person is not made redundant by an operation
being callable.

**What §5.5.2.8 actually proposed removing was the card binary**, which §5.5.2's
own heading had already ruled off the card and which today's `0.1.10` image still
carries. That is a correction to the *image*, not to the editor, and the two were
adjacent enough in one paragraph to read as one thing. **Stated separately here
because a reader should not have to reconstruct which was which.**

##### What this settles and what it does not

* **Where it runs:** in `rivet-fleet`, on the operator's machine, over the
  control plane. Not on the card.
* **It does not settle the extraction arithmetic.** §5.5.2.7's third bullet
  stands: §89.3's numbers have not been redone under the client framing, and a
  pane is a different shape again from a standalone client.
* **Not built**, and the card binary is still installed.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.
Un-striking it here would put every retired claim back under audit.

Several of these were struck within hours of being written, which is the point
`AGENTS.md` §7 makes about a claim of absence outliving the absence — the
shortest of them outlived it by about an hour.

### In §3 — the mDNS responder was built on `simple-mdns`

**Changed 2026-10-04** (`.rulings/` R-252). The responder must open its socket
on its main thread and stop with a clear record when it cannot; `simple-mdns`
opens its sockets on a thread of its own and reports a failure only through a
log facade, so the responder was rewritten over `simple-dns`, the wire format
`simple-mdns` is built on and its named fallback. `ADR-017` §3.7.1 carries the
argument. The superseded cell was:

~~Rust — **`cmd/rivet-mdnsd`, ruled 2026-08-25**, over **`simple-mdns`, ruled
2026-08-26** (`PLAN.md` §39)~~

### In §5.4.1.1 — `/etc/shells` was generated by the image

**Changed 2026-08-23**, when the maintainer ruled that the runtime projection
owns the whole of `/etc`. Two superseded fragments:

* ~~`cmd/rivet-image` now generates it.~~ — true for part of 2026-08-23.
* ~~**this file does not exist on a running Rivet at all** until that projection
  is written~~ — which was a stronger blocker than the one the section was
  recording.

Both closed later the same day. The note that accompanied them read: ~~The
struck sentences above are left visible because `AGENTS.md` §7 is about exactly
this kind of claim, and because each was true for several hours between two
rulings.~~

### In §5.4.1.1 — the Talos model was rejected

**Changed 2026-08-28** by `ADR-035`, which accepts it. The superseded clause
read:

~~§5.4.2 rules `nushell` in and explicitly rejects the Talos model.~~

The clause also cited the wrong section: §5.6.5 records that the nushell ruling
is §5.1, and that the miscitation appeared in 18 places. The body's requirement
— that `cmd/rivet` refuse an interactive invocation — is unchanged by `ADR-035`
and gets more important, not less.

### In §5.5.2.7 — the extraction argument lived in `PLAN.md`

**Changed 2026-08-29**, when `PLAN.md` was split. The argument is now §5.5.2.3;
the superseded pointer was ~~`PLAN.md` §96.3~~.

### In §5.6.6 — the section was Open

**Changed 2026-08-24**, when the maintainer ruled it, and the recommendation is
what was taken. The heading carried the word ~~Open~~, and the recommendation
was introduced as:

~~**Not ruled. The recommendation, for whoever rules it:**~~

### In §5.6.7 — the session `PATH` was unruled

**Changed 2026-08-24**, ruled and built; §5.6.8 is the ruling. The superseded
clause read:

~~**Unruled, and it is the maintainer's** (`AGENTS.md` §1)~~

### In §7.4 — the `--debug` build was a recommendation

**Changed 2026-08-24**, when the maintainer ruled it. The superseded sentence
read:

~~**A recommendation, not a ruling: the second, until an extension is cheap.**~~

### In §7.5.2 — `cmd/rivet-image` was to do the stripping

**Changed the same day it was written, 2026-08-24, by measurement.** The
superseded sentence read:

~~The natural home is `cmd/rivet-image`, which already drives `sgdisk`, `mkfs`
and `mcopy` over a pipe (`ADR-009` §3.4) and for which `strip` is one more host
tool.~~

It was written before it was checked: stripping an `aarch64` ELF needs the cross
toolchain, and the host's GNU `strip` 2.47 refuses the file. The home is
`tools/card/build.sh`.

### In §7.6 — the tier was not being decided

**Changed 2026-08-24**, ruled Tier C for both. The superseded lead-in read:

~~**The counter-argument, which is why this is not being decided:**~~

The counter-argument itself is kept in the body, because the gap it names in
`ADR-001` §4's table is still live.
