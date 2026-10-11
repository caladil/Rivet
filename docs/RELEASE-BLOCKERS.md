# RELEASE-BLOCKERS: what stands between this tree and a public release

* **Status:** Living. A standing document, not a decision — see each entry for
  the ADR, SPEC or ruling that actually settles it.
* **Date:** 2026-09-13
* **Revised:** 2026-10-11
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. What this is

**The public-release blocker list.** Going public means copying this tree into
a fresh public repository that carries none of this repository's history
(`ADR-010` §6.2, `.rulings/` R-251). So a blocker is anything in the **tree**
that must not be published as it stands, or anything in the software a stranger
would hit that the maintainer would not ship.

| § | Entry | State |
| :-- | :-- | :-- |
| 2 | The Aura addon architecture | **Closed** — no relay client, and nothing that says what the reading card shows, is in this tree |
| 3 | How addons reach this repository | **Closed for release**; one dependency moves on release day |
| 4 | The fourth-identity trust ruling | **Deferred by ruling**; nothing waits on it |
| 5 | Uninstalling a library | **Closed** — `apply uninstall-library`, proven on dev-01 |
| 6 | `apply edit --append` | **Closed** — the join is refused, proven on dev-01 |
| 7 | The tree is zero-scrub clean | **Closed** |
| 8 | Which parts of the record are published | **Ruled** |
| 9 | Handing out a built image | **Closed** — Rivet publishes no images |
| 10 | CI on a public repository | **Closed by ruling** — the public repository starts with none |
| 11 | Which keys a card from the public tree trusts | **Closed** — the builder's own; the tree carries none |
| 12 | The README, and citations of private documents | **Closed** — README rewritten; citations left as they are, by ruling |
| 13 | Producing the public tree | **Closed** — `tools/release/cut.py`, into `caladil/Rivet`, made public by ruling |
| 14 | Rivet Monitor | **Separate** — published from its own repository |
| 15 | The maintainer's own words in published documents | **Closed** — the guard reads every published file, and it is clean |

Closing an entry here is a ruling, not a checklist item ticked in passing —
each one changes what the fleet does, who may reach it, or what an addon may
ship. When one closes, rewrite its entry to say so and cite the ruling; do not
delete the entry outright unless nothing would ever need to ask "was this
considered."

---

## 2. The Aura addon architecture

**Closed.** The relay client — the reason this repository could not go public
(`.rulings/` R-197 D1) — is not in this tree, and **neither is anything that
says what Aura's reading card shows** (R-375). The card is drawn from a shared
rivet of class `aura-reading-source`, built in its own private repository;
indoor temperature comes from class `aura-indoor-source`, built in the TempMon
repository (R-197 D2). `ADR-053`'s `accepts-class` is the primitive both rest
on, and `SPEC-04` §4.5 records the two-kinds-of-addon distinction the split
follows.

What stays in this tree is the host's side, and it knows nothing about the
reading. `aura-controller` opens whichever library provides each class, keeps
the settings that addon declares in the shape it declared them, and hands them
back across; the renderer draws a value, a unit, a trend and a state. **A host
that held the addon's settings in typed fields of its own would know what the
addon measures**, which is the coupling the addon boundary exists to remove.

The split needed one protocol change: a room's sensor reports temperature and
humidity from one read, so `Polled::also` carries a second, named reading. It
arrived as a new optional field without a version bump, under `abi::VERSION`'s
own rule.

## 3. How `aura-card` is reached from outside this repository

**Closed for release; one dependency moves on release day.** Consumers reach
this repository in two shapes:

| Consumer | Depends on | Shape |
| :-- | :-- | :-- |
| the reading-source addon (private) | `rivetos-addon` | **path** to a sibling Rivet checkout |
| TempMon's `aura-indoor-source` | `rivetos-addon` | **git**, `caladil/Rivet-Dev.git`, tag `v0.1.95` |
| TempMon's `tempmon-adapter-rivet` | `rivetos-sdk` | **path** to a sibling Rivet checkout |
| Trends' `server/rivet` | `rivetos-sdk` | **path** to a sibling Rivet checkout |

