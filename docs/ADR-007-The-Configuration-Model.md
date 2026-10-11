# ADR-007: The Configuration Model

* **Status:** **Accepted** — ruled 2026-08-22. §3.4 amended (fleet layer added,
  runtime tier removed) and §4.1 replaced; see §6 for what was ruled and §6.1
  for what is left open.
* **`/etc` has ONE generator, the runtime projection** (§3.5). A status line
  here once announced two, and it was already false when it was written — the
  amendment was withdrawn the same day, and §3.5 records the withdrawal in full.
  It is called out in the status block because `AGENTS.md` §7 is about exactly
  this: a claim corrected in the body and left standing in the header, which a
  reader who stops at the status block will believe.
  The build plan carried the same stale claim at the time; it is history now,
  archived and deleted per the maintainer's 2026-09-06 ruling.
* **§7 added 2026-08-23** — the schema, key by key, and the `[network]` table
  ruled that day.
  collapsing two of them either erases an operator's keys on a card fault or
  makes revocation unsayable. **§7.7 IS SPENT — `ADR-035`, 2026-08-29.**
  Setting the table changes nothing; it is still parsed so old documents are not
  refused. Said in the header as well as in the body because this status block
  has already been the place a corrected claim survived once, six lines above.
* **§3.5.1 added 2026-08-23** — the projection is built, and it is
  `cmd/rivet-sysd`'s (`ADR-017` §3.1.8). §3.6.1 item 1's read-only mount
  is reached by remounting after the write. **A running Rivet has an `/etc` and
  a login shell to put in it**: `cmd/rivet` shipped the same day and
  `cmd/rivet-image` installs it, so the projection's probe finds it and
  `/etc/shells` names a shell.
* **§7.6 superseded 2026-08-23** — `rivet-store` exists, so the "a `[network]`
  table is read, counted and dropped" claim is no longer true. §3.1 and §3.4 are
  annotated with what was built against each; §7.6.2 is what is still missing and
  §7.6.3 is why `ADR-004` §6 step 3 cannot be written yet.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`ADR-006` decided the administrative *surface*: one core, several adapters,
structured output, typed errors. This ADR decides what sits underneath it — how
configuration is stored, represented, layered, and reconciled with reality.

`ADR-006` section 1.2 established that the Windows Registry's coherence and its
opacity are separable properties, and that centralisation is available in a text
format. That leaves the actual question: **what does a system built now do,
given that both of the models it inherits are known to fail, and given that
several serious attempts have been made in the last fifteen years?**

---

## 2. What the serious attempts have already learned

None of these is being copied wholesale. Each contributes one idea that is worth
taking, and each has a failure worth avoiding.

| System | The idea worth taking | The failure worth avoiding |
| :-- | :-- | :-- |
| **systemd** | Layered drop-in directories with strict precedence — vendor, runtime, admin. `systemctl cat` shows the merged result with per-line provenance. | Still text files in directories with no schema, no validation, and no way to preview a change. |
| **NixOS** | The whole system is one declarative document; `/etc` is *generated*, never authored; atomic switch with rollback by generation. | The abstraction leaks — configuration frequently escapes back into hand-written file content — and a functional language is a steep entry cost. |
| **macOS `defaults`** | Typed values, domain namespacing, one access API, and a binary-at-rest form that converts losslessly to text for inspection. | No declared schema; increasingly, state moved into genuinely opaque databases. |
| **Windows DSC / MDM CSPs** | Microsoft's own conclusion that the Registry needed a schema'd, declarative layer above it. Corroborates the diagnosis. | Bolted on top of the thing it was fixing, so both surfaces remain live. |
| **Kubernetes** | `spec` and `status` as distinct first-class objects; reconciliation rather than one-shot apply; server-side apply recording *which manager owns each field*; `explain` and `diff` from the schema. | Enormous, cluster-shaped, and YAML. |
| **OpenWRT UCI** | Proof the whole pattern works at appliance scale: one unified config format, one CLI verb set, generated native config for the daemons underneath. | Weakly typed; no schema; no preview. |
| **Talos Linux** | Existence proof — an immutable Linux configured entirely by one declarative machine document and administered over an API, with no shell at all. | **It is exactly as far as Rivet wants to go, since `ADR-035` (2026-08-28).** The control plane is MCP over TLS and the interactive path is gone. This row's verdict was the opposite until then; the comparison was correct and the conclusion was overtaken. |

The pattern across all seven is consistent, and it is the answer to the
question: **the mess is not caused by text, and it is not cured by binary. It is
caused by the absence of a schema and the absence of a single authority.**

---

## 3. Decision: six moves

### 3.1 Separate the store from the representation

The Registry conflated them and paid with opacity; `/etc` conflated them the
other way and paid with fifty formats.

* The **store** is typed and structured — the Rust types are the authority.
* The **representation** is generated text, in one format, canonically
  serialised so that two machines with the same state produce byte-identical
  documents.

Neither is chosen at the other's expense, because one is derived from the
other. The document is diffable, greppable, reviewable and version-controllable;
access from code is typed and validated.

**Built 2026-08-23, and the byte-identity clause is a test rather than a
sentence.** `rivet-config::serialise` renders the canonical form and
`rivet-store` writes it; §7.6.1 records what each is and why the renderer is
in the schema crate. Three derived choices are worth knowing about here because
this section is what governs them: **key order follows the schema's own declared
order** rather than alphabetical order (there is already exactly one list of
keys, `ADR-011`); **named tables are sorted by name**; and the document carries a
**constant two-line header** naming the tools as the way to change it, which is
§3.6.1 item 2's provenance mechanism applied to the one Rivet file an operator
can open and edit with effect. That header is part of the canonical bytes, so
changing its text changes every machine's document — the cost is recorded in the
module rather than discovered.

### 3.2 A declared schema is the keystone

This is the thing both inherited models lack and the reason both rot.

**The schema is generated from the Rust types**, never maintained beside them,
so it cannot drift. It buys four things that neither `/etc` nor the Registry
can offer:

* **Validation** — a bad document is rejected at the edge with a typed error
  naming the field, rather than producing a machine that half-works.
* **Self-description** — `rivet get schema` (`ADR-006` section 7), which is what
  lets an operator or an agent work with a system it has not seen. **Built
  2026-08-23**, in `rivet-config::schema`; `ADR-006` §7.1 records how
  "generated from the types" is made true in a workspace with no `serde` and no
  `schemars`, and §7.2 records the one thing the emitted document cannot express
  and therefore says out loud.
* **Migration** — the schema is versioned, and a document from an older version
  is migrated explicitly rather than silently misread.
* **The ability to declare a setting dead.** This is the anti-rot mechanism. The
  Registry accumulates orphaned keys for ever precisely because nothing is
  authorised to say a key is no longer meaningful.

### 3.3 Desired state and observed state are different objects

Borrowed from Kubernetes' `spec` and `status`, and this is the single most
important move in the ADR.

> **The configuration document says what the machine should be. It never
> contains what the machine is.**

In `/etc` and in the Registry these two live in unrelated universes with no
defined relationship, which is why neither can answer "is this machine
currently what it was configured to be?"

* **Desired state** is persisted, layered (section 3.4), and operator-owned.
* **Observed state is never persisted.** It is read live from the kernel,
  netlink, `procfs` and the mechanisms at the moment it is asked for.

This is not a new rule; it is `AGENTS.md` section 5 — *the kernel is the
authority for system state; never cache its verdicts* — expressed in the data
model instead of relying on discipline. Two components cannot hold divergent
ideas of whether `wlan0` is associated if neither is permitted to hold one.

What it buys: **drift detection**, an honest `plan` (`ADR-006` section 6), and
reporting that cannot be stale.

### 3.4 Layering, with per-value provenance

Steal `HKLM`/`HKCU` and systemd's drop-in precedence, then go further than
either.

**Amended 2026-08-22.** The stack originally had a fourth, non-persistent
*runtime override* layer. It is **REMOVED** — see §3.4.1 — and a *fleet* layer
takes its place, following the ruling in `ADR-006` §5.1 that Rivet manages
several machines.

| Layer | Source | Set by | Persistent |
| :-- | :-- | :-- | :-- |
| 1. Schema defaults | the Rust types | — | n/a |
| 2. Image defaults | baked into the image | the build | yes |
| 3. **Fleet** | shared across the operator's machines | the tools | yes |
| 4. Machine | this specific machine | the tools | yes |

Every layer is persistent and every layer is written through the administrative
tools. There is no ephemeral tier and nothing is hand-edited.

