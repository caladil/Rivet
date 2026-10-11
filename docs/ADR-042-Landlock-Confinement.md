# ADR-042: Landlock Confinement — What an Application May Reach

* **Status:** Accepted
* **Date:** 2026-09-04, amended three times — twice on 2026-09-05 (§8 after first contact with hardware, §8.4 after the second, which is the one where the display came back), and on 2026-09-09 (§7, `LANDLOCK_ACCESS_NET_BIND_TCP` handled and denied outright)
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Implements:** `ADR-015` §5.1's *"Filesystem restriction — Landlock"* row
* **Code:** `rivet-core/src/confinement.rs` (the policy), `cmd/rivet-sysd/src/landlock.rs` (the ruleset), `cmd/rivet-sysd/src/kernel.rs` (where it is applied)

---

## 1. Context

`ADR-015` §5.1 named six mechanisms that together make a Rivet application a
sandboxed thing rather than a program running as a different user. Four were
built by 2026-08-31: a uid per application, every capability dropped,
`PR_SET_NO_NEW_PRIVS`, and cgroup limits. Two were not: the mount namespace and
Landlock.

The Landlock row was not deferred by choice. `ADR-024` §8.1's audit of
2026-08-22 found that the stock Raspberry Pi kernel does not have it —
`CONFIG_SECURITY_LANDLOCK` unset, `/sys/kernel/security/lsm` reading
`capability` alone — and §5.1's own text says the row *"has no kernel to run on
until Rivet builds its own."* Rivet now builds its own, so the conditional
`ADR-024` §8 set has fired and been discharged.

This ADR is what was built on top of that kernel: a **policy** computed from an
application's manifest, and the ruleset the supervisor makes of it.

**What this does NOT do**, said here so no reader has to infer it from silence:

* It is **not a mount namespace.** Every path is still there, and anything
  running as uid 0 still reaches it. What is restricted is what an application
  may *open*, not what exists. `ADR-015` §5.1's second row is still unbuilt and
  this does not build it.
* It is **not a network namespace**, still. `ADR-015` §5.1's `network` key
  promises an application with no interfaces at all, and Landlock's network
  rights are a different and narrower thing. §7 below says which one of them
  this ADR now handles — `bind` over TCP, denied outright — and which one it
  still does not.
* It is **not seccomp.** §5.1's syscall-filtering row is untouched.

The supervisor emits all three of those absences on every application launch,
because a machine that claims a sandbox it does not have is worse than one that
claims nothing.

---

## 2. The rule the policy obeys

`SPEC-03` §2, and it predates this change by a long way:

> **An application is handed what it needs and does not go and take one.**

Every row of the policy is therefore one of exactly two things:

1. **A path the machine told the application about** — `RIVET_APP_DIR`,
   `RIVET_APP_DATA`, `RIVET_APP_TMP`, `RIVET_APP_SETTINGS`, the closure the manifest declared,
   the sockets under `/run/rivet`, the device nodes a declared class expanded
   to; or
2. **A path it cannot run without and no manifest can describe** — the ELF
   interpreter, the `/etc` projection, the boot id the SDK stamps on a journal
   record, and (since 2026-09-05, §8) the image's shared typefaces.

The second kind are marked *implicit* in the source and they are the rows to
argue about, because they are the ones no author asked for.

### 2.1 The half of the rule that was left unwritten, and what it cost

*Handed* is not a figure of speech. It means a path the supervisor puts in the
child's environment or writes into `/run/rivet` for it. So the rule has a
mechanical consequence: **every variable `rivet_core::service` composes must
have a row in the policy.**

That sentence was not written down on 2026-09-04, and one variable did not have
one. `rivet_core::settings::path` moved out of the application-owned directory
on the same night (`ADR-038` §8.1), so `RIVET_APP_SETTINGS` named a file
beside `current` rather than inside `data` — and neither of the two directory
grants reached it. The supervisor told every application where its settings
were and the kernel refused it the file.

`handed_paths_are_granted_paths` in `rivet-core/src/confinement.rs` is now that
half of the rule, asserted rather than remembered, and it fails in `rivet-core`
with no kernel and no board.

---

## 3. Where the policy lives, and where the syscall does not

`AGENTS.md` §3: `rivet-core` names no platform type, and a Landlock access bit
is one. So the split is:

| | `rivet-core/src/confinement.rs` | `cmd/rivet-sysd/src/landlock.rs` |
| :-- | :-- | :-- |
| Owns | *what* an application may reach | *how* the kernel is made to believe it |
| Vocabulary | Rivet's verbs — read, list, execute, write, create, delete, truncate, reparent, device-control | `LANDLOCK_ACCESS_FS_*`, three syscall numbers, a packed struct |
| Testable | with no kernel at all | only where Landlock is |
| Reads | the manifest, and `crate::service`'s path functions | the policy, and the running kernel's ABI |

The property this buys is that the answers are asserted against the **same**
`crate::service` functions that compose `RIVET_APP_DIR`, `RIVET_APP_DATA` and
`LD_LIBRARY_PATH`, so a path that moves moves in one place (`ADR-011`).

The mapping from a Rivet verb to kernel bits is a table somebody can read rather
than a constant folded into a syscall site, and one row of it is load-bearing:
`Access::CREATE` maps to `MAKE_REG | MAKE_DIR | MAKE_SOCK | MAKE_FIFO |
MAKE_SYM` and **deliberately not** `MAKE_CHAR` or `MAKE_BLOCK`. A device node an
application made itself is a door to hardware no manifest declared.

---

## 4. The ruleset, row by row

`rivet_core::confinement::policy(volume, name, manifest)` is pure: it reads the
manifest and two path functions, touches nothing, and returns the same answer on
any machine. Whether each path *exists* is the adapter's question, and
`Presence` is how the policy says what the answer means.

