# ADR-015: Application Packaging and Sandboxing

* **Status:** Accepted — ruled 2026-08-22. Enforcement is staged; §6 states what
  is live at which stage and what must be reported as unenforced.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

**Rivet has a second purpose beyond being a well-designed base, stated by the
maintainer 2026-08-22: if he wanted to spin up a fast little server written in
Rust and get it running, Rivet would be the thing he reached for.**
*(`.rulings/` R-034.)*

That requires getting an application onto a machine without rebuilding the
operating system, which `ADR-008` §4 appeared to forbid. It does not, and the
resolution is the same one Android reached: **the "no package manager" ruling
was about operating system features**, and applications are a different category
with a different lifecycle and a different trust model.

---

## 2. Three categories, not two

| | Lives in | Replaced | Sandboxed | Trust domain |
| :-- | :-- | :-- | :-- | :-- |
| **Base image** | `/system` | wholesale, A/B | n/a | system |
| **System extension** | `/system/ext/<name>` | independently | no | system — `ADR-016` |
| **Application** | `<volume>/app/<name>` — `/state/app/<name>` on the built-in volume (`ADR-031` §3) | independently | **yes** | isolated |

This maps onto Android's system image / APEX / apps, and each row has a
genuinely different answer to "who wrote this and what may it do".

Applications live on a **volume** because a volume is machine-owned and survives
image updates (`ADR-008` §3) — which is exactly the property wanted for software
the operator installed. The built-in volume is `/state`, so on every Rivet
built to date the path is `/state/app/<name>` and nothing about this row has
changed; `ADR-031` §3 is why it stopped being the only answer, and §11 item 7 is
why the built-in one keeps its literal name rather than moving under a uniform
mount point.

**In the code, from 2026-08-25:** `rivet_core::service::app_root(volume)`,
with `service::BUILT_IN_VOLUME` spelling `/state`. Nothing chooses a volume other
than the built-in one yet (`ADR-031` §12).

### 2.1 It does not break the desired-state guarantee

`ADR-007`'s guarantee is that the document fully describes the machine.
Applications preserve it because **the set of installed applications is part of
the document.** Nothing is installed imperatively; an application is declared
and `rivet apply` makes it so.

Had installation been a side-channel, there would be two authorities over what
is on the machine, which is what `ADR-011` exists to prevent.

---

## 3. What a package is

Every simplification here was paid for by an earlier decision. Static musl
(`ADR-003`) and "no foreign binaries" (`ADR-003` §6.2) mean a Rivet
application has no runtime, no shared libraries and no dependency graph.

> **A package is one static binary, a typed manifest, optional assets, and a
> signature.**

That is the whole format. There is no dependency resolution, because there are
no dependencies.

### 3.1 The format is language-agnostic; the tooling is Rust-first

**Ruled 2026-08-22.** Nothing in "static binary plus manifest" is Rust-specific,
so a static C or Go binary is a valid package and will work. The tooling is
Rust-first because that is the actual use case.

The distinction is a promise, not a technicality: Rivet **supports** the Rust
path and **does not obstruct** others. It does not document, test, or guarantee
build paths the maintainer does not use. Claiming otherwise would be the
partial-compatibility failure `ADR-005` §5.4 rejects.

### 3.2 Cargo integration

Declared in the standard metadata extension point, so it is inert to every other
tool:

```toml
[package.metadata.rivet]
kind      = "daemon"              # daemon | oneshot | socket-activated
listen    = ["tcp:8080"]          # rivet opens these and passes the fds — §5.2;
                                  # tcp:<port>:direct beside a proxy-path — §5.2.1
network   = "egress"              # none | group:<name> | egress
state     = "private"             # gets /state/app/<name>/data, mode 0700 — §5.1.1
provides  = ["socket:api"]
connects  = ["photo-index:api"]
memory    = "128M"
restart   = "on-failure"          # always | on-failure | never — ADR-017 §4.1
ready     = "notify"              # none | notify | listening — ADR-017 §4.2
```

#### 3.2A CORRECTED 2026-08-29: `state = "private"` is `/state/app/<name>/data`

*(Numbered `3.2A` rather than `3.2.3` because it belongs before §3.2.1 in
reading order and nothing is ever renumbered — `ADR-035` §4A, §8A and §9B are
the same device.)*

~~`state = "private"` was annotated *"gets `/state/app/<name>`, mode 700"*.~~
**Superseded 2026-08-24 by §5.1.1 and struck here on 2026-08-29 —** the writable
directory an application is given is **`/state/app/<name>/data`**, mode `0700`,
owned by the application's identity. `/state/app/<name>` is its *tree*: it holds
the version directories, the `current` symlink and `data` as siblings, is
root-owned and is `0755` so the drop-to-uid can traverse it.

**This is the section a newcomer is sent to, and the correct answer was four
hundred lines further down the same file.** On 2026-08-29 an agent with this
repository and a board read the annotation above, wrote code against
`/state/app/<name>`, deployed it, and the board refused with `EACCES`. Its own
account: *"not a gap I noticed, but a confident sentence I followed."* The
sentence had been wrong since 2026-08-24, when §5.1.1 built the identity and
gave it `data`; §5.1.1 even quotes this line while contradicting it, and nobody
struck it.

**Read off the board that refused, not off this document:**

```
get file /state/app/todo        →  "uid":0,     "user":"root", "mode":"0755"
get file /state/app/todo/data   →  "uid":15189, "user":"todo", "mode":"0700"
```

**CORRECTED 2026-08-30 — the `user` field above is not what every board
answers.** ~~`get file /state/app/todo/data` returns `"user":"todo"`.~~ **Struck
2026-08-30.** A second cold agent, given this repository and the same board, ran
the same operation and got:

```
get file /state/app/todo/data   →  "uid":15189, "user":"15189", "mode":"0700"
```

`uid`, `gid` and `mode` match this section exactly. **`user` does not**, and
whether a board ever produced `"user":"todo"` is no longer checkable — the
reading above was copied into this document as a general answer, which is the
thing it is not.

**This is §3.2A's own defect, in §3.2A, two lines below the sentence that
corrects it.** This section was written on 2026-08-29 to strike a confident claim
that had rotted, and the evidence it replaced that claim with rotted the same way
and for the same reason. `AGENTS.md` §2 rule 4 is the rule about exactly this
class: nothing *overturns* a claim that was true and went stale, so there is
nothing to strike it with, and it survives by reading as evidence. Recorded here
rather than quietly fixed because the repeat is the finding.

**Why one board has two answers.** Verified 2026-08-30 by reading this tree, not
by asking a board:

* **`user` is not stored anywhere.** `cmd/rivet/src/edit.rs`'s `stat_path`
  resolves the uid through the `/etc` projection every other thing on this
  machine resolves through (`ADR-011`), and its `identity` ends by returning the
  number as its own decimal string: *"a number this machine has no name for is
  still a valid identity."* `"15189"` is a lookup miss, rendered — not an error
  and not a different directory.
* **An installed application does get a `/etc/passwd` row.**
  `cmd/rivet-sysd/src/etc.rs`'s `application_names` and `accounts` build it and
  `rivet_core::etc::render_passwd` writes it, which is `ADR-008` §7.3's *service
  accounts arrive with the services that need them*. So the second agent's own
  explanation — that the projection has no row for an application identity — is
  **wrong** about this tree.
* **`/etc` is projected once, at boot, before the first convergence**
  (`cmd/rivet-sysd/src/main.rs`, step 6 of its startup table; `ADR-017` §3.1.8
  is why it is there). `cmd/rivet-sysd/src/installed.rs` re-runs *discovery* when
  an install finishes — that is what makes a newly installed application start
  without a reboot — and it does **not** re-project `/etc`.

So **`"user":"todo"` is what a board says about an application it booted with,
and `"user":"15189"` is what the same board says about one installed since it
booted.** Read the `uid` and the `mode`: they are the machine's own numbers and
neither depends on a projection. A reader diagnosing an ownership problem against
`"user":"todo"` would conclude something is broken when nothing is.

**15189 is not a magic number either.** `rivet_core::service::app_identity` is
FNV-1a over the name, `% APP_UID_SPAN`, `+ APP_UID_BASE`; run over `todo` with
this tree's own constants it produces 15189, uid and gid. That is the one part of
the block above that a reader can recompute rather than trust.

**The window is a gap in the machine and not only in this document.** Between an
install and the next boot, `/etc/passwd` does not name an identity the machine is
already running a process as. It is queued at `PLAN.md` §102.4.2 rather than
closed here.

`rivet_core::service::data_dir(volume, name)` is the authority in the code, with
`DATA_DIR_MODE = 0o700`; `service::DATA_DIR` spells `data`. `tools/docs/claims.py`
is the guard that came out of this, and `AGENTS.md` §7 is the rule it enforces —
a claim that has stopped being true is exactly as expensive as one that was never
true, and more so for being confident. **It does not catch the correction above**:
`claims.py` derives paths, modes and counts from the tree, and a quoted answer
from a machine is none of those.

**And since 2026-08-29 a tool prints it.** `cargo rivet inspect` reports the path
in its `runs as` block, computed by calling `data_dir` rather than by quoting it
— see §5.1.1's last subsection. That is the remedy that survives a document
going stale: the guard catches a wrong sentence *eventually*, and a tool that
answers the question means nobody had to read the sentence.

**`restart` and `ready` were added 2026-08-22** (`ADR-017` §4.1), when building
the supervisor found that the record named where a service definition
*originates* and never said what one *contains*. They belong here rather than in
the operator's configuration for the reason §4.1 gives: **this manifest is
inside the signature.** An application that could have its restart policy or its
readiness contract rewritten outside the signed artifact would be a hole in
exactly the guarantee §4.1 exists to provide.