**Built 2026-08-23: layer 4 and nothing else.** `rivet-store` persists the
machine layer at `/state/config/machine.toml`. Layer 1 needs no store because it
*is* the Rust types (`rivet_config::Document::default()`); **layers 2 and 3 do
not exist, and nothing merges layers** — there is no precedence engine and no
per-value provenance, so the `--explain` output above is still a design and not
an implementation. §5 puts layering at Stage 2–3 for exactly this reason, and
notes that the deferral is cheap *provided values are typed*, which they are.
Nothing is stubbed: a fleet layer that parsed a file nobody writes and merged
nothing would answer "what is the effective value" with a number that had been
through no precedence at all (`AGENTS.md` §3).

The file is named `machine.toml` rather than being the directory's only file so
that the other two have obvious neighbours when they arrive. The *directory* is
`ADR-008` §3.1's; the file name is derived and is `rivet-store`'s to change.

Strict precedence, highest layer wins, and — the part neither prior model can
do — **every effective value can name where it came from**:

```
rivet config get net.wlan.country --explain
  value:    GB
  from:     machine configuration
  overrode: image default (US), schema default (unset)
```

`systemctl cat` is the closest existing thing and it is per-file, not per-value.
For an operator at midnight, and for an agent reasoning about a system it did
not configure, "why is this value what it is" is the question that gets asked
most and that existing systems answer worst.

#### 3.4.1 REMOVED 2026-08-22: the runtime override layer

The original stack had a fifth-in-spirit layer — `rivet set --runtime`,
writable, not surviving a reboot — offered as the honest escape hatch that made
a read-only `/etc` defensible.

**Removed at the maintainer's challenge**, which was that it contradicted
`set` → `apply` → *takes effect*. The objection is correct and it is an
`ADR-011` objection: an ephemeral tier is a second way to change the machine,
and a second way is a second authority.

Its one genuine benefit — recovering from a change that locks you out of a
headless machine — is served better by the confirmed apply in §4.1, which
protects by default rather than when the operator thought to ask.

The layer is recorded here rather than deleted because the *problem* it was
solving is real and will be raised again by anyone who has been locked out of a
remote box. The answer is §4.1, not a new tier.

### 3.5 `/etc` is generated output, never a source

The mechanisms of `ADR-001` are C daemons that want files — `resolv.conf`,
`passwd`, a supplicant's control socket. Rivet does not fight this and does
not let it propagate upward.

* **`/etc` exists, is generated, and lives on a tmpfs.** It is a build artifact
  of applying the configuration, not an input to anything.

  **`/etc` stays empty on the image until the runtime projection renders all of
  it** — ruled by the maintainer on 2026-08-23, withdrawing an amendment made
  earlier the same day (`ADR-008` §5.3) that had `cmd/rivet-image` generate five
  files at build time. That amendment was implemented and then removed;
  `cmd/rivet-image` generates nothing under `/etc` and the original rule stands
  unqualified.

  **Why the amendment's own argument did not carry it.** That some of `/etc` is
  a fact about the *image* rather than the machine is true — `/etc/shells` names
  which binary may be a login shell, and no configuration document changes that.
  It does not follow that such a file must be produced at build time. The
  runtime projection renders `/etc` before anything that reads it starts, so it
  can render **all** of `/etc`, and then there is one generator rather than two.

  **Two generators over one directory is what `ADR-011` exists to prevent**, and
  the defence offered — that they cover disjoint facts — is a bill that comes
  due the first time a fact stops being obviously one or the other. The version
  that keeps one generator is the one that needs no such argument, and the
  amendment was reaching for build-time generation to avoid an empty `/etc`
  rather than because anything required it.

  **What this costs, stated rather than buried.** Until the runtime projection
  exists, a running Rivet has an empty `/etc`, so a login program rejects every
  login before authentication. Stage 1 cannot finish
  without that projection. This changes the order of the remaining work, not its
  size.

  **Corrected later on 2026-08-23: there is a login shell now.** `cmd/rivet`
  is built and installed at `/system/bin/rivet`, and
  `rivet-core::etc`'s renderer is handed that path, so the *definition* of
  `/etc/shells` names a binary that is really on the card. **The projection is
  therefore the only thing left between here and an `/etc` on a running
  machine**, which makes this ruling's cost the whole of the remaining cost
  rather than half of it. That is a sharper position than this section first
  described and it is the one this ADR should be read as taking.

  **What was kept.** `rivet-core::etc` survives in full: the nine musl paths,
  the argument for shipping or not shipping each file, and the renderers. That
  knowledge cost a sweep of the linked `libc.a` and found a ninth path
  `ADR-008` §5.1 had only predicted. **The second renderer was deleted, not the
  definition** — the runtime projection is what will call it.

#### 3.5.1 The projection, built 2026-08-23 — and **who runs it**

  The paragraphs above ruled that there is one generator and left open what
  process is it. `ADR-017` §3.1.8 answers that: **`cmd/rivet-sysd`,
  before it converges any service.** Not PID 1 — `ADR-017` §3.1.5's job list is
  deliberately unchanged, and §3.1.8 records the argument, including why the
  `/state` amendment made hours earlier does not carry.

  The shape, so the boundary is legible from this file:

  | Piece | Where | Why there |
  | :-- | :-- | :-- |
  | the files, their modes and their bytes | `rivet-core::etc` | pure; no I/O, no platform types (`ADR-014` §3) |
  | where `/etc` lives — tmpfs, flags, `mode=755,size=1m` | `rivet-core::etc` | a *plan*, in the same shape as `crate::mount` and `crate::state` |
  | mounting, writing, remounting, reading back | `cmd/rivet-sysd::etc` | behind that crate's `Kernel` port, so every failure path is tested with no kernel |
  | supplying the document | `cmd/rivet-sysd`'s `run` | configuration is I/O; it is loaded at the composition root (`AGENTS.md` §10.1) |

  **§3.6.1 item 1 is satisfied by a remount, and that is the interesting
  detail.** A tmpfs cannot be mounted read-only and then written to, so the
  projection mounts it writable, renders, and **remounts it `MS_RDONLY`**. Until
  that last step a Rivet is in the top row of §3.6's table — an edit succeeds
  and is discarded at the next boot — so a refused remount is reported as an
  error naming that outcome rather than as a tidy-up that did not happen.

  It is also the one step whose success cannot be taken from its return value.
  With `MS_RDONLY` removed as an injection test, `mount(2)` returned **zero**
  and `/proc/self/mounts` still said `rw`. So the projection reads the mount
  table back and reports the kernel's word beside its own — `AGENTS.md` §5.

  **What the document contributes: `hostname` reaches `/etc/hosts`**
  (`rivet_core::hostname`, `PLAN.md` §40, ruled 2026-08-26) **and `timezone`
  reaches `/etc/localtime`** (`ADR-021` §9, ruled 2026-09-02).

  **The mechanism that produced both is the part worth keeping.** For as long as
  the document contributed nothing, the projection still took a `Document`, read
  nothing out of it, and carried a test that fails the moment
  `Document::DECLARED_KEYS` changes — so the question is asked when the schema
  grows instead of being missed. That guard fired twice, and both times the
  question was asked rather than missed. It was written that way rather than
  inventing a `hostname` key to make the argument earn its place, which is a
  schema decision and the maintainer's.

  **Of the three facts `/etc` carries that the document did not name:** the
  hostname key was ruled on 2026-08-26 and that row is closed; the `timezone`
  key was ruled on 2026-09-02 and that row is closed too (`ADR-021` §9), with
  `/etc/localtime` rendered conditionally and `ADR-008` §5.3.1 carrying the
  corrected table. What is left is the human half of `ADR-008` §7.3, which is
  not waiting on a schema key at all — it is waiting on an authorization layer.
* **Nothing hand-edits it**, and a system that has been hand-edited is
  detectable rather than merely disapproved of.
* The mess is therefore contained at the lowest possible layer, where it belongs
  — it is the *mechanisms'* format, not Rivet's.

NixOS demonstrates this works. It is also what makes `ADR-004` section 5's
"`wpa_supplicant.conf` does not exist" a general property rather than a
one-off.

---

### 3.6 The system never accepts an input it will discard

Added 2026-08-22 at the maintainer's insistence, and it is the move that makes
3.5 defensible rather than merely tidy. **He required that an attempt to edit
something under `/etc` inform the operator that what they are looking at is a
read-only view — that the system at least be honest about it.**
*(`.rulings/` R-020.)*

**Read-only and *honest about being read-only* are different properties**, and
section 3.5 only specified the first. A generated `/etc` on tmpfs has three
possible behaviours when someone edits it, and two of them are dishonest:

| Behaviour | Verdict |
| :-- | :-- |
| The write succeeds; the change is lost at reboot | **Silent data loss.** The worst outcome available. |
| The write succeeds; the next `apply` overwrites it | **Worse** — non-deterministic, so it is not even reliably learned. |
| The write is refused | Correct, but only if the refusal explains itself. |