| Path | Access | Presence | Why |
| :-- | :-- | :-- | :-- |
| `<volume>/app/<name>/current` | read, list, execute | Required | `RIVET_APP_DIR`. The program the supervisor is about to `exec` is inside it, so `EXECUTE` here is what lets the `execve` succeed at all |
| `<volume>/app/<name>/data` | read, list, write, create, delete, truncate, reparent | Required | `RIVET_APP_DATA`, the one writable directory (`SPEC-03` §3.3) |
| `<volume>/app/<name>/tmp` | read, list, write, create, delete, truncate, reparent | Required | `RIVET_APP_TMP`, and `TMPDIR` set to the same path (`ADR-015` §3.2B). A **sibling** of `data`, so it can be emptied without risking the history; emptied by the supervisor at every spawn and every exit. §4.3 |
| `<volume>/app/<name>/settings` | read | Optional | `RIVET_APP_SETTINGS` (`ADR-038` §8.1). §4.1 |
| each declared library's object dir | read, list, execute | Required | the same list `LD_LIBRARY_PATH` is composed from, through the same function |
| `/lib` | read, list, execute | Optional | implicit: the ELF interpreter and the image's shared objects, which no manifest key names |
| `/etc` | read, list | Optional | implicit: the read-only projection every libc resolves against (`ADR-007` §3.5) |
| `/system/share/fonts` | read, list | Optional | implicit: the image's shared typefaces. §4.2 and §8 |
| `/run/rivet` | read, list, write | Optional | the journal socket, `sd_notify`, granted peers, `display-hours` — every one a path the supervisor told it about |
| `/proc/sys/kernel/random/boot_id` | read | Optional | implicit: the boot id the SDK stamps on every journal record (`SPEC-03` §3.6) |
| `/dev/null`, `/dev/zero`, `/dev/urandom`, `/dev/random` | read, write | Optional | implicit: plain character devices a language runtime opens with no manifest to ask |
| each node a `devices` class expanded to | read, write, device-control | Optional | appended by the supervisor, from the same glob the chown walks (`ADR-011`) |

**Five things this table settles**, each of which is a decision and not an
implementation detail:

1. **`RIVET_APP_DATA` is writable and NOT executable.** Application volumes are
   mounted `exec` (`ADR-031` §5), so before this a program could write a binary
   into its own data directory and run it. `PR_SET_NO_NEW_PRIVS` stopped that
   binary *gaining privilege*; this stops it being run at all. **`RIVET_APP_TMP`
   carries the same rights and the same exception**, and the exception matters
   more there: scratch is where untrusted bytes land.
2. **`RIVET_APP_DIR` is executable and NOT writable.** `ADR-015` §7 makes an
   install an unpack-and-swap; a program that could rewrite its own version
   directory would be a program that installs itself, and launch-time
   verification (`ADR-015` §4.2) would be verifying whatever it had just
   written.
3. **A `Required` path that is absent is a fault, not a smaller ruleset.** Its
   own installed files and its own data directory are things the machine
   promised. If one is missing the application is broken, and starting it
   confined-but-differently would hide that behind an `EACCES` inside the
   program.
4. **An `Optional` path that is absent is dropped**, which leaves the
   application *more* confined rather than less — the safe failure direction.
   This is measured rather than theoretical: under `tools/qemu/run.sh` the root
   is an initramfs with no devtmpfs and there is **no `/dev/null` at all**
   (`AGENTS.md` §10.5, three defects). A policy that required one would refuse
   to start every application in the emulator.
5. **`ENOENT` is the only "not there" answer taken as one.** An `EACCES` or an
   `EIO` while opening a policy path is a machine that could not tell, and
   reading it as absence is `AGENTS.md` §7's defect exactly.

**An image service that runs under its own identity is confined by the same
table** (`ADR-060`). `rivet_core::confinement::image_service_policy(program,
volume, name, supervisor, devices)` has no manifest to read, so it composes the
rows that do not come from one — `data`, `tmp`, `settings`, the routing table
and every implicit platform row — through the same two functions `policy`
calls, and a row added for applications is therefore a row added for it. Its
`devices` are the classes the service's definition declares, read exactly as a
manifest's are: declaring `display` or `gpu` earns Mesa's device lookup, which
is how the compositor renders (`ADR-058` §5.6), and the nodes a class expands
to are appended by the supervisor for both kinds of program. It differs in
exactly one row and omits the manifest's:

| Path | Access | Presence | Why |
| :-- | :-- | :-- | :-- |
| the program, e.g. `/system/bin/rivet-proxyd` | read, execute | Required | the file the supervisor is about to `exec`. **The file and not `/system/bin`**, which holds the supervisor and every other image program |
| `<volume>/app/<name>/current` | — | — | not granted: an image service has no installed version directory |
| declared libraries, class-matched addons | — | — | not granted: nothing declared them |

`the_image_proxy_reaches_what_the_carried_proxy_reached` holds the two
functions to that statement, row for row, against the manifest the proxy
shipped with as an application.

### 4.1 The settings file is read-only, and that is a statement about views

`ADR-007` §3.6.2: *a file that can be written and have effect is not a view.*
The settings file is a view of the `[applications.<name>]` table, and the
document is the authority (`ADR-038` §7). It is written `0644 root:root`, so the
application's uid could not write it anyway — the policy says so as well,
because the policy is the half that is still true if the mode is ever got wrong.

### 4.2 `/system/share/fonts` is a leaf, not the tree

The grant is the one directory, and `/system/bin`, `/system/ssl`,
`/system/trust` and `/system/share/zoneinfo` remain refused.
`nothing_outside_the_declaration_is_granted` asserts each of them by name.

**A directory rather than three file grants**, and the trade is worth stating:
a file grant per face would be narrower by exactly nothing — all three faces are
in that directory and there is nothing else in it — and it would cost a *fourth*
face becoming a silent `EACCES` on a board, which is the failure this row exists
to repair, reintroduced one file along. The guard on the other side of that
trade is `rivet_core::etc`'s `the_font_directory_holds_every_system_face`,
which fails the day a face is added outside the granted directory.