#### 3.2B RULED 2026-09-07: an application is handed a SCRATCH directory, and `TMPDIR` names it

**Two writable directories, not one.** `state = "private"` gives an application
`/state/app/<name>/data` for what it must keep; **`/state/app/<name>/tmp` is
where it may put what it must not.** Both are `0700` and owned by the
application's identity; both are in the Landlock ruleset; neither is
executable.

**No manifest key asks for this.** Every application gets it, because the
alternative is a declaration whose only sane answer is *yes*.

| variable | names | why both |
| :-- | :-- | :-- |
| `TMPDIR` | `/state/app/<name>/tmp` | what the world already reads — SQLite, libc's `tmpfile`, Rust's `std::env::temp_dir`, every archiver and codec |
| `RIVET_APP_TMP` | the same path | this project's own name, on `RIVET_APP_DIR`'s and `RIVET_APP_DATA`'s footing, because `ADR-036` §2's first article is *told rather than derived* |

##### Why it exists, which is the part worth keeping

**A restore on the test board died with `SQLITE_CANTOPEN`.** The application was
confined to one writable directory and was never told it was the only one, so
SQLite looked in `SQLITE_TMPDIR`, `TMPDIR`, `/var/tmp`, `/usr/tmp`, `/tmp` and
its working directory — **and all six failed at once**: the two variables unset,
two directories absent from the image, `/tmp` present and `1777` but outside the
ruleset, and the working directory the read-only `current`.

**The cost was never the refusal, which was correct.** It was that what surfaces
is an error from whichever library wanted the file — here a result code naming
no path and reading like a corrupt database. An unset `TMPDIR` was therefore not
a neutral default; it was a promise this machine could not keep, kept quiet
until something reached for it.

##### It is a sibling of `data`, and that placement is the decision

Pointing `TMPDIR` inside `data` would have cost nothing and worked the same day.
It was rejected: an application's persistent state would double as its scratch
space, so a crash leaves debris in the directory holding the history, the writes
land on the same SD-card wear budget, and **nothing could ever empty one without
risking the other.** Separate, it can be emptied wholesale.

##### Emptied at the lifecycle edges, and NOT on a timer

The supervisor empties it immediately before a spawn and immediately after an
exit — the two moments it knows for certain that nothing is using it. The exit
edge is what reclaims the space; the spawn edge is for the boot after a power
cut, where no exit was ever observed. Emptying is a rename aside and a fresh
directory, with the deletion done off the supervisor's loop by a forked helper
— `ADR-017` §3.6.2.

**A periodic sweep was considered and rejected.** `ADR-039` §2 separates a
predicate over the clock from an event that fires, and this is neither: the
moment it should happen is a lifecycle transition the supervisor already
observes, so a scheduler's missed-occurrence and overlap semantics buy nothing.
Worse, a timed sweep must **guess** whether a file is in use by a live process,
and guessing wrong deletes a running operation's scratch file mid-write.

**If an application ever leaks scratch while it is up, the answer is a size cap
and not a deleter** — a cap refuses the write and the application sees the
error. Nothing has measured how large an application's scratch gets, so nothing
is capped.

##### It is a plain directory today, and a tmpfs only once three things are settled

A tmpfs is the better endgame and this layout is chosen to leave room for it:
a kernel-enforced `size=`, no card wear, reclamation at unmount. Three things
are unsettled and none is settled by wanting it — **a size**, which is a
manifest key and so a change to this contract; **what overflow means**, since a
tmpfs converts a wear problem into an out-of-memory one on a 4 GB board; and **a
mount per application**, re-established every boot, global on a machine with no
mount namespaces (`ADR-042`).

#### 3.2.1 Amended 2026-08-24: `listen` is this manifest's, and `memory` is a request

**Ruled by the maintainer**, 2026-08-24. This section and
`ADR-007` §7.2 both declared `listen` and `memory`; both ADRs were accepted on
2026-08-22 and `ADR-011` says both cannot be the authority for one fact. That is
a contradiction in the record rather than a gap in it, and the ruling resolves
it **by direction** rather than by giving one document precedence:

> **The manifest owns what the application *is*** — restart policy, readiness
> contract, the sockets it listens on. **The document owns what the operator
> *limits*** — the memory cap. **Where both name `memory`, the lower wins.**

So of the two contested keys, exactly one moves and it moves *here*:

| Key | Authority after the ruling | Why |
| :-- | :-- | :-- |
| `kind`, `restart`, `ready`, `listen` | **this manifest, alone** | facts the author asserted about the program. §4.1 exists so none of them can be rewritten outside the signature, and a document that could set them is that rewrite by another route |
| `memory` | **both, and they mean different things** | here it is a **request** — §3.3 makes this manifest the permission request, and this is one of the permissions. `ADR-007` §7.2's is a **cap**. The effective limit is the lower |
| `network`, `state`, `provides`, `connects` | this manifest | unchanged; requests, per §3.3, and where the operator's *grant* is recorded is still undecided (§10) |
| `enabled` | `ADR-007` §7.2's document, alone | it has no counterpart here and never had. An application that could declare itself enabled would be one an operator could not turn off (`ADR-017` §6) |

**Why the direction test rather than a precedence rule.** §4.1's whole argument
is that a manifest outside the signature could be edited to ask for more; a
*document* that could edit the same fields is the same hole with a different
editor. But a memory cap cannot be used that way — it only ever takes away — so
the guarantee this ADR provides is untouched by an operator setting one, and
`ADR-007` §7.2 is right to grant it. The test generalises: **a key belongs to
the operator only if the worst an operator can do with it is give the
application less than it asked for.**

**`memory` here is therefore not a grant and must not be read as one.** An
application asking for `128M` on a machine whose document says `32M` gets
`32M`; one asking for `128M` on a machine whose document says `512M` also gets
`128M`, because a cap cannot raise a value the signature covers. §5.4's `plan`
already showed `128M memory cap` under **grants** rather than **requests**,
which is this half of the ruling written down before it was asked.

**A manifest that asks for no `memory` gets half of the machine's.** That
is the default the supervisor gives an application whose manifest is silent,
read as `MemTotal` from `/proc/meminfo` at discovery and halved. **It applies
only where neither the manifest nor the document says anything.** Where the
document names a cap for such an application, the cap replaces the default in
either direction, higher as readily as lower. The direction test above does not
bind here because the default is not a value the signature covers: it is the
system's guess on the operator's behalf, and the operator has the final word on
their own machine. A manifest that does ask still meets the cap by the lower-wins
rule, unchanged. (`.rulings/` R-210 D4, R-211, R-246.) A fraction rather than a number,
so it scales from a 1 GB Pi to a much larger machine; half, so that one
application growing without bound meets its own cgroup's limit while the other
half still holds the image's services and its neighbours, and the kernel is not
left choosing among them. A machine whose memory cannot be read applies no
default and says which applications run without one. An image service has no
manifest and gets none. The supervisor's `capped` record names the source of
such a limit `default`.

**The unit this needs and never gave.** *The lower wins* is a comparison, and
this section writes `memory = "128M"` without saying whether `M` is 1024² or
10⁶. `rivet-config` has read it as binary since 2026-08-22 and its canonical
serialisation depends on that, so binary is the standing answer;
`rivet_core::service::MemoryLimit` is where it is written down, though no
formal ruling has closed the question — it remains the maintainer's.

**Built the same day.** `cmd/rivet-sysd` reads an installed
application's manifest, derives the program from the name (there is still no
`exec` field), and applies the cap by the rule. `ADR-007` §7.2.4 carries the
console and the `memory.max` read back from a running machine.

**There is deliberately no `exec` field.** The artifact is a Cargo package, so
the binary *is* the crate's binary — §3.2's whole premise. A service definition
that could name an arbitrary program would let a signed manifest launch
something the signature never covered.

`cargo rivet package` builds for `aarch64-unknown-linux-musl` and emits the
signed artifact. `cargo-deb` and `cargo-dist` are the shape being followed;
this is deliberately unremarkable.

#### 3.2.2 Ruled 2026-08-25: installing is not running, and the default is `false`

§3.2.1's table gives `enabled` to the document **alone**, on the argument that
*an application that could declare itself enabled would be one an operator could
not turn off*. This section closes the hole that argument left open.

**The hole.** `enabled` belongs to the document, and a definition still has to
carry *some* value for the case the document says nothing —
`cmd/rivet-sysd`'s `applications::definition` supplies it, and
`document::apply_desired_state` replaces it wherever the document names the
service. That starting value was `true` from the day applications existed
until 2026-08-25, and it was never a decision: a struct field with a one-line
comment. Its effect was that **an application nobody had mentioned anywhere ran
on the next convergence.** Silence read as consent is the same outcome §3.2.1
refuses, reached from the other side.

**The ruling: `false`.** An application is installed *and idle* until the
document says otherwise. A machine's set of running applications is therefore
always a statement somebody wrote down, and `rivet status` can never name a
service the document does not.

**Why the image's own services still start enabled, which is not an
inconsistency.** `cmd/rivet-sysd`'s `definitions` module carries the starting
values and each is argued. **`rivet-mcpd`, `rivet-netd` and `rivet-timed` start
enabled**, because `ADR-023` §4.1 requires a freshly flashed headless board to be
reachable: you cannot reach a machine to enable the thing you reach it with, a
machine that does not configure its own interface cannot be reached at all, and
a machine whose clock is wrong cannot complete the handshake the control plane
is (`ADR-021` §4). `ADR-017` §6.2 is the ruling and §6.2.1 is what proved
`rivet-mcpd` starts from the image default.

An application has none of those properties: the operator is already on the
machine, or they could not have installed anything. **The bootstrap argument is
the whole of why those three are exceptions, and it does not generalise** — a
fourth service claiming it should start enabled has to make that argument and
lose it.