`.rulings/` R-203 D1 ruled a git dependency pinned to a tag. The reading-source
addon has since moved to a sibling path on the maintainer's instruction,
recorded in its own `Cargo.toml` (that repository's commit `835d6ca`) as superseding
R-203 D1 while the addon ABI is still allowed to drift: an addon built against
the same checkout as the image it is deployed onto agrees with it by
construction, where a tag has to be remembered and bumped. The tag returns when
the ABI is locked, which the soname's `.1` already names. **That ruling is
recorded only in the addon's repository**, and TempMon's addon still carries
the tag.

What going public changes:

* **A path dependency needs no change.** It resolves against whatever sibling
  checkout is present, and a public clone serves as well as a private one, with
  no credential involved.
* **The git pin has to move.** `caladil/Rivet-Dev.git` stays private, and a
  fresh public repository has none of its tags. TempMon's `aura-indoor-source`
  either repins to a tag on the public repository or takes a sibling path like
  the reading-source addon. Either is mechanical.
* **The fetch scripts lose their reason.** The reading-source addon's
  `tools/fetch.sh` and `TempMon/tools/rivet/fetch-addon.sh` exist so the build container could
  build against a private repository without holding a credential.

**What an addon links is `rivetos-addon`, not `aura-card`.** The heading names
the crate the ruling named. `aura-card` is a path dependency inside this
workspace (`aura-renderer`, `aura-vitals-test`), and no addon has needed the
card vocabulary.

## 4. The fourth-identity trust ruling

**Deferred by ruling; nothing waits on it.** `ADR-052` §10 step 5: letting a
fleet server's key install software on every other machine in the fleet is a
fourth named identity beside `ADR-034`'s operator, agent and monitor, and it is
not decided by an implementation arriving at it. `.rulings/` R-155 R1 put it on
the roadmap and ruled that nothing waits on it, because the work in front of
this project does not need a fleet server installing software anywhere. It is
decided by its own ADR amending `ADR-034`, when it is taken up.

It is listed so a reader can see it was considered. It does not block a
release.

---

## 5. Uninstalling a library

**Closed, and proven on dev-01.** The upgrade outage is closed by retention,
and removal on purpose by `apply uninstall-library` (`SPEC-02` §13).

**Why it blocks a release**: `ADR-053`'s class mechanism is how an addon is
*replaced*, and a platform that can install a replacement but cannot retire
what it replaces cannot ship a second version of anything.

**What is closed.** Installing a new build of a library used to leave two
libraries of one class on the machine, which `.rulings/` R-200 correctly
refuses to choose between, and the host stopped drawing (§5.1). The supervisor
now keeps one build per library product at the install edge (`.rulings/` R-202
D1; `KEEP_LIBRARY = 1`, `cmd/rivet-sysd/src/retention.rs`): before discovery
runs, older builds of what was just installed are set aside and deleted, each
eviction journalled by name. An ordinary upgrade lands as a replacement.

**What closed it.** `rivet apply uninstall-library <product|reference>
[confirm]` removes a library on purpose — one that is no longer wanted, rather
than one superseded by its own newer build. A product name removes every build
of the product, a full reference exactly one. It is refused, removing nothing,
when an installed application declares a selected build in its `library`
closure or has a manifest that cannot be read; an application that only accepts
the class is named in the plan and loses the addon at its next start. The
supervisor decides again when it takes the request up, sets each build aside,
journals it by name, and re-runs the install edge's discovery, so every
service's library path describes what is left **without a reboot**. Nothing
running is restarted. `SPEC-02` §13 is the specification.

**Still true of a removal by hand.** The file surface (`ADR-006` §8.10) can
still delete a library's files one at a time, and the supervisor does not notice
until a reboot: only an install and the verb re-run discovery. The verb is the
supported route.

**Proven on dev-01** at `v0.1.95-884-g9b926e04`: a library planned, removed,
absent from `/state/lib` and from the journal's recomposed library paths, on the
same boot, with the two applications that accepted it named and not restarted.
The declared-closure refusal is proven by tests only, because nothing on a
board declares a library by reference. rivet-01 has it from its next update.

### 5.1 The upgrade outage, closed by retention

Installing `aura-background-pack` 0.1.5 by the ordinary route on `rivet-01` made
two libraries of class `aura-background` present, and `aura-renderer` drew no
background at all:

```
REFUSED 2 libraries of class `aura-background` are installed and opening one
of them would be a choice nobody made
```

The refusal was right. The defect was what produced it: deploying a new version
of the one pack there was. Retention on the install edge is what makes a
library upgrade atomic — the old build is evicted as the new one lands, so the
host never sees two. Recovery at the time took four file-surface calls and a
reboot, which `apply uninstall-library` now replaces.

---

## 6. `apply edit --append` could comment out what it appended

**Closed, and proven on dev-01.** An append adds exactly the bytes it is given, so appending to a file
whose last line has no trailing newline, with text that does not begin with
one, used to continue that line. On `rivet-01` this put
`[services.rivet-displayd]` inside a trailing comment of the desired-state
document, leaving `enabled = true` as a key of the preceding table. The
document still parsed, nothing warned, and the machine booted with its
compositor disabled and a dark screen.

**That append is now refused**, at the plan and at the confirmation, with
`E_UNSAFE` and both unambiguous spellings: begin the text with a newline, or
change the last line with `--expect`/`--replace` (`ADR-006` §8.9.4 has the
reasoning, including why the refusal is general rather than limited to
comments). Every tool that composes an append already begins its text with a
newline; the skills' examples and `apply install`'s printed step now do too.

Re-check, from a checkout:

```
cargo test -p rivetos-core --lib operation::edit
```

**Proven on dev-01** at `v0.1.95-884-g9b926e04`: the refusal came back with
`E_UNSAFE` against a file ending without a newline, nothing was written, and the
same append with a leading newline went through. rivet-01 has it from its next
update.

---

## 7. The tree is zero-scrub clean

**Closed.** Anything about the maintainer's health, workstation paths, his home
network, fleet names and location, his e-mail address, and identifiers read off
his hardware are out of every file a release carries.

Because a release copies this tree into a fresh repository, `AGENTS.md` §8.3's
zero-scrub rule stops being a habit and becomes a precondition: what is in the
tree on the day is what is published. History is not carried, so only the tree
matters. **This file is published too**, so it describes the personal values
rather than quoting them.

### 7.1 The maintainer's health — clean

**No published file states or implies anything about the maintainer's
health** — not a reading, not a diagnosis, not an application described by
what it treats, not the vocabulary or units of a measurement in example code
(`.rulings/` R-374). Rivet's own *machine* health (`ADR-045`), a generic class
of sensitive personal data, and the security senses of the same words are
outside the rule.

**Where an example needs an application, it uses TempMon** — an application
that gathers temperatures from Bluetooth sensors — or, where a fixture already
holds TempMon, the network-service template's notes application. **Where a
passage characterises what a machine holds, it says *personal data***, which is
the argument such a passage makes.

**The cut enforces it.** `tools/release/cut.py`'s private list carries the
vocabulary, so a tree in which any of it survives is refused, and the re-check
is the cut itself — the words cannot be written here.

### 7.2 Absolute paths — clean

No tracked file names a real workstation path. A tool that needs a location
takes it as an argument or derives it (`tools/docs/section.py --backup-dir`,
defaulting to the platform's temp directory), and evidence kept outside the
repository is cited as a file in the maintainer's scratchpad rather than by its
path. The placeholders that remain — `C:\Users\you`, `someone`, `operator` —
are examples, not anyone's machine. Re-check:

```
git grep -n -i -E "[A-Za-z]:[\\/](Dev|Users)[\\/](Projects|Scratchpad)|~/dev/" -- .
```

The expected hit is that command's own line in this file.
`tools/coldagent/preflight.sh` keeps a default workspace relative to `$HOME`,
which names nobody.

### 7.3 Network details and names — clean in every published file

The maintainer's home subnet, his fleets' names and his username were spread
through test fixtures, doc comments and documents. In every file a release
carries they are replaced: the subnet by `192.168.50.0/24`, a private range no
file used before, so a fixture that depends on an address being private still
does and no two distinct fixture values collapse into one; the fleets by
`home-fleet` and `dev-fleet`, which keep the originals' sort order; the username
by `someone`. `ADR-063` says the fleet names stand for an operator's own.

**The private files keep the real values** — `STATE.md`, `.rulings/` and the
journal (`ADR-010` §6.5) — because agents operating the real fleet read its
names and addresses from them. The re-check uses the maintainer's own values as
its patterns, so it is kept in the journal rather than written here.

**Deliberately kept:** the maintainer's name where it credits him as the
author or the ruler of a decision, as `LICENSE-MIT` and `Cargo.toml` already
do; `America/Chicago`, which the tree uses only as an example zone and an entry
in the zone list, never as a default; and the WebAuthn vectors captured from a
real browser, whose relying party is `dev-01.local` and names nobody.
`docs/INDEX.md` and `docs/index.json` are generated from every document,
private ones included, and are regenerated from the public tree when it is cut.

### 7.4 The weather client's contact is the operator's setting — closed

The weather service asks every client for a way to reach its operator, and
Aura's controller used to compile one person's address into every request. The
contact is now `weather_contact` in Aura's configuration, set from the
portal's Weather section and **empty by default**: unset, a request names only
`Aura/1.0`, so no address ships in the program. A value a request header
cannot carry is never saved and never sent. Ruled by the maintainer, who chose
an operator setting over a project-wide contact, so a board sends an address
only when its own configuration holds one.

The controller's location fixtures, a ZIP code and its coordinates, went with
it: they now name a well-known public address rather than an area anyone
lives in.

### 7.5 Identifiers read off the maintainer's hardware — clean

The sweep above looked for values about the maintainer and missed values read
off his equipment: his two temperature sensors' identifiers (one fixture paired
them with the rooms they sit in), his Raspberry Pi's CPU serial (in the very
tests that prove a board's derived hostname hides its serial), and a board's
Ethernet address. Fixtures and examples now carry made-up identifiers, and a
reading quoted in an ADR keeps its shape with the device's own bytes masked.

A board's **derived hostname** is not in this class and stays: it is a hash of
the serial, made so that the serial never reaches the network (`rivet-core`'s
`hostname` module), and the machine announces it anyway.

**The cut enforces this rather than trusting it.** `tools/release/cut.py`
refuses to stage a tree in which any byte matches a private list of the
maintainer's own values, kept beside the script and never copied.

### 7.6 His machines' names, and his name as sample data — clean

The names of his own machines are out of every published file: his x86 server
and his former fleet server were sample hostnames in tests and scripts and
were named in several ADRs, one of which also said what data the server held.
Tests and scripts use neutral names (`arbor`, `cedar`); documents say what the
machine is (*an x86 target*, *the fleet server*). The cut's private list now
carries both names. *(`.rulings/` R-368.)*

**His name is not sample data.** Where an example or a test needed a person's
name it uses `Your Name` (and `Someone's` for a possessive, `you@example.com`
for an address — `example.com` is reserved for exactly this). His name stays
where it credits him: the licence, the workspace's `authors`, and the
attribution of a ruling to the maintainer who made it.

---

## 8. Which parts of the record are published

**Ruled** (`ADR-010` §6.5, `.rulings/` R-254, R-255, R-256).

| Part | Published | What it holds |
| :-- | :-- | :-- |
| the code, `docs/ADR-*`, `docs/SPEC-*`, `skills/`, the generated references | **yes** | the software and its settled record |
| `STATE.md` | **no** | the live state of the maintainer's boards, fleets and backups |
| `.rulings/RULINGS.md` | **no** | his own words, verbatim |
| `docs/PLAN.md` and the other `docs/PLAN-*.md` files | **no** | the working journal: what was run, on which board, and what it found |
| `AGENTS.md` | **no** | the maintainer's standing practice for agents in the private repository |
| `docs/design-candidates/` | **yes**, after §7's sweep | proposals, kept for their reasoning and for what they got wrong |

The ADRs and SPECs are written to read completely without `.rulings/`
(`AGENTS.md` §2), so leaving it out costs a public reader only the provenance of
the R-numbers. The journal and `AGENTS.md` are harder: ADRs, SPECs and skills
cite sections of both, and the public tree leaves those citations pointing at
nothing. For the journal that is debt already counted, paid down by promoting
what is worth citing into an ADR; a rule from `AGENTS.md` that a public reader
needs — zero-scrub is the obvious one — belongs in an ADR for the same reason.

**The design candidates are published** because ADRs and SPECs cite them as the
reasoning behind decisions and the alternatives rejected. Some were written
against the maintainer's boards' logs, so §7's sweep covers them before
release.

## 9. Handing out a built image

**Closed: Rivet publishes no images** (`ADR-010` §6.5, `.rulings/` R-254).
People build their own card images and updates from the source. The kernel's
and BusyBox's GPL source offer and the image's attribution file attach only to
an image somebody is handed, so neither is owed and no release waits on them.
`ADR-010` §6.1 says what changes if that ever does.

## 10. CI on a public repository

**Closed by ruling: the public repository starts with no CI** (`ADR-010` §6.5,
`.rulings/` R-257). The workflows run on the maintainer's own self-hosted
runner and trigger on pull requests, and on a public repository a fork's pull
request would run on that machine. `.github/` stays in the private repository.
CI for the public one, on GitHub's own runners, is a later step of its own.

**Pull requests are switched off on `caladil/Rivet`, and issues are on.**
Anyone can read, clone and fork the repository, and anyone can report a problem
or ask a question; nobody outside it can propose a change to it. Contributors
with push access are added by the maintainer. *(`.rulings/` R-370.)*

## 11. Which keys a card from the public tree trusts

**Closed: the builder's own** (`ADR-010` §6.5, `.rulings/` R-257). A card
trusts exactly the keys it was built with, so a `keys/` carrying the
maintainer's public keys would make every card built from a clean clone accept
his signatures. The public tree carries `keys/README.md` and no `.pub`;
`cargo rivet setup` mints a builder's own, and the card build refuses with no
system key and says how to make one.

## 12. The README, and citations of private documents

**Closed.** Published files cite `AGENTS.md`, `PLAN.md` and `STATE.md`
thousands of times; those citations are left as they are, by ruling
(`.rulings/` R-257). The README was the exception because it is the front
door: it is rewritten to describe the software and to be true in both trees,
and it says how the repository is published and which files stay private.

## 13. Producing the public tree

**Closed.** `tools/release/cut.py` writes it from one committed revision into a
clone of `caladil/Rivet`, which was created **private** (`.rulings/` R-258). It
leaves out the private paths, rewrites the workspace's `repository`,
regenerates the docs index and claims baseline from the published tree alone,
and refuses to stage a tree carrying a private path, an absolute workstation
path, any of the maintainer's own values, a failing claims gate, or a Cargo
workspace that does not resolve from its own lockfile. It stages with each
executable mode set and commits nothing.

**It also refuses a clone that would commit under a private address.** Every
commit's author and committer, and an annotated tag's tagger, carry an e-mail
address the public repository shows to everyone, and a fresh clone inherits
the workstation's identity. So before writing anything the cut resolves the
clone's author and committer identities as `git var` does — configuration and
environment both — and refuses either one that matches `forbidden.txt`. The
public repository's commits carry the maintainer's GitHub no-reply address.
*(`.rulings/` R-371.)*

Its first run found two test fixtures still using the maintainer's username
in capitals, which the case-sensitive sweep of §7.3 could not see.

**`caladil/Rivet` is created private, reviewed, and made public by the
maintainer's decision** (`.rulings/` R-368, R-372). The current repository's
first commit is tagged `v0.1.137`.

**Its history starts from a clean cut.** The first staging repository held
cuts made before §7.6's and §15's work, so its early commits carried text the
tree no longer does. A force-push would not have removed them — GitHub keeps a
replaced commit reachable by its hash, and a public repository's push events
name those hashes — so that repository was set aside, still private, and
`caladil/Rivet` was created again with the current cut as its first commit.
*(`.rulings/` R-369.)* The same was done once more when that first commit was
found to carry the maintainer's personal address rather than his no-reply one
(R-371), and a third time when the record's scrub was widened after the
repository had been made public (R-374): the public repository was renamed
aside and made private, and `caladil/Rivet` was created again from a cut that
passes the widened check. Repinning TempMon's addon
off this repository's tag (§3) is TempMon's to do, and it does not stand in
Rivet's way: the pin keeps working against the private repository.

## 14. Rivet Monitor

**Published separately, from its own repository**, by an agent working there
(`.rulings/` R-257). This repository's ADRs, the backup skill and
`cargo-rivet`'s help name it as the program that schedules and mirrors
backups, which is why it is published at all.

## 15. The maintainer's own words in published documents

**Closed.** `.rulings/` R-008, the ruling that created the rulings file, is that
his quotations do not ship in what a release carries: a published file states
his ruling in its own words and cites the R-number, and the quotation lives in
`.rulings/RULINGS.md` alone.

**The guard reads everything a release carries.** `tools/docs/rulings.py`'s exact
half compares every recorded quotation against every published file — not
`docs/` alone, but doc comments, manifest and script comments too — after
stripping each line's comment and quote markers, so a quotation wrapped across
`//!` or `>` lines is found. It reads clean, and it fails the day a recorded
quotation is pasted back anywhere published.

**What it cannot see, and how that was closed.** A quotation nobody recorded is
invisible to an exact check. Every passage in the published tree that put a
quoted span near an attribution to him was therefore read, about 450 of them:
those that were his words were recorded and their passages rewritten to read
completely without the citation; the rest — error messages, console output,
other documents' text, the document's own emphatic sentences — were left. The
fuzzy half's remaining reports are all of that second kind.

**What stays a judgment.** Some rewritten passages state a ruling closely in the
order he gave it, without quotation marks. They read as the document's own
statement and cite the ruling; whether any should move further from his wording
is a reading, not a check.

```
uv run tools/docs/rulings.py
```

---

## History

**2026-10-11 — §2 and §7.1 rewritten.** §2 had closed when the relay client
left the tree, and accepted the host keeping the addon's settings in typed
fields of its own because none of it was a client or held a credential. It now
closes on the host knowing nothing about what the card shows (`.rulings/`
R-375), which is the coupling the addon boundary exists to remove. §7.1 was
widened by ruling (R-374).

**2026-10-09 — rewritten to current state.** §2 and §3 had closed in September
and still read as open work. §5 had described the library-upgrade outage as live
after retention closed it; it now separates that from the missing removal verb.
§3 had stated that both addons pin `rivetos-addon` by git tag
`v0.1.95` and that neither holds a path to this tree; the reading-source addon has
since moved to a sibling path. §7–§9 were added because `.rulings/` R-251 made a
release a copy of the tree, which turns what the tree contains into the
blocker.

**2026-09-20.** Items 5 and 6 were added, from the board work in which both were
met.

**2026-09-13.** Created from the feature-set plan's release-blocker list
(`PLAN-Feature-Set-2026-09-12.md` §9.2), which had the findings scattered across
it.