`rivet_core::etc::SYSTEM_FONT_DIR` is the one spelling.
`cmd/rivet-image::system::FONT_DIR` is an alias for it and
`cmd/rivet-launcher::text` no longer carries its own copies, because a path the
kernel now enforces is one that must have a single spelling: the failure of a
second one is not a missing file, it is `EACCES` on a board with nothing on the
screen.

### 4.3 `/tmp` is not here, and 1777 is the reason

`/tmp` and `/dev/shm` are world-writable **by mode**, so Unix permissions never
separated two applications there at all. Any application could read another's
scratch file, replace it, or plant a symlink where a root process was about to
write — which is not a hypothetical shape in this project: the security audit of
2026-09-03 found a CRITICAL symlink truncation of exactly that form, and
`ADR-038` §8.1 moved a file out of an application-owned directory because of it.

So `/tmp` stays out of the ruleset, and that part of this section has not
changed.

**What HAS changed is that the machine now provides the alternative itself.**
Since 2026-09-07 every application is granted `/state/app/<name>/tmp`, `0700`
and owned by its identity, with **`TMPDIR` and `RIVET_APP_TMP` both pointed at
it** — `ADR-015` §3.2B is the contract. Scratch space is therefore private to
one uid by mode *and* by ruleset, which is a strictly better answer than
`/tmp` ever gave, and a library that reaches for the POSIX convention now finds
somewhere real.

**Leaving `TMPDIR` unset was the defect, and calling the failure discoverable
was the mistake.** A program that hardcodes the literal `/tmp` still fails with
`EACCES`, and that is intended. But almost nothing hardcodes `/tmp`; it consults
`TMPDIR` and falls back through a list, and with the variable unset **every
entry in that list failed at once** — `/var/tmp` and `/usr/tmp` are not in the
image, `/tmp` is refused, and the working directory is the read-only `current`.

What surfaces then is not a refusal naming a path. It is whatever the library
says when it runs out of options: a whole-database restore on the test board
answered `SQLITE_CANTOPEN`, which names nothing, reads like a corrupt file, and
took five separate observations to trace back to this ruleset. **A journal line
listing what was granted does not help a reader who has no reason to think the
problem is confinement at all.**

---

## 5. The ABI, and the difference between a missing right and a missing Landlock

Landlock's ABI has grown, and a kernel that predates a right cannot **handle**
it: `landlock_create_ruleset` answers `EINVAL` for a `handled_access_fs`
carrying a bit the kernel does not know. So the handled set is intersected with
what the running ABI supports.

| ABI | Adds | Rivet verb |
| :-- | :-- | :-- |
| 1 | the original thirteen | read, list, execute, write, create, delete |
| 2 | `FS_REFER` | reparent |
| 3 | `FS_TRUNCATE` | truncate |
| 5 | `FS_IOCTL_DEV` | device-control |

**ABI 0 is refused and the application does not start.** The refusal names the
errno and says why:

> a Rivet ships its kernel and its image together (`ADR-018` §5.2), so a machine
> without Landlock is a broken build of Rivet's own kernel and **not a
> configuration an operator chose**.

That is the whole argument for refusing rather than degrading. On a general
distribution, "this kernel has no Landlock" is a fact about the user's machine
and refusing to run would be arrogant. On a Rivet it is a fact about a build
this project produced, and starting the application anyway would keep a promise
the journal has already made in the same breath — `ADR-015` §5.1.1's argument
about the uid, in a new place.

**An older-but-present ABI is a named degradation, not a refusal.** The minimum
is **1**, because every verb the policy uses for a path — read, list, execute,
write, create, delete — is in Landlock's first release. The three later ones are
refinements whose absence widens the sandbox in a way the code can name exactly,
so `unhandled_words` reports them **by name** on the launch record:

```
"not_enforceable_on_this_kernel":"reparent, truncate"
```

Not a boolean and not a count. An operator reading *"truncate"* knows that a
file inside a read-only grant can still be emptied, which is a specific thing to
reason about; *"degraded"* is not.

Two smaller decisions inside this:

* **The ABI is asked of the kernel at every spawn**, never cached. `AGENTS.md`
  §5: the kernel is the authority for system state. An A/B update replaces the
  kernel under a running `/state`, and a value read once at boot would answer
  for the wrong one.
* **A grant whose every verb is unhandled is skipped rather than added.**
  `landlock_add_rule` with an empty mask is `EINVAL`, and the honest reading is
  that there is nothing to say about that path on that kernel —
  `unhandled_words` is where an operator learns it.

### 5.1 The three calls, and which side of the `fork` each is on

| Call | Where | Why there |
| :-- | :-- | :-- |
| `landlock_create_ruleset(NULL, 0, VERSION)` | the parent, before every spawn | it is a **question**, and its verdict is never cached |
| `create_ruleset(attr)` + `add_rule` per grant | the parent, **before** `fork` | each rule needs a directory *opened*, and the child between `fork` and `exec` may not allocate the `Vec` that would take |
| `landlock_restrict_self(fd, 0)` | the **child**, before `exec` | it restricts the calling thread and everything it later execs; in the parent it would confine the supervisor |

That last row is the failure this feature is most likely to have, and it is why
`restrict_self` is a free function taking a raw descriptor rather than a method
on anything: there is nothing that could be called on the wrong process by
accident and still compile.

**Ordering against the rest of the privilege drop** — `confine()`, then this,
then `drop_privilege` — and both edges matter. After `PR_SET_NO_NEW_PRIVS`,
because `landlock_restrict_self` answers `EPERM` to a caller with neither that
bit nor `CAP_SYS_ADMIN`; setting it first is the precondition, not an
optimisation. Before `setuid`, which is not required and is chosen anyway: the
ruleset is inherited across `execve` and can never be removed, so the narrowest
window between "still root" and "confined" is the one with nothing in it that
could fail *after* the drop and leave a child running unconfined as its own uid.

