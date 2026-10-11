# ADR-010: Licensing and Distribution Obligations

* **Status:** **Accepted** — fully ruled 2026-08-22. See §6 for the repository
  and mirror model, §6.4 for the licence, §6.5 for what is published, and §4.1
  for the gate.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

**The maintainer's note, 2026-08-22: MIT licensing for this project is an
aspiration, and it can change if it needs to.** *(`.rulings/` R-025.)*

That flexibility is appreciated and — this ADR argues — **not needed.** The
concern behind it appears to be that shipping GPL software forces Rivet's own
code to become GPL. For a distribution, it does not, and the reason is worth
stating precisely because the distinction it turns on is the same one that
decides several other questions in this project.

**Nothing in this document is legal advice.** It is an engineering position on
which obligations attach where, so that the build can be arranged not to create
the expensive ones by accident.

---

## 2. The distinction that decides everything: aggregation versus linking

**Aggregation.** Shipping independent programs together on one filesystem or in
one image. Each program keeps its own licence; none reaches into the others.
Every Linux distribution in existence relies on this — Debian ships GPL, MIT,
BSD and Apache binaries side by side and its own tooling is not thereby GPL.

**Linking.** Combining code into a single binary. This is where copyleft
propagates, and it is where a project actually gets caught.

Rivet's exposure is therefore almost entirely determined by **what is linked
into a Rivet binary**, not by what is present in the image.

### 2.1 The rule

> **Aggregate copyleft. Never link it.**

A GPL or LGPL library statically linked into a `rivet-*` binary is the one
mistake that would force the licensing question. Every other arrangement —
executing a GPL program, driving one over a socket, shipping one in the image —
is aggregation.

Note how much of the existing design already complies without having been aimed
at this. `ADR-004` section 4 drives `wpa_supplicant` over a **socket** rather
than linking it, for architectural reasons; the licensing benefit is a side
effect. The same is true of every mechanism in `ADR-001`.

---

## 3. What is actually in the image

**These licences must be verified from each project's own source tree before
being relied on.** They are recorded here as the expected position, and
`AGENTS.md` section 7 applies — a licence claim is exactly the kind of assertion
that must not be repeated from memory.

| Component | Expected licence | Relationship | Consequence |
| :-- | :-- | :-- | :-- |
| Linux kernel | GPLv2 (with the syscall exception) | aggregated; executed | source offer if distributed |
| BusyBox | GPLv2 | aggregated; executed | source offer if distributed |
| musl | MIT | **statically linked** | none — see 3.1 |
| `wpa_supplicant` | BSD | aggregated; driven over a socket | attribution |
| `uutils/coreutils` | MIT | aggregated; executed | attribution |
| `nushell` | MIT | aggregated; executed | attribution |
| Rust crates | overwhelmingly `MIT OR Apache-2.0` | **statically linked** | attribution; see section 4 |


### 3.1 musl already solved the hardest problem

`ADR-003` chose musl over glibc for size, auditability and the NSS trap. It also,
without being asked to, removed the project's single worst licensing hazard.

Rivet links its libc **statically** (`ADR-003` section 3). Static linking is
precisely the case where copyleft bites. glibc is LGPL, and statically linking
LGPL code triggers relinking obligations — the distributor must supply whatever
is needed for a recipient to relink against a modified library, which for a
statically linked embedded image is a genuine and recurring burden.

musl is MIT. Static linking is unencumbered.

This is a good illustration of a decision paying twice, and a reason to be
careful about ever revisiting `ADR-003` on other grounds.

### 3.2 The first C mechanism audited, and the gap it exposed — 2026-08-23

The first row of §3's table to be **checked rather than expected**, because
§3's opening sentence says a licence claim must not be repeated from memory. It
was read out of the tree Buildroot actually built, not out of a summary.

**Two things came out of it, and only the second is still live.** The audit
itself found no copyleft, so nothing triggered §2.1 — but it found **six
separate attributions inside one component**, where the table had assumed one.
A permissively licensed C program is routinely an aggregate of other people's
permissively licensed code, and the count is not knowable without opening the
tree. That is the durable lesson and it applies to every C mechanism, present
or future. It also found a **conditional** clause — a naming obligation that
bites only if the component is patched — which is the shape to look for: an
obligation that is satisfied today because Rivet ships a binary unmodified, and
becomes live the day it carries a backport or a local fix.

