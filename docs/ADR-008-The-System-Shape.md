# ADR-008: The System Shape — Rivet Is Not a Distribution

* **Status:** **Accepted** — ruled 2026-08-22. §4 is **superseded in part** by
  §4.2: image-only is the current unit of change, not a permanent property.
  §3 and §5 stand; §7 is answered. **§5 was amended and §5.1.1 and §5.3 added
  2026-08-23:** a second sweep of the linked `libc.a` recorded what musl does
  when each of its nine compiled-in paths is absent.
* **`/etc` has ONE generator, the image ships none of the five files, and the
  runtime projection renders all of them.** §5.3 carries the withdrawal of the
  ruling that briefly said otherwise on 2026-08-23. It is said in the status
  block because a reader who stops here would otherwise believe a ruling the
  body reverses — the same defect `ADR-007`'s status block records against
  itself. **§5.1.1's readings are untouched** and are the durable part of that
  day's work.
* **The one generator was built 2026-08-23.** `cmd/rivet-sysd`, before
  it converges any service — `ADR-017` §3.1.8, `ADR-007` §3.5.1.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`ADR-006` and `ADR-007` were written inside an assumption that was never
examined: that Rivet is a *Linux distribution* — a system that happens to be
tidier than the others, but that still has `/etc`, `/usr`, `/var`, a package
manager, a POSIX shell, and the Filesystem Hierarchy Standard underneath it.

**The maintainer's framing, 2026-08-22: Rivet is permitted to differ from
traditional Linux distributions to the same degree that *Android* differs from
them.** *(`.rulings/` R-021.)*

That is a much larger permission than "make the configuration tidy", and it is
worth taking literally, because Android is the existence proof that a system can
keep the Linux kernel and discard essentially everything conventionally built on
top of it — libc, the filesystem hierarchy, the init system, the package model,
the userland, the IPC mechanism and the administration model — and be more
coherent for it rather than less.

---

## 2. What Android actually discarded

Listed because the scale of the departure is the point, and because each row is
a decision Rivet now has to make deliberately rather than inherit.

| Conventional Linux | Android | Rivet position |
| :-- | :-- | :-- |
| glibc | Bionic | musl, static — `ADR-003`. Same move, already taken. |
| FHS (`/usr`, `/etc`, `/var`) | `/system`, `/vendor`, `/data`, `/apex` | **Departed — §3, ruled** |
| SysV / systemd init | Android `init` with `.rc` files | `rivet-initd` — `ADR-005`. Already ours. |
| Package manager | image updates; APKs for apps only | **Image-only for now — §4.2** |
| GNU coreutils | Toybox | `uutils` — `ADR-005`. Same move, already taken. |
| Config files in `/etc` | the property service | `ADR-007`, and see section 5 |
| Shell-based administration | API-based; nothing administers via shell | `ADR-006`. Already aligned. |
| Sockets / D-Bus | Binder with AIDL | **Not taken — section 6** |
| Unix DAC by convention | per-app UIDs, SELinux everywhere | **Not taken — §6. Conventional multi-user retained, §7.3** |

Four of the nine were already decided in Rivet's favour without the framing
being explicit. This ADR makes it explicit and takes two more.

### 2.1 The property service is corroboration

Android's property service is a typed-ish key-value namespace with access
control, in which **the key prefix encodes the value's lifetime and
mutability**: `ro.` is set once at boot and never changes, `persist.` survives
reboot, everything else is runtime state.

That is the same conclusion as `ADR-007` section 3.4's layering, reached
independently by a team solving the same problem at enormous scale. It is
useful corroboration that the layering idea is not merely tidy-minded.

Its failures are equally instructive, and `ADR-007` already avoids both: the
properties are string-typed, and there is no schema, so nothing can say which
properties are dead.

---

## 3. Decision: the layout encodes ownership and mutability

**Ruled 2026-08-22, as proposed, with lowercase names.** Configuration lives at
`/state/config` — see §3.1.

The Filesystem Hierarchy Standard describes what things *are* — binaries,
libraries, configuration, variable data. It says nothing about what matters
operationally, which is **who owns a path and whether it survives an update.**
That omission is why `/etc` contains both vendor defaults and local edits with
no way to distinguish them, and it is the root of the merge problem every
distribution upgrade has.

Android's layout answers the useful question instead. Rivet should do the
same:

| Path | Contains | Mutable | Survives an image update |
| :-- | :-- | :-- | :-- |
| `/system` | the Rivet image: binaries, image defaults | **no** — read-only, verified | replaced wholesale |
| `/state` | machine configuration, keys, persistent data | yes | **yes** — never touched by an update |
| `/run` | runtime state, generated files, sockets | yes | no — tmpfs |

The property this buys is the one FHS cannot state: **an update replaces
`/system` entirely and cannot touch `/state`.** There is no merge, no
`.dpkg-new`, no three-way reconciliation of configuration files, because
vendor content and machine content were never mixed in the first place.

It is also what makes A/B images and rollback straightforward rather than
heroic — `/system` is an image, and images are swappable.

**Designed 2026-08-22 in `ADR-018`.** The layout is five partitions, `/system`
is `erofs` and therefore read-only *by format* rather than by mount flag, and
the A/B switch is the Raspberry Pi firmware's own `tryboot` mechanism rather
than anything Rivet implements.

**A C mechanism goes in `/system/bin` beside the Rust binaries — 2026-08-23.**
The first one arrived from Buildroot and the question it raised is
whether §3's table has anything to say about it. It does, and the answer follows
from the table rather than needing a new row: **the columns are ownership,
mutability and update lifetime, and none of them is "who wrote it" or "what
language is it".** A statically linked C program built by Buildroot is vendor
content that no application may write and that an update replaces wholesale,
which is `/system` exactly. Nothing about it being C, or about it coming from
somebody else's build system, moves it.