`EROFS` alone is not honesty. It produces "read-only file system" from an
editor, which tells the operator what happened and nothing about why or what to
do instead. That is a true statement and a useless one.

#### 3.6.1 What honest refusal requires

Four mechanisms, in increasing cost. The first two are free and are required;
the others are proposed.

1. **The mount is genuinely read-only**, so the write fails rather than being
   accepted and discarded. Non-negotiable — everything else is decoration
   without it.
2. **Every generated file carries a provenance header naming its source field
   and the command that changes it.** This is the highest-value mechanism in
   the list because it is read by exactly the person about to make the mistake:
   they have the file open.

   ```
   # Generated by Rivet from `net.dns`. Do not edit — this file is
   # regenerated on every apply and this filesystem is read-only.
   #   rivet config set net.dns ...
   ```

   Note that this is `ADR-007` section 3.4's per-value provenance reaching into
   the generated artifacts, not a separate idea.

3. **A reverse lookup**, so the question can be asked before the mistake rather
   than after it: `rivet explain /etc/resolv.conf` returns the field it is
   generated from, that it is read-only, and the command to change it. Cheap,
   because the generator already knows what it produced from what.

   **Built 2026-08-23**, and it is Rivet's first administrative operation —
   `rivet-core::operation::explain`, `ADR-006` §8.2. Three things were asked
   for and they are three fields of the result, taken from
   `rivet_core::etc::ETC_FILES`: `derived-from`, `writable` (always `false`,
   with the filesystem that makes it so), and `change-with`. **No `change-with`
   may say "edit it"** — a view is never an interface (§3.6.2) — and a test
   enforces that rather than a reviewer noticing.

   `/etc/resolv.conf` is the example this section chose and it is the
   interesting case: Rivet does not generate it, deliberately, so the lookup
   refuses with `E_UNKNOWN_SUBJECT` and says the absence is a decision and that
   musl's own fallback is the correct answer. A bare "unknown" would be true and
   useless, which is the same defect §3.6.1 finds in a bare `EROFS`.

   **It matters most for the three files that cannot carry item 2's header.**
   `/etc/passwd`, `/etc/group` and `/etc/shadow` have no comment syntax musl's
   parsers are known to honour, so this lookup is the only place their
   provenance can be read at all — which `rivet-core::etc` had recorded as
   unwritten.
4. **The refused attempt is logged.** `EROFS` cannot be turned into a good
   message at the point of failure, but the *attempt* can be made visible —
   "a write to `/etc/resolv.conf` was refused; that path is generated from
   `net.dns`" — which is what turns a confusing 2am experience into a
   diagnosable one. This costs real machinery and is the one to defer.

#### 3.6.2 Ruled, 2026-08-22: files are a view, tools are the interface

> Let's enforce the rules. You **change** things via the admin tools. You
> **view** them, for convenience, as files.

This settles the mechanism list above — items 1 to 3 are required rather than
proposed; only item 4, logging refused attempts, remains deferred machinery.
More usefully, it names the relationship precisely:

> **A generated file is a read-only projection of state owned elsewhere. It is
> never an interface.**

The relationship is a database view to its tables. Convenient to read, never
written through, and always reconstructible from the thing that actually holds
the state. This is the same sentence as `ADR-001` section 5's *mechanisms are
never the interface*, applied to files instead of to programs — which is a sign
it is the right shape rather than a new rule.

**"View" therefore becomes vocabulary** (`AGENTS.md` section 10.2) and means
exactly this. A file that can be written and have effect is not a view, and
calling it one would be the beginning of the drift this ADR exists to prevent.

#### 3.6.3 The tension this exposes, and its resolution

The ruling appears to contradict `ADR-006` section 6.1, whose argument for
AI-friendly administration is that *an agent edits a declarative document and
runs `plan`*. If everything changes through tools, is the document edited or
not?

Both are true, and the distinction is where the document lives:

* **There is no live, editable configuration file on a running Rivet system.**
  The on-system representation is a view, per 3.6.2.
* **A configuration document is an input artifact**, authored anywhere — a
  workstation, a git repository, an agent's scratch space — and fed to the tool:
  `rivet apply -f <document>`. Passing a document to a tool *is* changing
  things via the admin tools. The document is an argument, not a location.

**Proposed, and the piece that makes this pleasant rather than merely correct:**
`rivet config edit`, on the model of `visudo` and `kubectl edit` — check the
document out to a temporary file, open an editor, validate on save, show the
plan, apply on confirmation. It preserves the feel of editing a file while every
property above holds, and a validation failure discards the edit instead of
installing a broken machine.

This has a dependency the ruling on `/bin/sh` created: it requires an editor to
exist. See `ADR-005` section 5.5.

**That dependency is met, 2026-08-24: the editor is `cmd/femto`** (`ADR-005`
§5.5.1). It is worth noting what it does *not* settle, because the two are
easily conflated: `femto` is an interactive editor that refuses to write a view
and says what to change instead — it is §3.6.1 item 3's answer delivered by the
tool an operator already has open. `rivet config edit`, the `visudo`-shaped
flow this section proposes, is still unbuilt: it needs the checkout, the
validation on save, the plan and the confirmation, none of which is an editor's
business. What has changed is that the flow now has something to invoke.

#### 3.6.4 A possible extension, not proposed

If files are a view, there is no principled reason to stop at `/etc`. The whole
model — desired state, observed state, provenance — could be projected as a
read-only filesystem, with all mutation through `rivet`. That is what `/proc`
and `/sys` already are, arrived at deliberately rather than by accident, and it
would make the system explorable with nothing but `ls` and `cat`.

**Noted, not proposed.** It is a genuinely appealing idea and it is also a
FUSE-shaped amount of work with a Stage 3 audience. Recorded so that the
question is asked deliberately later rather than half-answered early.

#### 3.6.5 The rule generalises

This is not a rule about `/etc`. It is the third face of a discipline already
stated twice elsewhere: `AGENTS.md` section 5's *refuse rather than guess*, and
section 3's *stubs refuse; they never fake success*.

> **Rivet does not accept an input it intends to discard.**

Applied consistently, it also governs:

* **Unknown configuration fields** are rejected at parse time with a typed error
  naming the field, never ignored. A document containing a misspelled key that
  loads successfully is the same defect as a discarded `/etc` edit.
* **A confirmed apply that times out** (section 4.1) reverts and says so. It
  does not quietly leave the machine half-changed, and the reversion is an
  event the operator can see rather than something discovered later.
* **Added 2026-08-23: a document that contradicts itself is refused, never
  resolved by precedence.** §3.6's table above lists three behaviours a
  read-only `/etc` can have and finds two of them dishonest. A schema has the
  same three, and the same two are dishonest — see §7.4, which is the first
  place the schema has had two keys capable of disagreeing.
* **Added 2026-08-24: an *edit* of a view is refused, in `explain`'s own
  words.** See §3.6.6.

#### 3.6.6 The write half of §3.6.1's reverse lookup, 2026-08-24