**The gap that audit exposed is the part that matters.** §4.1's `cargo-deny`
gate covers the **Rust** dependency tree and generates the attribution manifest
from it. A C mechanism in the image is outside that gate entirely: `cargo-deny`
has never heard of it, and `wpa_supplicant` and BusyBox are behind it
(`ADR-005` §3). So **the image's attribution file
cannot be generated by the Rust gate alone.** Because Rivet distributes no
image (§6.5), the image owes no attribution file and nothing builds one; the
gap is live only if images are ever handed out, and the natural place for the
other half is then Buildroot's own legal-info machinery, which §6 of `ADR-009`
already notes Rivet inherits by choosing Buildroot.

---

## 4. The Rust dependency tree

`ADR-009` makes reuse the default, which means the crate tree becomes large.
The good news is that the Rust ecosystem is overwhelmingly `MIT OR Apache-2.0`,
so the default outcome is compatible with an MIT project.

Two things still need attention:

* **Apache-2.0 carries attribution and patent terms.** Compatible with an MIT
  project, but it means a generated attribution file is an obligation rather
  than a courtesy.
* **A small number of crates are copyleft**, and any one of them linked into a
  `rivet-*` binary triggers section 2.1. This will not be noticed by review.

### 4.1 The gate — ruled 2026-08-22: from the first crate

Per `AGENTS.md` section 3, a rule that depends on a human noticing is the
weakest kind. **`cargo-deny` with a licence allowlist, in CI**, is the gate: an
unexpected licence fails the build rather than reaching a release.

**Adopted from the first crate**, not at first release. It is a configuration
file and one CI step; starting clean is trivial and auditing several hundred
crates afterwards is not. It also covers the advisories database, which matters
more once `ADR-009` makes reuse the default.

It also generates the attribution manifest, which turns the obligation in the
previous paragraph into a build artifact instead of a maintenance task.

---

## 5. Consequence: Rivet's own code can stay MIT

The conclusion of sections 2 to 4:

* **Rivet's own source stays MIT.** Nothing in the image forces otherwise,
  provided section 2.1 holds.
* **A published Rivet *image* is an aggregate** whose components keep their
  own licences, and it carries the obligations of the strictest ones — which for
  the kernel and BusyBox means a source offer.
* **The flexibility the maintainer offered is not needed**, and should not be
  spent. MIT for Rivet's own code is achievable and is worth keeping, because
  it is what makes Rivet's components reusable by others — including by the
  maintainer's own sibling projects.

---

## 6. Ruled, 2026-08-22: private now, sanitized public mirror later

**The maintainer's answer: start with a *private* repository, and if the project
produces something worth sharing, create a *public copy* of it — exactly what he
did with Ignition.** *(`.rulings/` R-026.)*

That is the Ignition model recorded in `AGENTS.md` section 8.4 — a private
development repository and a separate sanitized public mirror that does not
share commit SHAs, with consumers pinning tags on the mirror.

### 6.1 Publishing a repository is not distributing an image

This distinction decides how much of section 3 is live, and it is easy to
collapse by accident.

| What is published | GPL source-offer obligation |
| :-- | :-- |
| Rivet's own Rust source and build recipes | **none** — it is MIT code, and users build their own images |
| A prebuilt `.img` containing the kernel and BusyBox | **live** — the corresponding source must be offered |

So a public mirror of the *source* triggers essentially nothing. The obligation
attaches the first time a **built image** is handed to anybody, and Rivet hands
none out (§6.5): people build their own.

**If that ever changes**, the image tooling must emit a source archive
corresponding exactly to what it shipped, and Buildroot's own machinery is where
it comes from (`ADR-009` section 6) — which is why choosing Buildroot keeps that
door cheap to open.

### 6.2 Zero-scrub stops being aspirational

`AGENTS.md` section 8.3 binds history, not merely the working tree. Under the
mirror model that rule becomes load-bearing rather than good practice: **if
nothing sensitive was ever committed, publishing is an export rather than a
scrub.**

The two shapes, and this is worth settling before there is history worth
mirroring:

* **Shared history.** The mirror carries the private repository's commits. Only
  possible if every commit is zero-scrub clean, for ever, with no exceptions.
* **Re-created history.** The mirror is built from the tree, not the log.
  Ignition works this way — the note that the repositories "do not share commit
  SHAs" is the observable consequence.

**Ruled 2026-08-22: re-created**, matching Ignition. Private working history
stays private; only the tree is published. Zero-scrub still binds regardless
(`AGENTS.md` §8.3) — this decision means a single careless commit cannot
permanently foreclose publishing, not that carelessness is now affordable.

### 6.3 The dual-licensing question gets sharper

Section 6.4 below asks whether Rivet's own crates take `MIT OR Apache-2.0`.
Under the mirror model that question matters more than it did, because the
mirror exists specifically so that other people — including the maintainer's
sibling projects — can depend on this code. `MIT OR Apache-2.0` is what Rust
consumers expect and what makes a crate frictionless to adopt.