The two alternatives were considered and both would have introduced a
distinction this layout exists to avoid. A separate `/system/mechanisms` would
sort by provenance, which §3 says is not what a top-level name is for. An
FHS-shaped `/usr/sbin` is the one `ADR-009` §6 names as the failure mode of
adopting Buildroot at all: *"if Buildroot's own root filesystem conventions ever
reach the image, the boundary has been crossed — those conventions are the FHS,
which `ADR-008` §3 rejects."*

### 3.1 Rivet's own configuration lives at `/state/config`

Not `/etc`, which is the mechanisms' compatibility address (§5.1). Not a
top-level `/config`, which was considered and rejected: **a top-level name is
supposed to state ownership and lifetime, and `/config` states neither.** The
whole point of this layout is that the first path component answers "who owns
this, and does it survive an update"; adding a root directory that does not
answer it would be the first crack in the principle.

`/state/config` answers both. It is the operator's, and an update never touches
it.

**Built 2026-08-23. It is a directory**, which this section implies and never
says — `ADR-017` §3.1.7 lists it beside `/state/home` and `/state/ext`, and
`ADR-007` §3.4 has three persisted layers, so what sits at this path is one file
per layer. The machine layer is `/state/config/machine.toml`; the file name is
derived rather than ruled here and is `rivet-store`'s to change, and
`rivet_core::state::CONFIG_DIR` is the one place the *directory* is spelled.

**It is also the one Rivet file an operator can open and edit with effect**,
because §3's table makes `/state` writable and `ADR-018` §4 makes it the only
writable filesystem. That is not a contradiction of `ADR-007` §3.6.2 — the file
is the store, not a view of it — but the mistake it invites is the same one, so
the document carries the provenance header `ADR-007` §3.6.1 item 2 asks for.

Home directories, by the same reasoning, are `/state/home` — see §7.3.

### 3.2 Names are lowercase

`/system`, `/state`, `/run`. Considered and not taken: capitalised `/System`
and `/State`, which read more deliberately and would visually distinguish
Rivet's own paths from the inherited compatibility ones. No mechanism cares
either way; lowercase matches Android and Unix habit, and surprises nobody.

---

## 4. SUPERSEDED IN PART, 2026-08-22 — see §4.2

*The original text is retained below because its argument about what a
desired-state document can guarantee is still correct and still governs. Only
the word "ever" is withdrawn.*

**The unit of update is the image. Rivet does not have a package manager and
is not going to grow one.**

This follows from section 3 and it eliminates a large class of problems by
construction rather than by care: dependency resolution, partial upgrade states,
a machine whose installed set no document describes, and the divergence between
two hosts that were "installed the same way".

It also makes `ADR-007`'s guarantee real. A desired-state document can only
fully describe a machine if the machine's contents are not independently
mutable; a package manager is precisely a mechanism for mutating them
independently.

**The cost, stated honestly:** adding software means building an image, not
installing a package. That is slower and less convenient, and it is the correct
trade for a system whose whole premise is that its state is describable.

---

### 4.1 What "add-on" must mean, so it does not become a package manager

`ADR-005` section 5.4 rules that a POSIX environment may arrive later "as an
add-on". That word is the one most likely to reintroduce a package manager
through the side door, so it needs a definition before it acquires a
convenient one.

**An add-on is a build-time image variant, not an installable artifact.**
Adding the POSIX environment means building an image that contains it — the
same mechanism as any other change to `/system`, which is read-only and
replaced wholesale (section 3). There is no install step, no dependency
resolution, and no machine whose contents differ from the image it was built
from.

The alternative, if variants later prove too coarse, is an **Android APEX-style
mounted layer**: a signed, versioned, read-only image mounted into the namespace
at boot, still immutable and still fully described by the desired-state
document. That is real machinery and it is not proposed now, but it is the
direction that stays consistent with section 4 if variants become unwieldy.

**Taken up 2026-08-22 — see `ADR-016`.** Variants did prove too coarse, and
`helix` was the case that proved it: taking "add-on = image variant" literally
meant reflashing the operating system to get a text editor. System extensions
are that mechanism, mounted at `/system/ext/<name>`.

**Variants are not superseded, they are narrowed.** Variants produce genuinely
different *base* images. Extensions add to a base. Neither does the other's job.

**Amended 2026-08-22** (`PLAN.md` §4.13): this paragraph named "headless and
GUI" as the example of genuinely different base images. **The GUI was ruled a
system extension**, so that example is withdrawn and **no example survives it**
— every candidate base variant so far has turned out to be an extension. The
distinction above is still sound; it currently has no instance. If none is ever
found, `ADR-016`'s mechanism has quietly replaced this one rather than narrowing
it, and that would be worth ruling on explicitly rather than discovering.

What is **not** available is installing software into a running Rivet system
such that the machine's contents diverge from a document. That is precisely the
mutation `ADR-007` cannot describe, and section 4's whole argument.

### 4.2 Ruled 2026-08-22: image-only *for now*, and built so packaging is additive

**The maintainer narrowed §4. He ruled: no package management for the operating
system's features — one image, for now. His plan at the time was a headless
image and a GUI image, with everything else packaged by hand in the interim. But
he expected the network management to lead to genuinely wanting to push updates
to hosts, so the project should be smart about *how* it declined a package
manager for the present — start by organising things such that **when** packaging is wanted it is
no great matter, and the work is building a package manager rather than
rearranging everything first.** *(`.rulings/` R-022.)*