§3.6.1 item 3 asked for a reverse lookup *"so the question can be asked before
the mistake rather than after it"*, and §3.6.2 made it required. `rivet
explain <path>` answers it. **`rivet apply edit <path>` is the same question
asked by something that is about to write**, and `ADR-006` §8.9 is the ruling
that created it.

The two share one source of truth rather than two, and this is the sentence that
matters: `rivet_core::operation::edit` calls `explain`'s own implementation and
refuses with the explanation's own `change-with` string. A second list of
unwritable paths inside the write path is the second authority `ADR-011` §3 is
about, and it would be consulted at exactly the moment somebody is already
confused. `every_generated_path_is_reported_as_a_view_and_never_as_writable` and
`a_generated_path_is_refused_as_a_view_and_says_what_to_change_instead` are the
two halves as tests.

**One thing `edit` refuses that `explain` will not answer for**, and it is worth
recording because it looks like an inconsistency: the rest of `/etc`. `explain`
answers only for paths Rivet *generates* (§3.6.1), so `/etc/motd` is
`E_UNKNOWN_SUBJECT` there — a true answer to *"what generates this"*. `edit`
refuses it anyway, because §3.5.1's projection is the **whole tree** on a
filesystem remounted read-only, so a write there is discarded at the next boot
in the best case and refused by the kernel in the ordinary one. The two are
answering different questions and each is right about its own; the refusal says
which, and points at `explain`.

---

## 4. The honest costs

### 4.1 The Unix escape hatch is removed

`vi /etc/something` will not stick. This is a genuine loss and it is the single
thing people dislike most about systems built this way. Pretending otherwise
would be dishonest.

**Amended 2026-08-22 — the mitigation changed, and improved.** This section
originally offered the runtime-override layer as the escape hatch. The
maintainer rejected it as inconsistent with `set` → `apply` → *takes effect*,
and he was right: an ephemeral tier is a second way to change the machine, which
is precisely the second authority `ADR-011` forbids.

**The replacement is a confirmed apply**, borrowed from network equipment, where
this exact failure has been solved for decades:

```
rivet apply --confirm-within 5m
```

The change is applied. If it is not confirmed inside the window, it reverts
automatically. This addresses the real danger — applying a change that severs
your own access to a headless machine, leaving no way to undo it — and it does
so **by default rather than when the operator remembers to ask**, which is the
decisive advantage. The runtime tier only ever helped someone who anticipated
needing it, and anticipation is exactly what is absent at the moment it matters.

It costs no new concepts: one flag on an existing verb, and no second class of
value.

**One implementation constraint, from `ADR-021` §5:** the confirmation window is
measured on `CLOCK_MONOTONIC`, never the wall clock. A machine with no RTC
corrects its clock by potentially hours shortly after boot, and a wall-clock
deadline would revert a good change early or leave a bad one applied past its
deadline — on a headless machine the operator may have just locked themselves
out of.

So `vi /etc/something` genuinely does not stick, and there is no ephemeral tier
softening that. What replaces it is better than what `/etc` offered — an
emergency change at 2am is a recorded, attributed, revertible apply rather than
an edit indistinguishable six months later from a deliberate decision.

### 4.2 Schema versioning is a permanent obligation

Once a schema is published, migrations are owed for ever. This is real,
recurring work and it does not get cheaper. It is accepted because the
alternative — no schema — is the failure mode being designed against.

### 4.3 It is possible to over-build this before there is anything to configure

Restated from `ADR-006` section 9 because it applies with more force here.
Stage 0 configures almost nothing. The commitment at Stage 0 is only that
configuration is *typed*, that there is *one document*, and that observed state
is never written into it. The layering engine, `plan`/`apply`, and migrations
are Stage 3 machinery that these three make possible without being needed early.

---

## 5. What this commits to, and when

| Commitment | Stage | Cost if deferred |
| :-- | :-- | :-- |
| Configuration is typed; the Rust types are the authority | 0 | rewrite every call site |
| One document; no second on-disk form of any setting | 0 | migration |
| Observed state is never persisted into the document | 0 | the model becomes uncorrectable |
| `/etc` is generated, on tmpfs, never authored | 1 | every mechanism added meanwhile leaks its format upward |
| Schema generated from the types; `rivet get schema` | 1 — **met 2026-08-23**, `ADR-006` §7.1 | drift begins immediately |
| Layering with per-value provenance | 2–3 | moderate — layers can be added if values are typed |
| `plan` / `apply` / `export` | 3 | moderate, if the above holds |
| Schema versioning and migration | first release | — |

---

## 6. Open for the maintainer

**All three ruled 2026-08-22.**

1. **Is this the model? — Yes.** Adopted, and it shapes the configuration types
   from the start (`ADR-006` §6).
2. **Document format — TOML**, with canonical serialisation so that identical
   state produces identical bytes. Note what the format is *for*, since this was
   originally justified badly: a document authored off-device and handed to
   `rivet apply -f`, the output of `rivet export`, and the read-only
   projection readable on the machine. **It is never edited in place** — §3.6.2
   governs. The eventual dashboard API speaking JSON is not in tension with
   this; §3.1 separates the store from its representations, and there may be
   more than one representation.
3. **How far does the runtime layer reach? — Void.** The layer is removed;
   see §3.4.1 and §4.1.

### 6.1 Left open

* **Whether `--confirm-within` becomes the default for changes that could sever
  the operator's own access.** It would need the system to know which settings
  are risky, which is a schema annotation and therefore cheap — but it is
  irritating if wrong. Deferred until there is enough configuration to judge.

---

## 7. The schema, key by key

Added 2026-08-23. §3.2 makes the schema the keystone and §5 commits Stage 0 to
*one document, no second on-disk form of any setting*. This section is where the
document's actual keys are recorded as they are ruled, so that the schema is
readable from the record rather than only from `rivet-config`'s types.

**The types remain the authority** (§3.1). This section describes them; when the
two disagree, the types are right and this section is a defect.

### 7.1 `version`

Required, an integer. Derived rather than quoted from the record: §3.2 requires
that a document from an older version be "migrated explicitly rather than
silently misread", which is impossible unless the document says which version it
is.

### 7.2 `[services]` — ruled 2026-08-22, `ADR-017` §4.1

One sub-table per service name. `enabled` required; `memory` and `cpu`
optional (§7.2.5). What a service *is* — its program, its restart policy, how
readiness is known — is refused with an explanation rather than as an unknown
key, because an operator who reaches for `exec` has a wrong model rather than a
typo.

#### 7.2.1 Amended 2026-08-24: `listen` leaves, and it was a real conflict

**Ruled by the maintainer, 2026-08-24.** `listen` was accepted here
for two days, and the thing it was half of is the point: **this section and
`ADR-015` §3.2 declared the same two keys, in two places, and `ADR-011` says
both cannot be the authority.**

| | `ADR-007` §7.2 said | `ADR-015` §3.2 said |
| :-- | :-- | :-- |
| `listen` | the operator's document | the signed manifest |
| `memory` | the operator's document | the signed manifest |

Both ADRs were accepted on 2026-08-22, neither cited the other on these two
keys, and `rivet-config` implemented this one — so a machine had an operator's
`listen` that an application's manifest could contradict, with nothing to say
which won. That is not a wording problem. It is the exact defect `ADR-011` §3
tabulates, inside this record rather than in somebody else's system.

**The ruling, and the test it turns on:**

> **The manifest owns what the application *is*** — restart policy, readiness
> contract, the sockets it listens on. **The document owns what the operator
> *limits*** — the memory cap. **Where both name `memory`, the lower wins.**

**The direction is the test.** `ADR-015` §4.1 protects an application's
*behaviour* from being rewritten outside its signature: a manifest editable
after signing could ask for a privileged port or a wider grant and still verify.
A memory cap runs the other way — it is the machine's owner protecting the
machine **from** the application — so nothing about it is weakened by living out
here, and this section is right to grant it.

So the operator's whole surface is **`enabled` and `memory`**, and `memory` is a
**cap** rather than a value: it can only ever make a service's effective limit
*tighter*. An operator who writes `memory = "512M"` for an application whose
manifest asked for `128M` has not granted it 512 MiB — they have said *no more
than* 512 MiB, and the application's own statement is still the tighter of the
two. Raising it would be this document rewriting a value the signature covers.

**Why the answer is not "the document always wins", which was the obvious one.**
It would make `ADR-015` §4.1 conditional: an application's declared behaviour
would hold only until an operator wrote a line, and the signature would then
guarantee a program's *bytes* while guaranteeing nothing about how it is run. A
signature that covers the manifest exists precisely so that the permission
request cannot be edited (§4.1's own argument), and a document that could
rewrite `restart`, `ready` or `listen` is that edit by another route.

**Why it is not "the manifest always wins" either.** Then a memory cap would not
be a cap: an application could grant itself any amount by asking for it, and the
machine's owner would have no way to protect the machine from software they
installed. `ADR-015` §5.4's `plan` shows *"128M memory cap"* under **grants**
rather than under **requests**, which is this section's half already written
down in the other ADR.

#### 7.2.2 What the refusal has to say, and why this one is different

Every other key in `ADR-017` §4.1's refused set has *always* been the
definition's, so an operator who writes `exec` has a wrong model and one
sentence corrects it. **`listen` is the first key an operator can have read out
of an accepted ADR** — this section declared it, and `ADR-015` §5.2 says in as
many words that *"'may listen on 443' therefore becomes a line in the
desired-state document"*. A refusal that gave them the general sentence would
read as a defect in Rivet rather than as a decision, so the refusal names the
ruling and its date:

```
configuration key `services.rivet-mcpd.listen` at line 4, column 1 is part of the
service's definition, not its configuration; a definition ships inside the
signed artifact that provides the service and is not operator-editable. Ruled
2026-08-24: which sockets a service is served on is a fact its author asserted
about the program, so it lives in the signed manifest and an operator cannot
move it. This key WAS accepted here until that ruling, and ADR-007 §7.2 and
ADR-015 §5.2 both still read as though it were. A service accepts: enabled,
memory
```

#### 7.2.3 What this costs, stated rather than discovered

Three things, and the first is the real one.

1. **An operator can no longer move a port**, and nothing replaces that. If an
   application's author declares `tcp:8080` and it collides with something on
   the machine, the remedy is a new signed artifact. That is the correct
   consequence of the direction test — the port is behaviour — but it is a
   capability this document had and does not have, and `ADR-015` §5.2's
   sentence is now wrong rather than merely unimplemented. `PLAN.md` §0.4.3
   carries the question of what, if anything, should replace it.
2. **Two services claiming one socket is no longer refused at parse time.** One
   document held every claim, so `rivet-config` could refuse a collision with
   no knowledge of the machine; the claims are in separate signed manifests now
   and only the supervisor holds them all at once. It **reports** rather than
   refuses there, because refusing would mean choosing which of two signed
   applications does not run, at boot, on a headless machine.
3. **`memory` now needs a unit, and `ADR-015` §3.2 never gave it one.** *The
   lower wins* is a comparison. `rivet-config` has parsed the suffix as binary
   (1024) since 2026-08-22 and its canonical serialisation depends on it, so
   that is the standing answer and `rivet_core::service::MemoryLimit` is where
   it is now written down; no formal ruling settles the question and it
   remains the maintainer's.

#### 7.2.4 Built the same day, and what a running machine showed

`rivet-config` refuses `listen` with the message above; the supervisor reads
an installed application's manifest, builds a service definition from it, and
applies the operator's cap by the rule.

**Observed under QEMU on `ADR-024`'s own kernel — not on a board.** An
application called `demo-app`, whose manifest asked for `restart = "always"`,
`listen = ["tcp:8080"]` and `memory = "128M"`, with a document that said
`enabled = true` and `memory = "32M"`:

```text
{"sev":"info","msg":"an installed application defines a service; what it IS comes from its
signed manifest and only `enabled` and a memory cap are the operator's (ADR-017 §4.1, ruled
2026-08-24)","svc_name":"demo-app","program":"/state/app/demo-app/current/bin/demo-app",
"restart":"always","memory_request":"128M"}