**What it is not.** It is not a security boundary and must not be read as one.
The payload, the document and the application all live on `/state` and root
writes all three; anything that can install can also edit the document. What the
default buys is **legibility** — the machine's behaviour and its configuration
cannot disagree — which is `ADR-007` §3.4.1's property applied to a service the
operator never mentioned.

##### The two-step cost, accepted rather than avoided

Deploying an application and having it run are now two statements. The second is
`ADR-007` §7.2's table:

```toml
[services.photo-index]
enabled = true
```

reachable on a running machine with `rivet apply edit` (`PLAN.md` §10), and
from a workstation with `cargo rivet deploy --enable`, which performs that
document edit rather than bypassing it. The convenience lives in the **host
tool**, where it is a word the operator typed, and never in the machine, where
it would be a default nobody chose. That placement is the point: `ADR-011`'s
single authority is intact because the tool writes *to* the document.

##### The migration, stated rather than discovered

**An application installed before this change stops at the next boot of a
machine whose document does not name it.** There is no grandfathering and none
is proposed: a machine cannot distinguish "installed under the old default" from
"installed and never enabled", and inventing a marker to tell them apart would
be a third authority for a question the document already answers. For a single
board this is one appended line. For a fleet it is a release note, and
`ADR-019`'s enrollment would be where the document arrives from anyway.

#### 3.2.3 Ruled 2026-09-09: `cpu` is a request too, on `memory`'s footing

**Nothing in this project capped CPU**, and §5.1's mechanisms table had named
`memory, CPU, pids` under "Resource limits" since this ADR was written without
the middle one ever being built. `pids` was never the gap beside it — `cmd/
rivet-sysd` already sets `RLIMIT_NPROC` to 256 on every service — so one
application spinning could still starve a board that is also driving a
television and serving the maintainer's own personal data. That is what this
amendment closes.

**The manifest may now declare `cpu = "0.5"`**, exactly as it declares
`memory = "128M"`, and §3.2.1's ruling applies to it unchanged: a request
here, a cap in `ADR-007` §7.2's document, and the lower wins. Nothing about
the direction test changes — a CPU share only ever taken away by the operator
is not a hole in §4.1's guarantee, for the identical reason a memory cap is
not.