**"Ever" is withdrawn.** The image is the only unit of change *today*, and
variants are the mechanism (§4.1). It is not claimed to be permanent, and the
network management of `ADR-006` §5.1 is the thing most likely to make packaging
genuinely wanted.

**Two parts of what he said were themselves superseded, and the ruling is
recorded as it was made rather than tidied.** The two-image plan — one headless,
one GUI — was superseded on 2026-08-22 (`PLAN.md` §4.13): the GUI is a system
extension and there is one base image. The expectation of packaging everything by
hand was answered the same day by `ADR-016`. The instinct the ruling records —
*organise now so that packaging is later a build rather than a rewrite* — is what
actually governs, and it survived both rulings intact.

The instruction is the useful part: **be organised now so packaging is additive
later.** Three things do that, and each is cheap today and expensive to
retrofit.

1. **The build emits a manifest.** What is in the image — component names,
   versions, the files each component owns, and hashes. A package is then a
   *subset of a manifest that can be installed separately*. Without one, the
   image is an opaque blob and packaging starts from nothing. This is the single
   highest-value item.
2. **The configuration schema carries a components field from the start.** The
   document says which components a machine should have, even while the only way
   to satisfy that is building an image containing them. This is what makes
   packaging **an implementation of `apply` rather than a parallel system** —
   which is the `ADR-011` requirement, since two ways to determine what is
   installed would be two authorities.
3. **Artifact delivery is built generically the first time.** Signing, transport
   and verification for A/B image updates are the same machinery packages would
   need. Writing it image-specific and generalising later is a rewrite;
   writing it artifact-generic now costs almost nothing.

**What does not change:** software may never be installed such that the
machine's contents diverge from the document. If packaging arrives, it must
*satisfy* the document, not escape it. That is the guarantee §4 was protecting,
and it survives the withdrawal of "ever" intact.

---

## 5. Decision: `/etc` is a compatibility shim, not a location

`ADR-007` section 3.5 already made `/etc` generated output on tmpfs. Under this
ADR the framing sharpens: **`/etc` is not part of Rivet's design at all.** It
exists solely because the C mechanisms of `ADR-001` expect files at paths they
hard-coded, and it is generated into `/run` and mounted where they look.

**The gap that "generated into `/run`" left was real, and it was closed by
writing the runtime generator** (`ADR-007` §3.5.1, `ADR-017` §3.1.8) rather than
by adding a second one. An amendment on 2026-08-23 proposed the second generator
and was overruled the same day; §5.3 carries that.

**One clause of the original paragraph above is now a divergence rather than a
prediction, and it is worth naming.** §5 says `/etc` "is generated into `/run`
and mounted where they look". What was built mounts a tmpfs **directly at
`/etc`** and renders onto it; there is no staging directory under `/run` and no
bind mount. The two are equivalent in what a program at `/etc/shells` sees, and
the direct form has one property the staged form does not: `ADR-007` §3.6.1
item 1 wants the filesystem the mechanisms read to be genuinely read-only, and a
tmpfs that is remounted `MS_RDONLY` after the write is that, whereas a writable
`/run/etc` behind a bind mount is not. If a staging directory is ever wanted —
to render a whole `/etc` and swap it atomically, say — that is a change with an
argument, not a correction of a drift.

The distinction matters because it settles a question that will otherwise recur
for ever: when a new mechanism wants a configuration file, the answer is never
"add it to `/etc`". The answer is that Rivet's document gains a typed field
and the generator gains an output. **The mechanisms' formats stay at the bottom
of the stack and never propagate upward.**

### 5.1 Why `/etc` must exist at all — and it is not POSIX

The maintainer asked, 2026-08-22, whether the generated text representations
need to be at `/etc` given that POSIX compliance is not a goal. The honest
answer is that **`/etc` is mandatory, but not for any of the reasons one would
expect.**

Not the bootloader: the Raspberry Pi firmware reads the FAT partition and the
kernel image, and never looks at the root filesystem's layout. Not the kernel,
which reads almost nothing from userspace paths. Not POSIX, which Rivet has
already declined to guarantee (`ADR-005` §5.4).

**It is libc.** musl has these paths compiled in, and they are not configurable
at runtime:

| Path | Read by | Reached through |
| :-- | :-- | :-- |
| `/etc/passwd`, `/etc/group` | any `getpwnam` / `getgrnam` caller — **every login, and every C mechanism that resolves a name** | musl |
| `/etc/resolv.conf` | DNS resolution | musl's resolver |
| `/etc/hosts` | name lookup | musl's resolver |
| `/etc/services` | port-name lookup | musl |

So as long as Rivet runs C mechanisms linked against musl — which `ADR-001`
says it does, permanently, for `wpa_supplicant` —
`/etc` exists at that address because that is where those programs will look.
Patching musl is not on the table.

**CONFIRMED in musl's sources, 2026-08-22.**
The paths are string literals, and there are more of them than the table above
listed:

| Path | musl source |
| :-- | :-- |
| `/etc/passwd` | `src/passwd/getpw_a.c`, `src/passwd/getpwent.c` |
| `/etc/group` | `src/passwd/getgr_a.c`, `getgrent.c`, `getgrouplist.c` |
| `/etc/shadow`, `/etc/tcb/%s/shadow` | `src/passwd/getspnam_r.c` |
| `/etc/resolv.conf` | `src/network/resolvconf.c` |
| `/etc/hosts` | `src/network/lookup_name.c` |
| `/etc/services` | `src/network/lookup_serv.c` |
| `/etc/localtime` | `src/time/__tz.c` — the fallback when `TZ` is unset |
| `/etc/shells` | `src/legacy/getusershell.c` — and an account whose shell is not listed is rejected before authentication, measured on hardware |

