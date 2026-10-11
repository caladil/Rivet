# ADR-027: Artifact Naming

* **Status:** Accepted. **Amended 2026-08-23** (§3.2.1, a dirty tree),
  **2026-08-24** (§9.1 — items 2, 3 and 4 ruled; §3 gains an application's
  shorter grammar; §9.2 — the first tool that composes a name; §9.4 — both
  grammars from one function, and why an update's version is not composed),
  **2026-09-04** (§3.2.2A2 — the dirty digest covers the WHOLE PAYLOAD, not the
  program alone; the old rule is struck in place), and **2026-09-13**
  (§3.4 — a card image distributes as a signed `card` `.rvt`, not a bare
  `.img.xz`; §9.5 — the fifth kind, `card`, `R-177`/`R-177.1`).
* **Date:** 2026-08-23
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

Rivet produces files that leave the machine that built them. Today there is
exactly one — the card image — and it has been called `rivet.img` since before
the first card was written. `ADR-015` §4.4 already names two more kinds that
will exist: applications and system extensions. `PLAN.md` §9.4 adds a fleet
server that serves images to machines rather than to a person holding a card
reader.

A filename is a small thing. It becomes a large one the moment a directory holds
two of them, and larger still when the two are for different boards.

**Raised by the maintainer on 2026-08-23** as a nit-pick prompted by the second
platform, which is exactly the right moment for it: settled now this costs a
convention, settled later it costs a recall.

---

## 2. `rivet.img` is already ambiguous — not one day, now

The prompting question was about the x86 port. But the name is insufficient
before the x86 target exists:

| What the name cannot distinguish | Why it already matters |
| :-- | :-- |
| **Architecture** | `ADR-002` §2 keeps the **Pi Zero W** as a deferred compatibility target. That is ARMv6 and `arm-unknown-linux-musleabihf` — an image that cannot execute a single binary from the Pi 4 image. |
| **Platform** | `PLAN.md` §9.2 adds an x86 target, `x86_64`, whose firmware contract is not the Pi's at any point. |
| **Version** | The workspace is at `0.0.1` and two builds an hour apart are indistinguishable the moment either is copied out of `out/`. |
| **Which build a measurement describes** | `ADR-018` §10.5 tracks the image's size *per build*. Every row of that table measured a file with the same name. |

**The failure mode is the quiet kind this project keeps designing against.**
Wrong-image-for-board is silent until a boot that does not happen;
wrong-version is silent for ever. Nothing else in the design catches either —
`ADR-015` §4.4's signature proves an artifact is **ours**, not that it is **this
board's** or **newer than what is already running**.

---

## 3. The scheme

```
<product>-<version>-<arch>-<platform>-<kind>.img.xz
```

```
rivet-0.0.1-aarch64-rpi4-card.img.xz
rivet-0.1.0-x86_64-uefi-card.img.xz
rivet-0.1.0-armv6-rpizw-card.img.xz
```

Lowercase throughout, `-` between fields, no field ever empty or omitted. A
field with nothing to say is a sign the grammar is wrong, not a licence to drop
it.

**Amended 2026-08-24 — the grammar above is the *card image*'s, and one kind has
a shorter one.** The maintainer's 2026-08-24 ruling removes `<platform>` from
applications and extensions (§9 items 3–4), so an application's name is:

```
<product>-<version>-<arch>-<kind>.rvt
hello-app-0.1.0-g3e47fde-dirty-aarch64-application.rvt
```

That is not the sentence above being broken. **The field is not empty; it is
not part of that kind's grammar at all**, which is precisely the distinction the
sentence was drawing — a firmware token on an artifact with no firmware contract
would be a field with nothing to say, and dropping it is the grammar being made
right rather than a licence being taken. The rule that survives, and is the one
worth quoting later, is: *a field either says something or it does not exist for
that kind.*

### 3.1 Architecture and platform are both present, and neither implies the other

They look redundant on a Pi 4 and are not:

| | `<arch>` | `<platform>` |
| :-- | :-- | :-- |
| Answers | *will this binary execute* | *will this firmware boot it* |
| Values | `aarch64`, `armv6`, `x86_64` | `rpi4`, `rpizw`, `uefi` |
| Breaks the shortcut | Pi Zero W and Pi 4 are both "a Raspberry Pi" and are not the same machine | `x86_64` is one architecture across every chassis the x86 target might turn out to be |

**`<platform>` names the boot contract, not the chassis.** That is why the x86
token is `uefi` rather than a model: what an image must match is the firmware's
handoff, and any machine presenting the same handoff takes the same image. The
Pi tokens are board names only because on the Pi the board *is* the contract —
firmware blobs, DTB, and `ADR-018` §5's `tryboot`.

**`<arch>` earns its place a second way.** `ADR-015` §3.1 makes a static C or Go
binary a valid package, and `PLAN.md` §9.1 intends to prove that with Aura. An
application artifact has an architecture and no platform; one vocabulary across
all three artifact kinds is worth a redundant token on the Pi.

### 3.2 The version, and two builds that share one

`0.0.1` will not move for a while, so the version alone does not identify a
build. **Any build not made from a tag carries the git short SHA in the version
field:**

```
rivet-0.0.1-g7b84d4b-aarch64-rpi4-card.img.xz
```

Dash-separated rather than SemVer's `+build`. `+` is legal in a filename and a
nuisance in every other place this name travels — URLs, shell arguments, and the
FAT boot partition that `ADR-002` §4.1 makes the file-transfer channel.

**The absence of a SHA is therefore a claim**: that this artifact came from a
tag. §7 rules that the tool composes the name precisely so that claim cannot be
made by hand.

#### 3.2.1 Amended 2026-08-23: a dirty tree appends `-dirty`

**The presence of a SHA is a claim too, and the record has one artifact that got
it wrong.** `PLAN.md` §0.0 says so of `rivet-0.0.1-g1319389-aarch64-rpi4-card.img`:

> the SHA is `HEAD`'s, and the working tree was **dirty** when the image was
> built, so the name claims a commit that does not contain what is on the card.

That is this section's own reasoning turned around. `g1319389` asserts *these
bytes came from commit 1319389*, and if uncommitted changes were compiled into
them the assertion is false — in the direction that is hardest to notice, because
the name looks exactly like a name that is true.

**Ruled: when the working tree is dirty, the version field carries `-dirty`
after the SHA.**

```
rivet-0.0.1-g1319389-dirty-aarch64-rpi4-card.img
```

Three reasons it is spelled that way rather than invented:

1. **It is `git describe`'s own convention** (`--dirty`), so nobody has to learn
   it and no tool has to be told what it means.
2. **It reads as part of the version field**, which is where the imprecision
   actually is. The architecture, platform and kind are exact either way.
3. **It is a suffix, so sorting still groups a dirty build beside the commit it
   came from** — which is what somebody comparing two cards actually wants.

**What "dirty" means here is `git status --porcelain` being non-empty**, tracked
files only — the same question `git describe --dirty` asks. Untracked build
output does not make a build dirty; `.gitignore` already covers `*.img`.

**This does not make a hand-composed name safe.** §7 still rules that the tool
composes the name and still records that the tool does not yet, so every
hand-composed name — including a correctly-suffixed one — is a standing
exception. What this section changes is only that the exception now has a
*correct* form to be an exception to.

#### 3.2.2 Amended 2026-08-29: `-dirty` alone is not unique, so it carries a digest

**§3.2.1 made `-dirty` a claim and stopped one step short of making it a
distinguishing one.** Every build of a dirty tree at one commit produced the
same version and therefore the same artifact name:

```
hello-0.1.0-g7b84d4b-dirty-aarch64-application.rvt
```

**Two artifacts built from two different working trees were then distinguishable
by nothing a machine can read**, which is the defect this section rules out.
**Same file name, same header
`version`, same `/state/app/<name>/<version>` directory** — so a redeploy of
changed code looked exactly like a redeploy of identical bytes, and §7's *"the
tool composes the name so the claim cannot be made by hand"* was satisfied while
the claim itself said nothing.

It was found on 2026-08-29 by a cold agent that deployed an application to a
board in 19m31s (`PLAN.md` §102.6) and is Pile B item 1. Its cost is not
cosmetic: `ADR-015` §7 puts an install at `/state/app/<name>/<version>`, so two
different builds share one **directory**, and §3.2.2A below is the other half of
the same ruling.

**Ruled: a `-dirty` version carries eight hex characters of BLAKE3 of the
WHOLE PAYLOAD, after the `-dirty` token** — §3.2.2A2, 2026-09-04. This section
first covered only the program member's bytes; the token, its position and its
width are unchanged, and what was too narrow is the set of bytes it covers. The
narrow rule produced a real failure, which is why History keeps it.

```
hello-0.1.0-g7b84d4b-dirty-1a2b3c4d-aarch64-application.rvt
```

| | |
| :-- | :-- |
| What is hashed | **every member of the payload** — see §3.2.2A2 |
| With what | BLAKE3, through `rivet-core`'s `Digest` port and `rivet-crypto`'s implementation — `ADR-029` §7.2's algorithm, not a second one |
| Where | `rivet_remote::name::Version::distinguished_by`, called by `cargo rivet package` **after** the build, because a version composed before it would name bytes that did not exist yet |
| A clean version | **untouched.** §3.2's bare version claims a tag and §3.2's `-g<sha>` claims a commit; both already identify bytes the history holds. Only `-dirty` names bytes nothing records |

**Why after `-dirty` rather than replacing it.** §3.2.1's three reasons all
survive: the word is still `git describe`'s, it still reads as part of the
version field, and it is still a suffix, so sorting still groups a dirty build
beside the commit it came from *and* now groups two dirty builds of that commit
beside each other. A reader who does not know what the last token is still reads
the name correctly; a reader who does can compare two.

**Eight characters is a budget, not a security parameter.** `ADR-029` §3.1 makes
the header's `version` field 32 bytes and **refuses** a longer one rather than
truncating it, so `<crate>-g<7>-dirty-<8>` has to fit. Four bytes of BLAKE3 is
not offered as collision resistance and nothing verifies against it — what it has
to do is tell two builds of one working tree apart on one bench, which is the
whole of this section's scope. `rivet_remote::name::DIRTY_DIGEST_CHARS` is the
constant and a test pins the arithmetic: a crate version of `10.20.30` reaches
**exactly** 32 bytes, so a longer SemVer plus a dirty tree is refused by the
container rather than silently shortened. That is the correct failure and it is
recorded here so it is recognised when it happens.

**What this does NOT cover, said plainly.** `cargo rivet update` takes
`--version` as a literal argument typed by the caller (`cmd/cargo-rivet/src/update.rs`),
so an **update payload's** version is still hand-composed and this ruling does
not reach it. §7's standing exception therefore still stands for that verb.

##### 3.2.2A2 Amended 2026-09-04: the digest is of the PAYLOAD, not the program

**The fix above hashed the program and stopped, and the manifest is the half the
machine acts on.** Edit only `[package.metadata.rivet]`, rebuild, deploy: the
program's bytes are unchanged, so the version is unchanged, so §3.2.2A's
installer refuses the install as **already present** — and the change silently
does not happen. Reported in the review of 2026-09-04 and recorded there as the
maintainer's to rule; ruled the same day.

It is the same defect §3.2.2 was written for, one member along. §3.2.2's own
sentence names the property it wanted — *"the thing an edit changes"* — and then
picked one member out of a payload where three of them can be edited:

| Member | Can change alone | What the machine does with it |
| :-- | :-- | :-- |
| the program | yes | executes it |
| **the manifest** | **yes** | decides restart policy, sockets, devices, settings, the library closure — every fact the supervisor acts on (`ADR-015` §3.2) |
| an asset | **yes, and now easily** | `.rulings/` R-142 ships **everything** in `assets/`, so the set is whatever is in a directory |

**Ruled: a `-dirty` version carries eight hex characters of BLAKE3 of every
member of the payload.** The token, its position, its width and the algorithm
are exactly as §3.2.2 left them; only the input widens. `ADR-029` §3.1's 32-byte
budget is therefore untouched.

**And the inputs are ORDERED, which is the other half of the ruling.** A digest
over a set is not a function of the set until the order is fixed, and an
unordered one would give two identical payloads two versions — the same defect
wearing the other face, and worse for being intermittent rather than constant.
R-142 makes this concrete rather than theoretical: the asset set comes out of a
**directory**, so the order members reach the digest is the filesystem's.

The framing, stated normatively because a caller must not be able to reach the
answer:

1. members are sorted by `(name, class, variant)` — a member's whole identity in
   the container (`ADR-029` §13.2 item 4), so two payloads that differ only in
   which class a byte-identical member was filed under are still two versions;
2. each member contributes `name`, a `NUL`, `class`, `variant`, then the 32-byte
   BLAKE3 of its own content. A member name is printable ASCII
   (`rivet_core::package::format::read_name`), so the `NUL` cannot occur inside
   one and two members cannot be run together into a third spelling;
3. a member whose file cannot be read **stops the build** — §3.2.2's rule
   unchanged, and for its reason.

**Two levels rather than one long stream, and it is not an optimisation.** A
member's length is not known before it is read, so a single stream would need
either a length prefix — a second read, and a race against it — or nothing
between members, which is (2)'s ambiguity again. Digesting each member's content
on its own yields a fixed-size 32 bytes to place in the outer hash.

`rivet_remote::name::Member` and `Content` are the types; `cmd/cargo-rivet`
projects its `emit::Part` list into them so the digest is taken over **the parts
that are actually written** rather than over a second list that could disagree.
The version is therefore composed later in `package` than it was — after the
member list exists rather than after the build — and that move is the fix.

##### 3.2.2A The other half: nothing unpacks over a directory that is there

Ruled in the same breath, and it belongs to `ADR-015` §7 rather than here — the
pointer is recorded so a reader of this section is not left thinking a unique
name is the whole fix. **`ADR-015` §7.2** is the installer's refusal. The two are
one ruling because a unique version is what makes refusing tolerable: with
§3.2.2, a changed build no longer *wants* the directory a previous build is
using.

### 3.3 What is deliberately not in the name

* **The slot.** `ADR-018` §5.2 chooses A or B at write time and the same bytes
  go to either. A slot in the name would be false half the time.
* **The card size.** `ADR-018` §10.2 made the image *minimum-device*-sized and
  handed growth to first boot. The image no longer has an opinion about the card
  it lands on, and it should not sound like it does.
* **A date.** The version and the SHA already order builds. A date is a third
  ordering that can disagree with the other two.

### 3.4 A card image distributes as a `card`-kind `.rvt`, not `.img.xz`

**Superseded 2026-09-13, `R-177`/`R-177.1`.** The `.img.xz` naming below was
written before this ADR had a `<kind>` field at all, on the assumption that a
card image would travel as a bare compressed file the way it sits in `out/`.
It does not: `cargo rivet image` wraps the built `<name>-card.img` as one
opaque, **zstd-compressed** member of a signed `card` `.rvt` — §9's `<kind>`
token, exactly like every other artifact this project distributes — and that
`.rvt`, not a `.img.xz`, is what leaves the workstation (`ADR-052` §7.2.3.5).

`ADR-018` §10.5 and §10.6's own numbers are unaffected: they measure the
*build-intermediate* `.img` in `out/`, which is still produced uncompressed
and still never distributed on its own. What changed is only what a
*distributed* card looks like — `rivet-0.1.87-aarch64-rpi4-card.rvt`, `ADR-029`
§11's container, not a bare compressed file with no signature and no kind
token to say what it is.

`.gitignore` covers `*.img` and `*.rvt`; neither is ever committed.

---

## 4. The word "image" is about to mean two things

`ADR-018` §5.2's update writes a system filesystem into the inactive `system-`
partition and a kernel into the inactive `boot-` partition. **That is not the
artifact `cmd/rivet-image` writes today.**

| | **Card image** | **Update payload** |
| :-- | :-- | :-- |
| Produced by | `cmd/rivet-image` | `ADR-018` §5.2's update; format not yet decided |
| Contains | GPT plus all five partitions | one system filesystem and one kernel |
| Consumed by | an imager, writing a whole device | a running Rivet, writing two partitions |
| Provisions | a new machine | never — it updates an existing one |

Both are naturally called "the image". Once `PLAN.md` §9.4's fleet server holds
both, a `.img` that could be either is a mistake waiting for a tired evening.

**Hence the `<kind>` field, and `card` for the artifact that exists.** The
update payload's own token is ruled when the payload has a *shape* — `ADR-018`
§5.2 describes a flow, not a file format, and inventing a name for a format
nobody has designed is the failure `AGENTS.md` §7 is about. **What this ADR does
is reserve the position**, so that naming it later is not a rename of everything
else.

`AGENTS.md` §10.2 gains both terms, which is what §10.2 is for.

---

## 5. The name is for humans; the manifest is the authority

**Nothing may parse these filenames to decide anything.**

`ADR-011` asks which single thing owns a fact. The authority for *"which
architecture is this image for"* is the signed metadata — never a substring. If
`PLAN.md` §9.4's fleet server learns a machine's architecture by matching a
filename, then the scheme becomes load-bearing, can never be changed, and a
renamed file becomes an attack rather than a mistake.

The name's job is that a person listing a directory, reading a download log, or
asking six months later what a card was flashed from can tell at a glance. That
is a real job worth doing well. It is not an interface.

**Consequence for the fleet server:** it serves from a manifest carrying
architecture, platform, version and digest, with the filename as a *field* in
that manifest rather than a source for it. The same posture as `ADR-007`'s
generated `/etc` — a rendering, never the authority.

### 5.1 The distributed name is not the installed name

`ADR-018` §6 installs an extension at `/state/ext/<name>.img`, with no version
and no architecture, and that stays. **The two names answer different
questions.** On the device the system refers to an extension *by name*, so the
path is an identity and a version in it would make every reference unstable. In
a directory of downloads the name is a *label* on one of many candidates and
must say which. Applying this ADR's grammar to an installed path would be a
plain error.

---

## 6. The development loop keeps a stable way in

`PLAN.md` §0.0.1's loop points QEMU and an imager at a file, and the loop must
stay cheap. A path that changes on every build breaks scripts and muscle memory
for no benefit.

**The build writes the qualified name; the loop resolves the newest match in
`out/` and prints the path it chose on every run.** Printing it is the point: a
"newest match" that silently picks a stale artifact after a failed build is
exactly the class of thing `AGENTS.md` §5 forbids reporting as a success.

**No stable symlink.** The maintainer's host is Windows — `PLAN.md` §0.0.1 has
the two boot partitions arriving as `E:` and `F:` — and a symlink inside a
working tree there is a needless problem.

---

## 7. Who composes the name

`cmd/rivet-image` takes the output path positionally today
(`usage: rivet-image OUT.img …`), so the name is whatever the caller types.

**That makes the caller the authority for three facts the caller does not own.**
The tool knows the version it stamped, the target it built for, and the binaries
it installed. A human typing `-aarch64-` into a filename is a second, unchecked
copy of a fact held elsewhere — the shape `ADR-011` exists to remove, and the
same shape that produced the path collision recorded in `PLAN.md`
§0.0.2 item 1.

**Ruled: the tool composes the name.** The caller supplies a *directory*;
`cmd/rivet-image` composes `<product>-<version>-<arch>-<platform>-<kind>.img`
from what it already knows, and reports the path it wrote. **There is no
override**, because an override is a supported way to make the name disagree
with the contents.

> **Not implemented as of 2026-08-23.** This section rules the destination. The
> CLI still takes a positional path, and until it does not, the scheme is a
> convention a caller can break in silence. `PLAN.md` §0.0.2 item 1 is the
> epitaph for believing a documented convention is a mechanism. §8 tracks it.

---

## 8. What this changes

| Place | Change | State |
| :-- | :-- | :-- |
| `docs/ADR-027` | This file | Done 2026-08-23 |
| `docs/PLAN.md` §3 | Decision-log row | Done 2026-08-23 |
| `docs/ADR-018` §5.2, §10.5 | Pointers here | Done 2026-08-23 |
| `docs/ADR-015` §4.4 | Pointer here | Done 2026-08-23 |
| `AGENTS.md` §10.2 | **Card image** and **update payload** as vocabulary | Done 2026-08-23 |
| `README.md` | The ADR range | Done 2026-08-23 |
| `cmd/rivet-image` test fixtures | Model a real name rather than `rivet.img` | Done 2026-08-23 |
| `cmd/rivet-image` CLI | Compose the name; take a directory | **Open — §7** |
| `cmd/rivet-image` CLI | Append `-dirty` when the tree is dirty (§3.2.1) | **Open — it is part of §7's job and arrives with it** |
| `tools/qemu/run.sh` | Resolve newest and print the choice (§6) | **Open — §6** |
| The fleet server's manifest | §5 | Does not exist — `PLAN.md` §9.4 |
| §9 items 2–4 | Ruled | Done 2026-08-24 — §9.1 |
| §3 | An application's grammar has no `<platform>` field | Done 2026-08-24 |
| `cmd/cargo-rivet` | Compose an application's name; §7 and §3.2.1 both honoured | Done 2026-08-24 — §9.2 |
| `cmd/cargo-rivet` | Compose an **update payload's** name, `<platform>` included, from one function that produces both grammars | Done 2026-08-24 — §9.4, `ADR-029` §15 |

---

## 9. Open

1. **The update payload's shape, and therefore its `<kind>` token —
   CLOSED 2026-08-23 by `ADR-029`**, which designed the format and ruled the
   token `update` (§8) and the suffix `.rvt` (§8).
2. **The container suffix for applications — CLOSED, see §9.1.**
3. **Whether applications carry `<platform>` at all — CLOSED, see §9.1.**
4. **Whether extensions carry `<platform>` — CLOSED, see §9.1.**

### 9.1 Items 2, 3 and 4 — ruled 2026-08-24

**Ruled by the maintainer**, 2026-08-24, and
written here because §9 is where the questions were asked. **The ruling: `.rvt`
for all three kinds, with `<kind>` inside the file as `ADR-029` §8 already has
it, and no `<platform>` for applications or extensions.** §9 item 3 had already
leaned that way — an application is architecture-shaped rather than
firmware-shaped — and item 4 follows from it, an extension's architecture being
the base image's. As the maintainer's 2026-08-24 ruling put it:

> `.rvt` for all three kinds, `<kind>` inside the file as `ADR-029` §8 already
> has it, and **no `<platform>` for applications or extensions**. `ADR-027` §9
> item 3 already leaned this way — an application is architecture-shaped, not
> firmware-shaped — and item 4 follows: an extension's architecture is the base
> image's.

| Item | Answer |
| :-- | :-- |
| 2 — the application suffix | **`.rvt`.** Not a suffix of its own. `ADR-029` §11's argument decides it: *"the verifier is the most security-sensitive parser on the machine. Three formats means three verifiers, each with a third of the scrutiny."* One suffix follows one format, and `<kind>` — **inside the file, not in the name** — is what distinguishes |
| 3 — applications and `<platform>` | **No.** Item 3's own lean, taken |
| 4 — extensions and `<platform>` | **No**, on item 3's reasoning. Nothing has been built for `ADR-016` and this is a ruling about a grammar rather than about a mechanism, so it stands until something contradicts it |

**Why the suffix answer does not weaken §5.** *"Nothing may parse these
filenames to decide anything"* is unchanged and is in fact strengthened: a
reader that wanted to know an artifact's kind must now open it, because the name
no longer distinguishes. `rivet_core::package::read::Header::kind` is the
authority and the suffix is a convention — a payload renamed to `.bin` verifies
exactly as well, and a `.rvt` whose header says otherwise is refused exactly as
hard.

**One consequence, which is item 3's answer having a price.** The header's
`product` field is 16 bytes and it is now an *application's name* — so
`ADR-015` §5.4's own worked example, `metrics-collector`, is one character too
long to package. `cargo rivet package` refuses it with the limit named rather
than truncating. Whether the answer is a wider field (`format_version` 2), an
identity that lives in the manifest instead, or a cap on application names is
the maintainer's, and not yet ruled.

### 9.2 What implements this — 2026-08-24

`cmd/cargo-rivet`, and `AGENTS.md` §6 applies to every sentence of this:

* **`cargo rivet package` composes the name and there is no override**, which
  is §7's ruling, honoured by the first tool to be written after it. §7's own
  note that `cmd/rivet-image` still takes a positional path is **unchanged and
  still open** — a second tool doing it right does not fix the first.
* **§3.2 and §3.2.1's claims are made by the tool**: `-g<sha>` unless HEAD is a
  tag with a clean tree, `-dirty` when `git status --porcelain` is non-empty.
  The tool **prints what it asked git and what git answered**, because
  `AGENTS.md` §10.5 records that WSL's `git` and Windows' `git` disagree about
  this tree and a build run under the wrong one would stamp a permanent false
  claim into every filename.
* **`<arch>` is a table, not a split.** `arm-unknown-linux-musleabihf` maps to
  `armv6` and not to `arm`, so §3.1's own token for the Pi Zero W is produced
  rather than approximated. An unrecognised triple is refused before anything is
  built.
* **Run, not merely compiled**: a real crate cross-built for
  `aarch64-unknown-linux-musl` and packaged, verified back through
  `rivet_core::package::read` under real Ed25519 and BLAKE3, and the binary
  extracted byte-identical. `PLAN.md` §0.9 is the account, including what is
  still unproven.

### 9.3 The binary is `rivet-deploy` — ruled 2026-08-24

`cmd/rivet-update`, as it then was, installed **system updates and
applications** from the day
`ADR-015` §7.1 was built, so its name described half of it. `PLAN.md` §0.4.6
carried the question and recommended `rivet-install`.

**Ruled: `rivet-deploy`**, which is the maintainer's and is better than the
recommendation for a reason worth recording. Neither `update` nor `install` is
the natural umbrella here — but **`deploy` already was**: the maintainer's 2026-08-24 ruling is
titled *"the deploy chain's questions"* and §0.8 traced *the deploy chain*, both
written before this binary existed. The name was already in use for the path;
it now names the thing that walks it.

It also avoids the `-d` daemon suffix that `rivet-netd` and `rivet-timed`
carry, correctly — the binary is **one-shot** (ruled by the maintainer the
same evening, 2026-08-24), not a daemon.

**Taken while it was free.** No service definition named the binary yet, so
nothing had to be migrated — a window that closes the moment `apply install`
lands. Ten files moved, `2095` tests unchanged.

**Two string literals deliberately keep the old name**:
`cmd/rivet-deploy/src/bench.rs` and `install.rs` derive test keys from
`"rivet-update … seed, not a key"`. Those are **domain separators, not
names** — their only requirement is never to change, since the keys they derive
are the ones those tests sign and verify with. Both say so at the literal.
### 9.4 Both grammars are now produced by one function — 2026-08-24

`cargo rivet update` (`ADR-029` §15) is the second artifact kind this tool
emits, and it is the one that exercises the **other** half of §3's amendment:

```
rivet-0.0.1-7354bd8-aarch64-rpi4-update.rvt     <platform> present
hello-app-0.1.0-g3e47fde-aarch64-application.rvt  <platform> absent
```

Three things about that, each a consequence of a rule already written here:

1. **One function composes both**, taking `Option<&str>` for `<platform>`. §3's
   surviving rule is *"a field either says something or it does not exist for
   that kind"*, and two separate name-builders would be two spellings of the
   same `-` waiting to disagree.
2. **`<platform>` comes from `BootSlots::platform_token`**, not from a constant
   in the tool. §3.1 says *"`<platform>` names the boot contract, not the
   chassis"*, and the boot contract is exactly what a `BootSlots` implementor
   is — the same value `ADR-029` §9.2 step 2 compares a signed header against on
   the machine. A token with no implementor is refused rather than named.
3. **§3.2's claim is deliberately NOT made for an update.** The tool is handed a
   `/system` image it did not build, so composing `-g<sha>` would assert
   §3.2.1's *"these bytes came from commit X"* on somebody else's behalf.
   `--version` is required with no default and the report says the string was
   taken as given — see `ADR-029` §15.6. **This is §7's ruling holding rather
   than being broken**: §7 removes the *caller's* authority over facts the tool
   owns, and the provenance of bytes handed in on the command line is not one of
   them.

### 9.2 A FOURTH kind — `library`, 2026-08-31

`SPEC-02-Shared-Rivets.md`. Shared Rivets adds an artifact kind, and §9.1's
ruling covers it without amendment:

```text
libgreet-1.0.0-aarch64-library.rvt
```

**`.rvt`, `<kind>` inside the file, and no `<platform>`** — item 3's reasoning
applies unchanged, because a library is architecture-shaped for exactly the
reason an application is. Nothing in `name::artifact` changed to compose this;
the same function with the same `None` produced it, which is §9.1 footnote 1's
one-builder rule holding the first time it was tested by a new kind.

### 9.5 A FIFTH kind — `card`, 2026-09-13

`R-177`. A card image, wrapped by `cargo rivet image` rather than built by
`cmd/rivet-image` directly, joins the fleet repository (`ADR-052` §7.2.3.5):

```text
rivet-0.1.87-aarch64-rpi4-card.rvt
```

**`.rvt`, `<kind>` inside the file, and `<platform>` present** — the opposite
of item 3's answer for an application, and for the reason item 3 itself gives:
a card image *is* firmware-shaped, exactly as an update payload is, because
what it boots is the whole point of the artifact. `name::artifact` composed
this with the same `Option<&str>` parameter §9.4 already built for an update's
platform; nothing in that function changed for a fifth kind either, which is
§9.1 footnote 1's rule holding a second time.

**The one property no other kind has: its one member may be compressed.**
`R-177.1` — zstd, `rivet_core::package::read::COMPRESSION_ZSTD` — legal only
on a `card` payload's member, because a card image is the one kind never
installed by a board (`ADR-029` §13/R-107 keeps every board-installed kind
uncompressed on purpose). This is a fact about the container's contents, not
about the name; §3's grammar is unaffected; §3.4 above is what changed.

**Three of `ADR-016`'s four are now ruled and the extension is still not.**
`Kind` remains a validated token rather than an enum, and the reason recorded in
`rivet_core::package::kind` survived a second addition: a token read out of
somebody else's header is text this build may not recognise, and a closed enum
would make *"a kind I do not implement"* indistinguishable from a malformed
field.

---

## 10. The version scheme — RULED 2026-08-26

**Ruled by the maintainer: the base is `0.1.0`, and every image build
increments the patch.**

### 10.1 What was wrong before the ruling

Two facts found while scoping it, and the second is a defect:

* The workspace version was **`0.0.1`**, not `0.1.0`.
* **`0.0.1` was a hardcoded literal in `tools/card/build.sh`**, in two places
  — the output name and the retention parser — and **not read from
  `Cargo.toml` at all.** The two agreed by luck. `ADR-011` §3 names this
  exactly: one fact with two spellings and no vocabulary for their
  disagreement.

`Cargo.toml`'s workspace version is the single source and `build.sh` reads it.

### 10.2 The sequence, and why the bump is committed

**Refuse a dirty tree → increment the patch → commit that one change alone →
build.**

§3.2.1 makes both halves of a name a claim about the artifact, and `-dirty` is
a claim about uncommitted changes. A bump that is *not* committed leaves every
image stamped `-dirty` forever and lets two different images carry one version.
Committing it first makes **version and commit a permanent 1:1 mapping**, which
is what §7's "who composes the name" wanted and could not previously have.

**Rejected: bumping after a successful build**, so failed builds do not consume
numbers. Appealing, and wrong — the artifact would then carry the *old* version
while being named for the new one, which is the one thing this ADR exists to
prevent.

### 10.3 The consequence: versions become orderable

`PLAN.md` §16 and §26.1 both record the installer saying:

> the payload's version differs from the one running; **this build does not
> order version strings**, so whether it is newer is not asserted

Monotonic patch increments make that sentence unnecessary. **This ADR does not
rule that the installer starts ordering them** — that is `ADR-029`'s and it is a
policy question about downgrades, not a naming one. It records that the
obstacle is gone.

---

## 11. RULED 2026-08-29: what "newer" means, and it reports rather than refuses

`ADR-029` §9.2's install record has said, in the machine's own words, that it
cannot tell an upgrade from a downgrade — and installed anyway. **The reason it
was unwritten was never difficulty. It was that nobody had ruled what *newer*
means for these version strings.** *(`.rulings/` R-105.)*

> **Compare the semver prefix, ignore the suffix, and report the answer.
> `older`, `newer`, or `cannot tell`.**

### 11.1 It reports and never refuses, and that is deliberate

`ADR-028` §6.1 already rules that an older payload is **reported, not blocked**,
and this does not disturb it. **Rolling back on purpose is a legitimate thing to
do** — it is the recovery move when a new build is bad — and a tool that fought
an operator over it would be worked around, which is worse than one that says
what it sees. The value of this ruling is entirely in the *saying*.

### 11.2 What is compared, and what is deliberately not

§10's scheme is `<semver>-g<short>[-dirty[-<digest>]]`. Only the **semver
prefix** is ordered:

* **The commit is not ordered.** `g433c4d0` against `g932f464` is two hex
  strings; nothing in them is a sequence. A checker that sorted them would be
  inventing an order and would be wrong roughly half the time, confidently.
* **Commit *dates* were considered and rejected.** They would be precise, and the
  payload does not carry one — adding it is a change to a **signed container
  format** (`ADR-029` §3) for precision this project would almost never use. The
  wrong trade.
* **Equal semvers answer `cannot tell`, and that is a real answer** rather than a
  failure to produce one. Two builds of `0.1.10` from different commits are not
  ordered by anything this format records, and `AGENTS.md` §5 asks for the honest
  gap rather than a plausible guess.

### 11.3 A dirty build is never `newer`, and this one is the agent's call

**Recorded as such**, because the maintainer ruled the comparison and left this
detail: a `-dirty` payload at the same semver reports **`cannot tell`, naming the
dirt**, and never `newer`.

**A dirty tree is not a point on a line.** It is *some* uncommitted state, and
two dirty builds of one semver are not comparable to each other or to the clean
build they came from. Since 2026-08-29 a dirty version also carries a digest of
the built binary (§3.2.2) — which makes two dirty builds *distinguishable*, and
distinguishable is not ordered. Calling it `newer` because it was probably built
later would be exactly the plausible guess §11.2 refuses.

**Overturn this cheaply if it is wrong**: it is one branch in one function, and
this subsection is the only thing that would need striking.

### 11.4 What this does not settle

* **`cargo rivet update` still takes `--version` as a literal the caller types**
  (`ADR-027` §3.2.2's own note), so this comparison does not reach an update
  payload's declared version by that route.
* **Not built.** No comparison function exists yet. It leaves `PLAN.md` §102.4.1
  when it is built, not when it is ruled.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §3.4 — compression used to be a bare `.img.xz`

**Superseded 2026-09-13, `R-177`/`R-177.1`.** No code ever produced this form
— §0 of the design draft that led to `R-177` found the same thing, grepping
`cmd/rivet-image` for a compression step and finding none — so the change is
from an aspiration to a built fact rather than from one built thing to
another. The superseded text:

~~`ADR-018` §10.5 and §10.6 both quote the artifact in **compressed** KiB,
because compressed is what moves. So compressed is what gets named: `.img.xz`.
The uncompressed file is a build intermediate and is not distributed.
`.gitignore` already covers `*.img` and `*.img.xz`.~~

Kept because a reader who remembered the old naming should be able to find why
it changed rather than conclude the ADR contradicts itself.

### In §3.2.2A — the defect this section rules out

**Superseded by the section's own ruling.** The sentence stating the defect
read:

~~Two artifacts built from two different working trees are then distinguishable
by nothing a machine can read.~~

It is kept because it states the defect precisely, and the body says the same
thing in the past tense.

### In §3.2.2A — the digest covered only the program member

**Superseded by §3.2.2A2, 2026-09-04 — it is a digest of the WHOLE PAYLOAD.**
The token, its position and its width are unchanged; what was too narrow is the
set of bytes it covers, and **the narrow rule produced a real failure**. The
superseded ruling and table cell read:

~~**Ruled: a `-dirty` version carries eight hex characters of BLAKE3 of the
program being packaged, after the `-dirty` token.**~~

~~the **program member's bytes** — the thing the artifact is *for*, and the thing
an edit changes~~

### In §9 — four open items that closed

**Item 1 closed 2026-08-23 by `ADR-029`; items 2, 3 and 4 on 2026-08-24, §9.1.**
The superseded questions:

~~**The update payload's shape, and therefore its `<kind>` token.**~~

~~**The container suffix for applications.** `ADR-015` §3 fixes the
*contents* — one static binary, a typed manifest, optional assets, a
signature — and no file format. `<arch>` and `<version>` apply the moment
there is a file; the suffix belongs to whoever rules the format.~~

~~**Whether applications carry `<platform>` at all.** An application is
architecture-shaped, not firmware-shaped, so probably not. A *mechanism* that
had to match a board's DTB would be — and no such thing exists yet to decide
from, so this waits for one rather than being guessed.~~

~~**Whether extensions carry `<platform>`.** They are images (`ADR-016`)
mounted by the running system, so their architecture follows the base image's.
The platform half is item 3's question in a different hat.~~