---

### 6.4 Ruled 2026-08-22: `MIT OR Apache-2.0`

Rivet's own crates are dual licensed, matching Rust ecosystem convention. The
repository carries `LICENSE-MIT` and `LICENSE-APACHE`; `Cargo.toml` will declare
`license = "MIT OR Apache-2.0"`.

The reason is not symmetry. **Apache-2.0 carries an explicit patent grant that
MIT does not**, and some organisations treat MIT-only code as requiring legal
review for exactly that reason. Since the public mirror exists so that the code
can be reused — including by the maintainer's own sibling projects — the
difference is between frictionless adoption and a conversation with somebody's
counsel.

**This is a deliberate deviation from the house standard** in `AGENTS.md`
section 8.2 and the maintainer's global configuration, both of which specify
MIT. It is recorded here rather than applied silently. Whether the other
projects follow is his decision and belongs in his own configuration, not in
this repository.

The Apache-2.0 text was fetched from `apache.org` rather than reproduced from
memory, and checked for its version line and terms terminator. A licence is
exactly the kind of text that must not be written from recollection.

### 6.5 What the public repository carries, and no images

*(`.rulings/` R-254, R-255, R-256.)*

**Rivet publishes source and no built artifacts.** No card image and no system
update is handed to anybody: people build their own from the source, with the
tooling the repository carries. So §6.1's second row never fires, the image
owes no attribution file (§3.2), and no release waits on either.

**The public tree carries** the code, the ADRs, the SPECs, `skills/` and
`docs/design-candidates/`, with the generated references beside them. The
design candidates go because ADRs and SPECs cite them as the reasoning and the
rejected alternatives behind their decisions. **It does not carry**
`STATE.md`, `.rulings/`, the journal (`docs/PLAN.md` and the other
`docs/PLAN-*.md` files) or `AGENTS.md`: the first three describe the
maintainer's own machines and working sessions rather than the software, and
`AGENTS.md` is his standing practice for agents working in the private
repository. The ADRs and SPECs are written to
read completely without `.rulings/`, so a public reader loses only the
provenance of an R-number. Citations of the journal and the brief are left as
they are, and the README is written to be true in both trees.

**Nor does it carry CI or trusted keys.** `.github/` stays private because its
workflows run on the maintainer's own self-hosted runner, and a self-hosted
runner on a public repository runs any fork's pull request on his machine;
CI for the public repository, on GitHub's own runners, is a later and separate
step. `keys/*.pub` stays private because a card trusts exactly the keys it was
built with: a card built from the public tree must trust its builder's keys,
and with none present the card build refuses rather than producing a card
that trusts nobody. *(`.rulings/` R-257.)*

**The public tree is written by `tools/release/cut.py`**, from one committed
revision, so every release is the same act. It leaves out everything above,
regenerates the generated references from the published files alone, and
refuses to stage a tree that carries a private path, an absolute workstation
path, or any of the maintainer's own values — a list kept beside the script in
the private repository and never copied. It commits nothing.

**It lands in `caladil/Rivet`.** That repository was created private so the
first tree could be reviewed from a clone and tagged before anyone else could
see it, and it was made public only by the maintainer's separate decision.
*(`.rulings/` R-258, R-372.)*

---

## 7. Superseded question, retained for its reasoning

**Is Rivet's image intended to be distributed, or is it personal-use only?**

Answered in §6.5: Rivet publishes source and no images. The analysis is kept
because the distinction it draws — obligations attaching to *binaries*, not to
source — is the one that governs every future decision about what gets
published.

This is the question that decides how much of this document is live, and it has
not been asked:

* **Personal use only.** The GPL obligations in section 3 are largely dormant —
  they attach on distribution, and there is none. `cargo-deny` is still worth
  having, because the posture is much cheaper to establish now than to
  retrofit.
* **Images published anywhere.** The source-offer obligations for the kernel and
  BusyBox become live, and the build must be able to produce a corresponding
  source archive for exactly what was shipped. That is a build-system
  requirement and belongs in the image tooling from the start, not bolted on at
  first release. It also interacts with `ADR-009` section 6: Buildroot already
  has machinery for this, which is a further argument in its favour.


---

## 8. MPL-2.0 is allowed — ruled 2026-08-23

`deny.toml` named MPL-2.0 alongside GPL, LGPL and AGPL as *"deliberately never
on this list without the maintainer's own ruling"*, on the stated grounds that
its **"file-level weak copyleft interacts awkwardly with static linking"**.

**The maintainer ruled it in, and the stated reason was backwards.**

### 8.1 What MPL-2.0 actually requires