---

## 6. Proving it, and why one half of the proof is worthless alone

**A test that only shows the refusal cannot tell a working ruleset from a
missing file.** That is not a caution, it is the diagnosis of §8: a ruleset that
refused everything it was supposed to refuse *and also* the three files the
launcher needed passed every check this project had and took a display down.

So every proof of this feature has two halves and a control.

### 6.1 In `cargo test` — the uid is held constant

`cmd/rivet-sysd/src/landlock.rs`'s
`a_confined_child_is_refused_a_path_an_unconfined_one_opens` forks **two**
children of one process: same binary, same boot, same uid, and only the second
one calls `restrict_self`. It asserts four things, and the two easy ones are
what make the other two mean anything:

1. the unconfined child writes the granted path;
2. the unconfined child reads the path outside the grant — *without this the
   confined run below would prove nothing about Landlock*;
3. the confined child still writes the granted path — *the ruleset did not break
   the application*;
4. the confined child cannot read the outside path.

It does not skip on a kernel without Landlock. A machine that cannot enforce
this ADR is one where every application is refused, and a green suite there
would say otherwise.

### 6.2 In QEMU — the kernel is Rivet's own

`tools/qemu/adr-042-proof.sh`, against `ADR-024`'s kernel — which is the thing
neither the container nor the workstation can settle, because the question is
whether *this project's* kernel build carries Landlock and at what ABI.

It boots one supervised, signed, confined application —
`tools/qemu/testapp` under a name that selects its `confinement` behaviour —
which opens both classes of path in one process and prints the errno for each.
Run on 2026-09-05, verbatim:

```text
confined: RIVET_APP_DIR=/state/app/confinement-probe/current
confined: RIVET_APP_DATA=/state/app/confinement-probe/data
confined: RIVET_APP_SETTINGS=/state/app/confinement-probe/settings
confined: HANDED  /system/share/fonts/Geist.ttf: READ 169056 byte(s)
confined: HANDED  /state/app/confinement-probe/data: LISTED, 0 entr(ies)
confined: HANDED  /state/app/confinement-probe/settings: READ 250 byte(s)
confined: HANDED  /state/app/confinement-probe/current/manifest.toml: READ 276 byte(s)
confined: OUTSIDE /state/config/machine.toml: refused, Permission denied (os error 13)
confined: OUTSIDE /system/bin/rivet-sysd: refused, Permission denied (os error 13)
confined: OUTSIDE /state/app: refused, Permission denied (os error 13)
confined: OUTSIDE /tmp: refused, Permission denied (os error 13)
confined: VERDICT handed 4/4 reachable, 4/4 refused with EACCES (0 of them prove nothing) - BOTH HALVES
```

and the supervisor's own line from the same boot:

```text
"landlock_abi":6,
"granted":"/state/app/confinement-probe/current (read+list+execute),
 /state/app/confinement-probe/data (read+list+write+create+delete+truncate+reparent),
 /state/app/confinement-probe/settings (read), /lib (read+list+execute), /etc (read+list),
 /system/share/fonts (read+list), /run/rivet (read+list+write),
 /proc/sys/kernel/random/boot_id (read), /dev/null (read+write), /dev/zero (read+write),
 /dev/urandom (read+write), /dev/random (read+write)",
"not_enforceable_on_this_kernel":"none: this kernel handles every right the policy uses"
```

**Rivet's kernel answers ABI 6**, so every right the policy uses is enforced and
the degradation path of §5 is not exercised on it.

Three things about that run are deliberate and each was got wrong once first:

* **Every path in the refused list EXISTS on the guest.** `run.sh --document`
  plants the document; the supervisor's own binary is in the initramfs. A
  refusal that came back `ENOENT` would prove nothing at all, so the probe
  counts those separately and the verdict says how many.
* **The probe `open`s and does not `stat`.** Landlock governs opening a file and
  opening a directory; it does not govern `stat(2)`. The first draft used
  `std::fs::metadata` and reported all four paths REACHED on a machine where the
  ruleset was working perfectly — the same species of mistake as the regression
  it was written for.
* **The control is on the same boot**: the supervisor read
  `/state/config/machine.toml` and acted on it, so the `EACCES` above is a
  ruleset and not an unreadable file. It is a *weaker* control than §6.1's,
  which holds the uid constant, and both are in the record because neither is
  the other.

### 6.3 The proof was run against the defect, and it caught it

`AGENTS.md` §4: a regression test that cannot fail proves nothing. With the two
grants of §8 removed and nothing else changed, the same script reports:

```text
confined: HANDED  /system/share/fonts/Geist.ttf: NOT READABLE - Permission denied (os error 13)
confined: HANDED  /state/app/confinement-probe/settings: NOT READABLE - Permission denied (os error 13)
confined: VERDICT handed 2/4 reachable, 4/4 refused with EACCES (0 of them prove nothing) - NOT PROVEN
```

— the board's own errno, reproduced under the emulator, and note that the
**refusal half was still 4/4**. A proof that reported only that number would
have called this defect a pass.

### 6.4 What the QEMU proof needed that the harness could not do

Two things, both now built, and both worth knowing about before extending this:

* **`run.sh --app` leaves an application with no signed header**, and says so.
  `cmd/rivet-sysd`'s `verify` refuses every launch without one (`ADR-015` §4.2),
  which is correct and which also means the supervisor *defines* the service and
  never starts it — a machine nothing can be observed on. `--app` now takes an
  optional fourth field, the retained header, and `tools/qemu/fixture-signer`
  writes it and the matching trust store with the **real** `Builder`,
  `Ed25519Signer` and `TrustStore` rather than composing either format by hand
  (`ADR-011`; `run.sh`'s own `--trust` help already refused to be a second
  producer of a security-relevant file). Its seed is thirty-two identical bytes
  in a tracked file and is not a credential — no machine outside the harness
  trusts the key it derives.