{"sev":"info","svc":"demo-app","msg":"capped","memory":"32M","memory_bytes":33554432,
"memory_source":"document-lower"}
```

and, read back **from the running machine** rather than from the
console that wrote it:

```text
/sys/fs/cgroup/rivet/demo-app/cgroup.procs  66
/sys/fs/cgroup/rivet/demo-app/memory.max    33554432
/sys/fs/cgroup/rivet/rivet-mcpd/memory.max  max
```

The third line is the one that makes the second mean anything: a system service
nobody capped is uncapped, so `33554432` is this ruling's arithmetic and not a
default.

**What that settles.** The operator's lower value wins on a running machine and
reaches the kernel, and where a cap is refused the supervisor reports that at
`Severity::Error` rather than assuming it worked.

**A board settled it on 2026-08-28**, which this section had recorded as
unsettled: `PLAN.md` §66.4 read the whole delegation chain back off the kernel —
`/sys/fs/cgroup/rivet/rivet-mdnsd/memory.max` at `33554432`, the service's pid
in the capped cgroup, and the kernel accounting against it. **And it is
installed**: `cmd/rivet-deploy` is a workspace member and first ran on a
Raspberry Pi on 2026-08-25 (`PLAN.md` §14.2). *(It was `cmd/rivet-update` when
this section was written — `ADR-029` §14 carries that correction.)*

#### 7.2.5 Ruled 2026-09-09: `cpu` joins `memory`, on the identical footing

**Nothing in this project capped CPU.** `ADR-015` §5.1's mechanisms table
named `memory, CPU, pids` under "Resource limits" from the day it was written,
and only the first was ever built. `pids` is not the gap beside it: `PID 1`'s
child, `cmd/rivet-sysd`, sets `RLIMIT_NPROC` to 256 on every service, so
process count was already bounded. One application spinning could still
starve a board that is also driving a television and serving the
maintainer's own personal data, which is what closed the gap.

**The ruling does not re-argue §7.2.1's split; it applies it a second time.**
`cpu` is declared in the machine's document *and* may be requested in an
application's signed manifest, for §7.2.1's identical reason: a CPU share is a
policy the machine's owner is entitled to set, Rivet enforces it from outside
the service (a `cpu.max` the service does not read and cannot refuse), and
changing it cannot change which code runs. `rivet_core::service::
effective_cpu` is `effective_memory` with a different unit — **the lower
wins, whichever side states it** — and the operator's `cpu` key can therefore
only ever tighten what a manifest asked for, never raise it past what the
signature covers (`ADR-015` §4.1).

**The spelling is a decimal number of cores**, because that is what an
operator means: `cpu = "0.5"` is half a processor, `cpu = "2"` is two. cgroups
v2's `cpu.max` wants two numbers in microseconds instead —
`"<quota> <period>"` — so `rivet_core::service::CpuLimit` fixes the period at
100000 µs (100 ms, the scheduler's own default) and derives the quota as
`cores × 100000`:

| Written | Cores | Quota | `cpu.max` |
| :-- | :-- | :-- | :-- |
| `cpu = "0.5"` | 0.5 | 50000 | `50000 100000` |
| `cpu = "2"` | 2 | 200000 | `200000 100000` |

A quota larger than the period is not refused: cgroups v2 defines it as
permission to use more than one CPU's worth of time inside each period, which
is exactly what `cpu = "2"` asks for on a multi-core board.

**There is deliberately no default.** A machine with one application should
be able to give it the whole processor, and a default cap invented here would
change how every already-provisioned machine behaves the moment it updated,
with nobody watching — the third row of `AGENTS.md` §1.1's decision table.
Absent means `cpu.max` is left at `max`, the kernel's own uncapped value.
`memory` differs here, by ruling: an application whose manifest asks for no
memory gets half of the machine's, because a process that grows without bound
takes the machine with it, where one that spins only slows it. A `memory` cap
in this document replaces that default in either direction (`ADR-015` §3.2.1).

**`cpu.weight` is deliberately NOT built, and the omission is a decision
rather than a gap.** `cpu.max` is a hard ceiling — *this application may
never exceed X, whether or not anything else wants the processor*.
`cpu.weight` answers a different question — *when two or more things want
the processor at once, divide it in this proportion* — and costs nothing
when nothing is contending, which arguably makes it the better default for a
fleet of single-purpose boards. It is not ruled out; it is simply a second
mechanism this ADR does not need to add the first one, and adding it is the
maintainer's to decide, not an agent's to bury in an implementation.

**Built and proved under QEMU the same day.** `cmd/rivet-sysd` mirrors
§7.2.4's memory proof exactly: a capped probe's own cgroup accounting
(`cpu.stat`'s `usage_usec`) is read back over a known interval rather than
trusted from the write, and a second probe with no `cpu` declared is
asserted to start and run unaffected. `tools/qemu/adr-007-cpu-proof.sh` is
that proof.

### 7.3 `[network]` — ruled 2026-08-23

**The ruling.** How a Rivet host is told its own address had three shapes
offered. The maintainer took **option 2 — a `[network]` table
in the document plus first-boot provisioning** — and added that he expects
Rivet will still end up using DHCP for many devices. *(`.rulings/` R-267.)*

That second sentence folds option 3 in rather than deferring it: **DHCP is a
value of this table, not a competing design and not a later feature.** It is
first-class in the schema from the first line, and what is missing is the
mechanism to satisfy it, not the vocabulary to ask for it.

**Why the table exists.** `cmd/rivet-netd` took `--interface`, `--address`
and `--gateway` on its command line. That works, and it makes a service's
argument list a second thing that also looks authoritative about what this
machine's address is — which is the whole of `ADR-011` §3. The same ruling had
been made the same day over `/etc`'s generators (§3.5), and that is the
governing precedent.

One sub-table per interface name:

```toml
[network.eth0]
method  = "static"
address = "192.168.1.50/24"
gateway = "192.168.1.1"