**It is file-level copyleft, and that is a scoping rule rather than a
weakening.** The obligation attaches to the individual MPL-licensed *source
files*, never to whatever they are linked into. §3.3 explicitly contemplates a
**"Larger Work"** distributed under other terms with the MPL files remaining
MPL. **Static linking does not infect** — that is precisely the design
difference from LGPL, which *is* linkage-scoped, and the two were being treated
as the same hazard.

Applied to how Rivet actually works:

| Obligation | Fires here? |
| :-- | :-- |
| Publish modifications to MPL files under MPL | **No.** The crates go in unmodified from crates.io. |
| Make MPL files' source available to recipients on distribution | **Only if a binary is distributed**, which Rivet does not do (§6.5) — and it would be a URL: unmodified public crates. |
| Relicense the larger work | **No.** §3.3. Rivet's own crates stay `MIT OR Apache-2.0` (§5). |
| Anything at all before a binary leaves this machine | **No.** These are *distribution* obligations, and §6 has the repository private. |

**That duty is a smaller version of §6.1's, and like it, it would fire only on
distribution.** The GPL source-archive machinery Buildroot brings exists for a
strictly harder case; satisfying MPL-2.0 means listing two crates and their
upstream URLs.

### 8.2 What this does not open

**GPL, LGPL and AGPL are untouched** and remain §2.1's red line. The distinction
being drawn is not "copyleft is fine after all" — it is that MPL-2.0's copyleft
has a **stated scope that stops at the file**, and that scope is the thing §2.1
cares about.

The occasion was the `nushell` survey (`ADR-005` §5.6.4), which found exactly two
MPL-2.0 crates in a 572-crate closure — `option-ext`, reached through `dirs`,
which is how nushell finds `$HOME`, and `nucleo-matcher`, reedline's completer.
Feature-trimming removes neither; they are in nushell's core. **The same survey
found nothing GPL, LGPL or AGPL anywhere in that tree**, which is the result
§2.1 was actually protecting against and it came back clean.

### 8.3 The bookkeeping that came with it

Thirteen further permissive licences were added to `deny.toml`'s allow list in
the same ruling: `Unicode-3.0`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Zlib`,
`CC0-1.0`, `0BSD`, `BSL-1.0`, `CDLA-Permissive-2.0` and `CC-PDDC`.

`deny.toml`'s own comment had predicted eight of them. **It missed four** —
MPL-2.0, CDLA-Permissive-2.0, CC-PDDC and BSL-1.0 — which is a fair record of
how well the ecosystem's licence spread can be guessed in advance, and an
argument for the gate existing rather than against it.

---

## 9. The gate must cover the host, not just the image — RULED 2026-08-26

**Found while scoping `ADR-034`'s `rivet-fleet`**, and it is a hole rather than
an omission.

`deny.toml` declares exactly two targets:

```
aarch64-unknown-linux-musl
x86_64-unknown-linux-gnu
```

`rivet-fleet` runs on **Windows and macOS** (`ADR-034` §2.2), and `ratatui`'s
`crossterm` backend pulls a different dependency tree on each. **None of it
would be evaluated** — `cargo deny check licenses` would pass while never
looking at the crates in question. A gate that reports success without
inspecting is `AGENTS.md` §3's failure, in the tooling rather than in the code.

**Ruled: extend the targets to `x86_64-pc-windows-msvc` and both Darwin
triples, and keep ONE allow-list.**

The obligations genuinely differ — §2's aggregation-versus-linking rule and §3
are about **what is in the image**, and a tool the maintainer `cargo install`s
is not distributed by him at all. A second, looser policy for host tools would
be defensible on the merits.

It is refused anyway: **two licence policies in one repository get confused
exactly once, expensively.** One list costs a few more entries and §4.1 already
ruled the gate applies *"from the first crate"*. If a host-only crate ever
needs a licence the image must not carry, that is the moment to split — with a
named crate to point at, rather than in advance.

---

## History

**2026-10-09 — Rivet publishes no images (`.rulings/` R-254).** §6.1 and §7 had
recommended building for distribution from the start:

~~**Recommendation, unchanged:** build for it anyway. The image tooling should
be able to emit a source archive corresponding exactly to what it shipped,
because adding that later means rebuilding the tooling.~~

~~**Recommendation:** decide now and assume distribution. The cost of building
for it and never publishing is small; the cost of publishing and then
retrofitting it is a rebuild of the image tooling.~~

The source archive was never built, and the maintainer then ruled that people
build their own images, so the obligation it prepared for does not attach. The
part of the argument that still holds — Buildroot keeps that door cheap to open
— stays in §6.1.
