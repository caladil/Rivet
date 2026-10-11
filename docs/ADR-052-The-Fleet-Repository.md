# ADR-052: The Fleet Repository — one place artifacts and desired state live, and deployment selects from it

* **Status:** Accepted. **§9 steps 1 through 3 are built, §7.2 — the
  machine's artifact store, and an install that names a digest — is built, and
  so is §7's workstation half: `deploy` publishes into the repository before it
  sends.** Step 1: the repository's layout, the §4 refusal, `cargo
  rivet fleet init|publish|list` and `deploy --artifact`. Step 2:
  `rivet-provision` records a composed document under the repository and
  `rivet-fleet` projects its register there. Step 3: `cargo rivet fleet drift`
  compares what a machine is running against the manifest, read-only. §7.2: a
  machine keeps a digest-keyed store of the artifacts it holds, and the form
  of `apply install` that named a filesystem path no longer exists. **§9's
  own exception is also built**: card images join the repository as a `card`
  kind (`cargo rivet image`, zstd-compressed, `R-177`/`R-177.1`), and a system
  update now publishes too, against `rivet-system`, ending the exception §2
  and §7 used to name (`R-177` D6). **§9 step 4 is built** (`R-184`): a board
  serves its repositories over the control plane, a fleet server's document
  declares `[repos.fleet]`, and `cargo rivet fleet push` puts the repository's
  text there, and `cargo rivet fleet sync` sends the application objects it
  names into `rivet-fleetd`'s own store (§7.4) — each proven on a QEMU guest,
  not on a board. §9 step 5 remains; §10 says what it is blocked on.
* **Date:** 2026-09-09, amended 2026-09-12, 2026-09-13 (`R-177`: card images
  and system updates join the repository), 2026-09-14 (`R-182`/`R-184`: §8.1,
  §9 step 4 and §7.4), 2026-10-03 (`R-233`: §7.6)
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Ruled:** by the maintainer, 2026-09-09, in conversation (`.rulings/` R-307).
  The shape is his: all desired state and all system and software artifacts
  belong in the fleet repository, which is the single source of truth, and the
  way `.rvt` files get into it flips, so that artifacts are placed into the
  repository and then deployed from the repository to devices. So is the
  correction that followed — that a git repository has no authority to argue
  about, and that a default location for storing secret keys is an absurdity.
  **§7.2 is his C1 ruling of 2026-09-12**: deploy publishes then sends, the
  machine installs by digest from its own store, and the path form of install
  stops existing.
* **Answers:** *"what exactly is on `rivet-01`, and can I put the identical
  thing on another machine?"*, which before this had no answer.
* **Supersedes nothing**, and it deliberately does not change `ADR-028`'s
  signing, `ADR-015`'s manifest, or `ADR-035`'s control plane. It gives them a
  place to keep what they produce.

---

## 1. Context

**The facts about a fleet live in four places today, and one of them is a
cache.**

| what | where | what happens if it is lost |
| :--- | :--- | :--- |
| the authority, the identities, the signing seeds | `~/.rivet/` | the fleet is unrecoverable |
| the register | `%APPDATA%\rivet-fleet\register` | the fleet forgets its own machines |
| images and `.rvt` payloads | `out/`, inside a build tree | `cargo clean` removes them |
| what is *actually* installed | the boards | nothing else knows |

Nothing joins them. The provisioning documents composed by `rivet-provision`
are in a fifth place, `~/.rivet/provision/`, and the documents actually running
are on the machines, where they have been hand-edited — `rivet-01` carries five
edits made on 2026-09-07 after its card had booted, and no record says who made
them or why.

### 1.1 The symptom that names the problem

`cargo rivet deploy` builds an application and pushes it in one act. What lands
is whatever the tree was at that instant. Applications installed on
`rivet-01` on 2026-09-09 are stamped, for example:

    tempmon-rivet-0.1.0-gbb80b9d-dirty-6cf29875
    reliquary-rivet-0.1.1-g598efb9-dirty-c563f94f

**Those digests exist because the build was ambiguous and the tool knew it.**
`ADR-027` §3.2.2 hashes the whole payload precisely so two builds of two
different dirty trees cannot share a name. That is a correct answer to a
question that should not have to be asked, and it is a workaround for the
absence of this ADR.

---

## 2. Decision

**A fleet repository holds the desired state and the artifacts, and deployment
selects from it rather than producing into it.**

Three parts, and they are separable:

1. **The text** — machine documents, the register, and the manifest of which
   artifact each machine should run — kept in **git**, for history and
   attribution.
2. **The artifacts** — `.rvt` payloads and card images — kept in a
   **digest-keyed object store** beside the text, referenced from it by digest.
3. **Deployment** — `cargo rivet deploy` **selects** an artifact from the
   repository. **Nothing reaches a device without having passed through it,
   with no exception** — a system update publishes too, verified against the
   `rivet-system` seed rather than the application one (§7; `R-177` D6). The
   board's own door still verifies every update on arrival regardless of
   where it came from.

**The third part has two ends**, and §7.2 is the one on the machine: a device
keeps its own digest-keyed store of the artifacts it holds, and an install names
a digest in it. Without that end the sentence above is a convention a tool
follows; with it, naming something the store does not hold is not a form the
command has.

---

## 3. Why the artifacts are not in git

**A `.rvt` is a signed, compressed archive.** An application is a few megabytes;
a system update is about 700 MB. None of it delta-compresses, because it is
already compressed and because a signature over changed bytes is entirely
different bytes.

A literal git repository accumulating system updates is unusable within months
and unclonable well before that, and the cost is paid by exactly the machine
that can least afford it — a fleet server on an SD card.

**So: git for the text, `objects/<digest>` for the artifacts, and the text names
artifacts by digest.** The reference already exists — every `.rvt` carries a
BLAKE3 digest and `cargo rivet deploy` already checks it against the board's
copy. It simply has nowhere to point.

---

## 4. Why the secrets are NOT in the repository, and this is a refusal

**`~/.rivet/` holds `keys/rivet-system.seed`, `keys/rivet-app.seed`, and a
`device.seed` in every identity directory.** If the fleet repository were
`~/.rivet/` itself, and it were a git repository with a remote, then one `git
push` publishes the key that signs this operating system.

The repository is therefore a subtree — `~/.rivet/fleet/` — and **the tooling
refuses to initialise or commit a tree containing a seed.** A `.gitignore` is a
convention and this is a check, because the failure is unrecoverable and
silent: a published signing key cannot be unpublished, and every board that
trusts its public half is compromised at once.

### 4.1 The authority is backed up by hand, to a directory named on the day

**Ruled 2026-09-09:** a default location for storing secret keys is an absurd
idea, and the operator must specify a directory. *(`.rulings/` R-307.)*

`rivet-provision ca backup <directory>` and `ca restore <directory>` take a
directory and have no default, for the reason `cargo rivet backup --into` and
Fleet Monitor's `backups.folder` have none: a tool that invents a destination is
one whose output is somewhere the operator did not choose, and they find out
when they need it.

**Backing the authority up is a step in the mint instructions**, before the
step that destroys the old one. A copy of the authority is a copy of the
fleet's whole trust, and that makes the backup folder the most sensitive thing
on the workstation. It is said out loud rather than arranged quietly.

**The repository's own backup is built — §7.3.** The argument this section
used to make for it (*"there is an argument for a copy of the repository
living in the backup folder as well, and it is a good one"*) is now
`cargo rivet fleet objects backup`/`restore`, for the object store; the text
half is backed up by ordinary git, needing no Rivet-specific tool.

---

## 5. Documents are not signed, and that is a decision

**Signing buys something only when the store cannot be trusted.**

An artifact crosses a real trust boundary: built on one machine, fetched from
anywhere, and **executed** on a board that must verify it without trusting
whoever handed it over. `ADR-028`'s signing is load-bearing there.

A document is not in that position. It is applied by a machine that has already
authenticated whoever handed it over, over `ADR-035`'s mutual TLS, against the
fleet's own authority. What is actually missing is **attribution** — who changed
this and when — and git supplies that with no cryptography at all.

**The gate on who may push is access control, not a signature**, and it is where
`ADR-034`'s named identities land: an operator, an agent and Fleet Monitor
should not all be able to rewrite a machine's desired state. Fleet Monitor reads
a register and pulls backups; it has no business pushing a document.

**One place signing would not help even if it were added**, recorded so nobody
reaches for it there: the document on a card's FAT boot partition. Anybody
holding the card can rewrite it, and the machine would need the verifying key on
the same card. Physical possession is the boundary and cryptography does not
move it.

---

## 6. Git has no authority to argue about, and that is the point

**Ruled 2026-09-09**, correcting an earlier framing in this project's own
conversation: in a git repository, which copy counts as the authority is a
matter of semantics. *(`.rulings/` R-307.)*

It is, and the reason is sharper than distribution. **The signature carries the
authority, not the repository.** An artifact is installable because a seed
`ADR-028` names signed it and the board's trust store carries that key. A member
holding a complete clone with push rights still cannot forge an artifact any
machine will install.

So there is no need to nominate a canonical holder. The repository starts on the
workstation because that is where artifacts are built and signed and where the
interview that composes documents runs, and it is pushed outward from there.
Which clone somebody calls authoritative is a convention, and nothing depends on
it.

### 6.1 What a member should hold

**The text, and only the artifacts its own documents name.**

A member with the text half knows its own desired state and can recover without
reaching the fleet server, which is worth having. A member holding every
artifact ever built is carrying gigabytes it will never run, on an SD card.

---

## 7. What deployment becomes

**Installing selects; it does not produce.** The scripted verb is `cargo rivet
install`; `cargo rivet deploy` is the interview that drives the same sequence
for a person, and takes no arguments (`ADR-055` §3.5).

    cargo rivet install <package options> builds, signs, and puts it in the repository, then sends it
    cargo rivet fleet publish FILE        puts an .rvt or an .apk in the repository
    cargo rivet install --artifact <id>   sends a repository artifact to a machine

**Handed a crate, `install` publishes first and then installs**, so the
one-step flow survives. The invariant worth defending is not that a person
types two commands — it is that **nothing reaches a device without passing
through the repository.**

**The repository is the path.** What the maintainer types did not change;
what happens underneath it did:

| typed | what happens |
| :-- | :-- |
| `cargo rivet install <package options> … confirm` | build and sign, **publish** into the repository, send the repository's copy, install by digest |
| `cargo rivet install --rvt FILE … confirm` | **publish** `FILE`, then the same path |
| `cargo rivet install --artifact ID … confirm` | send what the repository already holds — the explicit spelling of the default's second half |
| `cargo rivet fleet publish FILE --trusted-key PATH` | publish without sending |
| `cargo rivet fleet publish FILE.apk` | read, check, wrap and sign an APK as kind `apk`, then publish it (`ADR-061` §4) |
| `cargo rivet fleet publish FILE … --sync` | publish, then send the fleet server every object it lacks, then push `main` (`ADR-061` §4.4) |

**A publish commits `manifest.toml` and nothing else.** Whatever else the index
holds stays uncommitted, so a publish never records a change somebody staged
for another reason.

**`--sync` is one flow for every kind** (`.rulings/` R-222): files first, then
the list, to the register's fleet server, carrying every committed record. That
is safe because nothing on the server applies a pushed document (§7.4.4 reads
only the manifest, and `ADR-044` forbids its poller this repository).
Uncommitted files never travel, so they do not block it; it names them.

The repository is `~/.rivet/fleet` unless `--at DIR` names another, and a deploy
initialises an absent one first, because a first deploy is still one command. `fleet publish` sits under `fleet`
rather than bare, alongside `fleet init` and `fleet list` (`crate::fleet`'s
module documentation argues the name).

**The automatic publish verifies against a signing seed — which one depends on
the artifact's kind.** An application or a library verifies against the
application seed, reached the way `cargo rivet package` reaches it —
`--signing-key` when a build names one, then `$RIVET_APP_SEED`, then
`~/.rivet/keys/rivet-app.seed` (`.rulings/` R-155 R3). **A system update
verifies against the SYSTEM seed instead** — `$RIVET_SYSTEM_SEED`, then
`~/.rivet/keys/rivet-system.seed`, `cargo rivet image`'s own lookup
(`R-177` D6). §4 is why either is a lookup and not a setting: the repository
holds no seed and no path to one. A deploy that finds no seed is refused
before anything is made, published or sent, and the refusal names both places
it looked. Publishing an artifact the repository already holds is the
idempotent add, not an error, so re-deploying a build goes through.

**Without `confirm` nothing is published.** A plan changes nothing on either
machine, and the repository is on one of them.

**A system update publishes exactly like everything else — `R-177` D6 ends
the exception this ADR used to name here.** The header's unauthenticated kind
field is read for one decision alone — which seed's public half a publish
verifies against — never to decide whether to publish at all: a file that
falsely claims to be an update is checked against the system seed's public
key, fails, and is refused exactly as any other wrongly-signed artifact is.
So the invariant in §2 now holds without exception, and a system update
reaches a machine's door (§7.2.3) having already passed through the
repository, which verifies it against the machine's own trust set a second
time — `ADR-052` §7.2.3.2's own two reasons for that second check apply to an
update exactly as they do to everything else.

### 7.1 What it earns beyond versioning

**Drift.** Once the repository says what each machine should be running, `get
status` can be compared against it, and *"`rivet-01` is running an artifact that
is not in the repository"* becomes a question with an answer. That is what makes
this worth building rather than merely tidier, and it is the same shape
`ADR-045` gave health: a fact nobody could observe becomes one somebody can.

### 7.2 The machine's artifact store, and an install that names a digest

**Ruled 2026-09-12. `rivet apply install` takes a BLAKE3 digest and there is no
form of it that names a filesystem path.**

This is the half of §2's third part that lives on the *machine*. §7 makes
deployment select from the repository; that is worth nothing on its own, because
a machine that can be handed a path can always be handed a path to something the
repository never saw. Six of the seven ways software reached a machine bypassed
the repository, and `--artifact` was the only one that did not.

**There is nothing to enforce, because the capability was removed.** The
operation resolves a digest in the machine's own store. Nothing checks that a
caller "went through the repository": there is simply no argument in which a
path can be written.

#### 7.2.1 The store

A digest-keyed object store under `/state`, holding what the machine installed
and what is one step from being installed.

    /state/artifacts/
      objects/<digest>/<ADR-027 name>.rvt      a stored artifact
      incoming/<digest>/<ADR-027 name>.rvt     a transfer that has not finished

**One directory per object, holding one file**, and two decisions are in that.
The digest is the *directory* so that an object arrives by one `rename(2)` of a
directory that is already complete — nothing that resolves a digest can see a
partial artifact, or an artifact whose name has not landed yet. And the
`ADR-027` name is *kept*, because §3.2.1 of that record makes a payload's name a
claim about the commit that built it, and a store that threw it away would leave
the installer describing an update by hash.

**The transfer fills it.** `ADR-035` §3.2's bulk endpoint already required a
`Rivet-Blake3` header and already hashed the body as it arrived; it now uses that
digest as the key. This is the sole place the two halves meet: the digest is
stated by the sender, verified against the body, and then becomes the name — so
bytes cannot be stored under a name that is not their hash, and an install
naming a digest gets the bytes whose hash it is.

#### 7.2.2 Retention — the three newest of each product

Rolling back becomes **local** rather than a trip to the workstation, and growth
is bounded at three times the installed footprint rather than being unbounded.
`R-135` predicted this rule would be needed and explicitly did not make it;
eighty-six payloads had to be removed from a build directory by hand first.

**"Newest" is the highest VERSION**, because versions of one product are what
retention chooses between and `ADR-027` §7's name already carries one. The
comparison is componentwise over the numeric core, so `0.3.10` is newer than
`0.3.9`, which every string comparison gets backwards.

**Only the numeric core is ordered.** `ADR-027` §3.2 appends `-g<short>`,
`-dirty` and §3.2.2's payload digest as **build metadata** — dash-separated
because `+` is a nuisance in a URL and on a FAT partition, not because those
tokens are SemVer pre-releases. Two dirty builds of one commit have no order,
and neither has a tag against a commit carrying the same crate version; reading
`-g3d28ede` as a pre-release would order a described build *below* the bare tag,
which is backwards. So the metadata is discarded and two equal cores fall
through to arrival.

**Arrival is the tie-break and only the tie-break.** It is the object
directory's modification time, which `ADR-021` makes sufficient for that and no
more: a Pi's clock is a floor that never goes backwards, so it orders arrivals
within a machine's life and says nothing about a date. Ties in the modification
time are broken by digest, so one answer comes out rather than whichever the
filesystem listed first.

**A version this build cannot order is retained APART from those it can.** To
reach that case a name must have parsed as `ADR-027` §7's grammar and then
carried a version whose core is not dot-separated numbers. A packaged
application's version is composed from a crate version, which always reads; an
update's is typed by the operator, so `0.1.84rc1` on a system update is how one
realistically arrives. Unreadable versions keep their own three, ordered by
arrival — the only order they have — and are never counted against the readable
versions' three. Both simpler answers fail the store's purpose:

| Unreadable versions ordered… | What goes wrong |
| :-- | :-- |
| below every readable version | *deleted first* becomes the silent answer to *not understood* |
| above every readable version | they pin: the arrival outranks them, so each later deploy evicts the lowest **readable** version, until the store holds the running version, the unreadable ones and nothing to roll back to — every eviction reported by name and looking orderly |
| **apart, three each** | neither kind costs the other a place; the cost is that one product can occupy up to six slots rather than three, and only when three or more unreadable versions of it reached one machine |

**A genuine stray high version holds its place, and that is the cost of ordering
by version.** An artifact signed by nobody this machine trusts can no longer be
one — §7.2.3 refuses it before it is named. What remains is an *authentic*
artifact numbered above the release line: a test build `0.9.9`, signed with the
real key, for a product shipping `0.1.x`. It keeps one of the product's three
until something higher arrives, leaving one rollback target rather than two. The
store verifies that an artifact is authentic, not that it is the build an
operator meant, and the rule accepts that rather than guessing.
`tools/qemu/adr-052-proof.sh` sends an *untrusted* `0.9.9` among `0.1.1` to
`0.1.4` and asserts it holds no slot; `rivet-core`'s retention tests replay that
sequence without it, and separately pin the cost for a genuine `0.9.9`.

**The artifact that has just arrived is never the one dropped.** Four artifacts
written inside one clock tick share a timestamp, so a tie-break would otherwise
sometimes evict exactly the bytes a deploy had just transferred — leaving it to
ask for a digest the store no longer held. The transfer names its own arrival to
the retention rule, and that object is ordered ahead of its product's group.
Ordering by version narrows the case and does not close it: sending an old build
back to a board is precisely when the arrival is also the lowest version.

#### 7.2.3 The store is a trust boundary: nothing enters it unverified

**Ruled by the maintainer, 2026-09-12:** one way or the other, the store should
not have garbage in it (`.rulings/` R-308). He offered two shapes — the store calls the
installer's own check, or the check is abstracted into the core — and ruled the
second, so the store and the installer share one implementation.

**The bulk endpoint verifies at the door.** Before the `rename(2)` that names an
object, a transfer passes three gates, in this order:

1. **The digest** — the body's BLAKE3 against the `Rivet-Blake3` header, as it
   arrives. A corrupted or truncated transfer is refused here and **no signature
   work is done**: the refusal is about the wire, and saying so is what stops an
   operator being sent to examine a key that is fine.
2. **The artifact** — `rivet_core::package::verify::artifact`, against the trust
   store in `/system`: the header signed by a key the store holds, that key
   entitled to sign this kind (`ADR-028` §5.5), and every member's bytes the ones
   the signature covers. `cmd/rivet-mcpd` runs as uid 0 unconfined and already
   links `rivetos-crypto`, so it reads `/system/trust` exactly as the installer
   does — on every admission, never cached, for `ADR-028` §5.5.4's reason.
3. **The architecture** — `rivet_core::package::verify::architecture`, for every
   kind and only after the signature has verified: the header's `<arch>` against
   the token the machine's **image** recorded at `/system/arch`
   (`rivet_core::package::arch`). An artifact this machine cannot execute is
   garbage under the same ruling: it would install and fail when it ran
   (`PLAN-Feature-Set-2026-09-12` §3.3g). The refusal is a 422 naming both
   architectures.

An artifact that fails any gate is removed from `incoming/` and **never has a
name in `objects/`**, so it never occupies a retention slot and retention needs no
special case: everything §7.2.2 sorts is something this machine would install.

**The token is the image's and never the compiler's.** `std::env::consts::ARCH`
answers `arm` on the `armv6` board `ADR-027` §3.1 names, so a comparison against
it would refuse every application there, and `arm` cannot be mapped back to a
token because it is armv6 and armv7 alike. The card build chose the target
triple every binary was compiled for, so `cmd/rivet-image --arch` — required,
and refused unless it is one of §3.1's tokens — writes the token beside the
trust store. A machine whose record cannot be read admits nothing, as the
machine's fault (a 500), for the trust store's reason: a record that could not
be read is not a record that names an architecture.

**The architecture belongs at the door; the rest of applicability does not.**
The trust store and the architecture record are both files in `/system`, fixed
for the life of the image, so a verdict on either is as true at the door as at
install. Whether a slot will take a system update, whether an application's
closure is satisfied, whether a version directory is occupied, whether the
artifact is already what the machine is running — each depends on state that
changes between the transfer and the install, or on a boot port only the
installer links, and each is refused by the installer with nothing written. A
system update's product and platform stay the installer's too: the product is a
constant only the installer is built against and the platform is the boot port's
answer. **The installer asks the architecture again**, with the same function,
for §7.2.3.2's reasons.

**There is no `--trusted-key` at the door.** The installer's override exists for a
card built `--no-trust` and is given on its own command line; the supervised
install path never passes one, so an artifact only an override would verify is
one the store could never lead to an install of. A `--no-trust` machine admits
nothing, and its refusal says so.

##### 7.2.3.1 Why this is one verifier and not two

`ADR-029` §11's rule is *"three formats means three verifiers, each with a third
of the scrutiny"*, and `ADR-006` §8.8.2 applies it: the weaker of two verifiers is
the one that eventually gets believed. **The rule is about implementations.** Two
copies of the sequence — read the header, build the trust set, stream every
member — could disagree: one skips the members, one maps a short read to success,
one narrows a key's mask. One copy consulted at two points cannot disagree with
itself. So the sequence was lifted out of `cmd/rivet-deploy` into
`rivet_core::package::verify`, and the door and the installer both call it.

**This is not new in kind.** `cmd/rivet-sysd` has consulted `read::open` at every
launch since 2026-08-25 (`ADR-015` §4.2), a second point of consultation in a
second process, and `ADR-028` §5.5.4's revocation is built on it.

##### 7.2.3.2 The installer keeps its check

Two reasons, and either would do:

* **A verdict is true of the trust store it was reached against.** Revocation is
  an image update that replaces `/system/trust`; an object admitted under the old
  store may not verify under the new one.
* **The store is not the only way bytes reach `/state`.** The boundary against
  uid 0 is unchanged: anything running as root can write under `objects/` without
  the endpoint, exactly as it can write the request file. What the install
  request lost on this date is narrower and worth naming: it used to be a general
  *"run the installer against this path"* channel, and it can now point only at
  an object in the store.

##### 7.2.3.3 Why the store is a trust boundary now, having been ruled not one

This section said the opposite earlier on 2026-09-12, and the record of it is in
History. **The store was not a trust boundary only because unverified bytes
were allowed into it.** That was a property of the endpoint, not of stores, and
the correction was right about the endpoint as it stood: the digest is a
statement about the wire, and calling a store of wire-checked bytes trusted would
have been claiming a check nobody made. The remedy was to make the check, not to
keep the disclaimer. Once nothing enters unverified, everything in the store has
passed the machine's own signature check against the trust store in force when it
arrived — within the two limits in §7.2.3.2.

##### 7.2.3.4 What a refusal looks like, and what it costs

**A 422 for the artifact's fault, a 500 for the machine's** — a trust store
missing, corrupt or empty, or bytes that would not read back. The 422 says the
transfer was intact and names the digest the machine hashed; `cmd/rivet-mcpd`
writes the reason to its own log, and `rivet_remote::tls::transfer` reports *the
machine REFUSED THE ARTIFACT* and *not a transfer failure*. That wording is
§7.2.8's defect turned around: there, a success was reported as a failure; the
risk here was a refusal about the artifact reported as a wire fault, and an
operator retrying something refused identically every time. `cargo rivet deploy`
stops at a refused push and asks for no install. A `201` relays a `verified` line
naming the key that signed, because only the machine can say it checked.

**On a 700 MB system update the door costs one more read of the bytes just
written and one more BLAKE3 pass**, after the transfer and before the `201`. The
body is held in `incoming/<digest>/` while it happens, where nothing that resolves
a digest looks, and nothing is held in memory beyond one 64 KiB buffer and the
4 KiB header. **It is not measured on a board**; `ADR-029` §7.2's BLAKE3
measurement is still owed. It is not folded into the streaming hash because member
boundaries live in the header, and reading them before the signature has checked
out would put a parser upstream of the verifier (`ADR-029` §5).

##### 7.2.3.5 A store written before the door, and why nothing sweeps it

A store filled by a build from before 2026-09-12 can hold objects nothing
verified. **No sweep runs over it**, for three reasons:

1. **It is permanent code for a state that exists once.** The one real store is
   `rivet-01`'s, and `PLAN-Feature-Set-2026-09-12` §3.3d records what it holds:
   artifacts each deployed and then installed, which the installer does not do
   for one that fails verification. A QEMU guest's store is tmpfs. §3.3a refused a
   compatibility shim on the same grounds.
2. **It costs a read and a hash of every object**, up to three 700 MB updates, on
   every start of the daemon that is a headless board's only way in — or a marker
   saying it has run, which is state that can disagree with the store.
3. **It deletes unattended**, and the moment a sweep is most likely to be wrong is
   the moment a trust store is being replaced.

**What happens instead costs nothing until it matters.** A copy the store already
holds goes through the same door before a re-send is answered `201`, so the
machine never vouches for bytes it has not verified. A held artifact that fails is
removed; one the machine cannot judge — its trust store unreadable — is left. An
object nobody sends again is one nobody installs, and if somebody asks, the
installer refuses it.

**The fleet repository holds card images too, as a `card`-kind `.rvt`** —
`cargo rivet image` (`R-177`), wrapping an already-built card image exactly as
`cargo rivet update` wraps a system image: through the same signature check
and the same kind of manifest entry, never a second verifier or a second
index (`ADR-029` §11). §2's third part covers it without amendment. **A card
image is never sent to a board and never reaches this section's own
`/state/artifacts` store** — it is decompressed once, on the workstation,
when provisioning writes it — so nothing here changes: this section is still
about a device's own store, which holds no card images because nothing ever
sends one there.

#### 7.2.4 What an operator types, which is unchanged

`cargo rivet deploy` already computed the payload's digest — it had to, for the
transfer check — so it asks for the install by the digest it just sent. Nothing
about the command line moved.

The one casualty is `--remote-dir`, and it is **refused rather than ignored**:
there is no longer a directory for it to name, and a flag that was accepted and
quietly did nothing would be worse than one that is gone.

The plan an operator confirms is better than it was, because the machine can now
answer a question it could not before. `apply install` reports **what it would
install** — the artifact's own name, read back from the store — since sixty-four
hex characters say nothing about what is in the file.

#### 7.2.5 What a run of `tools/qemu/adr-052-proof.sh` settles, and what it does not

**That script is the demonstration of this section**, and it is re-runnable
rather than a report. It boots an `aarch64` guest on `ADR-024`'s own kernel
under QEMU `-M virt` with `cmd/rivet-deploy` packed as the installer and a trust
store carrying `keys/rivet-app.pub`, packages seven real `.rvt` files with `cargo
rivet package` — six sealed with the application seed, one of them with an
`armv6` header, and one with a key the guest does not hold — and sends every one
of them **over `ADR-035` §3.2's bulk
endpoint**, whose door verifies each before naming it in the store under the
digest the endpoint computed, then asks for `rivet apply install <digest>
confirm` over mutual TLS for each one admitted.

What it asserts, each against the machine's own records or the machine's own
answer to `get file`, never against what the client printed:

* the supervisor resolves the digest and hands the installer
  `/state/artifacts/objects/<digest>/<name>.rvt`;
* `cmd/rivet-deploy` records *"the payload's header is signed by a key this
  machine trusts"* and *"every member's bytes are the ones the signature
  covers"*, then installs the application — **this is where a signature is
  verified out of the store, and it is the thing the store exists to lead to**;
* the supervisor adopts it and `hello-rivet`'s own stdout reaches the console;
* the artifact signed by an untrusted key is **refused at the door** (§7.2.3):
  the deploy exits non-zero saying the machine *REFUSED THE ARTIFACT* and that
  this is *not a transfer failure*, with the verifier's reason and the digest the
  machine hashed; `rivet-mcpd`'s own log records *REFUSED AT THE DOOR*; the digest
  is in neither `objects/` nor `incoming/`; no install is requested; and no
  version directory of it exists under `/state/app`;
* the trusted artifact whose header says `armv6` is **refused at the door** against
  the `aarch64` the guest's `/system/arch` records, naming both, and is neither
  stored nor installed — the program inside it is hello-rivet's aarch64 build,
  because the door refuses the header's claim and never looks at the bytes;
* four trusted artifacts of one product arrive, **the three highest are held**
  and the refused `0.9.9` holds none of them, one of another product is
  untouched, and the newest arrival is among the survivors — §7.2.2's rule that
  an arrival is never what falls off, exercised on a machine rather than in a
  unit test.

**Three things it does not settle, and they are not small.**

1. **`/state` there is the initramfs tmpfs, not ext4 on a card.** `tools/qemu/run.sh
   --card` gives a guest a real GPT and a real ext4 and **refuses `--document`,
   `--secrets` and `--state` with it**, because PID 1 mounts the card over the
   directory all three write into — so a machine with a card cannot hold a
   control-plane identity and cannot receive a transfer at all. No single run of
   this harness can have both a real filesystem and a real transfer. The
   syscalls the store makes are real; the medium is not.
2. **QEMU is not a board** (`ADR-026` §4). Nothing here says anything about SD
   timing, wear or the firmware.
3. **Every artifact is an application.** A system update in the store, and the
   slot write that follows it, is untouched by this.

#### 7.2.6 What the transfer's answer says, and who relays it

**Retention announces an eviction in the `201` body and nowhere else, so a
client that drops that body drops the announcement.** It reaches the operator
now. `cmd/rivet-mcpd`'s bulk endpoint
composes a `pruned <names>` line; `rivet_remote::deploy::Remote::push` answers
with that body rather than discarding it, and `run` relays the lines that carry
something the tool does not already know — `pruned`, and the transport's own
`note`. The rest of the body repeats what the deploy has already printed and is
not echoed.

The selection is by first word rather than by a parse, because the body is a
report for a person: a line a newer machine invents is dropped until this list
names it, which is the failure that loses information rather than the one that
invents it.

#### 7.2.7 One install request at a time, and the tool waits for it

`rivet_core::deploy`'s request file is **one slot**, and `rivet apply install`
refuses a second request while one is still waiting. That refusal is right —
two installers would be two writers on one inactive slot — and `cargo rivet
deploy` now waits it out: it asks again, backing off from 250 ms to 5 s, for up
to two minutes, and says in the report what it is waiting for and that the
artifact is already transferred. Deploying several versions back to back works.

**The refusal is recognised by one constant**,
`rivet_core::deploy::REQUEST_WAITING`, stated by the machine that composes the
sentence and matched by the tool that waits on it. **Every other refusal from
that operation is returned at once**, without a pause: a machine with no
`/state`, an unreadable request file, or a digest the store does not hold will
be in the same state in two minutes, and retrying turns a clear error into a
long one.

**It retries the request rather than polling `get install`**, and the reason is
that the blocking state is not one `get install` reports. That operation reads
the installer's *journal*; the window that loses a deploy is between a request
being written and the supervisor taking it up, where the journal's latest
install is still the previous one and reads as `finished: true`. A poll there
answers *go ahead* and the request is refused anyway. Asking for the install is
the only question whose answer is about the slot.

A slot that never clears is a failure that names what was waited for, for how
long, and that **nothing needs re-sending** — the artifact is in the machine's
store and `rivet apply install <digest> confirm` asks for exactly what the
deploy would have asked for.

#### 7.2.8 A machine that answers early reads the body out

**The bulk endpoint drains a request body it has decided not to keep**, and the
one case where it decides that is a re-send of an artifact the store already
holds.

The short-circuit is kept: the bytes are not written, hashed or `fsync`ed a
second time, which is where the real cost of a re-send lies. What it no longer
does is hang up mid-sentence. **A socket closed while bytes are still arriving
sends a `RST` rather than finishing the `FIN` exchange, and a reset discards the
peer's receive queue — including the answer this machine has just written.**

Found on `rivet-01`, 2026-09-12, the first hardware push of the new client. The
client reported *"the transfer failed after 196608 of 402198 bytes: An
established connection was aborted"* — the identical offset on two runs, three
64 KiB buffers in — while the machine logged `201 Created` for every one of
three pushes and held the artifact complete at 402198 bytes, with nothing
stranded in `incoming`. The transfer had worked; the operator was shown a
failure and the install it had earned was never asked for. **A working path that
looks broken is worse than a broken one.**

**The client also reads the answer after a failed write, and that is a defence
rather than the fix.** A write failure is not a verdict on a transfer — the
machine is the party that knows — so `rivet_remote::tls::transfer` asks for
an answer either way and believes a `201` it gets, relaying it with a note. What
it cannot do is recover this case: measured over loopback on a Windows host,
2026-09-12, a peer that answered and closed without draining left the client's
read returning **zero bytes and the same abort**, twice out of two, while the
same peer draining first gave a clean send and a readable `201`. The consequence
is worth stating rather than implying: **a machine running a build from before
this change keeps reporting the failure until it is updated.**

`Expect: 100-continue` (RFC 9110 §10.1.1) would save the transfer rather than
merely make it harmless, and is **not** taken here. It is a negotiation — both
ends must know it, and a client waiting for a `100` from a machine that will
never send one hangs — so it carries a version-crossing decision larger than
this defect earns. A drain is one-sided, which is the property that makes it
deployable on its own.

#### 7.2.9 An install of what the machine already runs stops before it is requested

**A deploy of the artifact a machine is already running transfers it and stops.**
The install channel refuses the request before writing it, the installer refuses
it if a request arrives anyway, and `cargo rivet deploy` reports that the machine
already runs the artifact, that no install was requested, and exits zero.

Found on `rivet-01`, 2026-09-12 (`PLAN-Feature-Set-2026-09-12` §3.3f): the
running `0.1.85` update was re-sent, the request was delivered, and the installer
wrote a second copy of the running build into the inactive slot — **replacing the
`0.1.84` rollback**. Nothing warned. For an application the occupied-directory
refusal (`ADR-015` §7.2) already stopped the write, and told the operator a
directory was in the way rather than that there was nothing to do.

**What "running" is, and where each answer is read.**

| Kind | Running | Identity read from |
| :-- | :-- | :-- |
| application | the version `current` selects | `/state/app/<name>/current/package-header`, retained by every install (`ADR-015` §4.2) |
| system update | the build the running slot holds | `/state/slots/<a\|b>/package-header`, retained by `cmd/rivet-deploy` for each slot it writes |

**The artifact's identity is its signed header.** The 4 KiB header carries every
member's offset, length and digest, and a file with bytes past its last member does
not open, so two artifacts with byte-identical headers are one artifact. The stored
object's first 4 KiB are compared with the retained headers **as bytes**: nothing
is parsed, so this is not a second reader of the container (`ADR-029` §11), and a
retained header is one the installer wrote only after it verified.

**A slot's header lives on `/state`, not in the slot.** Its system partition is the
image the header describes, and slot A's boot partition holds `autoboot.txt`, which
`ADR-029` §9.3 keeps an update from opening for writing whenever it can. `/state` is
never touched by an update (`ADR-018` §5.2). **The record is exactly as current as
the slot for every write a running machine makes**, because on a running machine
the installer is what writes a build into a slot — arming and committing rewrite
`autoboot.txt`, which belongs to no build. It empties the record before the slot's
first byte — so a
write that dies leaves a slot no record claims — and writes the header once the
system member and boot half are in. What it cannot see is a slot written from
outside Rivet; an imaged card has an empty `/state`, so such a slot has no record
and is not known to be running anything, which is how every card behaved before.
A record that cannot be emptied refuses the install with the slot untouched.

**The running slot is the one the firmware reports**, in both places — the reading
the installer computes its target from, so the channel and the installer can never
name different slots — and never a binary hash, which two slots holding one build
answer alike.

**Why both ends.** The installer's check is the one nothing bypasses: it runs
whatever wrote the request. But it answers in the journal, after the deploy has
already printed *install requested*, so the channel asks the same function —
`rivet_core::deploy::already_running` — before it writes a request, and its refusal
reaches the tool on the same call. The refusal carries
`rivet_core::deploy::ALREADY_RUNNING`, recognised by `rivet_remote::deploy` on
§7.2.7's pattern. **The tool does not ask "is it running?" first**: a question and
then a request is a check and an act with a window between them, and §7.2.7 already
chose the request as the only question whose answer is about the moment it is acted
on.

**The artifact is still transferred.** The store should hold what the machine runs —
it is the rollback target once something newer is installed — and the transfer is
what tells the tool the digest it then asks about.

**Everything unreadable is "not known to be running", never a refusal** at the
channel: an object whose head will not read, a slot with no record, an application
with no `current`. Each leaves the decision to the installer, which is what the
channel did before it asked anything.

**Proven by unit test and, for applications, in QEMU; the system-update half has
run only against a fake card.** QEMU has no slots (`ADR-026` §4), and a board's
slots gain records only from an install made by a build carrying this change.

---

### 7.3 Backing up the object store — `cargo rivet fleet objects backup`/`restore`

**Built 2026-09-14, closing the gap §4.1 named** (*"there is an argument for a
copy of the repository living in the backup folder … and it is a good one"*):

```text
cargo rivet fleet objects backup <backup-dir> [--all] [--replace] [--at DIR]
cargo rivet fleet objects restore <backup-dir> [--replace] [--at DIR]
```

**Default scope is the digests `manifest.toml` currently references, not
every digest `objects/` holds.** §6.1's rule for a *machine* member —
*"only the artifacts its own documents name … a member holding every
artifact ever built is carrying gigabytes it will never run"* — applies
identically to a backup of the *repository*: a backup answers *"what would let
this fleet recover as it stands today"*, and superseded `-dirty-` builds
nothing references any more (§7.2.2's own retention already prunes them from
the manifest, though not from `objects/` itself — §7.2.3.5) do not belong in
the answer by default. `--all` is the explicit escape hatch for a copy of the
whole historical tree.

**Restore recomputes every object's own BLAKE3 before writing anything**,
refusing — by name, nothing written — any backed-up object whose filename
does not match the digest of its own bytes. This is the workstation running
the same check §7.2.3 puts at a machine's own door, against a backup rather
than a wire transfer: a restored repository is never worse-verified than one
freshly received.

**Machine-independent by construction.** An object's identity is its own
digest, so a backup made on one workstation restores correctly onto a fleet
repository initialised on another — unlike §4.1's authority backup, which is
one fleet's whole trust and belongs nowhere but back where it came from.

**The engine is shared, not reinvented.** `cargo rivet fleet objects backup`
reuses the exact refuse-or-`--replace`, named-and-sized copy primitive
`rivet-provision ca backup`/`restore` already proved — hoisted into its own
crate, `rivet-filecopy`, at this its third consumer (`cargo rivet key
backup`/`restore` is the second). The workstation `objects/` directory turned
out to be flat — one file per artifact, named by its own BLAKE3, exactly as
§3 already states — so no recursive extension to that engine was needed;
only a second entry point, `copy_named`, for copying a *subset* of a
directory's files rather than all of it, which the default referenced-only
scope needed and `copy_all` had no way to express.

### 7.4 A fleet server's repository store — `cargo rivet fleet sync`

**Ruled 2026-09-14, `.rulings/` R-184 D1 and D2:** a fleet server's objects live
in `rivet-fleetd`'s own digest-keyed store inside its data directory, pruned by
this repository's rules, apart from the machine-wide `/state/artifacts`; and
only the operator pushes and fetches for now, with a member's fetch designed
later (§9 step 5) and nothing built here ruling it out. **R-182** put the route:
a server's card carries the repository's text, and its objects *"follow over
the control plane after first boot, as system updates do"*.

#### 7.4.1 The store, and who writes it

    /state/app/rivet-fleetd/data/repository-objects/
      objects/<digest>/<ADR-027 name>.rvt      an object this server keeps
      incoming/<digest>/<ADR-027 name>.rvt     a transfer that has not finished

**§7.2.1's layout and discipline, in another place**
(`rivet_core::repository_objects::store`): every path into it is built from a
checked `rivet_core::artifact::Digest`, the file is `fsync`ed and its directory renamed into `objects/`
only once complete.

**Only the object files are handed to `rivet-fleetd`; the store's directories
stay root's.** The root, `objects/`, `incoming/` and every `<digest>/` are
uid 0, mode `0750`, group `rivet-fleetd`'s, and that group is the application's
read grant; each object file is `rivet-fleetd`'s. The reason is the direction
of the writes: `rivet-mcpd` writes into these directories as root, and an
application that owned one could plant a link in it and send root's next write
or `chown` wherever the link pointed. A directory it can only read gives it
nowhere to plant one. Every pass sets every entry, so a store an earlier build
handed over whole is put back to this shape by the next `PUT`.

**Every write into the store goes through a directory held open, never a
path.** The store's root sits in `rivet-fleetd`'s own data directory, which that
application can rearrange, so a path checked and then used is two lookups with
a window between them. `rivet-mcpd` opens the root once, below the data
directory, without following a link, and makes, creates, renames, removes and
re-owns every entry beneath it relative to that descriptor (`openat`,
`mkdirat`, `renameat`, `unlinkat`, `fchownat` without following). Renaming the
root away and planting a link in its place redirects nothing already open. The
machine-wide `/state/artifacts` is written the same way, by the same code.
Reads — the listing, the door's verification, an object read back — still go
by path; a read through a planted link discloses nothing a signature check or
a digest does not already refuse, and the object read-back refuses a link
itself (§7.4.2).

**`cmd/rivet-mcpd` receives into it, over the control plane, and not
`rivet-fleetd` on its own listener.** The design this was built from proposed a
route on `rivet-fleetd`'s 4443; building it found that a confined application
can do neither of the store's two jobs:

| | `rivet-fleetd` on 4443 | `rivet-mcpd` on 7777 — **built** |
| :-- | :-- | :-- |
| verify against `/system/trust` and read `/system/arch` | **refused**: `rivet_core::confinement::policy` grants neither, and its `nothing_outside_the_declaration_is_granted` asserts `/system/trust` is refused | uid 0, unconfined; §7.2.3's door already reads both |
| read `/state/git/fleet.git`, and run `/system/bin/git` | **refused**: no grant on `/state/git` and no EXECUTE on `/system/bin` | already runs git for `ADR-035` §4 |
| who authenticates | a certificate the fleet authority issued that names no host — the operator's browser certificate, inferred from the leaf's shape (`ADR-043` §5.3) rather than a key this machine was told to expect | a key in this machine's `peers` |
| R-182's words | a second route beside the control plane | *"over the control plane"* |

Granting a confined application the trust store would change `ADR-042`'s
policy for every application, which is a decision of its own and not one this
store needs.

#### 7.4.2 The routes

    PUT  /repository-objects/<ADR-027 name>   Rivet-Blake3, Content-Length   one object
    GET  /repository-objects/                                               the listing
    GET  /repository-objects/<digest>                                       one object, read back
    POST /repository-objects/prune                                          remove what the manifest stops naming

`cmd/rivet-mcpd/src/repository.rs`, beside `/payload/` and git on the same
listener. The object read-back answers the object streamed, with its digest as
`Rivet-Blake3` and its name as `Rivet-Name`; it is how a workstation's
fetch-only clone takes a bookmarked fleet's objects (`ADR-063` §6). The listing is `rivet_core::repository_objects::Listing`: which commit
of `main` the manifest was read at, every digest it names, and every object
held. It is the completion signal: a store is complete when nothing named is
missing.

#### 7.4.3 What a transfer passes

§7.2.3's door, **asked for a mirror** (`TrustStoreDoor::mirror_on_this_machine`):

1. **the digest** — the body's BLAKE3 against the header, streamed to disk in
   64 KiB pieces, so an object's size costs the board no memory beyond one
   buffer and the 4 KiB header; bounded by the bulk endpoint's own ceiling;
2. **the signature** — `rivet_core::package::verify::artifact` against this
   machine's trust store, read on every admission;
3. **the kind** — every kind this build can open, each by that kind's own
   reader: an `application`, a `library`, an `apk`, a system `update` and a
   `card` (`ADR-063` §6, `rivet_core::repository_objects::holds_kind`). A fleet
   server is the authority for its fleet's artifacts, so it holds what a
   restore needs: a server that cannot hold a card cannot write one for a host
   it recovers, and one that cannot hold an update cannot bring a member
   current. An update is opened as a payload — one system member, every
   slot-variant file present for both slots — with the reserved boot names
   left to the installing machine's own door, as the architecture is; a card
   has no reader beyond the container and must carry its image. An `apk` is
   held so the `gantry` application can hand it to a phone over
   `rivet-fleetd`'s socket (`ADR-061` §6). **The trust store still decides**:
   a card is admitted only where this machine's trust store permits its signer
   to sign cards, so holding a kind widens nothing a machine trusts;
4. **no image service's name**, as at §7.2.3's door.

**The architecture is recorded and never refused.** The store holds what the
manifest names for the whole fleet, and a fleet can hold a Pi Zero (`armv6`)
beside a Pi 4 (`aarch64`); an object this server cannot execute is what a member
of that architecture will fetch, and refusing it would leave a mixed fleet's
manifest unsyncable. The answer says which architecture the object was built
for and whether this machine executes it; a member asks its own image's
question at its own door.

**Retention is the manifest's, and it keeps three per kind.** §7.2.2's rule is
about what a machine installed and is not applied here; this store prunes what
the manifest stops naming (§7.4.4), and the repository's own retention keeps
the three newest of each product, kind, platform and architecture — cards and
updates included — and every digest a host's record names (§7.6). So a fleet
server holds what the repository holds, without counting anything itself.

A re-send of an object already held short-circuits after its held copy passes
the door again, reading the body out first (§7.2.8).

#### 7.4.4 What decides a prune

**The `fleet` repository's `main:manifest.toml`, read on the server with git's
plumbing** — `rev-parse refs/heads/main`, then `show <commit>:manifest.toml` —
out of `/state/git/fleet.git`: the text `cargo rivet fleet push` last pushed.
Never the copy a card carried into `rivet-fleetd`'s data directory, which is
frozen on the day the card was written. A prune removes every held object that
manifest does not name, of any kind the store holds, and nothing else.

**Pruning never runs in the middle of a sync**, and three rules make it so:

* **the server refuses a prune while anything its manifest names is missing**,
  or while the manifest could not be read, because a rule decided from no
  manifest would remove everything (`Listing::prune`);
* **`fleet sync --prune` asks only in its `confirm` run, after that run read the
  store back complete**, with nothing missing, refused or failed;
* **and only when the server's `main` is this workstation's `main`**: the server
  prunes by the text it holds, so a prune decided while that text is behind
  would remove what the operator's newer manifest still names.

#### 7.4.5 Who may ask

**R-184 D2, as the control plane expresses it:** a caller holding a key in this
machine's `peers` — the operator's identity and the fleet server's own, which
provisioning writes — reads and writes; a connection that presented no pinned
key is refused. `rivet_core::repository_objects::permits` is that rule as a
table of caller and access, so §9 step 5's member fetch is one more caller and
one more row granting it `Read`. What step 5 still has to decide is how a
member's key reaches a server without the full control plane a `peers` entry
grants — the trust change §10 names.

#### 7.4.6 The command

```text
cargo rivet fleet sync <host> [--host HOST:PORT] [--identity DIR] [--pin HEX]
                              [--at DIR] [--prune] [confirm]