* **`run.sh --font`** packs a typeface at `rivet_core::etc::SYSTEM_FONT`, which
  is the same imitation `/system/bin` already is. Nothing draws in QEMU; the
  file is there so that a READ can succeed instead of returning `ENOENT`.

---

## 7. Network rights: `BIND_TCP` is handled and denied; `CONNECT_TCP` stays out of scope

Landlock ABI 4 added `LANDLOCK_ACCESS_NET_BIND_TCP` and
`LANDLOCK_ACCESS_NET_CONNECT_TCP`. `cmd/rivet-sysd/src/landlock.rs::build`
handles the first, on ABI 4 and later, and adds no rule for it — which is the
whole implementation of *deny*: Landlock refuses everything a ruleset handles
and nothing allows. **An application cannot bind a TCP port at all, on any
kernel new enough to have the right.**

### 7.1 Why `bind` has no legitimate case to carve out

The gap this closes is the maintainer's own case: application A holds `8080`;
application B asks the supervisor for it. The supervisor's arbitration was
beside the point while either application could bind the port directly,
behind its back. `SPEC-03` §3.1's listening-socket contract is why there is
nothing to carve out for: the supervisor pre-binds every socket a manifest
declares and hands the descriptor over — `LISTEN_FDS`/`LISTEN_FDNAMES`
(`ADR-017` §3.4) and `rivet_sdk::listen` — so an application that binds its
own TCP port is doing something its manifest never declared and the
supervisor never arbitrated.

**The denial does not read the manifest's `network` key at all, and that is
deliberate.** `bind` is refused for every confined application regardless of
whether it declares `network = "none"` or `network = "group:…"` — those
values are about the network namespace §7.2 below still has not built, and a
Landlock right that only fired for `"none"` would be exactly the
two-promises-in-one-key mistake §7.2 exists to avoid, pointed at itself.

### 7.2 Why `CONNECT_TCP` is a different decision, not an omission

`CONNECT_TCP` is not handled here, and staying that way was weighed against
`BIND_TCP` rather than left as a leftover:

* **`BIND_TCP` could be denied to everything because no application has a
  lawful reason to bind at all.** Outbound connections have no equivalent
  answer: most applications need *some* host reachable, and deciding which
  hosts and ports needs a vocabulary the manifest does not have. Handling
  `CONNECT_TCP` today, with no policy row to narrow it, would deny every
  application every outbound TCP connection — a functional regression wearing
  the costume of a security feature, and inventing the vocabulary to avoid
  that is a separate decision this change does not make.
* **`ADR-015` §5.1's `network` key still promises a network namespace, not a
  Landlock right, and this does not quietly make `network = "none"` mean a
  connect denial** — that would be two different promises wearing one key.
  `rivet-core/src/confinement.rs`'s module header says exactly this. Even a
  built `CONNECT_TCP` denial would be a different and weaker thing than the
  namespace §5.1 promises: it would say nothing about UDP, raw sockets, unix
  sockets or anything else that `network = "none"`'s *no interfaces at all*
  covers. Half-honouring the key is worse than not honouring it, because the
  journal would then have to say something true and complicated where today
  it says something true and simple: *"no network namespace, so
  `network = "none"` and `network = "group:…"` mean nothing."* When the
  namespace is built it will mean what §5.1 says; until then this row stays
  wholly unbuilt and the supervisor keeps saying so.

### 7.3 `bind` means TCP, and that is Landlock's own boundary

Landlock's network access vector says nothing about UDP. `rivet-mdnsd`'s
bound `5353` and `rivet-netd`'s netlink socket are both untouched by this
right — not because either was considered and excused, but because neither is
the kind of thing this Landlock right can see at all.

---

## 8. First contact with hardware, 2026-09-05 — and the amendment it forced

**This section is worth more than the rest of the document**, because it is the
only part written after the design met a board.

A card carrying the ruleset was installed and booted. The journal, verbatim:

```text
rivet-launcher: NO SYSTEM FONT - /system/share/fonts/Geist.ttf: Permission denied (os error 13) — the image ships no system font
rivet-launcher: NO ICONS - /system/share/fonts/MaterialIcons.ttf: Permission denied (os error 13) — the image ships no system font
rivet-launcher: WARNING - /state/config/machine.toml could not be read: Permission denied (os error 13). This launcher is running as though the document were EMPTY
rivet-launcher: 0 application(s) offered:
rivet-launcher: WARNING - /state/config/machine.toml did not parse: configuration key `version` is required and is absent
```

No text on the screen, no applications in the menu, and `auto-run =
"aura-renderer"` never honoured — so a wall that shows a dashboard showed an
empty launcher. The document was fine: `version = 1`, mode `0644 root:root`, and
the supervisor read it on the same boot. Every one of those `EACCES` was this
ruleset.

**A third consequence is the dangerous one.** The launcher's own warning says
that a `[kiosk] locked = true` in a document it cannot read is **not in force**.
A confinement change that silently disables the kiosk lock is a security
regression wearing the costume of a security feature, and it would have shipped
looking like a hardening.

### 8.1 Two defects, and they are not the same defect

The tempting reading is *"the policy is too tight, widen it."* That is right
about one of the two and wrong about the other, and taking them together would
have produced the worst available outcome: a carve-out for the one application
that is also the display arbiter, which sets the precedent that any application
with a good reason gets one.

| | The typefaces | The document and `/state/app` |
| :-- | :-- | :-- |
| What was refused | `/system/share/fonts/*.ttf` | `/state/config/machine.toml`, a walk of `/state/app` |
| Who was wrong | **the policy** | **the launcher** |
| Why | it is image content the machine provides, on the same terms as `/etc` and `/lib` — and `rivet-sdk`'s `text` module *publishes* the path to every application | the operator's document is not something the launcher was handed; `SPEC-03` §2 forbids exactly this, and the launcher's own source said so |
| Fix | grant the leaf directory (§4.2) | a new verb on the kiosk socket (§8.2) |