Two of these were not in this section's table and change what Rivet generates:
**`/etc/shadow`** is read by `getspnam_r`, so any password authentication path
reaches it; and **`/etc/localtime`** is musl's default when `TZ` is unset, which
ties `ADR-021`'s timezone handling to a generated file rather than to an
environment variable alone.

**Method:** every file in `src/passwd`, `src/legacy`, `src/misc` and
`src/unistd` was searched exhaustively for `"/etc/…"` literals, plus targeted
reads of `src/network` and `src/time`. **Not swept:** the rest of the tree, so
this is a floor rather than a ceiling — treat an additional path found later as
expected, not as a contradiction.

#### 5.1.1 Swept a second time, 2026-08-23 — eight confirmed, one added, and the fallbacks matter more

The sweep above read musl's **sources**. This one read the **`libc.a` Rivet
actually links**: the `aarch64-unknown-linux-musl` archive from the Rust
toolchain's `self-contained` directory, `ar x`'d into objects and examined with
`strings` and `objdump -s`. Two independent artifacts, which is what `AGENTS.md`
§7 asks for before anything is written down.

**All eight paths above are confirmed present**, and a ninth was found exactly
as §5.1 predicted:

| Path | Object | Note |
| :-- | :-- | :-- |
| `/etc/zoneinfo/` | `__tz.lo` | one of three prefixes musl searches for a **named** zone, with `/usr/share/zoneinfo/` and `/share/zoneinfo/` |

**The more useful finding is not the list but what musl does when a file is
absent**, because that is what decides whether Rivet has to ship one. Three of
these turn "leave it out" from a gap into a wrong answer, and three make leaving
it out correct:

| Path | Absent means | Evidence |
| :-- | :-- | :-- |
| `/etc/shells` | `getusershell()` returns **`/bin/sh` and `/bin/csh`** | `.rodata.defshells` in `getusershell.lo` is exactly `"/bin/sh\n/bin/csh\n"`; the object imports `fopen`, `fmemopen`, `getline`, `fclose` and nothing that could parse a comment |
| `/etc/hosts` | `localhost` is resolved **over DNS** | the literal `localhost` appears **zero** times in the whole of `libc.a`; `name_from_hosts` in `lookup_name.lo` is the only reader |
| `/etc/passwd`, `/etc/group` | `getpwnam`/`getgrgid` find nothing | `getpw_a.lo`, `getgr_a.lo` |
| `/etc/resolv.conf` | musl uses **`127.0.0.1`** | the literal is in `.rodata.__get_resolv_conf.str1.8` beside the keyword set |
| `/etc/localtime` | musl keeps **UTC** | `__tz.lo` carries `UTC`, `GMT` and a `__utc` symbol |
| `/etc/services` | a service **name** fails; numeric ports are unaffected | `.rodata.__lookup_serv.str1.8` holds only `/etc/services`, `/udp`, `/tcp` — **there is no built-in table** |

The `/etc/shells` row is the sharpest and it corroborates the hardware
measurement from the other side: the rejection was not a missing
file producing an error, it was a missing file producing a *different answer*.
Today Rivet ships neither `/bin/sh` nor `/bin/csh`, so the outcome is the same
rejection either way — but the day a POSIX environment ships as an add-on
(`ADR-005` §5.4), an absent `/etc/shells` would silently authorise `/bin/sh` as
a login shell, which is the one thing that section removed from the
administration surface.

**One genuine kernel-side path exists too:** the firmware loader searches
`/lib/firmware` by default (`ADR-004` §8 depends on this). Unlike `/etc` it is
overridable — via the `firmware_class.path` parameter — but the default is
compiled in, so `/lib/firmware` is the path of least resistance rather than a
free choice.

### 5.2 What this does *not* settle

None of the above says anything about where **Rivet's own configuration**
lives. `/etc` is the mechanisms' address for the mechanisms' formats, generated
and read-only. Rivet's machine configuration is a different artifact with a
different owner, and its location is genuinely open — see §7.

---

### 5.3 Ruled 2026-08-23 and OVERRULED the same day: `/etc` has one generator


> **OVERRULED by the maintainer, 2026-08-23.** This section was written,
> implemented, and withdrawn on one day. `/etc` has **one** generator — the
> runtime projection — and it renders all of `/etc`. `cmd/rivet-image`
> produces nothing under `/etc`, and the directory ships empty. The full
> reasoning is at `ADR-007` §3.5; the short form is that nothing about a file
> being a fact-about-the-image *requires* it to be made at build time, and one
> generator needs no argument about disjointness to satisfy `ADR-011`.
>
> **§5.3.1 below survives and is the reason this section is struck rather than
> deleted.** Which files must exist, which must not, and the readings of the
> linked `libc.a` behind each are unaffected by *who renders them* — that work
> is the durable part and `rivet-core::etc` still holds it.
>
> **And the one generator is now built, later the same day.** It is
> `cmd/rivet-sysd`, running before it converges any service —
> `ADR-017` §3.1.8 for the argument, `ADR-007` §3.5.1 for the shape. Five files
> on a read-only tmpfs, observed under QEMU. So the sentence above — "`/etc`
> ships empty" — is still exactly true of the *image* and no longer true of a
> *running machine*, and those two were being read as one thing.

§5 said `/etc` is generated and never authored. It did not say **by what**, and
the unstated assumption — that the only generator is the one that projects the
configuration document — is what left `/etc` empty on every card written up to
2026-08-23 while hardware measurement made one of its files mandatory.

**The rule:**