**The unit is a decimal number of cores and there is no open question to
leave unruled**, unlike `memory`'s binary-vs-decimal gap: `rivet_core::
service::CpuLimit` is where the arithmetic lives, and `cpu = "0.5"` means
half of one processor on every machine that reads it, because a fraction of
a core has one reading. `cargo rivet package` builds it exactly as it builds
`memory`.

**No default.** An application that never declares `cpu` asks for no
particular share, which is `ADR-015` §3.3's closed end of the axis applied to
a second resource. `memory` has one (§3.2.1), and `ADR-007` §7.2.5 says why
the two differ.

Appended to `KEYS` at the end rather than beside `memory`, so every `.rvt`
signed before today parses unchanged — see `rivet_core::package::application`'s
own comment on the array for why the position, not only the key, is part of
the compatibility argument.

### 3.3 The manifest *is* the permission request

There is no second permissions file. What an application declares is what it is
asking for, and `ADR-016` uses the same manifest shape.

---

## 4. Signing and trust

**Ruled 2026-08-22: artifacts are signed from the first one, and the device
holds a configurable set of trusted public keys** — containing only the
maintainer's for the foreseeable future.

* The **private key never leaves the build machine.** The device needs public
  keys only, which is why this is safe on a headless Pi.
* The trust set is **configuration** (`ADR-007`), so adding or rotating a key is
  a `plan` you confirm, is attributed, and is visible in `--explain`.

  > **Constrained 2026-08-23 by `ADR-028` §5, and the constraint closes a loop
  > this bullet opened.** Configuration lives on `/state`, the one writable
  > filesystem — so as written, anything able to write configuration could add
  > its own trusted key and every artifact it forged would then verify. The
  > mechanism protecting updates was editable by the machine it protects.
  > `ADR-028` §5.2 keeps this bullet's rotation property and removes the loop: a
  > **root set ships in the read-only image**, and a configuration-supplied key
  > is trusted only if countersigned by a key already trusted. Configuration can
  > still rotate; it can no longer introduce a trust anchor by itself.
* A configurable set rather than a baked-in key costs one field now. Rotating a
  compiled-in key would mean rebuilding the operating system.

Retrofitting verification later would leave every artifact built before it
untrusted and would rework the delivery path, which is why this is not deferred.

This shares `ADR-008` §4.2's third commitment — artifact delivery built
generically the first time — with image updates and system extensions. **One
signing, verification and transport path serves all three.**

### 4.1 The signature covers the manifest, not only the binary

**Corrected 2026-08-22.** §3 listed the manifest as part of a package without
stating that the signature covers it. That is a defect rather than an omission.

**The manifest *is* the permission request** (§3.3). If it were outside the
signature, a legitimately signed application could have its manifest rewritten
to ask for `network = "egress"`, a wider filesystem grant or a privileged port,
and the binary's signature would still verify. The operator would then confirm a
`plan` showing permissions the author never requested.

Binary and manifest are signed as one unit.

### 4.2 Verified at install **and at every launch**

**Ruled 2026-08-22**, and stronger than the recommendation, which was
install-time verification with drift detection.

The gap that closes: ~~`/state/app` is root-owned and mode 0700~~ **`/state/app`
is root-owned and mode `0755` — corrected 2026-08-29**, but `ADR-020`'s
board-bound encryption covers only `/state/secrets`, so an installed binary sits
in plain `ext4`. Install-time verification alone would pass a binary modified
after installation — by someone who took the card and returned it, or by a
compromised process running as root.

> **The mode is `0755` and could not be `0700`.** §5.1.1 dropped applications to
> their own uid on 2026-08-24, and the program is resolved *after* the drop — so
> every directory from `/` down to the binary has to be searchable by an
> unprivileged uid. §5.1.1 said as much at the time and called this sentence *"a
> description of a directory nothing creates yet"*, and then left it standing.
> `cmd/rivet-deploy/src/install.rs` creates `/state/app`, `/state/app/<name>`
> and `/state/app/<name>/<version>` at `0o755`; only `data` is `0700`
> (§3.2A). **The security argument this paragraph makes is unaffected** — it
> rests on `/state` being plain `ext4` outside `ADR-020`'s coverage, not on the
> mode — which is why the wrong number survived: nothing downstream of it broke.

**The recorded hash is kept as well**, because the two do different jobs:
launch-time verification *stops* a tampered application; the hash recorded in
the desired-state document at install makes the tampering **visible as drift**
in `rivet plan` and `rivet status`, in the same vocabulary as every other
divergence (`ADR-007` §3.3). Failing to start is a symptom; drift is a
diagnosis.

**The honest cost:** every service start hashes its whole binary. The Pi 4's
A72 is expected to lack the ARMv8 crypto extensions (`ADR-020` §6.2), so this is
software hashing on a slow core, paid at every boot for every service.
**Measure it before assuming it is negligible**, and consider BLAKE3 rather than
SHA-256 — it is markedly faster in software and parallelises, which matters
precisely when there is no hardware acceleration.

#### 4.2.1 What it verifies AGAINST — ruled 2026-08-24, and the cost is now measured

§4.2 ruled *that* an application is verified at every launch. It never said
**what the supervisor compares the binary to**, and that was the whole of the
difficulty, because of one fact:

**The board can verify but cannot sign.** `ADR-028` §5.5's trust store holds
*public* keys. The only signing key in the project lives in `cmd/cargo-rivet`
on a workstation (`PLAN.md` §0.9). So a Rivet cannot mint its own signed
record of what it installed — whatever it checks against has to have **arrived
already signed**.

**RULED: keep each application's signed manifest beside it, and re-verify both
halves at every launch.** The installer already reads a manifest and a signature
over it, and today it discards the wrapper after unpacking. Instead it keeps the
manifest and its signature — a couple of kilobytes — and at every launch the
supervisor does exactly what the installer did: check the signature against the
trust store, hash the program on disk, and compare to the digest the manifest
carries. **No new key, no new format, no new trust decision, and the same
verifier runs in both places** — which is `ADR-029` §11's one-verifier argument
holding one level further out.

**What was rejected, and why it is worth writing down:**

| | Option | |
| :-- | :-- | :-- |
| **A** | keep manifest + signature per application | **RULED.** Kilobytes; reuses everything |
| **B** | keep the whole `.rvt` | Same security, doubles disk for every application |
| **C** | give the board a signing key so it can sign its own installed-set record | **Rejected as theatre.** The key would live on `/state`, which is precisely what an attacker with write access already controls — they would simply re-sign. Real hardware key storage would change this; a Pi 4 has none (`ADR-020` §6.2) |
| **D** | verify at install only | The status quo. Defensible against a corrupted download, useless against a modified card, and blind to bit rot |

##### BUILT 2026-08-25, and option A turned out to be **one** file rather than two

`cmd/rivet-sysd/src/verify.rs`, with the retaining half in
`rivet_core::package::install`.

**The thing kept is the container's signed header** — `HEADER_NAME`,
`package-header`, 4096 bytes at `FILE_MODE` beside `bin/` and `manifest.toml` —
rather than "the manifest and its signature" as this section anticipated. They
are the same idea and the header is the better spelling, because **there is no
detached signature to keep**: `ADR-029` §3 signs `[0 .. 4032)` of the header,
and that region carries every member's digest. So one file re-establishes the
whole chain — the key id, the signature, the manifest's digest and the
program's — and nothing has to be recombined from parts that could disagree.

Option **B** is refused a second time and more sharply than this table put it:
keeping the whole `.rvt` does not merely double an application's disk, it is
impossible for an update, which is 683 MB (`PLAN.md` §13).

**What is checked at each launch**, in order: the header verifies against the
trust store **as it is at that moment**; the kind is `application`; the program
on disk hashes to the digest that header covers. The store is read on **every**
launch and never cached — a cached one answers with the store the machine
booted under, and `ADR-028` §5.5.4's revocation works by replacing that file.

**Every failure is a refusal to launch, never a warning and a spawn**: no
header, an unreadable or empty trust store, an oversized program. A machine that
cannot say what it trusts does not run unverified code because of it.

**The migration cost is real and is not hidden.** An application installed
before this date has no header, so it is **refused with `no-header` and told to
reinstall**. There is no grace period and no "verify if present" mode, because
a check that can be skipped by deleting a file is not a check.

##### The window this does NOT close

The supervisor hashes the bytes at a path and then `execve`s that path, so the
kernel opens the file a second time. Between those two opens the file could
change. Closing it needs the supervisor to exec the descriptor it verified —
`fexecve` — and that is a separate change.

**It is not reachable by the application itself**: the program is `0555`,
root-owned, in a `0755` root-owned tree, so the identity that runs it cannot
write it (read off a board 2026-08-25). It *is* reachable by anything running as
root, which is exactly the actor a launch check is for. Stated here rather than
left to be discovered.

**This complements §4.2's recorded hash rather than replacing it.** That hash
lives in the desired-state document to make tampering *visible as drift*; the
document sits on writable `/state` and is not signed, so it diagnoses and the
manifest decides. Failing to start is a symptom; drift is a diagnosis; **only
the signature is an authority.**

##### §4.2's "measure it before assuming it is negligible" — discharged 2026-08-24

That instruction is answered, on the hardware it was written about.
`rivet measure hash` on a Raspberry Pi 4, against the real 640 MiB system
partition on an SD card:

| | rate |
| :-- | --: |
| read from the card | **142 MB/s** |
| `blake3` | **171 MB/s** |
| `sha256` | **101 MB/s** |

**BLAKE3 is 1.7× SHA-256 on this core, exactly as §4.2 guessed it would be**, and
that is the first hardware evidence for the choice. The practical answer to the
cost question: an application binary is a few megabytes, so **a 5 MiB program
hashes in about 30 ms**. Per service, per boot, that is negligible — and the
worry in §4.2 was well-founded but is now retired with a number rather than an
assurance. Note the shape too: at 142 MB/s the **card is the floor**, so the
hash is close to free relative to simply reading the program in order to run it.

##### What a failed check does

**Refuse to start that one application, and say so loudly in the journal.**
Never start it anyway, and never take the machine down over it. `ADR-017`
§3.1.4's principle applied here: one bad application is a failed service, not a
broken system. The refusal names the file and which half failed — signature or
digest — because those are different problems: a bad signature means the wrong
provenance, a bad digest with a good signature means the disk changed underneath
a correctly-installed program, which on an SD card is more likely to be rot than
malice.

### 4.3 The consequence: the maintainer is the gatekeeper

Since the trust set contains only the maintainer's key (§4), third-party
software is signed **by him**, or its author's key is added to the trust set —
which is a `plan` he confirms, attributed and visible in `--explain`.

That is correct for a personal fleet and is recorded so it reads as a decision
rather than an accident of there being one key.

### 4.4 The same question for the other two artifact kinds

Verification differs by how an artifact is *used*, not by what it is:

| Artifact | Verified |
| :-- | :-- |
| Application | signature at install **and at every launch** (§4.2) |
| System extension (`ADR-016`) | signature at install; `dm-verity` on the mounted image is the continuous equivalent, and is the option `ADR-016` §3 already names |
| Base image (`ADR-018`) | signature before `tryboot`; `dm-verity` likewise |

Extensions and the base image are mounted read-only for their whole lifetime, so
their equivalent of "at every launch" is continuous block-level verification
rather than a re-check at start.

**A signature says an artifact is ours; it does not say which machine it is
for.** All three kinds therefore carry architecture in their distributed
filename under `ADR-027` — and that ADR's §5 is the other half of this section:
the name is for a person reading a directory, and verification never consults
it. What the signature covers stays the authority.

---

## 5. The sandbox

### 5.1 Mechanisms

Android's sandbox is less exotic than its reputation: its primary mechanism is a
**unique UID per app** plus ordinary Unix permissions. Its permission model is
enforced in userspace by the service being asked, not by the kernel; the parts
needing kernel enforcement required patches that only exist in Android's tree.

Mainline Linux has since grown better tools, so Rivet composes existing kernel
features rather than inventing a security model:

| Property | Mechanism | Notes |
| :-- | :-- | :-- |
| Identity | **UID/GID per application** | Exactly Android's primary mechanism. Available because `ADR-008` §7.3 kept full Unix multi-user |
| Sees only its own files | **mount namespace + bind mounts** | `/system` read-only, its own ~~`/state/app/<name>`~~ **`/state/app/<name>/data`** (corrected 2026-08-29 — §3.2A), nothing else. **Not built:** there is no mount namespace, so an application sees the whole filesystem (§5.1.1) |
| Filesystem restriction | **Landlock** | Mainline since 5.13; a process restricting *itself*, with no policy language to author. The right fit for a one-maintainer system, where SELinux is not |
| No network | **network namespace** | An application with `network = "none"` has no interfaces at all — stronger than Android's group-membership trick |
| Resource limits | **cgroups v2** | memory, CPU, pids |
| No privilege | **all capabilities dropped** | The default, not an opt-in |
| Syscall filtering | **seccomp-bpf** | Later — §6 |

Prior art for the composition: systemd's service sandboxing, and
Flatpak/bubblewrap. None of this is novel.

**~~Unverified and worth checking early~~ — CHECKED 2026-08-22, and it came back
badly.** The stock Raspberry Pi kernel `6.18.34+rpt-rpi-v8` **does not have
Landlock**: `CONFIG_SECURITY_LANDLOCK` is not set, and `/sys/kernel/security/lsm`
reads `capability` alone. This table's "Filesystem restriction" row therefore
has no kernel to run on until Rivet builds its own — `ADR-024` §8's
conditional has fired and the custom kernel is scheduled (`ADR-024` §8.1).
~~**Stage 4 is gated on it**, which is recorded in `PLAN.md` §2.5.~~

**The Filesystem-restriction row is BUILT — `ADR-042`, 2026-09-04, amended
2026-09-05.** Rivet's own kernel carries Landlock and answers **ABI 6**, so
every right the policy uses is enforced; the supervisor computes a policy from
the manifest, builds a ruleset in the parent, and the child applies it between
`fork` and `exec`. **An application on a kernel with no Landlock is refused
rather than started unconfined** (`ADR-042` §5), because a Rivet ships its
kernel and its image together and such a machine is a broken build rather than a
configuration an operator chose.

The struck sentence above is left readable because the *measurement* it reports
is still correct about the stock kernel and is the reason the custom one exists.
What changed is only which kernel a Rivet runs.

**Two rows of this table are still unbuilt and one of them is next to this
one.** There is no mount namespace, so every path is still THERE and anything
running as uid 0 reaches it — Landlock restricts what an application may *open*,
not what exists — and there is no network namespace, so `network = "none"` means
nothing. `ADR-042` §7 says why Landlock's own ABI 4 network rights were **not**
used to half-honour that key. The supervisor emits both absences on every launch.

**cgroups v2 is present but crippled by default, which is worse than absent
because it is silent.** `/sys/fs/cgroup` is `cgroup2fs` as required, but
Raspberry Pi OS ships `cgroup_disable=memory` on the kernel command line and
`/sys/fs/cgroup/cgroup.controllers` reads `cpuset cpu io pids` — **no `memory`**.
The "Resource limits" row above names memory first, and a memory limit set
against that kernel is accepted and ignored. Rivet owns `cmdline.txt`
(`ADR-002` §4) so the fix is a deletion, but `ADR-024` §7's boot verification
must assert the *controller* rather than infer it from the filesystem type.

The other mechanisms in this table are available on the stock kernel:
`SECCOMP`, `SECCOMP_FILTER`, `NET_NS`, `PID_NS` and `NAMESPACES` are all set.

**The identity and filesystem rows apply to the image services
`rivet_core::service::IMAGE_SERVICE_HOMES` names as well** (`ADR-060`):
`rivet-proxyd`, `rivet-dnsd`, `rivet-mdnsd`, `rivet-timed` and the compositor,
`rivet-displayd`. Each ships in
the image and still runs under the uid its name derives and a Landlock ruleset
composed from the same rows an application's is, because nothing it does needs
root — except that `rivet-timed` keeps `CAP_SYS_TIME` alone to set the clock
(`ADR-017` §3.9). Being part of the operating system decides who ships a
program and when it changes; it does not decide what the program may reach. The
compositor's `display` class is granted to its uid exactly as an application's
is (§6.2, `ADR-058` §5.6).

### 5.1.1 The identity row is BUILT — ruled 2026-08-24

**Ruled by the maintainer 2026-08-24**: Rivet builds an identity for each
**service**, and human user accounts wait. *(`.rulings/` R-282.)* This section is the authority for how the first
row of §5.1's table works; §6's stage table is amended by it, and the *human*
half of `ADR-008` §7.3 is explicitly **deferred and not built** — there is no
authorization layer for a human user to attach to, and the supervisor has no
socket. The service half was urgent because it was live: applications
discovered from `/state/app` were executed **as root**.

> **An application's uid is a pure function of its name, it is created with the
> application and never pre-created, and the supervisor drops to it between
> `fork` and `exec`.**

#### Derived, not allocated

`rivet_core::service::app_identity` maps the application's name onto
`APP_UID_BASE..=APP_UID_LAST` — 4000 to 65533 — with FNV-1a, and the gid equals
the uid (the user-private-group idiom: one member, reached through
`/etc/passwd`'s primary gid).

| | **Derived — what was built** | **Allocated and recorded** |
| :-- | :-- | :-- |
| Stable across a reboot | by construction; it is a function | as long as the record survives |
| Needs state | none | a file, an allocator, a lock |
| Two applications can collide | **yes, and it is refused** | no |
| Authority for *what uid is `photo-index`* | its **name** | a file, which is a second answer beside the name |

The property that decided it is the failure mode: an application whose uid
changed between boots could not read its own files, and **nothing would report
it** — the `EACCES` happens inside the application, at a moment nobody is
watching. `AGENTS.md` §5's *derive, never store* points the same way, and
`PLAN.md` §0.11 states the alternative for the maintainer to overturn.

**4000 is read out of §5.4, not chosen.** That section's worked example of the
grant an operator confirms is `uid 4021`; it is the only application uid this
record has ever written down. The top stops two short of `u16::MAX` rather than
one: `65535` is `(uid_t)-1`, which `setuid(2)` reads as *do not change*, and
`65534` is the conventional `nobody`.

**A collision is refused, not warned about.** Two names can derive one uid. Two
applications sharing one is not a weak sandbox — it is *none between those
two*: each could read and rewrite the other's private data, and each would
answer the other's `SO_PEERCRED` (§5.3) as itself. So the second application is
**not started**, deterministically by name order, by the one thing that holds
every definition at once. That is the opposite of what the same module does
with a colliding *port*, and the difference is that nothing binds a port yet
while this identity is applied on the very next `execve`.

#### The drop, and the order

`cmd/rivet-sysd`'s `Kernel::spawn`, in the `pre_exec` closure — so the
program has never executed one instruction as uid 0:

1. `setgroups(0, NULL)` — the supplementary list, **while `CAP_SETGID` is still
   held**;
2. `setgid(gid)`;
3. `setuid(uid)`, last, because it is the one that cannot be undone.

Every return value is checked, and the child then asks the **kernel** who it is
(`getuid`, `geteuid`, `getgid`, `getegid`) before `exec`. All three are **raw
syscalls** rather than musl's POSIX wrappers, which broadcast to every thread
through `__synccall` and are not async-signal-safe after `fork`.

Doing 3 before 1 or 2 is the textbook bug: `setuid` to a non-zero uid discards
every capability, the later `setgid`/`setgroups` fail with `EPERM`, and a caller
that did not check would have a process running as an unprivileged **user** that
is still in root's **groups**.

#### What the identity brings with it

* **`/etc/passwd`, `/etc/group` and `/etc/shadow` name the account**, through
  `rivet_core::etc`'s one generator and a new `EtcInputs::applications`. A uid
  no name resolves to makes `ls -l` print a number and hands a listening service
  an unnameable `SO_PEERCRED` caller. The shell field is **empty** — an
  application account is an identity, not a way in — so a login path refuses a
  login as one before authentication, by the `/etc/shells` mechanism the login
  shell already has.
* **`/state/app/<name>/data`, mode 0700, owned by the identity** — §3.2's
  `state = "private"`. Without it the identity would be a *regression*:
  `/system` is read-only and everything else on `/state` belongs to root, so an
  unprivileged application would have nowhere at all to write. It is a sibling
  of the versions rather than a child of `current`, so an update replaces the
  program and leaves the data. The name `data` is chosen by
  `rivet_core::service` — the fourth such name, alongside `current`, `bin` and
  `manifest.toml` (`PLAN.md` §0.4.3(d)).
* **Every directory from `/` down to the binary must be searchable by the new
  uid**, because the program is now resolved after the drop. §4.2's *"`/state/app`
  is root-owned and mode 0700"* is a description of a directory nothing creates
  yet; whatever installs an application will have to make the version tree
  traversable while keeping `data` private. `PLAN.md` §0.11 records the QEMU
  boot that found this by refusing with `EACCES`.

#### System services keep uid 0, and that is the answer rather than the default

`rivet-netd` speaks netlink and `rivet-mcpd` spawns the CLI that installs,
arms and reboots the machine. A system service given an identity would start
and be unable to do its job, so `ServiceDef`'s `IMAGE_DEFINED` carries
`identity: None`. The image services that do no root's work are the exception
and are named: `IMAGE_SERVICE_HOMES` lists them, and a test asserts that the
compiled-in definitions with an identity are exactly those. A service whose
work needs one piece of root and nothing else joins them keeping that one
capability (`ADR-017` §3.9): `rivet-timed`, which sets the wall clock, keeps
`CAP_SYS_TIME`.

#### What is still not enforced, said on every boot

Only the first row of §5.1's table is live. There is **no mount namespace**, so
an application sees the whole filesystem; **no network namespace**, so `network
= "none"` and `network = "group:…"` mean nothing; **no Landlock**, which the
stock kernel does not have; and **no seccomp filter**. Unix permissions on its
own data directory are the whole of what separates one application from
another. The supervisor says exactly that, at `Warn`, once per discovered
application, per §6.1 — beside a matching `ENFORCED` line, because a machine
that reported only the first would have an operator believing in a sandbox that
does not exist.

**`Manifest::unenforced` still lists the identity among what is unenforced, and
it is now wrong about that.** `PLAN.md` §0.11 carries it rather than it being
corrected inside a crate another agent was editing.

#### The host tool says who it will be — 2026-08-29

**Ruled 2026-08-29**, Pile B item 2 (`PLAN.md` §102.3 item 2). `cargo rivet
inspect` described a package fully — product, version, arch, kind, key, every
member, every permission it requests — and never said **what account it would
run as**. `PLAN.md` §102.6's cold agent needed the uid, could not get it from the
tool, and inferred it.

Inferring it is not hard, and that is the point rather than a mitigation: the
uid is a pure function of the name and the function is in `rivet-core`, so
anyone can reach it — by reading source, or by deploying and looking. Both are
the workstation-side failure the agent's own verdict named: *"the board explained
almost everything it was asked; the failures were on the workstation side and in
the ADRs."*

`cargo rivet inspect` now prints a `runs as` block carrying:

| | Computed by |
| :-- | :-- |
| `uid` and `gid` | `rivet_core::service::app_identity` — **the same function the supervisor calls**, so this is a report and not a second answer (`ADR-011`) |
| the writable directory and its mode | `rivet_core::service::data_dir` and `DATA_DIR_MODE`, for §3.2A's reason: the tool that answers the question is worth more than the sentence that was wrong |
| that a **collision** is the machine's to detect | this tool sees one package; the supervisor is the only thing holding every definition at once |

**A product that is not a usable `ServiceName` reports NOTHING rather than a
number.** The container's text rules are looser than `ServiceName`'s (§7.1.1), so
a package can carry a product that will be refused at install — and a uid printed
for a name no machine will accept is worse than none.

**What it does not do:** it does not check the target machine. Whether *that*
board already runs an application deriving the same uid is not knowable from a
package, and the refusal for it stays where it is.

### 5.2 Privileged ports are solved by not granting privilege

> ~~**NOT BUILT, and if you are writing an application: bind your own socket.**
> Everything in this section is the design. No Rivet that exists opens a
> listening socket or passes a descriptor, so an application that waits for
> `LISTEN_FDS` waits forever. Stated 2026-08-30 — see *"What an application
> author has to do today"* below, which is the whole of what this changes for a
> reader.~~
>
> **BUILT 2026-08-31, and the advice above is now wrong in the other
> direction.** `cmd/rivet-sysd` opens every socket a manifest declares, before
> the spawn, and passes the descriptors from 3 upward on `ADR-017` §3.4's
> `LISTEN_FDS` convention. TCP and unix both. See §5.2.1 for what an author
> should write **now**, and `PLAN.md` §103.10 for what it was proven with.
>
> It is struck rather than deleted because it is the sentence somebody will
> quote when their application binds its own socket and works anyway — which it
> still does, and should keep doing (§5.2.1).

Rather than granting `CAP_NET_BIND_SERVICE`, **Rivet opens the listening
socket and passes the file descriptor to the application**, on systemd's
`LISTEN_FDS` model. The application never holds the privilege and need not know
the port.

~~"May listen on 443" therefore becomes a line in the desired-state document that
Rivet honours, rather than a capability the application carries.~~

**Superseded 2026-08-24 by §3.2.1**, and struck rather than deleted because it
is the sentence an operator will quote when their `listen` is refused. The
mechanism above is unchanged and is the whole point: Rivet opens the socket
and passes the descriptor, so the application never holds
`CAP_NET_BIND_SERVICE`. What changed is **who says which sockets** — that is a
fact the author asserted about the program, so it is a line in the *manifest*
that Rivet honours. The operator's remaining question about a port is whether
the service runs at all (`enabled`), and their remedy for a port they do not
want is not to install the application.

#### 5.2.1 What an application author has to do NOW — 2026-08-31

**Honour `LISTEN_FDS` when it is set, and bind when it is not.** That is what
the struck section below already advised for portability, and it stopped being
advice and became the way this works.

```text
LISTEN_PID     must equal your pid, or the descriptors are an ancestor's
LISTEN_FDS     how many, starting at descriptor 3
LISTEN_FDNAMES colon-separated, one per descriptor, in the manifest's order
```

`LISTEN_FDNAMES` carries **`tcp-<port>`** for a TCP socket and the declared
name for a unix one — colon-free, because the list is colon-joined. The
manifest's own `tcp:80` and `unix:api` spelling is what a *person* reads and is
deliberately not what is in this variable; a board reported
`LISTEN_FDNAMES tcp, 80, unix, api` before that was fixed.

**Where a TCP or UDP socket is bound depends on `proxy-path`** — `ADR-046`
§7.1, `.rulings/` R-188. An application that declares a `proxy-path` has every
port bound on `127.0.0.1`, so the network reaches it only through
`rivet-proxyd`; a port it must also serve directly is declared
`tcp:<port>:direct` (or `udp:<port>:direct`) and is bound on every address. An
application with no `proxy-path` has every port bound on every address. The
flag is refused on the routed port and without a `proxy-path`, and it does not
change the descriptor's name: `tcp:8443:direct` still arrives as `tcp-8443`.

**What this buys, and it is the whole of §5.2's promise:** a port below 1024 is
reachable. `examples/probes/socket-demo` serves HTTP on **80** as an unprivileged uid
having never called `bind(2)`. And for a unix socket it buys §5.3's other half —
the application does not choose the path, so it does not need write access to a
directory it cannot create.

**Three things it deliberately is not:**

* **not socket activation.** The socket is opened when the service starts and
  closed when it stops. Nothing starts a service *because* a connection
  arrived, so `kind = "socket-activated"` is still a token with nothing behind
  it and §3.2's ordering question is untouched.
* **not application-to-application communication.** A unix socket is `0600`
  root-owned — the closed end of the axis (§3.3). §5.3's grant is a bind mount
  into the namespaces of applications that declared and were granted the
  connection, and this build has no mount namespaces (§5.1), so any wider mode
  would grant it to *everyone*. systemd's `0666` default is deliberately not
  followed.
* **not `ready = "listening"`.** That still cannot succeed, and the reason
  changed: it is no longer *"no socket is opened"* but *"the socket is listening
  before the process starts, so it says nothing about the process"*. A different
  question, and `ADR-017` §4.2's.

~~#### What an application author has to do today — stated 2026-08-30~~

**Superseded by §5.2.1.** Struck rather than deleted: everything in it was true
for a day, its portability advice is what §5.2.1 now requires, and its account
of *how* the gap was found is the part worth keeping.

**~~Your application binds its own port, and it must pick one above 1023.~~**

This was true when this section was written and this section never said it. A
cold agent given this repository and a board on 2026-08-30 got it right, and
recorded *how*: not from here, but from §6's stage table — whose third column
`AGENTS.md` §10 warns is dated rather than a fact about the tree — and from a
warning the board printed at launch. **A reader who followed §5.2's own words
would have written a program that blocks waiting for a descriptor nothing
sends.** It is written here because §5.2 is where an application author is
standing when they ask the question.

The supervisor says the same thing, out loud, every time it launches something
with a `listen` in its manifest — `cmd/rivet-sysd/src/applications.rs`:

```
NOT ENFORCED the manifest asks for listening sockets and this supervisor cannot
supply them: ADR-017 §3.2's pre-created socket and §3.4's LISTEN_FDS handoff are
not built. The service starts with no descriptors passed to it
```

So:

* **`listen` in a manifest is a declaration and not a mechanism.** It is real —
  it is inside the signature (§4.1), the supervisor refuses two applications
  claiming one socket, and `ADR-023` will generate rules from it. It does not
  open anything.
* **Ports below 1024 are unreachable, not merely discouraged.** An application
  runs as its own unprivileged uid (§5.1.1) with no capabilities granted, so
  `bind(2)` on 80 or 443 fails with `EACCES`. §5.2's design is precisely what
  would have made them reachable, and it is the part that is not built.
* **`ready = "listening"` cannot succeed either**, and the supervisor says so
  separately: a readiness contract over a socket Rivet would have opened is
  never satisfied when none is opened (`ADR-017` §4.2).
* **Write for both if you want to be portable across this change.** Honour
  `LISTEN_FDS` when it is set and bind when it is not; that is what the agent
  did, and it costs about ten lines.

Nothing above is a new decision. It is §6's third column said in the place a
reader needs it, which is `AGENTS.md` §2 rule 4 — the claim was corrected where
it is *made*, not only where it is tabulated.

### 5.3 Application-to-application communication

**Ruled 2026-08-22: both mechanisms, because they answer different needs.**

**Declared Unix sockets — the precise path.** An application declares
`provides = ["socket:api"]`; another declares `connects = ["photo-index:api"]`.
Rivet creates the socket and bind-mounts it into *only* the mount namespaces
of applications that declared and were granted the connection.

The reason this beats loopback is specific: **`SO_PEERCRED` gives the listening
process the caller's UID.** Since every application has its own UID, a service
can know exactly which application is calling. TCP loopback can never tell it
that. There are also no port collisions and no shared bus.

Rivet's only contribution is the wiring, which is bind mounts. Every Rust
library already speaks Unix sockets, and it is the same mechanism as §5.2 —
one thing, not two.

**An endpoint has one consumer unless its provider declares it shared, and an
endpoint more than one application claims is given to none of them**
(`.rulings/` R-210, D3). Every claimant is refused and is not started, and
each refusal names them all. Granting it to whichever claimant came first
would let an application installed later take an endpoint away from the one
holding it by choosing its name; refusing the contest makes the conflict the
fault, visible on both sides, and leaves neither application in control of the
other. A provider that means to serve several consumers says so with
`provides = ["socket:<name>:shared"]`.

**Firewall rules are generated from these declarations** — see `ADR-023`.
There is no separate firewall surface, and ~~a declared port is localhost-only
until exposure is granted in the same `plan` that grants everything else~~ **a
declared port WILL BE localhost-only until exposure is granted. Tense corrected
2026-08-30** — this sentence described `ADR-023`'s design in the present tense
and a reader took it for a state. There is no ruleset generator in this tree
(`ADR-023` §3A), and `plan` grants nothing about a port. What bounds a
declared port's reach is where the supervisor binds it (§5.2.1): a proxied
application's ports listen on `127.0.0.1` unless declared `:direct`, and every
other declared port is reachable from wherever the machine is reachable.

**Network groups — the pragmatic path.** `network = "group:media"` places an
application in a network namespace shared with the others in that declared
group. Loopback works between them and nothing outside can hear it. This is what
Podman pods do.

A *flat* localhost tier was considered and rejected: every application reaching
every other application's port is a shared bus rather than isolation, with no
caller identity and no collision story. Scoped to a declared group, the same
convenience is available with a boundary around it.

**For the eventual GUI:** Wayland is itself a Unix socket protocol, so §5.3's
bind-mount mechanism already covers it. No new machinery is needed for graphical
applications, which is a useful thing to know before designing them.

### 5.4 Permission granting is a diff, not a dialog

The payoff of `plan`/`apply` (`ADR-006` §6). Installation shows what is being
asked for and what will be granted, and the operator confirms once:

```
$ rivet plan
+ app "metrics-collector" v0.3.1
    requests: bind tcp:8080
              network egress
              connect photo-index:api
    grants:   uid 4021, private mount ns, no capabilities, 128M memory cap