```

`<host>` resolves through the register as `fleet push` resolves it. The sync
reads this repository's manifest, asks for the listing, sends only what is not
held from `objects/<digest>`, and reads the listing again. **Resumable by
construction**: the server's listing is the state, so a rerun sends only what is
still missing. It reports each object as `sent`, `skipped` (already held),
`missing` (named here, absent from `objects/`), `refused` (with the door's
reason) or `failed`, and **exits non-zero unless the store holds everything the
manifest names**. Without `confirm` it prints `would send` — and, with
`--prune`, `would prune` from the server's own listing — and changes nothing.

#### 7.4.7 What is proven, and how

`rivet_core::repository_objects`'s tests hold the layout, the kinds, the
listing's round trip and the prune rule; `cmd/rivet-mcpd/src/repository_tests.rs`
drives the three routes with real signed artifacts, a real trust store and a
real git repository in a temporary directory; `cmd/cargo-rivet`'s
`fleet_sync_tests.rs` drives the sync against an in-memory store.
`tools/qemu/adr-052-objects-proof.sh` runs it on a booted guest: the text
pushed, two signed objects sent and listed back by `get file`, the store's
directories root's with `rivet-fleetd`'s group and its object files
`rivet-fleetd`'s, a rerun sending nothing, a tampered object refused at the
door, a signed system update admitted and held, an identity not in `peers`
refused, and `--prune` held back while the server's `main` was behind and then
removing exactly what the manifest does not name. `cmd/rivet-mcpd`'s
repository tests hold a card and an update admitted, a card from a key not
trusted for cards refused, and four cards pruned to the manifest's three.

**Unproven**: a board, a card's ext4 `/state`, `rivet-netd`'s network path,
objects of a realistic size over a real network and the time the door's second
read takes on one, and members fetching at all.

### 7.5 Taking a product out of circulation — delete and hide

`.rulings/` R-225. **A product leaves circulation from this repository, never
from a fleet.** A fleet's store is a copy of this repository and holds no
decision of its own. The armada is the one place an operator is authenticated
to decide what the estate offers (`ADR-059` §4.2), and a fleet server, or an
application on one, is not empowered to make that decision.

#### 7.5.1 Delete retires a product

**Deleting a product, or one version of it, removes its entries from
`manifest.toml` and its objects from `objects/`**, commits the manifest alone,
and reports what the fleets will lose at their next push, sync and prune.
**Nothing installed changes**: a machine running the product keeps running it,
and `fleet drift` names it as no longer in the repository; a phone holding the
app keeps it and its data, and gets no further updates from the fleet. An
`apk` product's signing-key pin lives in its held versions (`ADR-061` §4.1),
so it goes with them. The delete prints the key it drops, and a later publish
of that name pins afresh and says so.

**A delete is refused for a product a machine's card depends on**: one that a
machine's document or the register carries or pins (`ADR-050` §5.2). The card
builder resolves every carried application from this repository, so deleting
one makes that machine's next card build refuse. The refusal names the
machines and how to unpin. It reads only documents and the register, and
contacts no machine. Only an `application` is ever carried, so a phone app, a
library or a card image is never refused on this ground.

#### 7.5.2 Hide withholds a product from every fleet

**A hidden product stays in this repository and is withheld from every
fleet.** `fleet sync` does not send it, and a fleet server's prune removes it
from its store. So a phone, through Gantry's `artifacts` (`SPEC-06`), stops
being offered it, because a fleet offers exactly what its store holds. The
deploy menus and the tool's default view leave it out. **Unhiding** returns it
to the fleets at the next sync. A hide is always allowed: the armada keeps the
objects, so a card build that carries the product still finds it.

**The hidden set is part of `manifest.toml`**, a list of `(product, kind)`, so
it travels to each fleet server with the rest of the text at `fleet push`. A
fleet server's prune treats a hidden product's objects as not named, which is
what lets them leave its store. A server whose build predates the hidden set
reads an object as named and missing, and refuses to prune until it is
updated.

#### 7.5.3 The tool, and its twin

**`cargo rivet artifacts`** is the interview. It lists this repository by kind,
with hidden products folded into one line that opens. A product is chosen,
then an action: delete the product, delete one version, hide or unhide. It
returns to the list, and `x` leaves. **On the way out** it offers to push and
sync to the fleet's server `(Y/n)`. It then shows the exact list a prune would
delete, and offers the prune as its own step, defaulting to No. Declining
leaves it for `fleet sync <host> --prune confirm`.

**The scripted twin is the same core**, for an agent: `cargo rivet fleet
retire <product> [--kind K] [--version V] [--sync [confirm]]`, and `fleet hide`
and `fleet unhide <product> [--kind K]`. `fleet retire --sync confirm` pushes
and prints the prune's plan, and stops: the deletion from a store is always its
own command. `fleet list` marks hidden products, and `--hidden` shows them.

**The prune had never run against rivet-01**, because the manifest named two
`rivet-proxyd` application builds from before the proxy moved into the image
(`ADR-060`). `fleet sync` never sends an image service, so the server always
counted them missing. They are ordinary entries to delete. `fleet publish`
already refuses a new application under an image service's name, so none can
come back, and the prune needs no special case for them.

### 7.6 What a host runs is never pruned

`.rulings/` R-233. **A host's record in this repository is the authority for the
system update that host runs, and a publish never prunes a digest any host's
record names.** Keep-three alone counts versions and knows nothing about
machines, so a host that stays on one build while three newer ones are published
loses the build it runs, and with it the one a recovery would restore.

#### 7.6.1 Where the fact lives: the device record

**The device record, `register/devices/<id>.toml`, carries it** as a
`system_slots` table naming each A/B slot's system update by its digest here:

```toml
[system_slots]
a = "<64 hex characters>"
b = "<64 hex characters>"
```

The host's document was the other place inside the ruling, and it is the wrong
one:

| | key in the host's document | **key in the device record — chosen** |
| :-- | :-- | :-- |
| reaches a machine's parser | yes: a document is what a card is written with and what a machine applies, and `rivet-config` refuses a key it does not know, so every machine would first have to learn a key it never acts on | no: `rivet-fleet` composes a document field by field and composes nothing from it, and a fleet server's `rivet-fleetd` reads records leniently |
| written by | `rivet-provision`, from the register, at provisioning | the register's own store, then `rivetos_fleet_repo::record_register`, the pair provisioning already calls from `cargo-rivet` |
| survives the projection | not applicable | yes: the projection renders each record whole |

**The register under `%APPDATA%\rivet-fleet` is written first and the
repository's `register/` is its projection**, as for every other field (`§9`
step 2). So the field lives on `rivetos_register::Device`, which every reader
and writer of a record shares, and a projection carries it. Retention reads the
repository's projection, because this repository is where the record is the
authority.

#### 7.6.2 Install writes it when a system update commits — through `fleet record`

**`cargo rivet install` returns before the trial**, so the install cannot know
that its update committed. A record of what was merely installed would name a
build that rolls back at the next ordinary restart. So the step that records is
the one after the commit:

```text
cargo rivet fleet record [<host>] [--host …] [--identity …] [--pin …] [--at DIR] [confirm]
```

1. **`get slot` must answer a running slot and `committed: true`**, or nothing
   is written.
2. **The build is identified from the machine, not remembered from the
   install.** The installer retains each slot's signed header under
   `/state/slots/<slot>/package-header` (§7.2.9), and two artifacts with one
   header are one artifact, so the header is compared as bytes with the first
   4 KiB of each system update this repository holds. A slot with no retained
   header, or a build this repository does not hold, records nothing.
3. **Only the running slot's entry is written.** The inactive slot may hold a
   build installed and never trialled, and no reading tells it apart from the
   build the machine rolls back to. The other slot keeps what the record already
   said: the build it held when it last ran, committed and was recorded. A host
   recorded at each commit therefore names both builds it can boot.
4. **The host is a record the register already holds**, by name or by the pin
   `--pin` gives. A machine with no record is refused before it is asked
   anything; nothing invents one.
5. Without `confirm` it reads the machine and writes nothing.

An install of a system update ends its report with the trial restart, the
`get slot` to wait on, and this command. An application install records nothing
and prints none of the three.

**The cost of recording only what ran:** an entry can outlive what its slot
holds. When a later install writes that slot and its trial never commits, the
record still names the build the slot held before, and that build is kept until
the slot next commits and is recorded. That keeps one artifact too many; the
other way round would keep one too few.

**Once fleet-as-code drives deploys**, the operator declares what a host runs and
the fleet converges to it (R-233); the record is then the declaration rather
than something `fleet record` writes.

#### 7.6.3 Retention keeps what a host names, on top of three

Every publish keeps the three newest of each product, kind, platform and
architecture, **and every digest a host's record names**, which costs that group
no place among the three. The publish report lists each artifact kept for that
reason with the host and slot that name it. A record set that does not read
prunes nothing and says so: a prune that cannot see what a host runs could
remove it. A repository with no host recorded behaves as keep-three exactly.

A fleet server's store prunes to what the manifest names (§7.4.4), so a digest
kept here is kept there too.

---

## 8. What is already built, and it is more than expected

**A Rivet machine already hosts git repositories, and it is document-driven.**

`[repos.<name>] id = "…"` is a table in the machine document. `rivet-sysd`
reconciles the disk against it on every convergence, and `rivet-core/src/repo.rs`
is the pure arithmetic between what is declared and what is there — four states
and only four, with an undeclared directory reported and **never deleted**,
because the difference between removing a line from a file and destroying the
only copy of a year's work must not be a diff nobody read carefully.

A repository's identity is a file written inside it that git never touches, so a
machine answers *which repository is this* by reading rather than by trusting
where it sits. That is what makes a rename a `rename(2)`.

The image ships a vendored git with `git-http-backend`, because the Rust
alternative surveyed at the time was 93 crates and carried no receive-pack side
— the half that serves a push, which is the whole job. `rivet-core/src/git.rs`
holds that reasoning and the two settings git is always given.

### 8.1 And serving them

**A board serves every repository its document declares, over the control
plane.** `ADR-035` §4's smart HTTP is `cmd/rivet-mcpd/src/git.rs`: a git request
on 7777 is routed ahead of the bulk endpoint to `git-http-backend`, run as a CGI
with the caller's pinned key as `REMOTE_USER`, so an identity in `peers` can
clone, fetch and push. It replaced the daemon that used to serve repositories
as the authenticated account. `ADR-035` §4.1 records how it is proven —
`tools/qemu/adr-035-git-proof.sh`, on a booted QEMU guest — and what is not.

**A fleet server serves this repository's text as `fleet`** (`.rulings/`
R-184 D3). Provisioning writes `[repos.fleet]` into every server's document,
`rivet-sysd` creates `/state/git/fleet.git` on first boot, and `cargo rivet
fleet push <host>` pushes the workstation's `main` into it. Only the operator
pushes and fetches for now (R-184 D2); a member fetching from its server is
§9 step 5's to design, and nothing here rules it out — it is one more key in a
server's `peers`.

**The text travels in git; the objects follow beside it.** `objects/` is never
in git (§3), so neither a card (`.rulings/` R-182) nor a push carries a server's
application objects. `cargo rivet fleet sync <host>` sends them over the control
plane into `rivet-fleetd`'s own digest-keyed store, pruned by this repository's
manifest (R-184 D1) — §7.4.

---

## 9. The order to build in

1. **BUILT — the repository on the workstation.** The layout, the §4 refusal,
   `cargo rivet fleet init|publish|list`, and `deploy --artifact`. **Needed no
   board change, and none was made.** `deploy` publishing before it sends, from
   a crate or from `--rvt`, is §7's workstation half and is built on top of it.
2. **BUILT — the documents, and a projection of the register.**
   `cargo rivet provision` writes a machine's composed document to
   `<root>/documents/<host>.provisioning.toml` and commits it **after the
   card has been written and verified**, and never for a write that failed.
   `rivet-provision`, the interview it runs, saves its draft only to
   `~/.rivet/provision/<host>.provisioning.toml`, outside the repository, so
   a document in `documents/` is a record of a card that exists (`.rulings/`
   R-178). `rivet-fleet-repo` reports its own outcome separately and
   best-effort, so a fleet repository that cannot be written to (no `git`, a
   `.seed` file in the tree) never turns a provisioning session's real work
   into a failure. `rivet-fleet` renders the SAME projection of its register —
   `register.toml` beside `devices/<id>.toml`, `rivetos_register::render`'s
   own layout, so the projection is read back by `rivetos_register` itself
   and never a second spelling of the format (`SPEC-05` §4.9's rule about the
   card, applied here) — into `<root>/register/` on every save, alongside the
   real register it goes on saving wherever `rivetos-config`'s loader
   resolves it.

   **A projection, and this step's own answer to where the repository sits
   relative to the register.** The real register does not move: nothing reads
   the copy under `<root>/register/` back, and `rivetos-register` itself
   cannot be taught about a fleet repository at all, because `cmd/rivet-fleetd`
   links the same crate to run on a Rivet machine, where a git tree on a
   workstation is a fact that does not apply. What a projection gives up is
   currency — it can sit one save behind the register between a save
   succeeding and its own commit landing — and what it buys is §7.1's payoff a
   save alone cannot give: `git log` against `<root>/register/` answers what
   the fleet's register said on a day that has already passed.

   Every write here checks §4's refusal before it runs, the same posture step
   1 takes. `rivet-fleet-repo` is its own crate rather than a module folded
   onto `cmd/cargo-rivet/src/fleet.rs`; that module's own `[lib]`-less shape
   is why, and its own manifest says so.
3. **BUILT — drift.** `cargo rivet fleet drift --host … --identity … --pin …
   [--at DIR]` asks a machine's `get status` which services it holds, asks
   `get file` what each installed application's `/state/app/<name>/current`
   symlink names, and compares that version against the manifest. **The
   comparison is narrower than everything `get status` can report, and the
   narrowing is the point**: a service's health, restart count and whether the
   document calls it critical have no counterpart in the manifest at all, so
   none of that is reported as drift — only whether the artifact a machine is
   running is the one the repository says it should be. A service with no
   `/state/app/<name>` — an image-defined one, most likely — reads as `NOT
   COMPARABLE` rather than as a difference, and a machine that does not answer
   `get status` at all is refused rather than folded into either. **Reads
   only**: no `apply`, no deploy, no write to the repository. `cmd/cargo-rivet/src/fleet_drift.rs`
   is the module; `rivet_remote::tls::json_string_field` and
   `json_bool_field` were widened from private to `pub` so this reads a `get
   status`/`get file` answer through the one escape-aware JSON decoder this
   workspace already had, rather than a second one.
4. **BUILT — `ADR-035` §4, so a board serves a clone and takes a push; and a
   fleet server holds this repository's text** (`.rulings/` R-182, R-184).
   The serving half is §8.1's. The rest is three pieces, all built: a server's
   document declares `[repos.fleet]` (`cmd/rivet-provision`); `cargo rivet
   fleet push <host>` pushes `main` into it, planning without `confirm` and
   refusing uncommitted work, a missing repository and anything that is not a
   fast-forward, through a loopback relay inside that command which makes the
   pinned handshake with `rivet_remote::tls` and forwards every body with a
   length, so no key file, CA file or git configuration is written
   (`cmd/cargo-rivet/src/fleet_push.rs` carries the argument); and `cargo
   rivet fleet sync <host>` sends the application and library objects the
   manifest names, which neither the card nor the push carries, into
   `rivet-fleetd`'s own store over the control plane (§7.4, R-184 D1).
5. **The fleet server as an operator of its members**, which is what lets a web
   interface install an artifact onto another machine.

**Also BUILT, independent of this ordering — card images and system updates
join the repository (`R-177`, 2026-09-13).** `cargo rivet image` wraps an
already-built card image as a `card`-kind `.rvt`, zstd-compresses its one
member (`R-177.1`), signs it with `rivet-system` and publishes it through
step 1's own `publish_verified`; system updates now publish through the same
path, against the same key, ending §7's old exception. Neither needed
anything from steps 2 through 5 — both are workstation-only, on top of step
1's manifest and object store.

**Each is useful before the next one lands**, which is the shape a plan wants
when it is going to be interrupted.

---

## 10. What each part is blocked on

* **Step 1 is built**, and so is §7's workstation half, deploy publishing
  before it sends. Neither was blocked on anything.
* **Step 2 is built.** It needed step 1, and nothing else.
* **Step 3 is built.** It needed step 1's manifest and nothing from step 2 —
  drift compares an installed artifact's version against the manifest
  directly, and does not read a machine's document or the register at all.
* **Step 4 is built** — serving, text and objects — proven on QEMU guests and
  not on a board (`ADR-035` §4.1, §7.4.7).
* **Step 5 needs a trust change and it must be taken deliberately**: a member's
  `peers` file has to name the fleet server's key, which makes the fleet server
  a machine that can install software on every other machine in the fleet. That
  is a fourth named identity beside `ADR-034`'s operator, agent and monitor.
  **Ruled 2026-09-12 (`.rulings/` R-155 R1): deferred past the current feature
  set, and kept on the roadmap.** It is decided by its own ADR amending
  `ADR-034`, not arrived at by an implementation.
* **§7.2 is built**, and it was blocked on the ordering below rather than on
  anything technical.
* **The re-mint gives the operator's browser certificate an explicit marker**
  (`.rulings/` R-187 D1). Until then `rivet-fleetd` and `rivet-proxyd` infer the
  operator from a certificate that names no host (`ADR-043` §5.3, `ADR-046`
  §4.1), which holds only because every machine leaf provisioning mints names
  one. When browser certificates are reissued, the marker is minted into them
  and the two gates require it, so the operator stops depending on the absence
  of a field; which field carries it is decided with the re-mint. It is required
  with the re-mint and not before, because a marker required ahead of reissue
  would lock out every browser certificate already installed.

**§7.2 lands BEFORE the fleet is re-minted, and that is the whole reason it
lands now.** The rehearsal re-mints, so this is the sole moment where the
repository can become the only path without a migration: a machine with no
prior provisioning has no `.rvt` sitting in `/state` for the old form to
install, and nothing has to be moved into a store that is created empty. A
change made after the mint would need both.

**Step 4 lands before the maintainer's provisioning run** (`.rulings/` R-182),
so a fleet server can hold its repository from its first boot. Step 5 does not
land before the re-mint, because the re-mint runs on tooling that has been
rehearsed.

---

## 11. What this does not claim

**§9 step 1 is built and its own test suite is green** —
`cmd/cargo-rivet/src/fleet.rs`, verified by `cargo test`: the §4 refusal (and
that removing the seed lets the same directory initialise afterwards),
idempotent `publish`, a corrupt or wrongly-signed `.rvt` refused rather than
stored, `list` against an empty and a populated repository, digest-prefix
resolution including the ambiguous case, and that an argv with neither
`--artifact` nor `--at` reaches `deploy`'s parser unchanged.

**§7's publish-then-send is built and its tests are green** —
`cmd/cargo-rivet/src/fleet.rs`, against a scratch repository, scratch seeds
and a fake machine that records whether the repository held each payload at the
moment it was pushed: a crate deploy and an `--rvt` deploy each publish, push
the repository's object, and install the published digest; an artifact the
application seed does not vouch for is neither published nor sent; a missing
seed is refused naming both places before any repository is made;
re-deploying a published artifact goes through; a plan publishes nothing; a
system update publishes against the SYSTEM seed and is sent from the
repository like everything else (`R-177` D6), and one signed by the wrong key
is neither published nor sent, exactly as a wrongly-signed application is.

**What step 1's own suite does not run.** Every fixture is a synthetic `.rvt`
built in-process for the test; no maintainer has typed `cargo rivet fleet init`,
`publish` or `list` against a real `~/.rivet/fleet`. `AGENTS.md` §6: a green suite proves the thing is
*configured*, not that it *works* — that is steps 3 through 5's proving ground
as much as it is this one's.

**§9 step 2 is built too, and its own test suite is green** —
`rivet-fleet-repo`, verified by `cargo test`, with `cmd/rivet-provision` and
`cmd/rivet-fleet` each wired to call it: a document recorded and committed
with a message naming what changed, the same save run twice producing no
second commit, the register projected in `rivetos_register`'s own layout and
read back by `rivetos_register` itself, a device removed from the register
disappearing from its projection, and §4's refusal binding both halves.

**What that has not been is RUN, in step 1's own words.** No maintainer has
composed a real document through `rivet-provision`'s interview and watched it
land in a real `~/.rivet/fleet`, and no `rivet-fleet` command has projected a
real register there — every fixture in `rivet-fleet-repo`'s own suite is a
`Register` built in a scratch directory nowhere near a real one.

**The projection's root is a parameter, not something either program resolves
for itself at the point of use.** `project` in `cmd/rivet-fleet/src/main.rs`
and `record_in_fleet_repository` in `cmd/rivet-provision/src/main.rs` each
take `project_root: Option<&Path>`, resolved once in `main` from
`rivetos_fleet_repo::default_root()` and threaded down the same way every
other root in either program already reaches its handlers — `load` and
`store::save` took theirs this way from the start. Both programs' own
composition-root test suites call their handlers directly rather than through
`main`, and every one of those tests hands an explicit temporary directory (or
`None`) rather than reaching a real `HOME`/`USERPROFILE`, so `cargo test` in
either crate cannot write into a maintainer's real `~/.rivet/fleet` — see
`cmd/rivet-fleet/src/main.rs`'s `saving_a_register_projects_it_into_the_root_it_was_given`
and its neighbours, and `cmd/rivet-provision/src/main.rs`'s
`record_in_fleet_repository_projects_the_document_into_the_root_it_was_given`.

**§9 step 3 is built too, its own test suite is green, and — unlike steps 1
and 2 — it has been RUN against `rivet-01`, the maintainer's live board.**
`cmd/cargo-rivet/src/fleet_drift.rs`'s tests cover a matching machine, each
kind of difference, an unreachable machine reading differently from a
compared one, and a `get file` failure for one service not sinking the rest
of the report — all against a fake `CallsTools`, per `AGENTS.md` §4. Against
the real board, from an empty temporary fleet repository, every running
service resolved to either `NOT IN THE FLEET REPOSITORY` (the eight
applications, none published anywhere) or `NOT COMPARABLE` (the five
image-defined services) — `tempmon-rivet`'s and `reliquary-rivet`'s
versions came back exactly as §1.1 quotes them, `-dirty-6cf29875` and
`-dirty-c563f94f` included. A `MATCHES` and a `DIFFERS` row were then produced
by hand-writing two `[[artifact]]` entries into that same repository's
`manifest.toml` — one naming `tempmon-rivet`'s exact running version, one
naming a different version of another running application — because proving
the comparison against real data did not require a real signed `.rvt` to
publish it from; `fleet publish`
itself was already proven by step 1's own suite. The unreachable case was run
against the register's own stale `board` entry (`192.168.50.66:7777`), which
refused with `os error 10060` and no drift, before this end waited for it.

**What that leaves unmeasured**: a `fleet drift` run against a repository
populated by a genuine `cargo rivet fleet publish` of a real signed artifact,
rather than a hand-written manifest entry — step 1's own suite proves
`publish` and step 3's own suite proves the comparison, and nothing has yet
run the two in series against a board.

**§7.2 is built, its tests are green, and part of it has RUN on a real aarch64
kernel — but not on a board and not over the wire.** 2026-09-12.

What the suites cover: the store's layout and retention as pure arithmetic
(`rivet_core::artifact`, including that an object path cannot leave the store
and that a store given a root never reaches the machine's own); the operation
refusing a path by name, refusing a digest the store does not hold, and
reporting the artifact's own name in its plan (`rivet_core::operation::install`);
the request format refusing a path however it was written
(`rivet_core::deploy`); the supervisor resolving a digest, and refusing an
absent, unreadable or ambiguous object (`cmd/rivet-sysd`); the bulk endpoint
storing, refusing a corrupt or truncated body without keeping anything,
succeeding on a re-send, and pruning a fourth version against a **real
filesystem** in a temporary store (`cmd/rivet-mcpd`); and `cargo rivet deploy`
asking by the digest of the file it sent and refusing `--remote-dir`
(`rivet-remote`).

Under QEMU, against the aarch64 kernel and `tools/qemu/run.sh`, three cases were
run end to end through PID 1 and the supervisor: a request naming a digest the
store holds is resolved to the object's path and adopted; a request naming a
digest it does not hold is refused by name and removed; a request naming a path
is refused by the parser.

**What that leaves unproven**, and it is the part a green suite cannot reach
(`AGENTS.md` §6): **no artifact has been transferred into a real store over the
control plane, and no install by digest has reached a board.** The QEMU runs
pack a placeholder object rather than a signed `.rvt` and do not pack
`cmd/rivet-deploy` at all, so nothing has yet verified a signature out of this
store, and no `cargo rivet deploy` has run against `rivet-01` since the change.
Retention has been exercised against a temporary directory and never against
`/state` on a card. Closing all of that is one command — a `cargo rivet deploy`
of a real artifact at a real machine — and it has not been typed.

**§9 step 4's serving and text halves are built, tested and RUN on a QEMU guest
— not on a board.** 2026-09-14. `cmd/cargo-rivet/src/fleet_push_tests.rs` pushes
with a real `git` through the real relay into this host's own
`git-http-backend`, run the way `rivet-mcpd` runs it: plan, create, fast-forward,
a forced-chunked push arriving whole, and each refusal.
`tools/qemu/adr-035-git-proof.sh` runs the release `cargo rivet fleet push`
against a booted guest's `[repos.fleet]` and clones it back with a second,
independent client (`ADR-035` §4.1). **Unproven**: a board, its network path, a
card's `/state`, and a fleet repository of realistic size.

**§9 step 4's objects half is built, tested and RUN on a QEMU guest — not on a
board.** 2026-09-14. §7.4.7 says what each suite covers and what
`tools/qemu/adr-052-objects-proof.sh` ran; objects of a realistic size over a
real network, and a member fetching them, are what it leaves unproven.

**Every other section above is a decision rather than a measurement**, and
§1.1 carries the document's own measurement — three version strings read off
`rivet-01` on 2026-09-09.

**The 700 MB figure is one system update payload**, `rivet-v0.1.77-99-g7d8eb56-aarch64-rpi4-update.rvt`,
at 703,464,286 bytes. Nobody has measured what a git repository does with a
year of them, and nobody should have to.

---

## History

**2026-10-03 — every entry of the repository store was handed to
`rivet-fleetd`, directories included, and the store was written by path.
`.rulings/` R-210 D13 and D14 changed both.** §7.4.1 read:

> ~~every entry is then handed to `rivet-fleetd`'s uid — the way `cmd/rivet-sysd`
> hands an application what its card staged (`ADR-049`) — so the application
> whose directory it is can read what it holds.~~

**Why it changed.** Handing the directories over gave the application a
directory that root writes into. The 2026-09-24 audit's remediation guarded the
walk with `lstat` checks before each use, which closed a link planted once and
left in place, and left open the window between the check and the use. Owning
nothing root writes into removes the precondition for the whole class (D13),
and held directory descriptors close the window the checks left (D14, where the
maintainer chose to close it now over accepting it for the present).

**2026-10-03 — the workstation repository's retention was keep-three alone.
`.rulings/` R-233 added §7.6.** §7.4.3 read:

> ~~the repository's own retention keeps the three newest of each product,
> kind, platform and architecture — cards and updates included. So a fleet
> server holds three of each, exactly as the repository does, without counting
> anything itself.~~

**Why it changed.** Keep-three counts versions and knows nothing about
machines. It pruned the system update rivet-01 was running once already, because
three newer development builds were published after it; the build a host runs
is the one its recovery restores, and the one its other slot rolls back to is the
next. R-233 made the host's record the authority for what it runs and forbade a
prune from removing what any record names.

**2026-09-29 — §7.4.3 admitted applications, libraries and apks only, and said
no retention count applied. `ADR-063` §6 (`.rulings/` R-228) widened the store
to cards and system updates.** Item 3 read:

> ~~3. **the kind** — an `application`, a `library` or an `apk`, opened by that
> kind's own reader, and nothing else: a `card` is written by the workstation
> and an `update` is installed from the machine-wide store by the machine it
> is for.~~

and the section closed ~~*"**No retention count applies.** §7.2.2's
three-per-product rule is about what a machine installed; this store's rule is
the manifest's."*~~

**Why the reason stopped holding.** Both exclusions were true of a fleet server
that only relayed what its members install: a card is written onto an SD card by
a workstation, and an update is installed by the machine it is for, so neither is
something a relay needs. `ADR-063` makes the fleet server the authority for its
fleet, and an authority is what a host is restored from — which needs the card
to write and the update to bring a member current. The reason for the narrowing
was the server's role, and the role changed. **Members stay narrow**: §6.1's
rule is about a member's own store and is untouched. The retention sentence was
not wrong — this store still counts nothing — but read alone it implied the
store was unbounded, and with cards in it the bound is worth stating: the
manifest keeps three per kind, so the store does.

**2026-09-14 — §8.1 said a board serves no repository, two weeks after it did.
`R-184` found the code; `tools/qemu/adr-035-git-proof.sh` proved it.** §8.1 read:

> **`ADR-035` §4's smart HTTP for git is phase 3.** The control plane
> recognises a git request and refuses it by method name until that phase
> lands; `cmd/rivet-mcpd/src/dispatch.rs` names it in the refusal, and
> `cmd/rivet-mcpd/src/bulk.rs` says the same thing where a caller would meet it.
>
> So today a board **holds** repositories and serves none of them. A clone or a
> push against a Rivet is refused.

and §9 step 4 was *"`ADR-035` §4 phase 3, so a board can serve a clone and
take a push"*, with §10 calling it *"designed, partly wired, unbuilt"*.

**Why it was wrong, and for how long.** `cmd/rivet-mcpd/src/git.rs` fronted
`git-http-backend` from 2026-08-28/29, routed ahead of the bulk endpoint; the
refusal §8.1 described was reachable only by a request that missed git's
routes, and both source files kept the stale "phase 3" sentence after the code
moved. `.rulings/` R-182 repeated the claim, ruling git serving built before the
maintainer's provisioning run as though it were work still to do. The phase 3 design read the tree before building on
the record, R-184 ruled what was left, and the QEMU proof ran a clone, a push
and a fetch on a booted guest, finding one real gap on the way: a gzipped fetch
body `git-http-backend` was never told about (`ADR-035` §4.1). Both source
files' sentences are corrected in the same pass. Kept here because a reader of
R-182 would otherwise find a ruling premised on a gap this document no longer
has, and no record of why.

**2026-09-13 — §2, §7 and §7.2.3.5 named a system-update exception and said the
repository held no card images. `R-177` closed both.** The exception existed
because *"which key a deploy's publish of one should verify against is not
decided"* (§7's old text) and because no verb put a card image into the
repository at all. `R-177` D6 decided the key — `rivet-system`, the same one
`R-177` gives `card` images — so the reason the exception existed stopped
being true, and R-176's "the missing piece is a verb, not a concept" gave
`cargo rivet image` to close the other half. Superseded text:

> **A system update is the one payload a deploy sends without publishing.**
> The application seed does not sign system updates, and which key a deploy's
> publish of one should verify against is not decided. The header's
> unauthenticated kind field is read for that decision alone: a file that
> falsely claims to be an update is sent unpublished and still meets the
> machine's door (§7.2.3), which verifies it against the machine's own trust
> set. So the invariant holds for every application and library that reaches
> a machine, and a system update reaches the store through the door alone.

> **It holds no card images.** §2 intends them and nothing implements them:
> an image is not a `.rvt`, so accepting one means a second signature check
> and a second kind of index entry. Recorded here as intent, and staged with
> the update path.

The entry is kept because a reader of `.rulings/` R-157 R13 (kept updates
unpublished "until the update path joins the store with card images") would
otherwise have no record of the day that condition was met.

**2026-09-12 — §2 stated the invariant with no exception, while §7 already
named one.** It read:

> **Deployment** — `cargo rivet deploy` **selects** an artifact from the
> repository. Nothing reaches a device without having passed through it.

§7 was already correct — *"A system update is the one payload a deploy sends
without publishing"* — but §2 is the decision a reader meets first, and an
unqualified absolute sentence there reads as though §7's exception overturned
it rather than being named by the same decision. `.rulings/` R-157 R13 said
this plainly when the maintainer was asked whether deploy should publish a
system update this release: keep updates unpublished, **recorded as the one
known exception**, until the update path joins the store with card images. §2
now says so where the invariant is first stated, rather than leaving the
qualification to be found four sections later.

**2026-09-12 — §7 said deploy's workstation half was owed, and that a bare
deploy built and sent without the repository seeing it.** It read:

> **Teaching `deploy` to build, sign and publish from a bare crate is still
> owed**, so the two-line illustration above is not yet exact: a bare `cargo
> rivet deploy <package options>` builds and sends without the repository seeing
> it. … Until the workstation half lands, the invariant holds for everything that
> reaches a device and does not yet hold for everything that reaches the store.

and §9 step 1 closed *"teaching `deploy` to build and sign one from a bare crate
first is §7's remaining half and is still owed."* **The deferral expired for two
reasons.** The one it inherited — that nothing before the re-mint may change the
path the maintainer walks on today's tooling (the entry below) — had already
gone when §7.2 landed. The one of its own was a question: publish verifies
against a trusted key, the repository holds no seed, so an automatic publish had
no key to use. **The maintainer answered it** (`.rulings/` R-155 R3): the
publish reaches the application seed by `cargo rivet package`'s own lookup. With
nothing left to wait on, `deploy` publishes before it sends, and the invariant
§2 states holds on the workstation too — for applications and libraries. A
system update is the part the ruling did not cover, which is why §7 says it is
sent unpublished.

**2026-09-12 — §7.2.3 said the door checks authenticity and not applicability,
and recorded the architecture gap as the maintainer's call.** It read:

> **What the door checks is authenticity, not applicability.** Whether a system
> update's product, architecture and platform are this machine's, whether a slot
> will take it, whether an application's closure is satisfied, whether a version
> directory is occupied — each depends on facts the installer owns at the moment
> it installs … **One applicability check is absent, found while writing this and
> not fixed here:** the installer compares architecture for a system update only
> … It is not the door's to catch — the reasoning above holds — and wiring it in
> is a change to what the installer refuses, which is the maintainer's call.

**The maintainer made the call** (`PLAN-Feature-Set-2026-09-12` §3.3g): a
wrong-architecture package is garbage under the no-garbage ruling, so it is
refused at the door and at install. **The line the old paragraph drew was in the
wrong place, not wrong in kind.** It grouped architecture with the slot, the
closure and the version directory because all four are "applicability"; what
actually separates them is where the fact comes from. The architecture record is
a file in `/system` like the trust store, fixed for the image's life, while the
other three change between transfer and install — which is the argument the
current §7.2.3 carries. The gap's hazard, that the compiler says `arm` where
`ADR-027` says `armv6`, is what put the token in the image.

The entry is kept because a reader of the current section would otherwise wonder
why the door checks one applicability question and not the others.

**2026-09-12 — §7.2.3 ruled the store NOT a trust boundary, and a ruling later
the same day reversed it.** The section was headed *"What the store is
not"* and read:

> **It is not a trust boundary and nothing is trusted for being in it.** What has
> been checked is the *transfer*: the digest is a statement about the wire. The
> signature is checked by `cmd/rivet-deploy` when the install runs and by
> `cmd/rivet-sysd` at every launch thereafter (`ADR-015` §4.2), in those two
> places and nowhere else, because `ADR-029` §11's *"three formats means three
> verifiers each with a third of the scrutiny"* is the rule this must not quietly
> break.

§7.2.2 drew the consequence — *"A stray high version holds its place … an
artifact the installer refused … keeps one of the product's three"* — and §7.2.5
recorded the QEMU proof's untrusted `0.9.9` as **stored and refused**, with a
retention test asserting it was one of the three held.

**What changed was the premise, not the retention policy.** The first framing —
how should retention treat an artifact that never installed — was the wrong
question; asking why such an artifact was in the store at all was the right one,
and the answer was that the endpoint named bytes before anything checked them.
The disclaimer was accurate about that endpoint and it was the endpoint that was
wrong. **The reading of `ADR-029` §11 it rested on was also too broad**: the rule
protects against two *implementations* that can disagree, and the sentence *"in
those two places and nowhere else"* treated a count of call sites as the thing
protected — a count `cmd/rivet-sysd`'s launch check had already made two. The
current §7.2.3 moves the sequence into the core so the store and the installer
call one implementation, and §7.2.3.3 carries the argument for the reversal.

The entry is kept because the reversal would otherwise look arbitrary: the
correction and the ruling point opposite ways, and both were right about the code
in front of them.

**2026-09-12 — §7.2.2 ordered retention by ARRIVAL and argued that a version
could not be ordered at all. `rivet-01` disproved the argument the same day.**
The section read:

> **"Newest" is arrival, not version**, because a version string is not
> orderable: `ADR-027` §3.2 composes `0.1.1-g3d28ede-dirty`, and two dirty
> builds of one commit have no order at all — which is precisely why §3.2.2
> appends a digest to them. Arrival order is the object directory's modification
> time, which `ADR-021` makes sufficient and no more …

Four ascending versions of one product — `0.3.1`, `0.3.2`, `0.3.3`, `0.3.4` —
deployed back to back left the store holding `0.3.1`, `0.3.3` and `0.3.4`. It
evicted `0.3.2` and kept `0.3.1`: **a newer version deleted while an older one
survived.** The count was right, another product was untouched, and the eviction
was reported by name — so nothing looked wrong.

**The argument's flaw was that it generalised from the hard case.** Two dirty
builds of one commit genuinely have no order, and they are not what retention
chooses between most of the time. `0.3.1` and `0.3.2` have an order a board
could read straight off the file name, and the section threw it away because a
*different* pair does not have one. The rule now orders what it can and falls
through to arrival for what it cannot, which is what the old sentence's own
counter-example was asking for.

**And arrival was not merely imprecise, it was absent.** Four deploys over a LAN
finish inside the one clock second `ADR-021` grants a board, so all four
modification times tied and the digest tie-break — a content hash, bearing no
relation to a version — chose. §7.2.7's own note that a finer clock *"would make
that rarer and would not make it impossible"* was measuring the wrong thing: on
a board a tie is not rare, it is what a burst of deploys produces every time.

The entry is kept because the store's whole purpose is a **local** rollback, and
a rule that can delete the version before last while keeping one older than it
does not serve that purpose. That is the reasoning the current §7.2.2 rests on.

**2026-09-12 — §7.2.6 used to record two defects as unfixed, and §7.2.7 and
§7.2.8 did not exist.** The section read:

> **Two things the proof surfaces, and neither is fixed here.** … The machine
> says what it pruned and nobody hears it … Two deploys in a row to one machine
> lose the second … the fix is either a wait in `cargo rivet deploy` or a
> documented one in the operator's script; neither is decided here.

Both are fixed, and a third defect found on `rivet-01` the same day — a
successful transfer reported as a failure — is fixed with them. **The reasoning
that made those two worth recording survives in the sections above**: that
retention is the one thing a machine does silently, and that one request slot is
correct and therefore something the *tool* has to accommodate rather than
something to relax.

The entry is kept because a reader of §7.2.7 would otherwise wonder why a deploy
tool sleeps at all. It stopped being a list of open defects and became the
decision about what the tool does with them.

**2026-09-12 — §7 and §10 used to defer the change §7.2 now makes, and the
deferral's own reason is what expired.** §7 said, of teaching `deploy` to build
and sign from a bare crate:

> Teaching `deploy` to build and sign from a bare crate, so the two-line
> illustration above is exact, is not this step: it would change what a bare
> `cargo rivet deploy --rvt …` or `cargo rivet deploy <package options>` does
> today, and `ADR-052` §9's own ordering is explicit that nothing before step 4
> may change the path a maintainer minting a fresh fleet on today's tooling is
> already walking.

and §10 closed:

> **None of it lands before the fleet is re-minted.** The re-mint runs on tooling
> that has been rehearsed, and a deploy path that changed the day before is not
> that.

**Both were right and both were about a fleet that was about to be re-minted
anyway.** The deferral protected one thing: the path a maintainer walks on
*today's* tooling, because he was going to walk it to mint the fleet. The
rehearsal re-mints, so that path is being walked from scratch — and a change
made after the mint would have to migrate every `.rvt` already sitting in
`/state` on every machine, where a change made before it creates the store
empty and moves nothing. The moment the deferral was waiting for is the moment
it stops being worth having.

**What did NOT expire**, and is why §10's closing sentence survives for steps 4
and 5: a *transport* change the day before a rehearsal is still a bad trade, and
`ADR-035` §4 phase 3 is one. What landed is an operation and a file format,
both proved before the maintainer types anything.

**2026-09-09 — the projection's root used to be resolved by `project` and
`record_in_fleet_repository` themselves, and a `#[cfg(not(test))]` gate
compiled the whole projection away to a no-op under `cargo test`.** Neither
`cmd/rivet-fleet`'s `load`/`store::save` nor `cmd/rivet-provision`'s other
writes ever resolved their own root — every one of them took it as an
argument — but `project` and `record_in_fleet_repository` called
`rivetos_fleet_repo::default_root()` directly, reading the REAL
`HOME`/`USERPROFILE` with no test double. Running `cargo test -p
rivetos-fleet` once, before the gate existed, created a live
`~/.rivet/fleet` on the machine that ran it, with the test suite's own
fixture devices committed into it. The `#[cfg(test)]` gate stopped that live
write, but at the cost the gate's own presence should have named: a test
binary compiled `project` as a no-op whatever called it, so all 306 of
`rivetos-fleet`'s tests ran with the projection silently absent — production
behaviour with nothing over it, in a step the drift feature (§9 step 3) builds
on. The fix was to give the root the same shape every sibling root in both
programs already had — a parameter, threaded from `main` — rather than a gate
on the function that used it. The current state is in §11 above.