> **A file in `/etc` is rendered by the image when its content is a fact about
> the image, and by the runtime projection when it is a fact about the machine.
> Both render from one definition apiece, so neither is a second authority.**

`ADR-011` §6 is explicit that "the rule is violated by a second authority for
the *same* fact, never by a large number of components each owning their own".
Two generators over disjoint facts is legal; two over one fact is not. The
split, per fact rather than per file:

| Fact | Authority |
| :-- | :-- |
| which binary in this image may be a login shell | the image |
| which uid the image's own files belong to | the image |
| what `localhost` means | the image — a property of the loopback interface |
| which humans have accounts | the document — §7.3 |
| a password hash | the document, protected by `ADR-020`; **never the image** |
| which nameservers to use | the DHCP lease or `net.dns` — `ADR-023` |
| what timezone this machine keeps | the document — `ADR-021` |

Note `/etc/passwd` appears on both sides. That is the split working, not a
conflict: *does uid 0 have a name* is the image's fact and *does `joel` have an
account* is the document's, and they happen to be projected into one file.

**The mechanism is `ADR-017` §4.1's, borrowed.** System service definitions are
compiled into `rivet-sysd` rather than shipped as a data file under
`/system`, because "a file would buy nothing a constant does not, while costing
a second schema and a second parser — which is exactly what `ADR-011` exists to
prevent". The image-owned `/etc` facts are constants in `rivet-core` for the
same reason, and both generators call the same function — so when the runtime
tmpfs arrives, its projection is a **superset** of the image's by construction
rather than by anybody remembering.

#### 5.3.1 Which files ship, and the rule that decides it

> **Ship a file when its absence produces a *wrong answer*. Leave it out when
> its absence produces a clean failure.**

That is `AGENTS.md` §5's *refuse rather than guess* pointed at musl's fallbacks,
and §5.1.1's table decides all nine paths without a judgement call.

**The rule is untouched since it was written; the table below moved twice,
because the DOCUMENT grew a key each time and not because the rule was
relaxed.**

| Path | Verdict today | Since |
| :-- | :-- | :-- |
| `/etc/passwd`, `/etc/group`, `/etc/shadow`, `/etc/shells`, `/etc/hosts` | **ship, always** | unchanged |
| `/etc/resolv.conf` | **ship, when `[network.resolver]` names one** | ruled 2026-09-01 |
| `/etc/localtime` | **ship, when `timezone` names a zone this image has** | `ADR-021` §9, 2026-09-02 |
| `/etc/services`, `/etc/zoneinfo/` | leave out | unchanged |

Both movements are the rule applied to a machine that has been *told*
something, and neither changes what an unconfigured machine gets. *Absence
produces a wrong answer* was false for `/etc/localtime` while no document could
name a zone — UTC is the honest answer for a machine with no opinion — and
became true the moment one could: a document that says `America/Chicago` on a
machine with no `/etc/localtime` is a wrong answer arrived at in complete
silence, which is the sharpest instance of this rule in the table.

**`/etc/zoneinfo/` did not move with `/etc/localtime`**, and the two look as
though they should. It is one of the three prefixes musl searches for a zone *by
name*; nothing on a Rivet asks by name, because `TZ` is unset and the database
lives on the read-only system slot where the projection reads it directly
(`ADR-021` §9.4).

Two of those decisions are worth stating rather than leaving to the table.

**`/etc/shells` ships present, and if it has no entries that is an answer.** A
present-and-empty file says this machine has no login shell; an absent one would
say `/bin/sh` and `/bin/csh` are the login shells (§5.1.1). The file's content
is derived from the machine's own tree, so it gained the real path on the day
the `rivet` CLI shipped and not a day before.

> **Amended 2026-08-23 with the withdrawal above.** "The build refuses if it
> names a binary the image does not contain" was the build-time renderer's
> guard, and there is no build-time renderer. The property it protected is kept
> and is now *stronger*, because it is a fact rather than a check: the
> projection asks the running filesystem whether
> `rivet_core::etc::LOGIN_SHELL_PATH` is there, and lists it only if it is. A
> probe that *fails* — as distinct from one that answers "not there" — also
> lists nothing, and says why: an unlisted shell rejects a login before
> authentication naming the shell, while a listed-but-absent one fails at `exec`
> after authentication naming nothing.
>
> **And later the same day the CLI shipped**, so the struck sentence above is no
> longer the state of things: `cmd/rivet` exists, `cmd/rivet-image` installs
> it at `LOGIN_SHELL_PATH`, and the projection's probe now finds it. `/etc/shells`
> names a shell on a card built today. It is struck rather than rewritten because
> the reasoning around it — why *empty* was the honest answer while it was true —
> is what stops someone reintroducing an absent file as a tidy-up.

**Both halves of the struck sentence changed on 2026-08-23, in opposite
directions, and the combination is why it is struck rather than edited.** The
image stopped shipping `/etc` at all — `ADR-007` §3.5 gave the whole directory
to the runtime projection, so §5.3's build-time generator was deleted and this
subsection's list of five is the definition rather than the manifest. And
`cmd/rivet` was written, so the CLI **does** exist, is installed at
`/system/bin/rivet`, and is what `system::LOGIN_SHELL` names. The final clause
above is the one that survived intact and did its job: the path arrived on the
day the binary did.

**`/etc/shadow` ships with the account locked and no hash, ever.** §7.3's
closing warning and `AGENTS.md` §8.3 both forbid a hash in a signed artifact
that anybody may hold a copy of. The file is shipped anyway, at mode `0600`,
because "no account in this image has a password" is worth stating in the place
musl looks rather than leaving to be inferred from a fallback — `ADR-011` §2's
second clause.