```

This is Android's permission model with better ergonomics and a written record —
the grant is attributed and timestamped like any other change (`ADR-012` §4).

---

## 6. Staged enforcement, and the honesty rule that goes with it

**Ruled 2026-08-22: erect the cheap fences immediately and declare the expensive
ones from the start**, so that deepening enforcement never requires reworking
manifests or call sites.

> **The manifest declares everything from day one; enforcement deepens over
> time.** The declaration is the stable interface; the mechanism behind it is
> not.

| Stage | Enforced | Built? — as of 2026-08-24 |
| :-- | :-- | :-- |
| First | per-app UID, private data directory mode 700, all capabilities dropped, cgroup limits, network namespace | **partly.** The UID and the mode-700 data directory are live (§5.1.1). The cgroup memory limit is live and is reported when the kernel refuses it (§3.2.1). **Capabilities are not dropped** and there is **no network namespace** |
| Next | mount namespace with declared bind mounts, socket passing, Landlock | **partly, as of 2026-09-05.** Socket passing is live (§5.2, §5.3). **Landlock is live** — `ADR-042`, on Rivet's own kernel at ABI 6; ~~it has no kernel to run on~~. The mount namespace is not |
| Later | seccomp profiles | no |

**The third column is dated rather than a fact about the tree you have**
(`AGENTS.md` §10). It is here because a stage table with no built column reads
as a plan and gets quoted as a description, and the row that matters most is the
first: *"all capabilities dropped"* is in the **First** stage and is **not
done**. A process that drops to an unprivileged uid does lose the ability to
*use* most capabilities, which is not the same statement and must not be read as
one — the bounding set is untouched, and `ADR-024`'s kernel will decide what
that costs.

### 6.1 An unenforced declaration must be visible

`AGENTS.md` §3 requires that stubs refuse rather than fake success. Accepting a
manifest that requests something this version cannot enforce would tell the
operator an application is sandboxed when it is not — which is worse than not
offering the field.

So an unenforceable declaration is reported, not silently accepted:

```
+ app "metrics-collector" v0.3.1
    requests: syscalls strict   ⚠ NOT ENFORCED by rivet 0.4