**The fonts argument, on its own merits.** `ADR-015` §5.1's row was written
before `SYSTEM_FONT` existed. When `confinement.rs` excluded `/system` on
2026-09-04 it gave a reason — *"nothing an application is handed lives in
it"* — and that clause was already false: the typeface had lived there since
2026-09-01 **specifically so that several programs could share it** (the splash,
the launcher, every kiosk template), and `rivet_sdk::text::SYSTEM_FONT` is a
published constant of this platform's own client library. The SDK hands the path
over; the policy then refused it. That is the platform contradicting itself, not
an application taking something.

The alternative — every drawing application ships its own face as a package
asset — is available and is what Aura does with Source Code Pro, and it is the
right answer for an application with a *design opinion*. It is the wrong answer
for the fallback face, because the whole point of a system typeface is that a
board looks like itself before anything is installed on it. A splash screen
cannot ship an asset; it is in the image.

**And the third defect the same reading exposed** was not on the board at all:
`RIVET_APP_SETTINGS` (§2.1). Nothing had noticed because no application had yet
been restarted under both the new settings path and the new ruleset. It is the
same class as the fonts — a handed path with no row — and it is fixed the same
way.

### 8.2 `offer`: the launcher stops reading the document

`cmd/rivet-launcher`'s `offered` module read the document and walked
`/state/app`, and its own header said what that was:

> a `get kiosk` on the control socket is where this belongs. **That verb does
> not exist yet, and this is written to be deleted when it does.**

The verb now exists. `rivet_core::kiosk::Request::Offer` is a fifth verb on the
narrow socket the launcher is *granted* by declaring
`connects = ["rivet-sysd:kiosk"]`, and it answers with the `[kiosk]` mode, the
lock, the auto-run, and the offered set.

Both halves were already on the supervisor and were already the authority. It
parses the `[kiosk]` table at boot and on every reload; `display_offered` is
*enabled AND declares a display AND is not the launcher*, computed from the
definitions it holds — the same rule the launcher was applying to the same two
facts, and the same set `select` validates against.

So the verb removes a **second reader**, which is what `ADR-011` wanted anyway
and what `PLAN.md` §107.8 was spent on elsewhere. Landlock is what made the debt
due.

Three consequences worth naming:

* **A menu can no longer lie.** The launcher used to be able to offer something
  `select` would then refuse, in the window between a write and a reload. One
  source, one answer.
* **`.rulings/` R-137's fail-open is unreachable from the launcher, and is not
  overturned.** R-137 ruled that a document which will not parse leaves the
  kiosk *unlocked*, choosing reachability over a lock that survives a corrupt
  file. That ruling still stands and the supervisor still applies it. What is
  gone is a *second* reader that could fail differently from the first.
* **`unknown` had to survive the wire.** A supervisor that has not been told
  what the document says reports `unknown`, not `headless`
  (`KioskMode::unknown`), and a launcher that read the two the same way would
  draw a menu on a machine that might be locked. The launcher now says so on the
  console instead.

An unanswered `offer` is still a fail-open — an empty menu, unlocked, with the
reason printed — and that is stated rather than discovered: `ADR-018` §8 keeps a
machine repairable, and there is no longer a document to fall back to reading.

### 8.3 What this section is really about

The ruleset was correct on 2026-09-04 in the narrow sense: it refused everything
it was designed to refuse. What it did not have was **anything that checked the
other direction** — that an application could still open what it was handed.
Every test in the change asserted a denial. The one property nobody asserted is
the one that broke a display.

`§6` is the correction, and the rule generalises past Landlock: **when a change
takes something away, the test that matters is the one that proves what is
left.**

### 8.4 Second contact, 2026-09-05 — the display came back, and the thing that looked like confinement was a revoked lease

**§8.2's launcher was deployed onto the same board later the same day, and the
first three of the four things it had to do worked.** Its own records:

```text
rivet-launcher: this launcher is 0.1.68; the machine says it is 0.1.68
rivet-launcher: TOOK THE DISPLAY - /dev/dri/card0 connector 42 crtc 112 mode 1920x1080, master explicitly
rivet-launcher: type from /system/share/fonts/Geist.ttf
rivet-launcher: the supervisor answered `offer` on attempt 1: locked false, auto-run aura-renderer
rivet-launcher: 2 application(s) offered: aura-renderer, kiosk-demo
rivet-launcher: leased the display to aura-renderer (lease 1), still master
```

Then `aura-renderer` failed, and the sentence it failed with sent a whole
session looking in the wrong place:

```text
aura-renderer: no display was handed over: a display was handed over and could not be driven:
a leased display: no connected output with a mode — is a cable in?. The channel is working —
this is about the display behind it
```

**The cable had not moved**, and the launcher had set a 1920x1080 mode through
that same connector one millisecond earlier. The obvious reading was that the
ruleset in this document had taken the display down a second time.

**It had not.** Eleven seconds later, on the same boot, under the same ruleset,
with the same uid and the same `ENFORCED` records above it, the same binary
said:

```text
aura-renderer: drawing at 1920x1080
aura-renderer: the screen says: a leased display connector 42 crtc 112 mode 1920x1080,
master implicitly, by opening it (SET_MASTER said 13), double-buffered, page-flipped at vblank
aura-renderer: page flips are accepted, so frames are whole and the loop is paced by the panel
```

That pair is the control §6.1 asks for, delivered by the board rather than by a
harness: **two runs of one binary, one boot, one uid, one ruleset, one cable,
and only the provenance of the descriptor different.** Landlock is not what
failed, and neither is the launcher rebuild — `cmd/rivet-launcher/src/drm.rs`,
where the lease is cut, is byte-for-byte what it was in `0.1.66`.

#### 8.4.1 What actually happened, from the journal's own ordering

1. The launcher asked for `aura-renderer`. The supervisor answered `Taken` and
   `crate::main`'s `grant_display` opened the application's display channel and
   **wrote the lease into it** — which §106.2 does deliberately *before* the
   spawn, so the message is buffered and waiting when the child first reads.