#### 5.3.2 Two honesty mechanisms, and one that does not fit

`ADR-007` §3.6.1 requires a provenance header on every generated file and calls
it the highest-value mechanism in its list. **Only two of the five carry one.**

`/etc/passwd`, `/etc/group` and `/etc/shadow` are colon-separated databases with
no comment syntax musl's parsers are known to honour, and a header line handed
back as a *user record* would be considerably worse than no header. They carry
none, and `ADR-007` §3.6.1 item 3's reverse lookup — `rivet explain
/etc/passwd` — is where their provenance belongs.

`/etc/shells` and `/etc/hosts` carry one, with a wrinkle recorded so nobody
later assumes otherwise: **musl's `getusershell()` does not skip comments**
(§5.1.1), so it returns the header lines verbatim as candidate shells. Harmless,
because no account's shell is ever the string `# Generated by Rivet…` — but it
means nothing load-bearing may go in that header.


---

## 6. Deliberately not taken from Android

The framing licenses radical departure; it does not license every departure.
The distinction that matters is cost, and it is sharp:

**The cheap radical moves are decisions, not code.** Filesystem layout, the unit
of update, whether a package manager exists, where configuration lives. Each of
those is a paragraph in a document and a build script. Android's versions of
them cost almost nothing to adopt.

**The expensive ones are engineering programmes.** Android had a very large team
and could afford to replace mechanisms wholesale. Rivet has one maintainer.

Therefore **not taken**:

* **Binder and an interface definition language.** One typed IPC mechanism for
  all system services is genuinely better than sockets and ad-hoc protocols. It
  is also a kernel driver, a compiler, and a permanent maintenance obligation.
  `ADR-006`'s core-and-adapters boundary captures most of the benefit —
  operations are typed and declared in one place — at a fraction of the cost.
* **A comprehensive SELinux policy.** Correct, and a full-time job to author and
  keep correct. Revisit if Rivet ever runs untrusted code, which it currently
  has no plans to.
* **A bespoke libc.** `ADR-003` chose musl for reasons that still hold.

**Recorded risk:** the failure mode of this ADR is using "we are not a
distribution" to justify replacing things that were fine. Every departure needs
to name the specific conventional behaviour it is avoiding — as sections 3, 4
and 5 each do. A departure that cannot name one is novelty, not design.

---

## 7. Answered 2026-08-22

1. **The layout of §3** — accepted as proposed, lowercase, configuration at
   `/state/config`. §3.1, §3.2.
2. **"No package manager"** — narrowed to *for now*, with three organising
   commitments that make packaging additive. §4.2.
3. **The Unix user model** — see §7.3.

### 7.3 Ruled: full Unix multi-user

Against the recommendation, which was one administrative identity plus
unprivileged service identities. **The maintainer's answer is better and the
recommendation was based on a misreading of the cost.**

The reasoning: Rivet does not have to *build* a user model. UIDs and GIDs are
kernel concepts and are already implemented; musl and every C mechanism already assume
the conventional model (§5.1). "Full multi-user" therefore mostly means *not
fighting it*, which is cheaper than the partial model I proposed — and the
partial model would have been another instance of the incomplete-compatibility
failure `ADR-005` §5.4 rejects.

It also composes unusually well with the rest of the design:

* **Users are configuration.** The desired-state document lists them; `/etc/passwd`
  and `/etc/group` become *generated projections* of that document, exactly like
  every other file in `/etc` (`ADR-007` §3.5). No special case.

  **Narrowed 2026-08-23 by §5.3:** this is about *human* users. `root` is not
  one of them — it is uid 0, it is what every file in the image belongs to, and
  it exists whether or not anybody has written a document. So the image's
  `/etc/passwd` carries the system identities and the document's projection
  carries the humans, over disjoint facts. Service accounts arrive with the
  services that need them and are **not** pre-created.

  **Split 2026-08-24, and only one half was built — see §7.3.1.**
* **User management needs no new verbs.** `rivet config set` plus `apply`
  creates, changes and removes users, and the change is previewable and
  attributable like any other. There is no `useradd`.
* **Home directories live at `/state/home`** (§3.1), which is persistent and
  untouched by updates — the correct answer falls straight out of the layout.

The one thing to watch: a user model in the document means user records are
`plan`-visible and provenance-tracked, so password hashes must be handled with
the same care as the wireless PMK in `ADR-004` §5 — never plaintext, and never
in a provisioning file that outlives first boot.

### 7.3.1 Ruled 2026-08-24: **service identities, not human users**

The ruling splits §7.3 in two: Rivet builds an identity for each service, and
human user accounts wait. *(`.rulings/` R-282.)* Only the **service** half was
built, and the **human** half is deferred deliberately rather than left undone.

| | **Service identities — BUILT** | **Human users — DEFERRED** |
| :-- | :-- | :-- |
| What | one uid and gid per installed application | a `users` section in the desired-state document, projected into `/etc/passwd` |
| Why now / not now | it was **live**: applications discovered from `/state/app` were executed **as root**, with nothing verified | there is no authorization layer for a human user to attach to, and the supervisor has no socket |
| Where the fact lives | the application's **name** — `rivet_core::service::app_identity` | `ADR-007`'s document, which has no key for it |
| Where the record is | `ADR-015` §5.1.1 | this section, still |


**The service half is what this bullet list already required**, and building it
changed nothing here — which is the useful thing to notice. *"Service accounts
arrive with the services that need them and are not pre-created"* is now
**enforced rather than remembered**: the image's account table is a compile-time
constant (`rivet_core::etc::SYSTEM_IDENTITIES`, still exactly one row), so it
*cannot* hold a row for software the image has never seen. An application's
account exists because a directory under `/state/app` exists, is projected into
`/etc` by the same runtime generator as everything else, and stops existing when
the application does.