```

This is the difference between staged enforcement and security theatre.

---

### 6.2 RULED 2026-08-30: the display is a declared capability the SUPERVISOR arbitrates

While the boot splash was being built, the maintainer ruled that the supervisor
manages the display and ensures that only one device owns it at a time, that
this feature set is required before the Aura controller can be implemented, and
that the splash screen goes back up whenever any program exits.
*(`.rulings/` R-269.)*

**Four things, and the last one changes what the first three are for.**

1. An application that draws **declares it** in its manifest, beside `network`
   and `state`.
2. **The supervisor arbitrates**, because it is the only thing that starts and
   stops services and therefore the only thing that can know who holds what.
3. **One owner at a time.**
4. **The splash returns when the owner exits.**

#### Why this is the display becoming STATE rather than a free-for-all

Point 4 is not a courtesy. It makes the display something the supervisor
**converges**, exactly as `ADR-017` §4.1 has it converge services: there is a
desired state — *the splash, unless an owner is running* — an owner displaces it
for a while, and the supervisor restores it when that owner stops. A screen left
showing a dead program's last frame is drift, and drift is the thing convergence
exists to remove.

`cmd/rivet-splash` was written as a one-shot blit that holds nothing open, so it
is already the right shape for this: restoring the idle state is running it
again.

#### Unlike the rest of §6's table, the permission half is enforceable NOW

Measured on the board 2026-08-30: `/dev/fb0` is `rw------- root root`. An
application runs as its own uid (§5.1.1), so it **already cannot open the
framebuffer** — the kernel refuses, with no Rivet code involved. That falls out
of `ADR-024`'s devtmpfs and **no udev**: nothing relaxes the kernel's default the
way a distribution's rules would with `0660 root:video`.

So this capability does not need the machinery §6's table is waiting on. It
needs no namespace, no cgroup and no socket passing, because **a device node is
a filesystem object and Unix has arbitrated those since the beginning**:

| Half | What it costs |
| :-- | :-- |
| undeclared may not draw | **nothing — already true**, by default-deny |
| declared is granted | the supervisor chowns the node, or adds a supplementary gid, when it starts that service |
| one owner at a time | supervisor policy: refuse to start a second declarer |
| the splash returns | the supervisor runs `rivet-splash` on the stop edge |

The grant is **per boot**, because devtmpfs recreates the node every boot. That
is the right lifetime: a permission outliving the manifest that asked for it is
a permission nobody can audit.

**Which nodes *"the node"* means was narrowed on 2026-09-04**, and a reader
arriving here for that answer must go on to `ADR-036` §3.1.1 (`.rulings/`
R-138). The class is still one thing an author declares; what it expands to
depends on whether the declaring service is the one that holds DRM master and
leases the display out, or one of the applications handed a lease. A **lessee**
gets `/dev/dri/renderD*` alone — `/dev/dri/card*` and `/dev/fb0` are two ways
around the lease, and granting them made the platform's own rule untrue.

**The table's two forms of grant are both used, and the node decides which.**
A card node, the framebuffer and an input node are **chowned** to the one
service granted them, because each has one holder: DRM master is exclusive,
and two programs writing one framebuffer interleave. A **render node** is
**shared**, because any number of processes render on one GPU and two services
hold it at once, each under its own uid — the compositor as the display
arbiter, and every application declaring `gpu`. A chown to one uid would take
it from the other, so a render node is `root:render 0660` and the supervisor
adds the image's `render` group (`rivet_core::etc::RENDER_GID`) to the
supplementary groups of exactly the services it grants the node; the stop edge
puts the node back to that shared form. It is never `0666`: Unix permission is
half of what keeps an undeclared application off the GPU, and a world-writable
node would leave that half to nothing. (`ADR-058` §5.6 has the case that forced it.)

#### The sub-question this leaves, with a recommendation

**What should happen when a DOCUMENT enables two display-declaring services?**

Refusing at convergence — starting the first and declining the second — is the
obvious answer and it is the worse one. `ADR-007` §3.6 rules that *the system
never accepts an input it will discard*, and a document asking for two owners is
exactly such an input. It should be **refused where it is written**, by
`apply document` and `apply edit`, so the operator learns at the edit rather
than from a service that mysteriously never starts.

That is the same reasoning that made a table naming a service the image does not
carry invalid configuration on this day
rather than a key reported as spent, and the two should not
be decided differently a month apart.

#### It blocks Aura, deliberately

**Ruled: this feature-set is required before `aura-controller` is implemented.**
Aura is the first application that wants the screen
(`docs/design-candidates/Shared-Libraries-and-the-Build-Target.md` §5.7.1), and
porting it first would mean one program with unarbitrated access to a device
nothing else could be given afterwards without taking it away.

**~~Not built.~~ BUILT 2026-08-31** — all four points, and each is proven on
hardware in `PLAN.md` §104.1. Two decisions were taken beyond this section and
both are flagged there: `input` is exclusive on an argument this section does
not make, and the grant is revoked on the stop edge rather than merely per
boot.

**The document sub-question above is still NOT built.** Two display-declaring
services enabled in one document are refused at CONVERGENCE, loudly and by
name, which this section calls "the worse one" — refusing them where they are
written is `PLAN.md` §104.7 item 5.

---

## 7. Atomicity

**Ruled 2026-08-22: atomic per-artifact install is in scope. Transactional
apply is not, and is not needed.**

**Per-artifact atomicity is nearly free.** Unpack to
`/state/app/<name>/<version>/`, verify the signature, then swap a symlink — a
`rename`, not a copy. This matters more than it sounds: a Pi has no battery, is
usually on an SD card, and gets its power pulled. A half-installed application
is an ordinary failure, not a theoretical one.

**Transactional apply across heterogeneous changes is expensive and
unnecessary**, because the desired-state model already offers something cheaper:
**convergence.** A failed `apply` leaves a machine that does not yet match its
document, and the remedy is to run `apply` again. That is the reconciliation
model rather than the rollback model, and it is both less work and a better fit.

It is in scope now rather than later because **the shape must be right from the
start**: every operation must be idempotent and re-runnable (`ADR-006` §6.2).
Get that wrong early and convergence never works at all.


### 7.1 Built 2026-08-24, and what it turned out to cost

The install is a **typed plan** and an executor, split at `ADR-014` §2's line:

| | |
| :-- | :-- |
| `rivet_core::package::install` | the `Plan` — an ordered list of `Step`s. No I/O. |
| `cmd/rivet-deploy` | performs one. |

*(Read ~~`cmd/rivet-update`~~ until 2026-08-29. The binary was renamed
`rivet-deploy` by the maintainer on 2026-08-24 and the record was not followed
through; `ADR-029` §14 carries the correction and the twenty-two sites it
covered.)*

**`ADR-006` §6.2's dry run is reached by construction rather than by a flag.** A
dry run renders the plan; a real run performs it; there is one description of
what would happen and it is the same object. A `--dry-run` that skipped writes
would be a second code path, and the one that is not exercised is the one that is
wrong.

**The order is the correctness argument**, and it is tested by walking the plan
rather than by reading it: every directory is made before anything is written
into it, and the activation is last and singular. So a plan abandoned anywhere —
including by the power being pulled, which is this section's premise — leaves an
unreferenced version directory and a `current` that still points where it did.

#### 7.1.1 The name comes from the header, and a version may not be `data`

§3.2's manifest deliberately carries no `name`, so `cmd/rivet-sysd`
observes it from the directory. **An installer has no directory to observe** — it
is the thing deciding which directory — so the name comes from the container
header's `product`, which is the field `ADR-029` §3.1 grew to 32 bytes the same
morning.

Every name in a package is chosen by whoever built it, and the defence turns out
to be in four places rather than one. Recorded because the implementation's first
draft claimed more than it did:

| refused by | what it catches |
| :-- | :-- |
| `format::read_name` | `/`, a backslash, `.`, `..`, empty, non-printable — in a **member name**, before a signature is computed |
| `format::read_text` | non-printable in a header field, and an empty `version` |
| `service::ServiceName` | a name that could not also be a cgroup and a socket (`ADR-017` §3.1.3) |
| `install::safe_component` | a `/` in a **version**, which `read_text` allows — and any name that already *means* something under an application's root |

That last row is the least obvious and the most valuable. `current` and `data`
are perfectly well-formed names. **A version called `data` would put a directory
where everything the application has ever written lives.** Neither is a traversal
and both are worse than one.

#### 7.1.2 Nothing is written before everything is verified

Every member's digest is checked against the header before the first byte reaches
the filesystem — `ADR-029` §5 step 8. It costs reading each member twice, once to
digest and once to write.

The alternative, verify-as-you-write, leaves a half-written program on `/state`
when a member turns out to be wrong. It would then have to be removed, and **a
removal that fails is how a machine ends up with a binary nobody signed.**

Proven rather than asserted: removing the verification loop makes the
tampered-payload test *install the tampered application*.

#### 7.1.3 What this does not do

* **It removes nothing.** Older version directories stay, which is what makes
  going back a symlink swap rather than a reinstall. Nothing reaps them and this
  section does not say when to, so a second install costs disk. Named here rather
  than discovered on a full card.
* **It does not start anything.** The supervisor does, on its next convergence,
  and only if the document enables it (§3.2.1).
* ~~**It does not verify at launch.** §4.2 requires verification *at every launch*
  and `cmd/rivet-sysd` still executes what it finds. The trust store and
  now the installer are the preconditions for fixing that; neither is the fix.~~
  **Superseded 2026-08-25 by §4.2.1's *"BUILT 2026-08-25"*** — `cmd/rivet-sysd/src/verify.rs`
  re-verifies the retained `package-header` at every launch. Struck 2026-08-29;
  the two sections had contradicted each other for four days.
* ~~**It has not run on a board, or under QEMU.** The tests are against a fake
  kernel. `AGENTS.md` §6: *until it runs, it isn't working.*~~
  **FALSE since 2026-08-25 — struck 2026-08-29.** `PLAN.md` §14 is titled *"An
  application was installed, run and updated on hardware"*, and §14.2 records
  the first execution of `cmd/rivet-deploy` on a Raspberry Pi reached through
  this path: *"`apply install` wrote the request, the supervisor **adopted it
  and created a service instance that did not exist at start-up**, the installer
  read the machine's own trust store, verified the signature, and refused on
  entitlement."* §19.4 then installed a 683 MB payload, took a `trial-restart`
  onto the written slot, and ran `apply uninstall rivet-todo` with
  `/state/app/rivet-todo/data` kept — which is §3.2A's layout, observed on the
  board rather than asserted here.

  **What is still true, and is the sentence this should have been:** no torn
  write, no full card, and no power cut has been induced during an application
  install. The tests around those remain fake-kernel tests.

#### 7.1.4 An operator can ask for one now — 2026-08-24

**`rivet apply install <digest> confirm`** is the request and
**`rivet get install`** is the report. `ADR-006` §8.8 is the operation,
`ADR-017` §4.5 is the one-shot service definition, `ADR-029` §14.7 is what the
installer's own ADR records — the maintainer's 2026-08-24 ruling.

**Three things about it belong here rather than there**, because they are about
*applications* specifically:

1. **The command cannot tell an application payload from a system update**, and
   must not try: §7.1.1's name comes from the container header and reading the
   header here would be a second verifier (`ADR-029` §11). So the plan says
   *"if it is an application it is unpacked under `/state/app` and made
   current"* alongside the update case, and the installer decides.
2. **A machine that cannot read its boot slots is not refused.** An application
   install has no slot anywhere in §7, so the plan omits the field rather than
   refusing — otherwise `apply install` would be unusable for applications on
   every machine that is not a Raspberry Pi.
3. **The answer still says nothing started it.** §7.1.3's third bullet is
   carried into the operator's own output: the outcome
   `installed-application` explains that the supervisor starts it on its next
   convergence, and only if the document enables it (§3.2.1).

**The command cannot succeed on any machine yet**, because how the path reaches
the installer is unruled — `ADR-029` §14.7.2. §7.1.3's *"it has not run on a
board, or under QEMU"* is therefore still true of the whole path.

### 7.2 Nothing is ever unpacked over a version directory that is there — 2026-08-29

**Ruled 2026-08-29**, Pile B item 1 (`PLAN.md` §102.3 item 2), and it is the
other half of `ADR-027` §3.2.2.

§7.1's install is a plan of `Step`s and its first steps are directories.
`Kernel::mkdir_mode` **succeeds when the directory is already there**, on the
deliberate and correct argument that `ADR-006` §6.2 makes every operation
re-runnable and an install that died halfway is exactly the case somebody
re-runs. Combine that with §7's directory being a **function of the version**,
and installing a payload whose version matches one already on the machine writes
its files straight into the directory `current` is pointing at.

**Two things break at once, and neither is visible from the outside:**

| | |
| :-- | :-- |
| §7's atomicity is gone | the whole argument is that a plan abandoned anywhere leaves an *unreferenced* directory and a `current` that still points where it did. A plan writing into the live directory leaves `current` pointing at a **half-replaced** install |
| a running program is being overwritten | on Linux `open(O_TRUNC)` over a busy executable answers `ETXTBSY` — so the failure arrives **after** the manifest and the signed header have been replaced, leaving an application whose header no longer describes its program. §4.2 re-verifies at every launch, so the next start refuses |

It was reached in practice rather than reasoned about: a `-dirty` version was not
unique (`ADR-027` §3.2.2), so two different builds of one working tree wanted one
directory. That naming defect is fixed; **this ruling is the half that does not
depend on a name being right.**

**Ruled: an install whose version directory already exists is REFUSED, and
nothing is written.**

`rivet_core::package::install::Plan::occupied` is the decision and
`cmd/rivet-deploy` supplies the one fact the core cannot have — whether the path
is on the filesystem, through a `Kernel::path_exists` that exists for this
caller alone. The check runs **before the member-verification loop and before the
dry-run branch**, so a `--dry-run` reports the refusal too: a dry run answering
*this would work* about an install that will be refused is the answer an operator
plans a deploy window around.

#### Why refusing, rather than staging and swapping

Because **there is no atomic swap of a directory to be had.** `rename(2)` over a
non-empty directory answers `ENOTEMPTY`, so *stage then swap* is two renames with
a window between them — and a window is precisely what §7 spent the `current`
symlink to avoid. The symlink is already the atomic switch and it switches
between *version directories*, so the shape that follows is that a version
directory is written **once and never again**.

#### What it costs, stated rather than discovered

**Re-running an install of a version already on the machine is now a refusal
rather than a no-op**, which narrows `ADR-006` §6.2's re-runnability for this one
operation. Accepted, because the alternative is writing over files something may
be executing — and because the refusal names both ways forward rather than
leaving an operator to invent a workaround:

* the bytes changed, so **the version changes with them** — since `ADR-027`
  §3.2.2 a build from a dirty tree carries a digest of ~~its own program~~
  **its whole payload** (amended 2026-09-04, `ADR-027` §3.2.2A2), so a rebuild
  and re-package produces a directory that is free. The struck words were the
  defect: a manifest-only edit left the version identical and this refusal fired
  on a change that had really happened, so the change silently did not;
* the bytes did not change, so **the application is already installed** and there
  is nothing to do.

**Deliberately not built: an automatic uninstall-then-install.** Replacing this
exact version means removing the old directory first, and removal is `ADR-006`
§8.10.13's dangerous class — the installer holds no delete capability at all
today (`cmd/rivet-deploy`'s `Kernel::unlink_symlink` is its only deletion and is
narrowed to a link it made). Giving the installer one so it can overwrite is a
larger decision than this one and is not taken here.

**Unproven.** The refusal has run against a fake kernel and against
`Plan::occupied` directly; **it has not run on a board.** What a board would
settle is the one thing the fake cannot: whether the `ETXTBSY` failure mode above
is what a Pi actually produces for a live application, which is the measurement
that would say whether the pre-check is defence in depth or the only defence.

### 7.3 The installed size is recorded at install, not measured later — added 2026-09-13 (`.rulings/` R-163, R34)

The fleet portal's Software drawer wanted an application's size on disk and had
no real number to show: `get file`'s own contract is that a directory's
`size_bytes` is the directory's own inode weight, not what is inside it, so the
drawer walked one level into the version directory itself and named the
approximation for what it was — a nested directory deeper than that went
uncounted.

**A fifth name in a version directory** — `current`, `bin`, `manifest.toml`,
`package-header`, and now `install-size` — a plain-text decimal byte count,
written by [`Step::Size`], `FILE_MODE` like the header beside it.

**Summed while writing, not by a second walk.** `cmd/rivet-deploy`'s executor
already reads every member's bytes to write them and already has the retained
header's length in hand; `Step::Size` spends nothing beyond a running total it
was already computing member by member, so the recorded figure is correct *by
construction* rather than a number some other pass has to trust. It counts
every [`Step::Member`] and [`Step::Header`] that ran before it and nothing
past itself — the file recording the total is not part of the total, which is
why it is written last, immediately before [`Step::Activate`].

**What it does and does not describe** (the maintainer's own question,
`.rulings/` R-163.1): it is the size of what THIS INSTALL wrote, and nothing
about it changes while that version stays installed — `rivet_core::confinement`
grants an application only READ, LIST and EXECUTE on its own version
directory, so nothing but a future install can change what is there. It says
nothing about the application's **data** directory, which is separate, grows
independently, and is a second, live number if the portal ever shows it. And
**an application installed before this existed has no record** — there is no
fallback walk, so it is reported as not recorded rather than approximated
again.

---

## 8. What this does to Rust applications

Honest assessment, since the answer is "mostly nothing".

**Unaffected:** anything that computes, listens, connects out, and writes to its
own data directory. A typical `axum`/`tokio` service needs no changes.

**Breaks, mostly deservedly:**

* shelling out — already impossible, there is no shell (`ADR-005` §5.4)
* hardcoded absolute paths outside the sandbox
* assuming `/tmp` is shared with other processes
* reading `/proc/<other-pid>`
* `dlopen` — already impossible under `ADR-003`

**Needs adapting:** software that speaks only TCP and cannot use a Unix socket.
`network = "group:<name>"` exists for exactly this case.

---

## 9. This is what the platform crate is for

`ADR-013` §6 left an unnamed crate for "what it means to be a program on
Rivet" and §7 deferred it for want of a consumer. **This ADR is the consumer**,
and the crate now has a job description: receive configuration, emit structured
logs into the system journal, accept passed-in sockets (§5.2), and expose the
application's own operations through `rivet`.

**That crate is `rivet-sdk`, and `ADR-036` is where it was ruled and built.**
Three of the four things above are what it hands an application (`ADR-036` §3);
the fourth — an application publishing its own operations — is the row of
`ADR-013` §6.4 that no crate satisfies, and it is the one part of the app-side
contract that a library must carry rather than a convention.

---

## 10. Open

1. **Where applications are built.** Cross-compiled from the Windows host for
   day one, per the maintainer. Whether a development image variant with a
   toolchain is ever wanted is unasked; the base image deliberately has no
   compiler.
2. **Whether an application may declare a dependency on a system extension**
   (`ADR-016`) — for instance a runtime or a shared data set. *Recommendation:
   no, initially.* It is the first step toward dependency resolution, which §3
   exists to avoid.

---

## History

Superseded text, kept for the reasoning behind a change. **The body is complete
without this section**; nothing here is a live requirement. Superseded text
keeps its strike-through, because `tools/docs/claims.py` masks `~~…~~` before
it audits a claim.

### In §5.1.1 — the time client kept uid 0

The time client was listed among the system services that keep uid 0, because
setting the clock needs a capability and the identity drop discarded every
capability: ~~`rivet-netd` speaks netlink, `rivet-timed` sets the wall clock,
and `rivet-mcpd` spawns the CLI that installs, arms and reboots the machine.~~
It parses replies from the internet and needs exactly one piece of root, so the
supervisor gained a way for a homed service to keep a named capability across
the drop, and the time client moved to its own uid keeping `CAP_SYS_TIME`
(`ADR-017` §3.9). (`.rulings/` R-210 D1.)

### In §3.2.1 — the document could only lower the default memory limit

The default limit was first built to stand where the manifest's request would
have, so the document's cap met it by the lower-wins rule:
~~the operator's cap meets it by the same rule, so a document can hold such an
application lower and cannot raise it higher.~~ That made giving an
application more than half the machine a manifest change, which the operator of
a machine cannot make. The default is not a value the signature covers, so the
direction test gave it no protection worth keeping, and a cap in the document
now replaces it in either direction. (`.rulings/` R-246.)
