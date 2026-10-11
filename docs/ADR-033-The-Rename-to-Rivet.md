# ADR-033: The Rename to Rivet

* **Status:** **Accepted** — ruled and executed 2026-08-25.
* **Date:** 2026-08-25
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Supersedes nothing.** It renames the subject of every other document in
  `docs/`, and §2 is the warning a reader needs before trusting a quotation.

---

## 1. The decision

**FerriPi is Rivet.** The maintainer ruled it on 2026-08-25 and it was carried
out the same day: 7,807 occurrences across 253 files, 16 directories and 4 files
renamed, and 380 instances of the payload extension.

No reason beyond the maintainer's preference is recorded, because none is owed
— **this is a naming decision and naming decisions are his** (`AGENTS.md` §1).
What *is* owed, and is what this document exists for, is an account of the four
places the rename was not mechanical.

---

## 2. **The record was rewritten, and every quotation in it now says a word
## nobody said**

**Read this before quoting anything in `docs/` back at anybody.**

`AGENTS.md` §2 rule 3 is *supersede in place, never silently edit* — the record
should show its own history. A global find-and-replace across 32 ADRs and an
8,000-line `PLAN.md` is exactly the silent edit that rule forbids, and it was
done anyway, deliberately, because the alternative was worse.

**The ruling, and the declaration it requires:**

> **Every occurrence of "Rivet" in text dated before 2026-08-25 was written
> "FerriPi" — including inside quotation marks, including in statements
> attributed to the maintainer, and including in readings taken off hardware.**

The maintainer's framing on 2026-08-22, that the project may differ from
traditional Linux distributions to the degree Android does, was said under the
old name, and `ADR-008` §1 now shows it with a different noun in it.
*(`.rulings/` R-021.)* The same is true of
every board reading, every journal excerpt and every struck-through paragraph in
the tree.

**Why the alternative was worse.** Renaming forward-looking prose and leaving
dated passages alone would produce documents that use two names for one thing on
the same page, and a reader working out which is which on every paragraph. That
is a different dishonesty and a larger one: a reader who *misreads* the record is
worse served than one who is told, once and plainly, that a mechanical
substitution happened. One paragraph buys back what the substitution costs.

**Nothing else about the record was touched.** No claim was softened, no
struck-through text was un-struck, no date moved. The transformation was
`s/FerriPi/Rivet/g` and its case variants, and if a sentence reads oddly now it
read oddly before with a longer word in it.

---

## 3. The container magic, which is not a name

`ADR-029` §3's first eight bytes were `*b"FERRIPI\x1a"` — seven printable bytes
and DOS end-of-file, the trick PNG uses so that a payload catted to a terminal
stops rather than scrolling.

**`RIVET` is three bytes shorter, so the field no longer filled itself**, and
the maintainer ruled all eight bytes printable rather than name-plus-padding:

| | Was | Is |
| :-- | :-- | :-- |
| Update payload | `FERRIPI\x1a` | **`RIVETPKG`** |
| Trust store | `FERRIPIT` | **`RIVETTRS`** |

**What that trades away, recorded because it was a real property:** a payload
`cat`ed to a terminal now scrolls 683 MB instead of stopping at byte eight. The
text-mode-transfer check survives — those eight bytes are still the first thing
a reader compares, and a mangled transfer still shows up in them.

**What it buys**, and it is the stronger half: the two magics now differ in
three of eight bytes instead of one, and **both say in plain ASCII which kind of
file they are.** `ADR-028` §5.2's requirement that a payload and a trust store
never be mistaken for one another is better served than before.

**No compatibility branch is owed to anybody.** The old magic identifies a
format this one does not claim to read: a `FERRIPI` payload fails at byte zero
with the refusal `ADR-029` §5 step 1 already produces, which is the correct
answer rather than a missing feature.

---

## 4. Package names are `rivetos-*`, and everything else is `rivet`

**Found while renaming, and it is a fact rather than a preference:**
`rivet-core` is **taken on crates.io**, along with `rivet-http`, `rivet-logger`,
`rivet-metrics`, `rivet-error` and `rivet-foundation` — an entire published
family belonging to rivet.gg, an established company in game-server
infrastructure. `cargo-rivet`, `rivetos` and `rivetos-core` are free.