**The `/etc` half of this section is therefore already true for services and
still aspirational for humans.** `passwd`, `group` and `shadow` are generated
projections today; what reaches them is the image's own identities and the
machine's installed applications. Not one key of `ADR-007`'s schema reaches
them, which is why the human half is a *schema* question before it is a
projection question.

**The closing warning above applies to the deferred half only.** No password
hash exists anywhere on this path: an application account's `/etc/shadow` record
is `*` — no password, and none can be set — so nothing about service identities
touches `ADR-004` §5's territory.

---

### 7.3.2 The deferred half is scheduled — 2026-08-25, and `git` is why

**CORRECTED 2026-09-07: the LOGIN half of this section is gone, and the
accounts half is not.** Read the two apart before acting on anything below.

* **Gone.** SSH, `authorized_keys`, the forced command, and the login shell —
  purged 2026-08-30 when `ADR-035` §4 moved git to smart HTTP inside
  `rivet-mcpd`. A declared account's shell field is now **empty**, and **nothing
  authenticates as `git`**: reaching the git routes means presenting an operator
  certificate to the control plane, like every other operation. So wherever this
  section says an account *"exists to be logged into"* or carries *"its own set
  of authorised public keys"*, that is describing a transport this machine no
  longer has.
* **Still true and still shipped.** Accounts themselves — a uid that resolves to
  a name, `/etc/passwd` projection, `/state/home`, and §7.3.3's `root:repo`
  repository. `crate::account` kept `ACCOUNT_UID_BASE`, `AccountName`,
  `identity()` and `home()` **deliberately** through the purge, because a uid
  has to resolve to a name for network services and application sandboxing
  whatever the transport is.
* **Item 2 of the three below — "where an account's keys live" — is therefore
  answered by not arising.** No per-account key set exists or is needed.

**§7.3.1 deferred human users on a stated condition**, and the condition has
now been met from an unexpected direction. The reason given was:

> there is no authorization layer for a human user to attach to, and the
> supervisor has no socket

`ADR-032` ruled the socket on 2026-08-25. And `ADR-032` §4.2 produced the
requester: the maintainer ruled that the git server is reached as **`git@<host>`**
rather than as `root`, because a git server does not run privileged — which
means an account, with its own set of authorised public keys, its own login
shell, and its own uid.

**`git` is neither of §7.3.1's two columns, and that is the finding.** It is not
a *service identity* as that table means one — nothing logs in as an
application's derived uid, and `git` exists to be logged into. It is not a
*human user* — there is no person called git. It is a **service identity that
people authenticate as, by key**, and this project's record had no word for it
because nothing had needed one.

**Ruled: build the `users` half now, with `git` as its first entry.** Not a
special case beside it — the deferred design, arriving because something finally
needs it. Everything §7.3 already specifies applies unchanged and is the reason
this is cheap:

* **Users are configuration**, listed in the desired-state document and
  projected into `/etc/passwd` and `/etc/group` by the same generator that
  writes every other file in `/etc` (`ADR-007` §3.5). No special case, and the
  projection already exists.
* **No new verbs.** `set` plus `apply` creates, changes and removes an account,
  previewable and attributable like any other change. There is still no
  `useradd`.
* **Home directories are `/state/home`**, which is persistent and untouched by
  an update.

**Three things it must decide**, and none is settled here:

1. **What distinguishes an account a person logs into from one nothing logs
   into.** A shell, a list of authorised keys, or an explicit kind — the third
   is the most honest and the most typing.
2. **Where an account's keys live.** Nothing in the document carries a
   per-account key set today, and the machine's own peer list
   (`control_plane_keys`, `ADR-025` §2) is a different set with a different
   lifetime and must not be reused for this. Two places for one account's keys
   would be `ADR-011` in miniature, so whatever is built has to be the only one.
3. **Password hashes.** §7.3's own closing warning applies in full the moment an
   account can have one: never plaintext, never in a provisioning file that
   outlives first boot, handled like `ADR-004` §5's wireless PMK. **An account
   with keys and no password sidesteps this entirely**, and `git` is exactly
   that account — so this can be deferred one more time, deliberately, rather
   than by omission.

`PLAN.md` §24 carries the work.

---

### 7.3.3 Ruled 2026-08-27: a repository is `root:repo`, mode `2775`

§7.3.2 built the `users` half and left *what an account may reach* unstated,
because the only thing it could reach was its own home. `[repos]` shipped on
2026-08-26 and made that question load-bearing within a day: `cmd/rivet-sysd`
created a repository as `root:root`, `git receive-pack` runs as the account that
authenticated, and git refused the push with **`detected dubious ownership`** —
correctly, because the account could not have written to it in any case.

`PLAN.md` §53.2 put three arrangements to the maintainer. **Ruled: `root`, group
[`repo`], mode `2775`.**

| | |
| :-- | :-- |
| **owned by the account** | correct today, silently wrong at the second account |
| **`root:repo`, mode `2775`** | **ruled.** The classic arrangement: a group every account joins, and setgid so new objects inherit it |
| **`safe.directory` only** | addresses git's *ownership check* and not its need to **write** — it makes the error go away without making the push work |

**What decided it was a ruling already taken.** There is **no per-repository
access control** (2026-08-26), so every account reaches every repository. That
makes *owned by the account* correct only while there is one account — and there
is one today by accident rather than by design. An access arrangement that is
right by accident and goes wrong silently is the worst of the three.