[network.wlan0]
method = "dhcp"
```

| Key | Required | Holds |
| :-- | :-- | :-- |
| `method` | **yes** | `dhcp` or `static`. A closed vocabulary per `ADR-006` §6.4 |
| `address` | under `static` only | `A.B.C.D/N` — the host address and its subnet |
| `gateway` | no — optional, and only under `static` | the default route's next hop, a bare host address with no prefix |

**Four refusals, none of them type errors.** Each catches an operator mistake
that would otherwise be found late, on a headless board, over the link being
brought up:

1. **A contradiction** — `dhcp` beside an `address` or a `gateway`. §7.4.
2. **A gateway outside the interface's own subnet**, and **a host address that
   is the subnet's network or broadcast address.** Both are
   `rivet-core::net::StaticAddressing::validate`'s, run at parse time rather
   than at apply, because §3.2 wants a bad document rejected *at the edge*.
3. **One address on two interfaces.** Two entries asserting one fact is
   `ADR-011` in the small, and it is detectable with no knowledge of the
   machine — exactly like two services claiming one socket (`ADR-017` §4.1).
   Compared on the host address, not the whole CIDR: `10.0.0.2/24` and
   `10.0.0.2/16` are the same address on two links.
4. **A name the kernel would not accept.** `dev_valid_name()`'s rules, quoted
   from `rivet-core` rather than re-derived.

Note what is *not* refused: **a name the machine does not have.**
`rivet-config` performs no I/O (`ADR-014` §3.1) and the kernel is the
authority for which interfaces exist (`AGENTS.md` §5). A document written for a
machine that has not been built yet is the normal case — §3.6.3 makes the
document an input artifact authored anywhere. The refusal happens at apply,
where the information is, and `cmd/rivet-netd` produces it: *"there is no
interface called `nosuchif0` — check the name against `ip -br link`"*.

**The values are `rivet-core`'s types**, not new ones: `InterfaceName`,
`Ipv4Cidr` and `StaticAddressing` already existed, already carried the kernel's
own `dev_valid_name()` rules, and `ADR-009` §4 forbids a second declaration of a
type Rivet already owns. A second address parser is the plainest available
example of `ADR-011`'s second authority.

**What is not here, and will be.** Nameservers (`ADR-023` owns `net.dns`; a
lease also carries them, and which wins is not ruled), wireless credentials
(`ADR-004` §5 puts the PMK in `ADR-020`'s encrypted store, not in this
document), and anything the kernel observes — a carrier, a MAC, a lease
expiry — which §3.3 forbids outright and which the schema therefore has no
field for.

### 7.4 Why `method` is a key and not an inference

This is §3.6.5 applied to a schema, and it is the first time the schema has had
two keys capable of contradicting each other. It is recorded at length because
the cheaper design is genuinely tempting and is wrong.

The cheaper design: **let the presence of `address` mean static and its absence
mean DHCP.** No `method` key, one fewer thing to write, no contradiction
possible because there is nothing to contradict.

| Surface | What happens to a DHCP intent written beside a static `address` | Verdict |
| :-- | :-- | :-- |
| `address` present ⇒ static | the intent has nowhere to be written at all, so a stale `address` line silently wins | **the operator's intent is the discarded input** |
| `dhcp = true` beside `address` | two spellings of one fact, and no vocabulary to name their disagreement | `ADR-011` §3 in miniature |
| **`method` required; contradictions refused** | the document names the mistake, at a line and column, and nothing is applied | **chosen** |

The sharp edge is the first row, and it is worth stating plainly because it is
what §3.6.5 is for: **a document that says `method = "dhcp"` and also carries a
static `address` is refused, not resolved.** Not by precedence, not by "the
address is more specific so it wins", and not by ignoring the key that has no
effect. Two of those three silently discard something the operator wrote; the
third is the same defect as an ignored misspelling, which §3.6.5 names in as
many words.

**Absence is only allowed to mean something when it means exactly one thing.**
That rule — stated in `rivet-config`'s services module and applied here —
decides `method`'s requiredness by itself. An absent `method` could mean "use a
lease" or "the operator forgot", and it cannot mean both. A default of `dhcp`
also fails on the record's own terms: not every card carries the client that
would honour it — `tools/card/build.sh` puts BusyBox `udhcpc` on a card only
when Buildroot produced one — so a default that named `dhcp` would sometimes
default to a mechanism that is not there, and the rejected option 3's own
warning still applies: "a machine that cannot be addressed without it is
unreachable".

### 7.5 The schema is not the capability

A document asking for `method = "dhcp"` **parses and validates**, exactly as
`static` does. The supervisor runs BusyBox `udhcpc` for the interface as a
service of its own, and `rivet-netd` turns the lease it records into the same
plan a static document produces (`ADR-004` §8). **What fails, and fails
loudly, is a card built with no `udhcpc` on it at all** —
`DhcpUnavailable::MechanismNotBuilt` — which is a property of that card's
build, not of the schema.

Refusing `dhcp` at parse time would be easier and would be a mistake. It would
make the schema mean *what this build can do today* rather than *what a valid
statement of desired state is*, and the schema would then have to change when a
capability arrived — which is exactly the drift §3.2 versions the schema to
avoid, arriving from the other direction.

The general form, which is `AGENTS.md` §3's refusal rule split in two:

> **A schema says what may be asked for. An apply says what can be done. A
> refusal belongs to whichever of the two the operator actually got wrong.**

### 7.6 What is not finished — the second half was built, 2026-08-23

**Both halves of the ruling are built.** `ADR-004` §6 has PID 1 read
`rivet.toml` from the boot partition and write its values into the persistent
configuration; `cmd/rivet-initd` performs the read and `rivet-store` — created
on 2026-08-23 and in the workspace member list — performs the write. For part
of that day the write had nowhere to go and a `[network]` table on the boot
partition was read, validated, counted on the console and dropped.

#### 7.6.1 What was built

* **`rivet-store`** — `ConfigStore`, a `Filesystem` port, and a real adapter
  over `std::fs` that is `cfg(unix)` because `fsync` on a directory has no
  Windows equivalent and a stub that returned success there would be a faked
  durability barrier (`AGENTS.md` §3).
* **The canonical serialiser, in `rivet-config` rather than in the store.**
  §3.1's *"two machines with the same state produce byte-identical documents"*
  is now a property with tests rather than a sentence: documents written five
  different ways — key order, table order, comments, `128M` against
  `134217728`, an empty `[services]` table against none — render to one byte
  sequence, and every rendered document parses back to the value it came from
  and renders identically a second time.

  It lives beside the parser because `rivet export` (`ADR-006` §6.2) and a
  future API need those bytes with no filesystem in sight; a renderer inside the
  persistence adapter would make them depend on it, or produce a second
  renderer, which is `ADR-011` §3 at byte level.
* **`ADR-004` §6 step 2**, in PID 1: on a first boot the provisioning document
  becomes `/state/config/machine.toml`. `ADR-017` §3.1.8 is the amendment that
  grew PID 1's list for it and carries the argument.

#### 7.6.2 What is still not finished

* **Only §3.4's machine layer is persisted.** There is no image-defaults layer,
  no fleet layer, and **nothing merges layers** — no precedence engine and no
  per-value provenance, so `--explain` cannot yet say `overrode: image default`.
  §5 puts both at Stage 2–3 and this is that deferral, not a gap. Nothing is
  stubbed to look like a layer that is not there.
* **The stored document is read and applied — closed 2026-08-23**, and it is
  listed here because it was open when this list was written.
  `rivet-sysd` reads
  `/state/config/machine.toml` through `rivet-store` before it converges any
  service, and starts `cmd/rivet-netd` with that path as its `--document`
  (`ADR-017` §4.3). A first boot now stores what it was told **and applies it**;
  observed under QEMU, with `eth0` addressed and routed from a staged document.

  **The operator's whole surface is `enabled` and `memory`, and both are
  honoured.** §3.6 is why a refusal is said out loud rather than dropped.

  * **`listen` is refused permanently, not pending socket activation.** §7.2.1
    ruled on 2026-08-24 that which sockets a service listens on is a fact the
    author asserted about the program and belongs inside `ADR-015` §4.1's
    signature. It is not coming back, and an operator must not be left waiting
    for it.
  * **`memory` is honoured, and is proven on a Raspberry Pi 4.** §7.2.4 settled
    it under QEMU on 2026-08-24, and `PLAN.md` §66.4 read
    `/sys/fs/cgroup/rivet/rivet-mdnsd/memory.max` back as `33554432` on the
    board on 2026-08-28. **`ADR-024` §8.1's "controller absent" is superseded**
    by `ADR-024` §8.2, which struck its own `MEMCG` hypothesis on 2026-08-23.

  This section said the opposite of all three until 2026-08-29, and it said so
  by citing `ADR-024` §8.1 — a section that had already been corrected, which is
  why nothing downstream caught it. **A citation to a superseded section reads
  exactly like a citation to a live one.**

  **The `[network]` table's other end is unchanged**: `method = "dhcp"` still
  parses and still fails at apply, which is §7.5 working as designed.
* **`ADR-004` §6 step 3 is outstanding** — see §7.6.3.
* **It has run on hardware and under QEMU.** Hardware: `PLAN.md` §0.0 records
  the first card booting **provisioned**, *"so no `p1` was hand-edited —
  `provisioning document read;`"*, with a static address on `eth0` and the
  operator's trust material arriving that way; `AGENTS.md` §10.5's last
  bullet says the same. QEMU: `tools/qemu/seed-card.sh` and `run.sh --card`
  boot a real card image with a real boot partition, used in `PLAN.md` §66.3.
  **This is the bullet a cold agent reads to decide whether first-boot
  provisioning works at all**, and until 2026-08-29 it said no while a board in
  the next room had done it.

#### 7.6.3 The provisioning file's credential, and why step 3 cannot be written yet

`ADR-004` §6 step 3 removes the credential from the file after the first boot.
It is not implemented, and the reason is worth recording because it is not
laziness: **there is nothing to remove.**

§7.3 keeps wireless credentials out of this document entirely — the PMK belongs
to `ADR-020`'s encrypted store — so the schema declares no key a passphrase could
be written under, and a `rivet.toml` carrying one is refused at parse time as
an unknown key (§3.6.5). Step 3 becomes implementable when `ADR-020`'s store and
a `Secret`-typed schema key exist, and not before. Until then the sentence in
`ADR-004` §6 describes a sequence whose middle term the schema forbids, which is
a tension in the record rather than a defect in the code.
### 7.8 `[volumes]` — ruled 2026-08-25, `ADR-031` §7.1

**The ruling is `ADR-031`'s**, and this section is only the schema half of it:
what the document may say, what it may not, and which of the two absences apply.
Everything about *why* a Rivet has more than one filesystem — and everything
about what happens when a declared volume is not attached — is `ADR-031` §6.

```toml
[volumes.bulk]
guid = "1a2b3c4d-5e6f-4071-8293-a4b5c6d7e8f9"
```

| Key | Required | Holds |
| :-- | :-- | :-- |
| `guid` | **yes** | the partition's GUID, the full 36-character form, as `blkid` and `sgdisk` print it |

**It is the only key, and that is `ADR-031` §7.2's safety argument rather than
minimalism.** `device`, `path`, `label` and `uuid` are the four keys an operator
reaches for and each is refused **by name**, because §3.6.5 refuses an unknown
key rather than ignoring it. Each of the four names a way of identifying storage
that is not stable across a reboot, so accepting one would mean a document that
was right on the day it was written and wrong after a disk was added.

#### 7.8.1 Which absence this is

Two answers, not three — because a volume is a **statement about what should be
attached**, not a file whose contents are being replaced:

| The document says | It means |
| :-- | :-- |
| no `[volumes]` table | this machine has its built-in `/state` and nothing else |
| `[volumes]` with no sub-table | the same thing |

They are one value, so canonical form renders neither — which is `[services]`
and `[network]`'s rule (§3.1). A third answer would only be needed if *erase
what is there* had to be sayable, and it does not: a volume that should no
longer be attached is removed from the document and the machine simply stops
looking for it.

#### 7.8.2 `state` cannot be declared

The name of the built-in volume is refused as a volume name. A second thing
called `state` would be two answers to one name, and `/state` needs no
declaration: it is the volume a Rivet always has (`ADR-031` §2.1). The refusal
is the `ADR-015` §7.1.1 clause — *well-formed and already means something* —
applied one level up.

The name is otherwise a **single path component**, checked at construction for
the sharper version of the reason a service name is: it becomes a directory
under which an application's programs are found and run, so a separator, a `..`
or a leading dot is refused rather than sanitised.

#### 7.8.3 What is **not** checked, and why it cannot be

**That the GUID names a partition this machine has.** `rivet-config` performs
no I/O (`ADR-014` §3.1), and more importantly the question has no answer at parse
time: which devices are attached is the machine's answer at the moment it is
asked (`ADR-031` §4.2), and a USB disk that is unplugged this morning is not a
document that has become invalid. This is §7.3's rule for interface names, and
the same §7.5 split — the schema says what is well-formed, the machine says what
is possible.

**Two declarations naming one GUID are refused**, and that one *is* a property of
the document. A GUID is the identity of one device (`ADR-031` §4), so two names
for it would make which one holds an application depend on the order the tables
were read in. Comparison is case-insensitive, because the text is lowercased at
construction — `sgdisk` prints upper case and `blkid` prints lower, and an
operator who pastes one of each has made one mistake, not two declarations.

#### 7.8.4 What is built, and what is not

**Built 2026-08-25:** the schema, the parser, the refusals above, the canonical
rendering, and `rivet get schema`'s description of the table. The full test
corpus round-trips a document carrying volumes, and the anti-drift guard in
`rivet-config::schema` ties the published key list to the parser's own.

**Not built:** everything else in `ADR-031` §12 — the constant that becomes a
function of a volume root, the adapter's enumerate-and-match, and the
supervisor's attach-before-converge. **Nothing reads these declarations yet.** A
document may carry a `[volumes]` table today and the machine will parse it,
store it, render it back and act on none of it.

### 7.9 The schema declares its own order, its conditions and a prompt — built 2026-09-13

Plan §5.2 item 1 (`.rulings/` R-162, Wave 3a): what a generated interview
(item 3, a separate build) needs from the schema beyond a value's type.

**Declaration order is a guarantee, not an accident.** `rivet_config::schema::DOCUMENT`
lists the document's top-level keys, in text order, in
`cmd/rivet-provision`'s own interview order — `hostname`, `network`
(`Addressing`/`Address`/`Gateway`), `mode`, `fleet-role`, `timezone`, then
`kiosk` (`Auto-run`), with `version` always first and every key with no
question of its own following. `rivet_config::document::Document::DECLARED_KEYS`
carries the identical order, and a test beside each states it literally so a
later reorder is a one-line, deliberate change rather than a silent drift.

**Conditional rules are emitted as JSON Schema's own `if`/`then`/`else`,** not
only as a property's English description. `network.<name>`'s `method` deciding
`address` and `gateway` (§7.4) becomes one `allOf` entry: `if` names the
deciding property and its value, `then` names what becomes required, `else`
names what becomes refused outright. This does not add a rule — `ADR-007`
§7.4's refusal is unchanged — it republishes the identical rule in a form a
generic form renderer can follow without parsing prose. A table with no
conditional rule publishes no `allOf` at all.

**Every property carries a `prompt` distinct from its `description`.** The
description is written for a reader deciding whether a value is right; the
prompt is the short form an interview or a form asks with. A table (`[kiosk]`,
`[control-plane]`, …) carries one too, for a generated interview asking
whether to configure that section at all.

### 7.10 `mode` and `fleet-role` — built 2026-09-13, and what each composes

Plan §5.2 item 2 (`.rulings/` R-162, Wave 3a). **Addressing** was already a
real schema property before this pass — §7.3's `method`/`address`/`gateway`
is exactly the one-answer-composes-several-keys shape `cmd/rivet-provision`'s
`Addressing` question already used, and §7.9's `if`/`then`/`else` is what it
needed. **`mode` and `fleet-role` were not**: neither had any document
representation, so an interview's `Mode` and `Fleet` answers composed other
keys (`[kiosk]`, `[services.*]`, `[control-plane]`) without leaving a record
of the choice itself. Both are now top-level optional keys.

**`mode`** (`headless`/`kiosk`, `rivet_config::machine_mode::MachineMode`)
records whether this machine drives a screen. **It composes nothing else in
this crate** — that is unchanged — but outside it, `mode` is now the runtime
switch a document naming one decides by (`.rulings/` R-164, R-165):
`cmd/rivet-sysd`'s `document::kiosk_mode` reads `mode` first, and only when a
document names none does it fall back to `[kiosk]`'s presence, which is what
every document written before this key existed still means. `headless` runs
no kiosk even when a `[kiosk]` table is present; the table is configuration
`mode` decides whether to act on, never cleared and never itself a claim that
its mode is active — a headless machine may carry a `[kiosk]` table (or, one
day, some other mode's table beside it) it simply does not use. `mode =
"kiosk"` with no `[kiosk]` table is **tolerated and reported, not refused**: a
kiosk was asked for and nothing configures it, and the machine behaves exactly
as it does today with no `[kiosk]` table while still answering `kiosk`, so the
mismatch is visible rather than silently downgraded to `headless`. `ADR-032`
§8.3 is where `get status` states this for a reader; both it and the
launcher's `offer` answer are `kiosk_mode`'s readers, so they cannot disagree.

**More than one mode's table may be configured at once (R-165), and `mode`
alone picks which is active.** A `[kiosk]` table and, one day, a `[desktop]`
table may each sit fully configured in the same document, and switching modes
is changing one key, never writing or clearing a table. No `desktop` mode is
built.

**`fleet-role`** (`stand-alone`/`server`/`member`,
`rivet_config::fleet_role::FleetRole`) mirrors `rivetos_register::FleetRole`
and `cmd/rivet-fleet/src/compose.rs`'s already-ruled mapping for the identical
register field (`ADR-050` §5.1, `.rulings/` R-159 R21): a `server` or `member`
composes `0.0.0.0:7777` for `[control-plane] listen`
(`Document::effective_control_plane`) and supplies `ADR-050` §5.1's service
set as a default a caller may layer under `[services.<name>].enabled`
(`Document::role_services`) — a `server` names `rivet-fleetd`; either names
`rivet-proxyd` and `rivet-mdnsd`. **A pure default, never an assertion**: both
methods consult `fleet-role` only where the document says nothing explicit,
mirroring `SPEC-05` §5's own words for the register's identical field — *"the
device wins."* An explicit `[control-plane]` or `[services.<name>]` entry is
read exactly as written whatever role is named, so the two can never
contradict. `[applications.rivet-proxyd] upstream-name` is deliberately not
derived here: it needs the fleet authority's own certificate, which this
crate cannot read (§3, no I/O) — `cmd/rivet-provision` and `cmd/rivet-fleet`
still supply it from outside this crate.

**Every existing document still parses, unchanged.** Both keys are optional
and additive; a document written before they existed carries neither and is
read exactly as before — `rivet-config`'s test suite proves this against a
fixture shaped like a real machine's document, never against a real board or
register.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

This document is the one where the record went wrong most often, and the pattern
is consistent: a claim was corrected in the body and left standing in the status
block, or corrected in one section and left standing two hundred lines away.

### 2026-09-13 — §7.4 and §7.5 said `udhcpc` was not integrated, three weeks
after it was

`rivet-config/src/network.rs`'s own module documentation was corrected on
2026-08-30, the day `acquire_lease` started returning a real `Lease` instead of
`Result<Infallible, _>` — but §7.4's supporting fact and the whole of §7.5 here
kept describing the earlier state, and `tools/card/build.sh`'s own comment
still said `DHCP IS NOT BUILT` in as many words. Found auditing that comment
against the code (`PLAN-Feature-Set-2026-09-12` §10). Both sections now say
what `network.rs` already said: the client is built and takes a lease — proven
once on hardware, 2026-08-30, on the board now named `rivet-01` — no board's
live document has used it since, and what is still refused is a card with no
client on it at all — a build-time fact, not a schema one.
`AGENTS.md` §2 rule 4 was written after an audit of this file and `ADR-015`.

### Status block — `/etc` was said to have two generators

**Changed 2026-08-23**, the same day the amendment was made and withdrawn. The
status line was false when it was written:

~~**§3.5 amended 2026-08-23** — `/etc` has two generators, not one; `ADR-008`
§5.3 carries the ruling.~~

### Status block — there was no login shell to put in `/etc`

**Changed 2026-08-23**, when `cmd/rivet` shipped. The superseded sentence read:

~~**A running Rivet now has an `/etc`; it still has no login shell to put in
it.**~~

### In §2 — the Talos comparison's verdict

**Changed 2026-08-28** by `ADR-035`. The row's verdict was reversed rather than
deleted; the comparison was correct and the conclusion was overtaken. The

**Amended and withdrawn on 2026-08-23**, by the maintainer, in one day. The
amendment was implemented and then removed. It read:

~~**Amended 2026-08-23 (`ADR-008` §5.3).** "Generated" is right and "of
applying the configuration" is only half of it: some of what `/etc` holds is a
fact about the *image* rather than about the machine — which binary may be a
login shell, which uid the image's files belong to, what `localhost` means.
Those five files are generated at **build** time by `cmd/rivet-image`, from
constants in `rivet-core`, and they are on the read-only root rather than on
a tmpfs.~~

Why its own argument did not carry it is in the body: the runtime projection
renders `/etc` before anything that reads it starts, so it can render all of it,
and then there is one generator rather than two.

### In §3.5 — Stage 1 was blocked on a login shell as well as the projection

**Corrected later on 2026-08-23**, when `cmd/rivet` was built and installed. The
superseded clause read:

~~but it could not finish without a login shell either, and there is no login
shell.~~

### In §3.5.1 — the document contributed nothing to `/etc`

**Changed 2026-08-26**, when `greeting` and `hostname` both arrived. The
superseded heading was ~~**What the document contributes today: nothing**~~, and
the paragraph under it read:

**What the document contributes today: nothing**, and the code says so rather
than leaving it to be inferred. §7 lists the schema's keys — `version`,
`services`, `network` — and none of them is a fact any of the five files
carries. The three that would be are ~~the hostname (`ADR-025`; there is no
key)~~, human accounts (`ADR-008` §7.3, Stage 4) and ~~the timezone
(`ADR-021`; and `/etc/localtime` is not rendered at all, `ADR-008` §5.3.1)~~.
Rather than invent a `hostname` key to make the argument earn its place — a
schema decision that is the maintainer's — the projection takes a `Document`,
reads nothing out of it, and carries a test that fails the moment
`Document::DECLARED_KEYS` changes, so the question is asked when the schema
grows instead of being missed.

The hostname row closed on 2026-08-26 and the timezone row on 2026-09-02
(`ADR-021` §9). The guard-test mechanism is what produced both, and it is in the
body.

### In §3.5.1 — `/etc/shells` named no shell

**Changed 2026-08-23; struck 2026-08-29**, six days after it stopped being true,
and this document had already said so three times by then. The superseded
It survived because it sat inside a block introduced as *"The original text:"*
and a reader could not tell where the quotation ended — the ambiguity
`AGENTS.md` §2 rule 3 exists to remove. Moving the quotation into this section,
where its boundaries are explicit, is the structural answer to that.

### In §7.2 — `listen` was the operator's key

**Changed 2026-08-24**, ruled by the maintainer; §7.2.1
is the account. The superseded words were ~~`listen` and~~ in the list of
optional keys.

### In §7.2.4 — the memory cap was unsettled on a board, and nothing was installed

**Both changed 2026-08-29**, settled by a board on 2026-08-28. The superseded
sentences read:

~~It settles nothing about a **board** — `ADR-024` §8.1 records the `memory`
controller as *absent* on the Raspberry Pi and §8.2 found the same kernel
offering it under an emulator, so the cap may be refused there;~~

~~And nothing here was **installed**: `ADR-029` §12 item 4's installer does not
exist, so the manifest and the binary were put in place by the harness, unsigned
and unverified, which the machine says on every boot.~~

**`ADR-024` §8.1 was already superseded when the first of those cited it**, by
`ADR-024` §8.2 on 2026-08-23, whose heading is *"§8.1's `MEMCG` hypothesis does
not survive it"*. A citation to a superseded section reads exactly like a
citation to a live one, which is why this survived: nothing here was wrong about
`ADR-024`, it was just pointing at the half that had been struck.

### In §7.6 — the second half of the ruling was not built

**Changed later on 2026-08-23**, when `rivet-store` was created. The heading was
~~What is not finished~~ and the section read:

~~The ruling has two halves and **one of them is not built.**~~

~~`ADR-004` §6 has PID 1 read `rivet.toml` from the boot partition and write
its values into the persistent configuration. `cmd/rivet-initd` performs the
read and reports what arrived; **nothing performs the write**, because
`ADR-014` §2's `rivet-store` — persistence of desired state to
`/state/config` — does not exist. Checked 2026-08-23 two ways, per `AGENTS.md`
§7: it is absent from the workspace member list in the root `Cargo.toml`, and
`git ls-files` finds nothing under that name.~~

~~The consequence, said plainly: **a `[network]` table on the boot partition is
read, validated, counted on the console, and dropped.**~~

Kept because `AGENTS.md` §7 is about exactly this failure — a claim of absence
outliving the absence, corrected in one place and left standing in three others.

### In §7.6.2 — nothing read the stored document

**Closed later the same day, 2026-08-23** (`ADR-017` §4.3). The superseded item
read:

~~**Nothing reads the stored document.** `cmd/rivet-netd` is told a
`--document` path by whoever starts it, and `rivet-sysd` does not start
it. A first boot now *stores* what it was told and
nothing yet *applies* it.~~

### In §7.6.2 — two of three service keys were said to be unhonoured

**Corrected 2026-08-29**, and the superseded sentence was wrong in three
independent ways at once. It is the closest thing in this document to `ADR-015`
§3.2 — confident, load-bearing, and contradicted by a section two hundred lines
above it. It read:

~~**Two of the three keys in §7.2's table are still not honoured**, and they are
refused with an explanation rather than dropped — `listen` needs socket
activation, which is not built, and `memory` needs the cgroup memory
controller, which `ADR-024` §8.1 records as absent on the board.~~

1. **"three keys" is two.** §7.2.1 struck `listen` out of the operator's
   document on 2026-08-24. §7.2.1's own closing sentence says so: *"the
   operator's whole surface is **`enabled` and `memory`**."*
2. **The reason given for `listen`'s refusal is the wrong reason**, and it is
   the more damaging half. `listen` is not refused *pending* socket activation;
   it is refused **permanently**. An operator reading it waits for socket
   activation to arrive and get the key back. It is not coming back.
3. **`memory` is honoured, and is proven on a Raspberry Pi 4.** The citation to
   `ADR-024` §8.1 pointed at a section that had already been corrected, which is
   why nothing downstream caught it.

### In §7.6.2 — none of it had run on hardware

**Struck 2026-08-29; both halves were false.** The superseded bullet read:

~~**None of it has run on hardware**, or under QEMU: the emulator has no boot
partition, so it offers no provisioning document and exercises none of this.~~

This is the bullet a cold agent reads to decide whether first-boot provisioning
works at all, and it said no while a board in the next room had done it.