**Cargo separates three names that are usually conflated**, and this ADR uses
all three separations:

| | Value | Who sees it |
| :-- | :-- | :-- |
| `[package] name` | `rivetos-core`, `rivetos-initd`, … | crates.io, and nothing else |
| `[lib] name` | `rivet_core`, `rivet_config`, … | **every `use` statement in the tree** |
| `[[bin]] name` | `rivet`, `rivet-initd`, … | `/system/bin`, and the operator |

So **no source file changed for this**, `/system/bin/rivet-initd` is unchanged,
the login shell's `/etc/shells` contract is unchanged, and the OS is Rivet
and the CLI is `rivet` exactly as ruled. The only thing that moved is an
identity nobody has yet visited.

**Two packages are deliberately not `rivetos-`:**

* **`cargo-rivet`** — the name is free, and it is the one a person actually
  types (`cargo rivet deploy`). A cargo subcommand's package name *is* its
  user-facing name, so hiding it behind a different one would be the confusion
  this section exists to avoid.
* **`femto`** — `ADR-005` §5.5.1.3 ruled it prefixless on the grounds that a
  prefix means *this project's own machinery* and femto is a general tool that
  ships on the card as `nu` does. That reasoning never mentioned the project's
  name and is unaffected by it changing.

**The directories keep the `rivet-` spelling** (`rivet-core/`, `cmd/rivet-initd/`)
because they match the *library* and *binary* names, which is what the source
says. A directory named for a crates.io identity would be the one place the
three-way split leaked into somewhere it does not belong.

---

## 5. The migration: there is not one

**Ruled: rewrite the card.** No bridge release, no transitional acceptance of
both magics, no compatibility branch anywhere in the tree.

The alternative was a build that still identified itself as FerriPi on the wire
but whose installer accepted both — two deploys and no physical access. It was
rejected, and the reasoning is `ADR-018` §5's in a new setting: **Stage 2 is
pre-release, a card rewrite is the normal way to move a board forward, and
compatibility code written for one machine outlives the machine.**

The consequences, stated so nobody is surprised by one:

* **Every FerriPi-era board is unreachable by a Rivet update** and must be
  re-flashed. There was exactly one, and it was re-flashed.
* **`/state` does not survive**, so the journal, the installed applications, the
  clock file and the identity of any such board are gone. The maintainer
  ruled explicitly that nothing was worth rescuing from the one board that
  existed — the configuration document was captured to re-provision from, and
  nothing else was kept.
* **The signing keys are unchanged.** The seeds were not regenerated: only the
  trust-store *labels* moved to `rivet-system` and `rivet-app`, and the files to
  `rivet-system.seed` and `rivet-app.seed`. Verification is by `key_id`
  (`ADR-028` §5.8), so a label is metadata and regenerating two seeds that have
  no backup would have destroyed something to change a string.

---

## 6. What was checked, and how

`AGENTS.md` §6 — what was verified and by what method.

* **`cargo test --workspace` under WSL: 29 suites, no failures.** This is the
  gate that matters, because a rename is exactly the change a compiler catches
  in full — every crate path, every module reference and every dependency edge
  is checked by construction.
* **`cargo clippy --workspace --all-targets` under WSL: no new warning.**
* **The binaries are named right**, checked by listing them rather than assumed:
  `rivet`, `rivet-initd`, `rivet-sysd`, `rivet-netd`, `rivet-timed`,
  `rivet-deploy`, `rivet-image`, `cargo-rivet`, `femto`.
* **Two literals the compiler could not catch were found by tests**, and both
  are recorded where they happened: a text-field test that asserted a 7-byte
  slice for a 7-letter word, and a trust-store test that respelled the
  container's magic as a literal instead of referring to the constant. The
  second now refers to `format::MAGIC`, which is `ADR-011` in the one place
  that proved why it matters.

**What is NOT proved:** nothing renamed has run on hardware. The card is built
from this tree and has not booted a Raspberry Pi at the time of writing;
`PLAN.md` §24 carries that.