**The group is therefore the authorization layer**, and it is the whole of it.
An account is in `repo` or it is not; there is no third state and no
per-repository list. When one is wanted, it is a new mechanism and a new ruling,
not a tightening of this one.

**Three consequences the record should carry:**

1. **`/etc/group` gains its first group with members.** Every declared account
   is listed in `repo`. `rivet_core::etc`'s renderer had said the member field
   is empty for every group, with a good reason — the supervisor empties the
   supplementary list before it sets an application's gid, so a membership
   written there would be a claim the running machine contradicts. **That reason
   is about identities the supervisor runs.** An account is launched by
   a C mechanism calling `initgroups(3)`, which is musl reading this field. So
   a name here is honoured for an account and would be contradicted for an
   application, and the file now says so.
2. **The group is named `repo` and not `git`.** `/etc/group` is one flat
   namespace holding both system groups and the user-private group of every
   account, and `git` is the obvious name for the account a person pushes as —
   it is what `10.0.0.2` runs. Two records under one name resolve by
   whichever the generator wrote first. `rivet_core::account::AccountName` now
   refuses the name of any system group for the same reason, so the collision is
   prevented at both ends.
3. **Setgid is the half that makes it survive.** A directory with setgid gives
   its own group to everything created inside it, so `/state/git` being
   `root:repo 2775` means every repository created under it inherits the group
   without anything remembering to say so — and so does every directory git
   creates inside that.

**Existing repositories are repaired, and repairing them needed measuring.**
Re-running `git init --bare --shared=group` over a repository created without it
brings `$GIT_DIR` and `refs` to `2775` and leaves **`objects` at `755`** — the
one directory a push must write. That was measured on 2026-08-27 rather than
assumed, and it is why `Kernel::share_tree` exists: an explicit walk that sets
the group throughout and makes **directories** group-writable and setgid.
**Directories only** — a push creates entries in directories, never rewrites a
loose object, so no file needs to become group-writable, and every file that
does not is one an account cannot corrupt.

**Amended 2026-08-27, the same day, by running it.** The ruling is necessary
and it is not sufficient. A QEMU guest with every part of it built — `/state/git`
`root:repo 2775`, the tree walked, the account in the group,
`core.sharedRepository` set — was still refused with `detected dubious
ownership`, because **git's ownership check compares the directory's owner uid
with the caller's euid, and a group is not an owner.**

So the table above has a category error in it, kept rather than edited: it puts
`root:<group> 2775` and `safe.directory` in one column as competing answers, and
they answer different questions. The group is what makes the account **able** to
write; `safe.directory` is what makes git **willing** to. This ADR's *"solves
the message, not the problem"* is right about `safe.directory` alone, and is
what makes the pairing correct rather than a way around the ruling.
`rivet_core::git::arguments` composes it, scoped to the one repository and never
`*`, on the command line — one of the three scopes git honours it from.

**The push then succeeded**, both directions, and `PLAN.md` §58 is the account.

`PLAN.md` §54 carries the work.

---

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### Status block — `/etc` was said to have two generators

**Ruled and overruled on 2026-08-23**, in one day. The superseded clause read:

~~`/etc` has two generators over disjoint facts, five of musl's
nine compiled-in paths ship in the image,~~

It was struck rather than rewritten because a reader who stops at the status
block would otherwise believe a ruling the body reverses — the same defect
`ADR-007`'s status block records against itself. §5.1.1's readings are untouched
and are the durable part of that day's work.

### In §5 — the amendment that added a build-time generator

**Written, implemented and withdrawn on 2026-08-23.** The superseded paragraph
read:

~~**Amended 2026-08-23 — "generated into `/run`" turned out to be half the
answer, and §5.3 is the other half.** Taken literally, that clause left `/etc`
empty for as long as there was no configuration document, no generator and no
`apply` — none of which exists, and the first two of which are Stage 2 work —
while a login needs `/etc/shells` at Stage 1, and that gap was recorded
as open at the time. The resolution is not to relax
"generated"; it is that some of what `/etc` holds is a fact about the image
rather than about the machine, and those files are generated at *build* time
from the same definition the runtime generator will use. §5.3 rules it.~~

The gap it identified was real. It was closed the other way — by writing the
runtime generator rather than by adding a second one.

### In §5.1 — musl's compiled-in paths were expected rather than read

**Changed 2026-08-22**, when musl's sources were swept. The superseded phrase
was ~~Expected rather than verified~~.

### In §5.3 — the heading claimed two generators

**Changed 2026-08-23**, overruled the same day it was ruled. The heading read:

~~Ruled 2026-08-23: `/etc` has two generators, over disjoint facts~~

### In §5.3.1 — which of the nine paths ship

**Changed twice, on 2026-09-01 and 2026-09-02**, because the document grew a key
each time; the rule that decides the table was not relaxed. The superseded
sentence read:

~~Five ship: `/etc/passwd`, `/etc/group`, `/etc/shadow`, `/etc/shells`,
`/etc/hosts`. Four do not: `/etc/resolv.conf`, `/etc/localtime`, `/etc/services`
and `/etc/zoneinfo/`.~~

### In §5.3.1 — `/etc/shells` shipped with no entries

**Changed 2026-08-23**, when `cmd/rivet` shipped and the projection's probe
found it. The superseded sentence read:

~~**`/etc/shells` ships empty of entries, and that is an answer.** The `rivet`
CLI does not exist yet, so this machine has no login shell and no login can
succeed on it.~~

The argument it carried is unchanged and is in the body: a present-and-empty
file says there is no login shell, and an absent one would say `/bin/sh` and
`/bin/csh` are the login shells.