2. **The application was not started.** It had already exhausted its restart
   policy earlier in the boot and been marked failed, so the convergence that
   follows a `select` started nothing.
3. One health poll later the launcher saw nothing running, came back to the
   menu, and **revoked the lease**. It did not tell the supervisor, so the
   selection stood and the buffered descriptor stayed where it was.
4. Forty seconds afterwards a document reload cleared the failure and the
   supervisor started the application — which read the descriptor that had been
   waiting since step 1, and which was by then a **revoked** lease.

A revoked lease is emptied of its objects. The descriptor is real, the ioctls
answer, and what they describe is a display with no outputs — which is why the
application's own words were *"the channel is working"*. It is the one condition
the message could not name, because there was only ever one message.

#### 8.4.2 Three repairs, and only one of them is about the display

* **`rivet_sdk::outputs`** counts the funnel — listed, described, connected,
  with a mode, with an encoder — and says which step closed. A lease that lists
  no connector now reports itself as a lease and rules the cable out in the
  same sentence. It is a separate module for `crate::flip`'s reason: `screen.rs`
  links `libdrm` and cannot be compiled on the development host, so a message
  that lived there could never be tested, and this one had never been read by
  anything but a person on a bad afternoon.
* **`Supervisor::return_display` closes the display channel.** A descriptor
  written for a selection that has been withdrawn is a descriptor nobody may
  draw through, and `Kernel::close_display` is the call `release` already makes
  for the same fact.
* **The launcher tells the supervisor.** Its health poll had two places where it
  revoked a lease, cleared `handed_to` and said nothing; both now send `return`.
  `take_back` is *take the scanout back* and is still what §108.1's LONG BACK
  wants, because that question is asked over an application that is still
  running.

#### 8.4.3 What this section does NOT settle

**Which step of the funnel a revoked lease closes at has not been measured.**
The reasoning above says a revoked lease lists no connectors, and the
counts that would prove it did not exist when the failure happened — that is
what §8.4.2's first repair is for, and the next occurrence will say so in its
own words instead of being reconstructed from an ordering.

**And the repairs themselves are proven by `cargo test` and by nothing else.**
`ADR-026` §4: the emulator has no display pipeline under `-M virt`, so it can
prove a ruleset refuses a path and cannot prove a modeset, a lease, or a
revocation. A board is what closes this, and §10 says so.

---

## 9. Consequences

* `ADR-015` §5.1's *"Filesystem restriction"* row is **built**, on Rivet's own
  kernel, at ABI 6. §5.1's own text — *"has no kernel to run on until Rivet
  builds its own"* — and `ADR-024` §8.1's *"the stock Pi kernel has no Landlock
  so the sandbox is not enforced"* are both superseded for a Rivet-built kernel;
  both remain true of the stock Raspberry Pi OS kernel, which is what they were
  measured against.
* `ADR-015` §5's mount-namespace, network-namespace and seccomp rows remain
  unbuilt, and the supervisor says so on every launch.
* **An application cannot `bind` a TCP port at all, on any kernel reporting
  Landlock ABI 4 or later.** §7. `LANDLOCK_ACCESS_NET_CONNECT_TCP` is
  unaffected and so is every UDP right — Landlock's network rights say
  nothing about UDP — so `rivet-mdnsd`'s bound `5353` and `rivet-netd`'s
  netlink socket are untouched.
* **`/tmp` is not writable by an application.** §4.3. A program that hardcodes
  it fails with `EACCES`, discoverably, and `RIVET_APP_DATA` is the answer.
* **A machine whose kernel lacks Landlock starts no applications at all.** §5.
  That is a broken build of Rivet's own kernel, and the refusal names the errno.
* **Every new environment variable handed to an application needs a policy
  row**, and `handed_paths_are_granted_paths` fails in `rivet-core` if one is
  added without it.
* **An application is not granted a system program, `rivet-fleetd` included.**
  The policy has no row for `/system/bin` or `/state/git`, so a confined
  `rivet-fleetd` cannot run git; fleet-as-code's git runs in `rivet-mcpd` as
  control-plane operations it asks for instead, and the table in §4 is
  unchanged by it (`ADR-044` §5.5, `.rulings/` R-187 D2).
  `cmd/rivet-sysd/src/landlock.rs`'s
  `rivet_fleetd_confined_by_its_own_policy_cannot_run_git` builds that
  application's ruleset and asserts `execve` of git answers `EACCES`, so a row
  that granted it fails a test.
* **`cmd/rivet-launcher` no longer depends on `rivetos-config`**, because it no
  longer parses anything.
* **`rivet_core::kiosk::ANSWER_MAX_BYTES`** replaces the launcher reading a
  reply back at `REQUEST_MAX_BYTES + 1`. Measured: a `select` refusal quotes the
  whole offered set and is 253 bytes on a five-application machine, so a sixth
  would have delivered a sentence that stopped mid-word.
* **A display channel does not outlive the selection that opened it** — §8.4.2.
  `Supervisor::return_display` closes it, which is what stops a descriptor
  written for one start being read by a later one.
* **A lessee that cannot find an output says which step of the funnel closed**,
  and only one of the five answers is about a cable — `rivet_sdk::outputs`.
  This is not a Landlock consequence; it is in this document because a
  misleading sentence about a cable is what made a working ruleset look like
  the defect for most of a day.

---

## 10. What is still unproven

~~**The launcher itself has not been run on a confined board.** §8's fix is
proven by `cargo test`, and by a QEMU boot in which a *stand-in* application
reads its fonts and its settings and is refused the document. It is **not**
proven by a Raspberry Pi drawing a menu, and until it is, the honest statement
is that a confined launcher is *expected* to work and has not been watched
doing it (`AGENTS.md` §6).~~

~~**The `offer` verb has never crossed a real socket.** Both ends are tested
against `rivet_core::kiosk`'s own render and parse, and the supervisor's
accessor is tested against the set `select` validates. Nothing has yet written
`offer\n` into `/run/rivet/kiosk` on a machine.~~

~~**No display application other than the probe has been started under the
ruleset.** `aura-renderer` links Mesa and `libdrm` and reaches a DRM node
through a class the policy appends from a glob; that path is tested and has
not been observed drawing while confined.~~

**All three were closed on 2026-09-05 by the boot §8.4 quotes**, and they are
struck rather than deleted because each named exactly the measurement that
settled it. The confined launcher took `/dev/dri/card0`, read `Geist.ttf`,
wrote `offer` into `/run/rivet/kiosk` and was answered on the first attempt; the
confined `aura-renderer` drew at 1920x1080 through a lease and page-flipped at
vblank. **`ADR-015` §5.1's filesystem row is now enforced on hardware with the
display working through it**, which is the sentence this whole document was
waiting for.

What remains unproven, restated for what is true now:

* **§8.4.2's three repairs have been run by `cargo test` and by nothing else.**
  The emulator cannot cut a lease (`ADR-026` §4), so the sequence they are
  about — a lease written to a channel, revoked, and read later — cannot be
  reproduced anywhere but on a board. What closes it: install a build carrying
  them, let the launcher hand `aura-renderer` the screen and take it back, and
  read the journal for the `gave the screen back` line and for the absence of a
  second delivery.
* **A revoked lease's own shape has not been read.** §8.4.3.
* **Nothing has been observed under a DEGRADED Landlock ABI.** Rivet's kernel
  answers 6, so §5's `unhandled_words` path has never fired on any machine and
  its sentence has never been read by anybody.
* **§7's `bind` denial is proven by `cargo test` and by
  `tools/qemu/adr-042-net-proof.sh`, and by nothing else.** The unit test
  forks two children of one process, one uid, and confines only the second —
  the same control §6.1 uses for the filesystem half. The QEMU proof boots a
  confined application over the real control plane and reads both the refused
  bind and `rivet get status` for the confined application and for
  `rivet-netd` and `rivet-mcpd`, none of which are confined. **Neither has run
  on a Raspberry Pi.** What closes it: install a build carrying this ruleset,
  confine an application that tries to bind a TCP port, and read the errno the
  board's own kernel returns — `AGENTS.md` §6's rule applies here exactly as
  it did to §8's launcher: "it works in QEMU" is not "it works".

---

## History

### 2026-09-07 — §4.3 promised a remedy and left it to the application

Superseded text:

> **What it costs, because something always does:** a program or a library that
> hardcodes `/tmp` fails with `EACCES` rather than working. That is the intended
> outcome and it is discoverable — the refusal names the path, and the journal
> line at launch lists everything the application *was* granted, so the remedy
> (`RIVET_APP_DATA`, or `TMPDIR` pointed at it) is one line away.

**Why it changed.** The remedy was correct and nobody was positioned to apply
it: the paragraph left `TMPDIR` to the application, and an application that has
that has not hit this failure has no reason to set one. The supervisor sets it now
(`ADR-015` §3.2B).

The half worth keeping is the half that was wrong. *"It is discoverable — the
refusal names the path"* held only for a program that spells `/tmp` literally.
For the ordinary case — a library consulting `TMPDIR` and falling back through a
list — every candidate failed and the error named no path at all. **A prediction
about how a failure will present is a claim, and this one was tested by a
restore on hardware and found false.**

### 2026-09-09 — §7 was "out of scope, on purpose"; `BIND_TCP` is now handled

**Superseded text keeps its strike-through.** `tools/docs/claims.py` masks
`~~…~~` spans before it resolves a citation or checks a path, so struck text
is how a dead claim tells the checker it is dead. Un-striking it here would
put every retired claim back under audit.

The whole of §7 as first written:

~~Landlock ABI 4 added `LANDLOCK_ACCESS_NET_BIND_TCP` and
`LANDLOCK_ACCESS_NET_CONNECT_TCP`. They are not handled by this ruleset.~~

~~`ADR-015` §5.1's `network` row promises a **network namespace** — an
application with `network = "none"` has *no interfaces at all*, which the ADR
notes is stronger than Android's group-membership trick. Landlock's network
rights are a different and weaker thing: they govern two syscalls over TCP,
say nothing about UDP, raw sockets, unix sockets or anything else, and would
leave `network = "none"` meaning *"cannot bind or connect over TCP"*.~~

~~Half-honouring a manifest key is worse than not honouring it, because the
journal would then have to say something true and complicated where today it
says something true and simple: *"no network namespace, so
`network = "none"` and `network = "group:…"` mean nothing."* When the
namespace is built it will mean what §5.1 says. Until then this row stays
empty and the supervisor keeps saying so.~~

**Why it changed.** The maintainer named the gap a must-have: application A
holds a TCP port; application B asks the supervisor for it; the supervisor's
arbitration was beside the point while either could bind the port directly,
behind its back. `BIND_TCP` closes exactly that gap and nothing else was
asked to change — `CONNECT_TCP` and the network namespace are still both
unbuilt, for the reasons §7.2 now gives at the length they deserve rather
than as one shared paragraph.

The argument that was correct in the original text **stays in the body,
restated rather than struck**: half-honouring the `network` manifest key
would be worse than not honouring it, a built `CONNECT_TCP` denial is a
different and weaker thing than the namespace §5.1 promises, and the
supervisor is right to keep saying the row is unbuilt until the namespace
itself exists. What changed is that `BIND_TCP` no longer needs that argument
made on its behalf: unlike `CONNECT_TCP`, it can be denied to *everything*
without reference to what any manifest says, because no application has a
lawful reason to bind a port at all once the supervisor pre-binds every
socket a manifest declares (`SPEC-03` §3.1). That asymmetry — one right with
a legitimate blanket denial, one without — is what §7.1 and §7.2 now argue
separately, where the original single section argued only the harder half.
