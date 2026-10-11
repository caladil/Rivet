# ADR-006: Administration and Configuration Surface

* **Status:** **Accepted** — sections 5, 6 and 7 ruled 2026-08-22. See section
  5.1 for the ruling on the network API, which is narrower than "deferred".
* **§7 is implemented, 2026-08-23** — the operation catalogue and `rivet
  schema`, both generated from the types, and the first administrative operation
  behind them. §7.1 is what was built, §7.2 is the one thing the emitted schema
  cannot express and therefore says out loud, §8.2 is the operation, and §8.3 is
  a question this raised that the record does not answer and that blocks the
  *next* operation. ~~**Nothing here has run on a board** — §8.1's closing
  paragraph is the list of what hardware would still have to confirm, and it
  replaces an "Unproven" paragraph that had gone stale in both halves.
  This line is in the status block deliberately: `ADR-007`'s header carries a
  note about a status line that stayed false while the body was corrected, and a
  reader who stops here should not conclude §7 is still unmet.~~
* **CORRECTED 2026-08-29. It has run on a board, repeatedly, since 2026-08-25**
  — and the struck line above is the *fourth* status line in this repository to
  stay false while its own body was corrected, which is the failure it names in
  its last sentence and then commits. `PLAN.md` §16.2: *"`rivet get schema`
  answers on the board."* §14 installed, ran and updated an application on
  hardware. §19.4 took a `trial-restart` onto the slot an update had just been
  written to. Five paragraphs in this document said otherwise until today and
  each is struck where it stands — §8.3, §8.4, §8.5, §8.8.8, §8.9.9.
  **The citation in the struck line was also wrong**: the hardware list is
  §8.3's closing *"Three things a board would still have to confirm"*, not
  §8.1's, whose closing content is the "Not built" itemisation.
* ~~**Nine operations as of 2026-08-24 — §8.8.** `apply` has five subjects and
  `get` has three.~~ **Sixteen `get` subjects and sixteen `apply` subjects,
  2026-08-29** — and this line was right to say **do not read a count out of
  it**: `rivet --version` publishes the catalogue, generated from the types, and
  is the authority. `tools/docs/claims.py` now derives both counts and fails a
  build that lets this line drift again.
* **§8.9's exclusion of file management is AMENDED — §8.10, ruled by the
  maintainer, Joel Lindell, 2026-08-29.** Delete, rename, create, permissions and
  ownership exist as sibling **subjects**, each with its own plan and its own
  refusals. `apply edit` itself is unchanged and grows nothing. §8.10.3 is the
  full surface, §8.10.13 and §8.10.14 are the two parts of it that are specified
  and **not built** because they need a ruling, §8.10.4.4 is the ruling on the
  application tree — *permit, and inform* — and **§8.10.16 is a claim about
  time-of-check that was written, found false and struck in the same pass.**
  The second confirmation tier these subjects use is `ADR-012` §7.3's, which is
  where it is defined; nothing about it is restated here.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

Rivet is a headless, network-attached system with no graphical surface for the
foreseeable future. Everything an operator or an agent does to it happens over
the network, and for a long time the only interface will be a command line.

Conventional Unix administration is an accident of history: several dozen
configuration file formats under `/etc`, tools whose output is human prose that
must then be scraped, errors that are `printf` strings, and system state that
can only be discovered by interrogating each subsystem in its own dialect. It
works because humans are good at pattern-matching sloppy text. It is close to
the worst possible interface for a program — or an agent — to operate.

Rivet is being designed from nothing, so it does not have to inherit that.

### 1.1 The stated intent

**The maintainer's requirement is that the system should feel like it was
*designed* — not like a collection of configuration files in an `/etc` directory
structure that has evolved and mutated in several directions over fifty years
into a mess.** *(`.rulings/` R-014.)*

This is a functional requirement, not an aesthetic one, and it is worth stating
what it actually demands — because "designed" is otherwise the kind of word that
decorates a document without constraining anything.

A configuration surface that has been designed rather than accreted has
properties that are individually testable:

1. **One representation.** A given setting exists in exactly one place, in one
   format, expressed in one vocabulary. Not a file plus an override directory
   plus a command that mutates it invisibly.
2. **One vocabulary.** The same concept has the same name everywhere it appears
   — in the document, in the CLI, in log output, in error messages. `AGENTS.md`
   section 10.2 exists to enforce this.
3. **The whole state is visible at once.** An operator can read what the machine
   is supposed to be without interrogating nine subsystems in nine dialects.
4. **Nothing is discovered only by failing.** The surface describes itself
   (section 7) rather than requiring the reader to already know it.
5. **Changes are previewable.** The consequence of an edit can be seen before it
   is committed (section 6).

Each of those is a design decision that has to be made *before* there are
twenty settings, because every one of them is cheap at zero call sites and a
migration at twenty. That is the whole argument for this ADR being phase-0
rather than Stage 3.

The historical `/etc` is not the result of bad decisions. It is the result of
fifty years of individually reasonable local decisions taken by people who could
not coordinate, each adding one more format. **The way to avoid the outcome is
not to try harder at each decision; it is to make the coordination structural**
— one document, one schema, one core that owns every operation.

**The concern is a phase-0 concern even though most of the functionality is
not.** Where code lives, what shape output has, and whether errors carry types
are structural properties. They cost nothing to establish now and cannot be
retrofitted, because retrofitting them means rewriting every call site.

### 1.2 Prior art: what the Windows Registry got right

It is tempting to treat `/etc` as the only model and design against it. The
Registry is the more instructive comparison, because it is the counter-example:
walled garden, opaque, widely disliked — and **still structurally coherent
thirty years on**, which is precisely what `/etc` is not.

That is worth taking seriously rather than dismissing, because it isolates the
variable. The Registry's coherence and the Registry's opacity are *not the same
property*, and a design can take one without the other.

| Property | Registry | `/etc` | Rivet |
| :-- | :-- | :-- | :-- |
| One namespace | yes | no | **yes** |
| One access API | yes | no — one parser per file | **yes** |
| Typed values | yes | no — everything is text | **yes** |
| Declared schema | **no** | no | **yes** |
| Layered scopes with defined precedence | yes (HKLM/HKCU) | no | **yes** — `ADR-007` §3.4 |
| Change notification | yes | no | later |
| Human-legible without tools | **no** | yes | **yes** |
| Diffable, version-controllable | **no** | yes | **yes** |
| Provenance of a value | **no** | no | **yes** — `ADR-007` §3.4 |

**What to steal:** one namespace, one API, typed values, layered scopes. Those
four are what produced thirty years of coherence, and none of them required the
binary format.

**What to reject:** opacity. The binary hive was a 1990s performance decision,
orthogonal to everything in the column above it, and it is what makes the
Registry impossible to diff, review, grep, or put under version control — all
of which Rivet needs.

**And what neither model has:** a declared schema. This is the deeper reason the
Registry *is* in fact a mess, just an invisible one — it types its values but
not its structure, so anyone may write any key anywhere and nothing can say
which keys are dead. Its coherence is the interface's, not the content's.
`/etc` has the same disease with the additional handicap of fifty formats.

The conclusion is the load-bearing one for this ADR: **centralisation is what
produced the Registry's coherence, and centralisation is available in a text
format.** There is no trade to make here. The mess in `/etc` is not the price of
transparency.

---

## 2. The failure this is written to prevent

The default trajectory, which happens by itself if nothing prevents it:

1. A CLI is written first, because it is what is needed to make progress.
2. Its logic lives in the CLI, because that is the shortest path.
3. Later an API is wanted, and it either re-implements the logic or shells out
   to the CLI and parses its human output.
4. The two drift, and the system now has two answers to every question.

**The rule that prevents it:** no administrative logic exists in any interface.
Interfaces are adapters over a core that owns every operation as a typed thing.

This is not a new architecture. It is `AGENTS.md` section 3 — ports and adapters
— applied to the administrative surface. It is therefore almost free
structurally, provided it is done before there are call sites to move.

---

## 3. Decision: one core, several faces

```
   rivet-cli ────┐
   rivet-api ────┼──►  rivet-core   ──►  adapters out
   (future MCP) ───┤     every admin           (netlink, supplicant,
   (future GUI) ───┘     operation as a         procfs, image store)
                         typed operation
                         NO I/O, NO rendering
```

* **`rivet-core` owns the operations.** Every administrative action is a typed
  request producing a typed result or a typed error.
* **Every interface is an adapter containing no logic.** The CLI parses
  arguments and renders results. That is all it does.
* **The gate is the dependency graph**, not convention. `rivet-core` does not
  depend on the CLI crate, the rendering crate, or any transport, so a violation
  fails to compile rather than requiring a reviewer to notice. This is gate 3 in
  `AGENTS.md` section 3, and it is the only one that survives a hundred commits.

The payoff, stated concretely: **a future MCP server is one thin adapter, not a
project.** If it is anything more than that, the boundary was drawn wrong.

### 3.1 Co-equal, not merely possible

**The maintainer's formulation, 2026-08-22, and it is stronger than the one
above: the boundary makes an Admin GUI, an Admin TUI and an Admin REST client
all *co-equal citizens*.** *(`.rulings/` R-015.)*

"Co-equal" is a sharper claim than "possible", and it is worth holding to
because the weaker reading is how the failure in section 2 actually happens. It
never begins with someone deciding the CLI is privileged; it begins with one
operation that is only reachable from the CLI because that was where it was
needed first.

So the rule, stated as a property rather than an intention:

> **Every operation in `rivet-core` is reachable from every adapter. An
> operation that exists in only one interface is a defect in that interface, not
> a feature of it.**

### 3.2 This is testable, and should be tested

The operation catalogue of section 7 is generated from the core's types. That
makes co-equality a **conformance property** rather than an aspiration: an
adapter can be checked against the catalogue, and one that cannot reach a
catalogued operation is reported as incomplete.

That is the difference between "we intend the adapters to stay equal" — which
survives about a hundred commits, per `AGENTS.md` section 3 — and a check that
fails.

### 3.3 The honest asymmetry: capability versus cost

Co-equality holds for **what an adapter can do**. It does not hold for what an
adapter costs to secure, and pretending otherwise would be its own dishonesty.

| Adapter | Transport | Authentication |
| :-- | :-- | :-- |
| CLI, over `cargo rivet run` | the control plane, `ADR-035` | solved — mutual TLS, pinned Ed25519 keys |
| MCP | the same listener | solved — the same pins |
| a browser console | `rivet-proxyd` on 443, `ADR-046` | solved — mutual TLS against the fleet authority |

**Every adapter now costs the same to secure**, which was not true when this
section was written: a browser console once meant inventing transport, identity
and authorisation from nothing, and that asymmetry is what deferred it. `ADR-035`
built the first two and `ADR-046` added the browser's chain, so a new adapter is
a thin adapter rather than a new security story — which is what section 3's
boundary was for.

What remains genuinely asymmetric is **authorisation**, and the record should
not claim otherwise: both transports authenticate a caller and neither
distinguishes one caller's authority from another's.

---

## 4. Decision: structured output is the primary form

**Every command produces structured data. Human-readable text is *rendered
from* that structure, never produced instead of it.**

* Default behaviour is TTY-detected: a terminal gets the human rendering, a pipe
  gets the machine form.
* `--output json|text` forces either.
* **`--output json` is not a feature bolted onto a text tool.** If the human
  rendering and the structured form can ever disagree, the rendering is reading
  from the wrong source and that is a defect.

### 4.1 Errors are typed and stable

Every failure carries a stable machine-readable code, a structured payload, and
a human string — in that order of authority. An agent branches on the code; a
human reads the string. A failure that can only be distinguished by matching on
English prose is not a failure that can be handled programmatically, and this is
where almost every existing administrative tool fails.

Error codes are part of the interface contract and are not renumbered.

### 4.2 Consequence for the honesty rules

`AGENTS.md` section 3 requires that stubs refuse rather than fake success. A
typed error surface is what makes that enforceable rather than aspirational: an
unimplemented operation returns a specific, catalogued `Unimplemented` variant
that no caller can mistake for a result.

---

## 5. The control plane is the transport; the CLI is the remote API

The instinct on hearing "network-friendly administration" is to build a REST
API. This was deferred, for a reason that was never about effort:

```
cargo rivet run --host <host>:7777 --identity <dir> --pin <hex> -- get net status
```

That is an authenticated, encrypted, remote procedure call returning structured
data. `ADR-035`'s listener solves transport, authentication and key management
once, for every adapter; a REST API alongside it would have been a second
answer to all three.

So:

* **The CLI is the network administration interface**, reached over the control
  plane, and it is designed as an RPC surface rather than as a human
  convenience that happens to be scriptable.
* **A browser console was the obvious candidate for the thing this could not
  serve, and it exists** — `ADR-046`'s path proxy, which reuses the same fleet
  authority rather than inventing an identity system. Section 3's boundary is
  what made it a thin adapter.

### 5.1 Ruled, 2026-08-22: deferred, with a named destination

The maintainer ruled **deferred — but explicitly not "maybe someday"**. **He
wants to administer the box from his Windows 11 desktop over the REST API, so
the API is not a toy for the AI: one day he wants to sit at that desktop with a
dashboard application reading the state of his two or three Raspberry Pis, able
to push updates and change configuration. None of that is day one — a fully
working operating environment comes first, before remote management is worried
about at all.** *(`.rulings/` R-016.)*

Two things follow, and the second is a new requirement.

**1. The API is a destination, not an option.** The deferral is about
*sequencing*, not doubt. Nothing may be designed in a way that forecloses it,
and "we might never need this" is not available as a justification. The test in
section 3.1 applies with full force: when the API is built, it must be a thin
adapter over `rivet-core`. If it is more than that, the boundary was wrong.

**2. Rivet is a fleet system, not a single-machine system.** This had not been
stated before and it is not a small addition. A dashboard reading the state of
several machines and pushing updates to them implies:

* **Device identity** — each machine is addressable and distinguishable.
* **Shared configuration with per-device differences** — 2–3 machines that are
  mostly alike and deliberately not identical.
* **Remote image updates**, which is `ADR-008`'s A/B mechanism driven from
  outside the machine.
* **State aggregation** — one view over several machines' observed state.

The encouraging part is that the third and fourth need nothing new, and the
second is **already the layering model of `ADR-007` section 3.4**: a *fleet*
layer slots between image defaults and machine configuration, with the same
precedence rules and the same per-value provenance. Leaving room for it costs
nothing now; discovering the need after values exist is a migration.

This must be settled alongside the configuration model — see `docs/PLAN.md`
§4.4.

---

## 6. Proposed: one desired-state document, and a plan/apply model

This is the recommendation with the largest consequences and the one most worth
arguing about.

**Rivet has one configuration document describing the desired state of the
machine.** Not a directory of formats. One typed, schema-backed document, owned
by the project — which is exactly the division `AGENTS.md` section 8.4 already
requires, where the project owns the schema and Ignition owns the mechanics.

Administration is then not a sequence of imperative commands but a diff:

```
rivet plan     # compute and show what would change; mutate nothing
rivet apply    # make the machine match the document
rivet export   # emit the document that reconstitutes this machine
```

### 6.1 Why this is the answer to "AI-friendly"

An agent driving imperative commands cannot verify what it did, cannot preview
consequences, and cannot roll back. An agent editing a declarative document and
running `plan` **sees the full consequence of a change before committing to
it**, and the diff is machine-checkable rather than prose to be interpreted.

That single property does more for safe automated administration than any amount
of API surface. It also happens to be what makes the system safe for a *human*
operating it at midnight.

### 6.2 The invariants it requires

* **No hidden state.** Anything configurable is visible in the document. There
  is no operation that changes the machine in a way `export` cannot see. This is
  the invariant that makes the whole model trustworthy, and it is checkable:
  `export`, reflash, `apply`, compare.
* **Every operation is idempotent.** Agents retry. An operation that is unsafe
  to repeat is a defect.
* **Every mutating operation supports a dry run** returning the same typed diff
  `plan` would.

### 6.3 What it composes with

The model aligns with decisions already taken and one anticipated:

* First-boot provisioning from the boot partition (`ADR-004` section 6) is the
  same document arriving by a different route.
* An immutable root filesystem with A/B image updates — anticipated, not yet an
  ADR — is the natural substrate for atomic apply and rollback.

**Ruled 2026-08-22: adopted, from the start.** The desired-state model shapes
the configuration types, and the preview-before-change property is a stated
goal rather than an eventual nicety. `ADR-007` is ruled in to the extent that it
depends on this; its remaining open questions are the document format and the
reach of the runtime-override layer.

---

### 6.4 One vocabulary, enforced rather than intended

Borrowed from PowerShell's approved verb list, which is the best existing
solution to a problem every CLI has (`ADR-005` section 5.2).

Administrative commands are drawn from a **closed set of verbs** — `get`, `set`,
`list`, `add`, `remove`, `apply`, `plan`, `export`, `explain` — and a noun
vocabulary that matches the configuration schema exactly. Not "usually matches";
the nouns *are* the schema paths.

The value is not tidiness. It is that a reader who knows one command can predict
the others, and an agent can construct a valid command it has never seen from
the schema alone. A surface where `net status` and `service list` and
`image show` all mean "tell me about" is a surface that must be memorised
instead of understood.

The verb set is part of the interface contract (section 4.1) and is extended
only by amending this ADR.

### 6.5 Dry run is a property of the framework, not of each command

Also from PowerShell, where `-WhatIf` is supplied to every command by the
framework rather than implemented by each one.

Section 6.2 requires that every mutating operation support a dry run. That
requirement is only credible if it is structurally impossible to omit:
`rivet-core` defines a mutating operation as a type that **computes its diff
separately from applying it**, so a command that cannot be dry-run does not
compile rather than merely being undocumented.

This is the same discipline as the dependency-graph gate in section 3 — the
build refuses, rather than a reviewer noticing.

#### 6.5.1 BUILT 2026-08-26, and the associated type is the whole of it

`rivet-core::operation::plan`. `PLAN.md` §28.5 Phase 1 item 3; the cost of not
having it is §22.3.

**What it replaced.** Seven `apply` subjects supplied §6.2's dry run *by hand*,
and each put its plan in the human `message` of an `E_CONFIRMATION_REQUIRED`
refusal. §8.9's own closing list said so. What that cost is stated in §8.9.1's
words, pointed at the plan instead of at the command: *an agent can branch on
the code and re-issue the command but cannot read the diff structurally* — and
`ADR-012` §3 makes plan-then-apply the **safety** mechanism rather than the
friendly one, so an agent asserting *this is the change I intended* against a
prose sentence was doing string matching on the safety mechanism.

**How "does not compile" is made true.** A mutating operation implements
`Change`, and the separation is an associated type:

```rust
type Prepared;
type Applied;
fn plan(&self, board) -> Result<(Plan, Self::Prepared), OperationError>;
fn apply(self, prepared: Self::Prepared, board) -> Result<Self::Applied, OperationError>;
```

**`apply` cannot be called without a `Prepared`, and the only thing that produces
one is `plan`.** An operation that skipped its dry run would have to conjure a
value of a type it does not construct. `resolve` is the one driver, and it plans
on **both** paths.

Four consequences, each recorded because each is a change rather than a
restatement:

1. **The interlock now runs before the act it guards.** Every subject had
   separately argued for running its `can_*` check on the plan path; none ran it
   on the confirmed path, so a confirmed change was the one path the interlock
   never covered. It is the driver's property now and no subject argues for it.
2. **The human string is generated from the plan.** `Plan::narrative` is the only
   writer of a confirmation message in this workspace. A typed diff beside a
   hand-written sentence would be two accounts of one change, and the sentence is
   the one an operator reads.
3. **§4.1's structured payload is filled in.** `OperationError` carried a code, a
   subject and a message; it carries the `Plan` now, in the second slot. Nothing
   constructs an `E_CONFIRMATION_REQUIRED` any other way: `Plan`'s `From` is the
   only caller of the private method that sets that code, and the two public
   constructors carry a `debug_assert!` that fails any test which tries. **That
   is weaker than the associated type above and is stated as such** — the
   assertion is compiled out of a release build, so what it buys is that the
   suite refuses, not that the compiler does.
4. **`Operation` still has no `mutating` field**, and §8.1's reason is unchanged
   — it would be a claim about a distinction the type system does not make. The
   distinction is made one level down instead: a mutating operation is one that
   implements `Change`, and `explain` and `get` cannot.

**Still not built:** §6's *desired-state* apply — `rivet apply` with no subject,
*make the machine match the document*. Nothing is stubbed for it.

#### 6.5.2 `requires-reboot` is a field of the plan, and it is three states

`ADR-034` §5.3 rules it *"not optional"* and names the hazard from `PLAN.md`
§0.0.4: **a configuration that is wrong in a way indistinguishable from success
until the next boot.**

**`TakesEffect` is `Immediately`, `OnNextRestart`, or `NotAsserted { because }`,
and the third state is the one that had to be argued for.** A `bool` cannot carry
the answer honestly, because two of the seven subjects cannot make it:

* **`apply edit`** writes a path the *operator* chose. Its safety story is
  §8.9.2's replace-exactly-once and §8.9.4's reserved deny-list, not a schema, so
  nothing in this build knows what reads the file it changed. `false` would claim
  the change is live; `true` would claim a restart is needed. Both are assertions
  this build cannot make, which is the omission §5.3 exists to stop.
* **`apply install`** has not read the payload — deliberately, §8.8 — so whether
  it is a system update (inactive slot, needs a trial restart) or an application
  (unpacked, not started) is not known.

`NotAsserted` carries **why**, and `requires_reboot()` answers `Option<bool>` so
that a caller cannot read *not asserted* as *no*. Two states would have sufficed
for the other five; making these two lie to keep the type small is the trade that
was refused.

The same type is a field of `rivet_config::schema::Property` — §5.3's *"a
property of the setting, in the schema"* — so the plan, the emitted JSON Schema
and the fleet register read one fact through one type. See `ADR-034` §5.3.1.

## 7. Proposed: the surface describes itself

An agent operating a system it has not seen before needs the system to be able
to explain itself. Two mechanisms, both cheap if the types are the authority:

* ~~**`rivet schema`**~~ **`rivet get schema`** emits the JSON Schema of the
  configuration document, generated from the types rather than maintained beside
  them. **Respelled 2026-08-25 — §7.4.**
* **An operation catalogue** enumerating every administrative operation with its
  inputs, outputs and error codes, likewise generated.

If these are generated from the same types the core uses, they cannot go stale.
If they are hand-maintained documents, they will be wrong within a month and
worse than absent, because they will be trusted.

**Ruled 2026-08-22: required from Stage 1**, generated from the types. Neither
document may be hand-maintained; a generated artifact that cannot go stale is
the whole point, and a hand-written one would be trusted and wrong.

~~**Neither is built, 2026-08-23** — see §8.1. `rivet --version` reports the
operation catalogue as an **empty list**, which is the true count and the one
thing about it that cannot be wrong, and publishes the error codes of §4.1 from
their own definition. `rivet get schema` does not exist; `rivet-config` owns the
schema and nothing generates a JSON Schema from it yet.~~

**Both are built, later on 2026-08-23.** Struck rather than deleted, per
`AGENTS.md` §2 and §7 — a claim of absence that outlives the absence is this
repository's most expensive recurring defect, and the empty-catalogue sentence
had already been quoted into three other files.

### 7.1 What was built, and where the authority for each sits

| | Lives in | Generated from |
| :-- | :-- | :-- |
| The operation catalogue | `rivet-core::operation::CATALOGUE` | the `const OPERATION` each operation's own module declares |
| `rivet --version`'s `operations` field | `cmd/rivet` | `rivet_core::operation::catalogue()` — a projection, not a list |
| `rivet get schema` | `rivet-config::schema` | the parser's own `DECLARED_KEYS` and `RULE` constants |
| The verb vocabulary of §6.4 | `rivet-core::operation::Verb` | moved out of `cmd/rivet` with the first operation |
| §4.1's error codes | `rivet-core::operation::ErrorCode` **and** `cmd/rivet::refusal::Code` | see §7.3 |

**"Generated from the types" is a property with tests rather than a claim.**
Three of them are worth naming here because they are what makes this section met
rather than gestured at:

1. **A catalogued operation is reachable from every adapter.** §3.2 says
   co-equality is "a conformance property rather than an aspiration"; the test
   walks `CATALOGUE` — not a list written in the test — and drives each entry
   through `argv`. An operation the core catalogues and an adapter cannot reach
   fails there.
2. **A catalogued output is a field the result carries.** A catalogue naming a
   field the operation does not produce is the smallest available form of
   "trusted and wrong".
3. **The schema's published required-ness and types are the parser's
   behaviour.** For every declared key, a document with it, without it, and with
   a value of the wrong type is fed to `rivet-config`, and the schema is
   checked against what the parser did.

Each of the three was verified by breaking it: emptying the catalogue, adding a
declared output nothing produces, and marking `version` optional each fail
exactly the guard above and nothing else (`AGENTS.md` §4 — a regression test that
cannot fail proves nothing).

### 7.2 The emitted schema is necessary and not sufficient, and it says so

JSON Schema can express *this key is a string*. It cannot express **a gateway is
on the interface's own subnet**, and `rivet-config` performs five refusals of
that kind — `ADR-007` §7.3's four and §7.7.2's key structure.

**A sixth — §7.4's `method = "dhcp"` beside a static `address`/`gateway` —
joined this list until 2026-09-13 and left it.** Plan §5.2 item 1
(`.rulings/` R-162) asks that a conditional rule appear in the emitted schema
in a form a generic renderer can follow; `ADR-007` §7.9 is where it landed —
`if`/`then`/`else`, republishing the identical refusal rather than inventing
a new one.

A schema silent about the remaining five would tell an agent its document was
valid when the machine will refuse it, which is this section's own failure
mode arriving from a new direction. So the emitted document carries the list
on its root
`description` and names `rivet-config` as the authority (`ADR-011` §2's second
clause). A test asserts each of the five really is a refusal, so the clause
cannot come to describe refusals that no longer happen.

**One limitation, stated rather than discovered: nothing in this workspace can
validate the emitted document *as JSON Schema*.** There is no validator here and
adding one is a dependency decision — `rivet-config`'s `Cargo.toml` already
records that whether the schema types carry serde derives is the maintainer's
call. The tests assert the document's structure and its agreement with the
parser; they do not assert that a third-party validator reads it as intended.
`AGENTS.md` §4: a round trip against one's own writer proves the two agree and
nothing about the format.

### 7.3 Two error-code vocabularies, and why that is not two authorities

§4.1 makes the codes part of the interface contract. Four of the five that
`cmd/rivet` declared were an *operation's* failures rather than that binary's,
and they moved to `rivet-core` with the first operation. Two stayed:
`E_NOT_A_SHELL` and `E_NO_INTERACTIVE_SHELL` describe a login session, and
putting them in `rivet-core` would be the `core → anything CLI-shaped` edge
`ADR-014` §3 forbids.

**No code was renamed, renumbered, or changed its exit status** — what changed is
where each is *defined*. `ADR-011` asks for one authority per fact, not one enum
per system (§6 of that ADR is explicit), and the fact here is *what this code
means*. A test asserts the two sets are disjoint, so no code can acquire a second
definition; `rivet --version` publishes the union, each half read from its own
definition.

`E_UNKNOWN_SUBJECT` is new, and it is new because an operation that can refuse a
subject now exists.

---

### 7.4 `rivet schema` became `rivet get schema` — ruled 2026-08-25

**The maintainer asked whether the shape was right and it was not.** §7 above
wrote the command as a bare word, and `cmd/rivet`'s parser implemented it as
one, beside `--help` and `--version`, on the reading that §6.4 governs
*administrative commands* while §7 is *the surface describing itself*.

**Three things retired that reading.**

**1. It broke on `explain`.** That operation performs no I/O, reads no
filesystem, and answers identically on every machine running a given build — it
is a property of the build, exactly as a schema is — and it is a verb, in the
catalogue, first-class. So *"describes the build rather than the machine"*
cannot be what separates a verb from a bare word.

**2. It was undiscoverable, and that was measured rather than argued.** §7's
other half is the operation catalogue, and a board asked for its own catalogue
answered:

```
operations: apply, explain, get
```

`schema` is not in it. A machine reading the catalogue to learn what it could
do would never find the command — and `ADR-012` makes an agent a first-class
operator. **A surface with invisible exceptions cannot be driven from its own
description**, which is the one thing §7 exists to provide.

**3. It is not a command.** The maintainer ruled on the grammar: `measure` is a
verb and it names the thing being done, where `schema` did not.
*(`.rulings/` R-017.)*

`rivet measure hash` reads as verb-then-object and stays a bare word. A bare
noun as a whole command does not.

**And on a fleet it is genuinely a per-machine fact**, which the old reading
missed: `ADR-018` §4's A/B slots and staggered updates mean two boards can run
different builds and accept different documents. *Ask this machine what it
accepts* is the useful question; *what does the tool describe* stops being one
the moment there is more than one machine.

#### What changed, and what deliberately did not

**§6.4's closed verb set is untouched.** `schema` is a **subject of `get`**, not
a tenth verb — which is the whole reason for choosing an existing verb. §6.4
says the set "is extended only by amending this ADR", and this amendment does
not extend it.

**The result is now an object.** `rivet schema` emitted the JSON Schema
document bare; `rivet get schema` emits `{"subject": "schema", "schema": {…}}`.
Every other result on this surface is an object whose keys the catalogue
declares, and a subject answering with a foreign document would be exactly the
exception this move deleted. **It is a wire change**: a pipeline wants
`| get schema` on the end.

**The old spelling refuses by name**, rather than falling through to "unknown
verb" — it is the one form certain to be typed again — and it is a **refusal,
not an alias**. An alias that works for ever is how a surface acquires two names
for one operation and a catalogue that describes one of them.

#### `measure` was considered and deliberately left alone

Ruled in the same breath, and not as an endorsement: the maintainer is **not
convinced `measure` is something the project will never revisit**, but it is at
least a verb and it does name the thing being done. It stays a bare word.
*(`.rulings/` R-017.)* It is still absent from the catalogue, which is
the same defect in a smaller place, and `ADR-029` §7.2's measurement — the
reason it exists — was taken on a board on 2026-08-25.

---

## 8. What this ADR commits to at phase 0

Deliberately structural, not functional. Each item is free now and expensive
later, and none of them is a feature:

1. **`rivet-core` exists and owns every administrative operation as a type**,
   before the second interface exists to argue with the first.
2. **The dependency graph is the gate.** The core cannot compile against a
   transport or a renderer.
3. **The first command written emits structured output** and renders from it.
   Establishing this after ten commands means rewriting ten commands.
4. **The first error type is a typed, coded enumeration**, not a string.
5. **There is one configuration document with a schema**, from the first line of
   configuration read — including Stage 0's boot-partition provisioning.

Everything else in this ADR — plan and apply, the network API, self-description,
a GUI — is deferred functionality that these five make cheap.

### 8.1 What `cmd/rivet` implements, 2026-08-23 — and what it does not

The CLI was created ahead of anything to administer, for a reason that is not
§8's: **a login needs a binary to name in `/etc/shells`**, because
`getusershell()` is a membership test and an account whose login shell is not
listed there is rejected before authentication. `cmd/rivet-image` refuses to
name a binary the image does not contain, so the constant was `None` and
`/etc/shells` would have permitted nothing at all. This section exists so that
"the CLI exists" is never read as "the administration surface exists".

**Built:**

| | Against |
| :-- | :-- |
| The login-shell contract — `argv[0]`'s dash, `-c`, and silence on stdout | measured on hardware, `cmd/rivet`'s own tests |
| Structured value primary; text rendered *from* it; `--output json\|text`, TTY-detected | §4 — commitment 3 |
| Typed, coded, stable error enumeration, published by `rivet --version` | §4.1, §4.2 — commitment 4 |
| §6.4's closed verb vocabulary, ~~recognised and refused by name~~ **read from `rivet_core::Verb`, which owns it since 2026-08-23** | §6.4 |
| **The operation catalogue and `rivet get schema`, both generated** | §7.1, respelled §7.4 |
| **`rivet explain <path>`, the first administrative operation — in `rivet-core`, adapted here** | §8.2 |
| **`rivet apply trial-boot <a\|b> [confirm]`** — the first operation that changes a machine | §8.4 |
| **`rivet apply power-off\|restart\|trial-restart [confirm]`** — the only way anything on a Rivet board can stop it | §8.5 |
| **A `/proc/1/comm` interlock**, in `cmd/rivet`'s `shutdown` module: nothing is signalled unless pid 1 is `rivet-initd`, and an unreadable `/proc/1/comm` refuses | §8.5.2 |
| **`rivet get log [boot] [limit]`** — `ADR-022` §9's journal query | §8.6 |
| **`rivet get storage`** — how full every mounted filesystem is, over `ADR-014` §2's `rivet-adapter-system`, which this is what created | §8.6 |

**Not built, itemised so that no absence has to be inferred:**

* ~~**Every administrative operation.** All nine verbs answer
  `E_UNIMPLEMENTED`. `rivet-core` exposes no operation type, and §3 forbids
  the CLI from growing the logic instead — that is §2's named failure exactly.~~
  **One is built, later on 2026-08-23 — see §8.2.** The other eight still answer
  `E_UNIMPLEMENTED`, and they now do so because `CATALOGUE` has no entry for
  them rather than because a branch in the CLI says so.

  ~~**Two verbs and five operations by the end of 2026-08-23** — `explain`, and
  `apply` with four subjects (§8.4, §8.5). Seven verbs still answer
  `E_UNIMPLEMENTED`.~~ **Three verbs and seven operations by the end of
  2026-08-24** — `get` gained two subjects (§8.6). **Six** verbs still answer
  `E_UNIMPLEMENTED`. The count is corrected here rather than in the sentence
  above because the sentence above was true when it was written, twice — and it
  has now gone stale three times in two days, which is the argument for reading
  `rivet --version` rather than this paragraph. **The catalogue is generated;
  this is prose.**
* ~~**`rivet schema` and the operation catalogue** (§7)~~ — **both built, see
  §7.1.**
* **Plan and apply** (§6), the desired-state document, and §6.5's dry-run
  framework. ~~Unchanged: every operation that exists is read-only.~~
  **Amended later on 2026-08-23 — see §8.4.** The framework is still not built;
  the sentence about every operation being read-only is no longer true.
* ~~**Anything reachable from only one adapter.** Vacuously true today~~ —
  **no longer vacuous, and now a test.** §3.2's conformance property walks the
  catalogue from `cmd/rivet` and drives each entry through `argv`.

### 8.2 The first operation, 2026-08-23: `rivet explain <path>`

`ADR-007` §3.6.1 item 3, which §3.6.2 moved from *proposed* to *required* on
2026-08-22 and which `rivet-core::etc` recorded as unwritten:

> A reverse lookup, so the question can be asked before the mistake rather than
> after it: `rivet explain /etc/resolv.conf` returns the field it is generated
> from, that it is read-only, and the command to change it.

It answers for the eight files the runtime `/etc` projection renders (~~five~~ until `/etc/motd` arrived 2026-08-26; corrected 2026-08-29, and `tools/docs/claims.py` derives it now; ~~eight~~ until `/etc/localtime` arrived with the `timezone` key on 2026-09-02, §9 below — found by `claims.py`, which is the whole of what deriving it buys. It went to ten and back to nine the same day: `ADR-039`'s display window was built into this table and the maintainer asked whether it needed an `/etc` projection at all (`.rulings/` R-266) — **`/etc` is a compatibility surface for C runtimes that demand an exact path, and no C runtime asks for a display schedule.** It publishes to `/run/rivet` and appears in `generated::GENERATED_PATHS` instead, which is why the count below did not move; ~~nine~~ until `/etc/motd` left again on 2026-09-13, when `[greeting]` itself was removed outright rather than kept alive for documents that no longer exist — `.rulings/` R-168.1), from
`rivet_core::etc::ETC_FILES` — one list, read both by the projection and by
the lookup. For `/etc/passwd`, `/etc/group` and `/etc/shadow` it is the **only**
way that provenance can be read at all: those three formats cannot carry a
comment that musl's parsers are known to skip, so they carry no provenance
header (`rivet-core::etc`).

**Why this operation and not a more interesting one.** §8.1's list above says
the first operation must land in `rivet-core`, and `ADR-014` §3's forbidden
edges then decide which one *can*:

| Candidate | Why not first |
| :-- | :-- |
| `get <schema path>` — read the configuration | `ADR-014` §3.2 makes `rivet-config` depend on `rivet-core` **permanently**, so `Document` is not a type the core may name. See §8.3. |
| anything over observed state | the kernel is the authority (`AGENTS.md` §5) and `ADR-014` §2's `rivet-adapter-system` does not exist |
| `plan`, `apply`, `set` | §6.5's dry-run framework is not built and is a larger ruling |

What is left is the class of fact `rivet-core` is *itself* authoritative for,
and `/etc`'s definitions are the largest of them.

**It performs no I/O**, and that is the right answer rather than a shortcut: the
question is *what generates this path*, which is a property of Rivet rather
than of a particular boot, so a path can be explained from a workstation before
the mistake. The consequence is stated in the module rather than left to be
found — **`explain` does not tell you whether the file is there.** Whether a
file was written is the supervisor's journal (`ADR-022`), and conflating the two
would give a running machine's fact a second authority.

#### 8.2.1 It stopped knowing only `/etc` — 2026-08-25

**The scope above was a starting point and was read as a boundary.** `explain`
answered *"Rivet does not generate `<path>`"* for everything outside
`ETC_FILES`, and that sentence is a **claim** rather than a shrug. It was false
for everything Rivet writes outside `/etc` — the stored configuration under
`/state/config`, the journal's own files, the path a deployment request lands
on — so the operation whose whole purpose is *"ask before the mistake"* was the
thing telling an operator to make it, in the places where a hand-edit is
silently overwritten by the next boot.

**The paths live in two tables, and this operation owns neither:**

| Class | Declared in |
| :-- | :-- |
| the `/etc` projection | `rivet_core::etc::ETC_FILES` |
| everything else Rivet writes | `rivet_core::generated::GENERATED_PATHS` |

`Explanation` gains one variant per table, each **borrowing** the table that owns
the fact. That is `ADR-011` §2 structural rather than remembered: a new generated
path becomes explainable with no change here, and the coverage test iterates the
tables rather than a list maintained beside them. Both tables already carried
path, mode and reason as data for the supervisor, and copying either would have
been `ADR-011` §3's second spelling.

**Two fields changed shape**, and both changes are `AGENTS.md` §5 rather than
convenience:

* **`kind` is a closed vocabulary** (§6.4) of five words — `projection`,
  `directory`, `machine state`, `record`, `mechanism` — plus `/etc`'s own `view`.
  Each names a distinct thing that happens to a hand-written change, which is
  what the operator is actually asking.
* **`mode` may be a sentence.** For a path whose mode `rivet-core` does not
  own, the answer says so rather than inventing a number or omitting the key —
  an omitted key reads as *"this has no mode"*, which is false of every
  directory in the table.

**`writable: false` is explicitly not a claim about the kernel**, and the
catalogue now says so. `/state` is writable and root is root; the field means the
write will not survive, or will not do what it looks like it does. §8.9.10 is
where that distinction stopped being academic.

### 8.3 A question this raised that the record does not answer

**Where does an operation over the configuration document live?**

§3 says `rivet-core` owns every administrative operation. §6.4 says the nouns
*are* the schema paths, so `rivet get network.eth0.address` is the shape the
surface is designed around. But `ADR-014` §3.2 ruled on 2026-08-23 that
`rivet-config` depends on `rivet-core` and that "Cargo forbids a cycle, so
**this decides the direction permanently**: nothing in `rivet-core` may ever
depend on `rivet-config`."

Those three cannot all hold. `ADR-014` §3.2 anticipates the collision and names
the remedy — *"if a genuine need ever appears for core to hold a document, the
answer is to move the shared types down — not to reverse this edge, which cannot
be done"* — but it treats it as hypothetical, and it is not: it is the next
operation.

**Three shapes, none of them taken, because this is the maintainer's call:**

| | What moves | Cost |
| :-- | :-- | :-- |
| 1 | `Document` and the schema types move into `rivet-core`; `rivet-config` keeps the parser and serialiser | the largest move, and it puts the schema in the crate `ADR-014` §2 describes as holding "the desired-state model, layering, provenance" — which is arguably where §2 already put it |
| 2 | The document-shaped operations live in `rivet-config` and the catalogue composes two sources | a second place operations live, which is §2's named failure wearing a different coat |
| 3 | A fourth crate below both holds the shared types | a crate whose only job is to be depended on |

`ADR-014` §3.2's own sentence points at 1. It is recorded here rather than acted
on because reversing a permanent edge is exactly the kind of decision
`AGENTS.md` §1 says is the maintainer's.

**One thing it must never grow.** An interactive prompt. ~~`ADR-005` §5.1 rules
`nushell` the interactive login shell and **explicitly rejects** the Talos model
of making `rivet` the shell with no general shell at all.~~ A prompt here would
foreclose a ruled decision by satisfying its requirement badly, so an
interactive invocation refuses and says so.

**AMENDED 2026-08-28 — `ADR-035`.** The Talos model is accepted and `nushell` is
removed in `ADR-035`'s phase 4. **The prohibition stands and is strengthened**:
with no general shell on the machine, a prompt here would be the *only* one, so
growing it would recreate the thing phase 4 removes rather than merely
duplicating something that already exists. What changes is the refusal's
wording, which may no longer point at `nushell`.

**And the reason this section gives is now the load-bearing one.** `ADR-035`
§9's precondition for removing `nushell` is that the CLI's *read* surface must
first cover what a shell was doing for diagnosis — which is this section's
co-equality rule (§3.1) applied to reads. A capability reachable from a shell
and from no adapter is the same defect as one reachable from the CLI alone.

**What has run on a machine.** The catalogue, `rivet get schema` and `explain`
all ran on a Raspberry Pi on 2026-08-25 — `PLAN.md` §12.5 (*"`rivet explain` on
the board now answers `kind: projection`, `writable: false`"*) and §16.2
(*"`rivet get schema` answers on the board"*). Both were cross-compiled
`aarch64-musl` binaries, and the schema is the largest single value this binary
ever puts on stdout, so both were worth confirming separately.

**What a board has still not confirmed**, and a green suite does not imply it:
**that the paths `explain` describes are the ones a running machine actually has
under `/etc`.** The lookup answers from the definition and never looks —
deliberately, §8.2 — so the definition being right about a board is a separate
claim, and `AGENTS.md` §5 says the machine is the authority for it. **Reaching
the board proves the pipe, not the definition**, so neither run above closes it.
The count of paths is derived by `tools/docs/claims.py` rather than written here,
because a count written into prose is the thing that rots first.

---

### 8.4 The second operation, later on 2026-08-23: `rivet apply trial-boot <slot>`

**The first operation that changes a machine**, and therefore the first place
§6's plan/apply model and §6.5's dry-run rule had to be met by something rather
than deferred. `ADR-018` §5.3.1 carries the mechanism and its two safety
interlocks; what belongs here is the administrative shape and what it costs.

**Why `apply` and not a new verb.** §6.4's verb set is closed and extended only
by amending this ADR, and the catalogue is keyed by verb. `ADR-012` §7.1 is
titled *confirmation before apply*, and `ADR-028` §4 classes an update as
*"`ADR-006` §6 already requires a `plan` the principal confirms. An update is
that, not a special case."* So the record puts this under `apply` twice.

**What that forecloses, said rather than discovered.** §6 spells the
desired-state apply as bare `rivet apply` — *"make the machine match the
document"* — and that operation, when it lands, will have to **share** this verb
rather than replace it. The shape chosen keeps that cheap: `subject` is a
declared `Input` whose only accepted value today is `trial-boot`, so a second
subject is a match arm rather than a second catalogue key. **Whether that is the
right trade is the maintainer's**, and the alternative — keying the catalogue by
verb *and* noun — is an amendment to §6.4 that `AGENTS.md` §1 puts out of an
agent's reach.

**Confirmation, and the two things about it that are honestly incomplete.**

* `ADR-012` §7.1 makes unattended apply *"a scope on a credential (§5), never a
  default and never a flag anyone can simply pass"*. **Rivet has no
  credentials and no scopes**, so there is no way to grant it and the operation
  offers none.
* §6.5's framework — a mutating operation as *"a type that computes its diff
  separately from applying it"* — **does not exist**. The operation supplies the
  dry-run property **by hand, for itself**: without the confirmation word it
  reads the card, runs both interlocks, changes nothing, and refuses with
  `E_CONFIRMATION_REQUIRED`. That is §6.2 met and §6.5 unmet, and the difference
  matters — §6.5's whole point is that the property should be impossible to omit.
* **The plan travels as prose.** `OperationError` carries a code, a subject and a
  message, so an agent can branch on the code and re-issue the command but cannot
  read the diff structurally. That is the concrete cost of §6.5 being absent, and
  inventing a diff type to fix it would be inventing the ruling.

**Confirmation is a second invocation rather than a prompt**, because §8.3's
closing paragraph forbids a prompt and the operator's session is a single
non-interactive request carrying one command, which has no way to answer a
question.

**Three error codes were added to §4.1's contract**, appended rather than
inserted: `E_CONFIRMATION_REQUIRED`, `E_UNSAFE` and `E_MACHINE_REFUSED`. Each
names a different next action — say yes, stop, or look at the hardware — which
is §4.1's own test for whether a code earns its place.

**The first domain port.** `AGENTS.md` §3.1 predicted this: every port in the
repository was an adapter's own I/O seam, and *"the first operation that must
read something … is the one that will need a domain port, and that port goes in
`rivet-core`."* It is `rivet_core::operation::machine::Machine`, and it names
only `Slot` — nothing about mounts, errnos or `tryboot`. `perform` takes one, so
there is a single entry point and therefore a single list of operations, which is
§3.1's co-equality.

~~**What is unproven.** Everything. Nothing here has run on a board,~~
**FALSE since 2026-08-26 — struck 2026-08-29.** `PLAN.md` §34.2 carries
`apply trial-boot b` run on the hardware, unconfirmed, with the board's own plan
JSON — and its `from: slot a` is *"the firmware's own answer, read off the FAT
boot partition"*, which is precisely the column QEMU cannot emulate. What
remains true is the framing: `ADR-026` §4
puts `tryboot` and `/proc/device-tree/chosen/bootloader` in the column QEMU does
not emulate at all, and ~~`ADR-018` §5.3.1 records that a Rivet board cannot
currently perform the restart the arming needs — so the operation can be armed
and the trial cannot yet happen~~ — **struck later the same day; §8.5 is the
restart.** The first half stands: nothing here has run on a board.

### 8.5 The third, fourth and fifth operations, later on 2026-08-23: `apply power-off`, `restart`, `trial-restart`

**The three ways to stop a Rivet machine**, and until they existed there were
none: `ADR-017` §3.1.5 item 6 gives shutdown to pid 1, §3.1.10 gave pid 1 three
signals to be asked with, and **nothing in the image could send any of them.**
There is no shell and no `kill`; a board was stopped by removing power, and a
power cycle does not carry `ADR-018` §5.1's one-shot flag. §8.4's arming could
therefore be performed and never exercised.

**Why `apply` again, and not `reboot`.** §6.4's verb set is closed and *"is
extended only by amending this ADR"*. `reboot`, `shutdown` and `halt` are not
among the nine, and `AGENTS.md` §1 puts adding a tenth out of an agent's reach.
`ADR-012` §7.1 — *confirmation before apply* — is the same argument §8.4 used,
and it applies more strongly here: stopping a machine is the least reversible
thing this surface does.

#### 8.5.1 One catalogue entry, four subjects, and the two things it costs

The catalogue is keyed by **verb** (§6.4's nouns are values, not keys), so `apply`
has one entry and four subjects behind it: `trial-boot`, `power-off`, `restart`,
`trial-restart`. §8.4 predicted the shape and priced it — *"a second subject is an
added match arm"* — and three arrived at once for that price. What §8.4 did not
foresee is that the subjects would not all have the same **shape**, and they do
not:

| Cost | What it is | Who sees it |
| :-- | :-- | :-- |
| **The second positional input is polymorphic** | `apply trial-boot b confirm` puts the slot second; `apply power-off confirm` puts the confirmation second, because a stop takes no slot. So the `slot` input is declared optional, and its description says which subject requires it. | An **agent** reading the catalogue. An operator never sees it — each command reads naturally — which is the wrong way round for §7. |
| **The declared outputs are a union** | No result carries all ten fields. Each subject's result is a *subsequence* of the declared list, in the declared order, and that property is a test rather than a sentence. | An agent that reads §7's outputs as *"these are the fields you get"*. |

**The second cost is paid — §8.7, 2026-08-24.** The union is generated from a
subject table and the catalogue publishes the subjects; the sentence below stands
as the reasoning of the day and is superseded on that half only.

**Both are the maintainer's to overturn, and the overturn is the same one:** key
the catalogue by verb **and** noun. That is an amendment to §6.4, and §6.4 says
so about itself. The alternative fix for the second cost — outputs *per subject*
— is a change to `Operation`'s type and therefore to §7. Neither is an agent's.

**What was gained by not doing it**: `rivet --version` still publishes one
generated catalogue with one entry per verb, `operation(verb)` is still total,
and no adapter has to learn a second lookup. `rivet_core::operation::apply` is
the dispatcher and carries this argument at the code.

#### 8.5.2 The interlock, and why it is an administrative decision rather than a detail

**`kill(1, SIGTERM)` on a development host stops that host.** `cmd/rivet` runs
on the board *and* in a checkout. So the operation refuses unless `/proc/1/comm`
is exactly `rivet-initd`, refuses when it cannot be read, and refuses on the
**plan** path as well as the confirmed one. `ADR-017` §3.1.10.1 carries the full
argument, including why `/proc/1/cmdline` is quoted and not trusted.

It belongs in this ADR because it is the first time §4.1's code vocabulary has
had to carry *"this is the right command on the wrong machine"*. It is
`E_UNSAFE` — §4.1's test is whether a code names a different next action, and the
next action here is the same one an interlock always names: **stop, and run it
where it belongs**. `E_MACHINE_REFUSED` is reserved for the case where pid 1 *is*
Rivet's and the `kill(2)` failed, which is a hardware answer.

#### 8.5.3 A result means "delivered", and says so

`Ok` from `request_stop` means a signal reached pid 1. It does **not** mean the
machine stopped, and the result never says it did — `rebooted` is a constant
`false` for every subject, as it already was for the arming.

**And the answer probably does not arrive.** The request arrives over the network
(§5), pid 1 stops the supervisor first, and the supervisor stops the control
plane — which is the thing carrying the answer back. The
`next` field says exactly that, names the console line pid 1 will print, and tells
the operator the console is the authority. That is §4.2's honesty rule applied to
an operation whose own output stream is one of the things it shuts down.

~~**What is unproven.** Everything about a board. `SIGTERM`, `SIGINT` and `SIGUSR1`
have been delivered only to fakes;~~ **FALSE since 2026-08-25 — struck
2026-08-29.** A real signal reached a real `rivet-initd` on a Raspberry Pi:
`PLAN.md` §19.4 — *"`rivet apply trial-restart confirm` → the board came back on
`root=PARTLABEL=system-a`, the slot the update was written to"* — and again on
2026-08-28, `PLAN.md` §73.2. **The second half stands and is the half that
matters:** the interlock's *refusal* has been exercised
against a real `/proc` on a development host, which is the only half a
development host can settle. **`power-off` in particular has still never been
sent to a board**, for the obvious reason that a headless Pi that powers off
stays off.

#### 8.5.4 Stopping one service is not one of these

**All three subjects here act on the whole machine.** Stopping one application
is `apply service-stop <name> confirm`, released by `apply service-start <name>
confirm` — a hold the supervisor keeps until it is released or the machine
reboots, ruled and specified in `ADR-017` §6.5. The two share no word with the
three machine stops, for the reason `service-restart` shares none with
`restart`: a truncated or mistyped per-service command must not land on one
that stops a board. They reach the supervisor over its socket, not pid 1, so
§8.5.2's interlock is not theirs; the supervisor's own refusals are.

---

### 8.6 The sixth and seventh operations, 2026-08-24: `rivet get log` and `rivet get storage`

Both are `get`, both read a **machine fact**, and the second is the first
operation in this catalogue to read one off the *running* system rather than off
a card or out of a const table. §8.2's table recorded that gap in as many words:

> | anything over observed state | the kernel is the authority (`AGENTS.md` §5)
>   and `ADR-014` §2's `rivet-adapter-system` does not exist |

`rivet-adapter-system` exists now, because `get storage` needed it, which is
`ADR-014` §5's rule for when a crate is created rather than a decision taken
separately.

#### 8.6.1 Why `get` grows subjects instead of the verb set growing

`ADR-022` §9 writes `rivet log` and `ADR-005` §4.1.3 says *"a `get` subject
beside `log`"*. §6.4's verb set is closed and the first of those two spellings is
not in it, so both are `get` with a noun — the same answer §8.4 and §8.5 gave for
`reboot` and `shutdown`, for the same reason: **`AGENTS.md` §1 puts adding a
tenth verb out of an agent's reach.**

The consequence is §8.5.1's two costs, paid a second time by a second verb, and
worth restating because the second instance is what turns an observation into a
pattern:

| Cost | How it lands on `get` |
| :-- | :-- |
| **The positional inputs are polymorphic** | `boot` and `limit` are `log`'s; `storage` accepts neither. It **refuses** a second word rather than ignoring it, because `rivet get storage /state` is what a `df` habit types and an operator shown the whole machine would read it as the answer for `/state`. |
| **The declared outputs are a union** | Thirteen fields, of which `log` carries ten and `storage` carries six. Each is a subsequence of the declared order, checked rather than asserted. |

**Two verbs now pay this**, and the overturn is unchanged and still the
maintainer's: key the catalogue by verb **and** noun, which §6.4 says is done by
amending this ADR. ~~The cost of not doing it is now two dispatchers and two
unions rather than one, and it will be three when `set` arrives.~~ **Struck
2026-08-24 — §8.7**: it is two dispatchers and *no* unions, because the union is
now generated from a subject table. `set` will arrive with one dispatcher and
nothing to maintain.

#### 8.6.2 `get storage` takes no path, and that is the same wart declining to be paid twice

`df /some/path` is the obvious shape and it is deliberately absent. A `path`
input would have to occupy the position `log`'s `boot` already holds, which is
§8.5.1's polymorphic-input cost for a *third* time — and it answers a question
the result already answers: every record carries `holds`, naming which of
Rivet's own locations (`/system`, `/state`, `/state/config`, `/state/log`) that
filesystem is the one holding, by longest mount point.

**`holds` exists because `/system` is not a mount point.** `ADR-008` §3's table
reads as though it were; `cmd/rivet-image` builds the slot's `erofs` as the
whole root filesystem with `/system` as a directory inside it. A reader looking
for a `/system` row would find none and conclude the operation had missed it.

#### 8.6.3 Nothing is filtered, and absent is not zero

`df` hides pseudo-filesystems from a list compiled into it. This does not: a
hidden filter is how *"is `/state` full?"* becomes unanswerable on the one
machine where the filter is wrong (`AGENTS.md` §5), and two of the ones `df`
would hide — `/run` and `/tmp` — are tmpfs a runaway service genuinely fills.

Where a filesystem has **no** such number, the field is **omitted** rather than
reported as `0`. `crate::value` has no null, so absence is how a value says it
has none; `/proc` reporting `capacity-percent: 0` would be an answer about a
filesystem with no capacity to use.

**That choice has a measured cost**, measured against the emulator on
2026-08-24: nushell's `select` errors on a column absent from any row, so
`select capacity-percent` fails and `select capacity-percent?` is the form that
works. The catalogue entry carries that incantation, which is §7's job. It is the
right way round — a pipeline that errors gets fixed, and a `0%` gets believed.

#### 8.6.4 `uutils` was read and is not linked

`ADR-005` §4.1.5 rules out the dependency (45 crates and a localization engine)
and §4.1.6 rules the alternative: *"where Rivet's implementation is derived from
`uutils`, the source file says so and names the module it came from."* Three
files carry that notice. What was taken is narrow and specific — the mount
table's octal unescaping, `statvfs`'s field semantics including `f_frsize`
falling back to `f_bsize`, and the `df` arithmetic:

> `used` is `blocks - blocks_free`, **not** `total - available`, because a
> filesystem reserves blocks that are neither in use nor available; and the
> percentage is of `used + available`, not of `total`. A default `ext4` reserves
> 5%, so the naive form reports a freshly made filesystem as 5% full for ever.

That arithmetic is in `rivet-core`, not in the adapter, because it is the
domain's judgement — which is also what makes it testable with no I/O at all
(`ADR-014` §3.1).

#### 8.6.5 What ran, and what it settles

**It ran on a machine**, in the emulator `tools/qemu/run.sh` boots:
`/system/bin/rivet get storage` answered with ten filesystems, `unreadable: 0`,
and the nushell pipeline `ADR-005` §4.1.1 exists for produced a table.

**What that settles**, per `ADR-026` §4: the mount-table read, the parse, the
`statvfs(2)` seam through musl on `aarch64`, the arithmetic, `holds`, and the
structured output surviving into a `nu` pipeline without being parsed.

**What it does not settle, and it is the half the name suggests.** Under `-M
virt` there is no `/dev/mmcblk0`, so PID 1's `/state` mount refuses and `/state`
is a directory on the **initramfs**. The numbers are RAM. The answer says so
rather than hiding it — all four named locations appear under `holds` on the `/`
row, and `/` is `rootfs` — but **no card's free space has been measured by this,
and `ADR-018` §10's real question is still open on hardware.**

One finding came out of running it on a development host rather than a fake: a
`9p` filesystem reported 999 inodes and 1 000 000 free. The first version
saturated the subtraction and printed `inodes-used: 0` beside them, which is an
answer and a false one; `inodes-used` is now omitted when the machine's own two
numbers do not support it. `statvfs` is not a contract a filesystem driver has to
honour, and §4.2's honesty rule decides what to do when one does not.

---

### 8.7 Half the wart is paid, 2026-08-24: the union is generated and §7 gains a subject table

§8.5.1 and §8.6.1 each priced **two** costs of keying the catalogue by verb
alone, and each said the fix for the second one — *outputs per subject* — was *"a
change to `Operation`'s type and therefore to §7"* and therefore the
maintainer's. **It was ruled and built on 2026-08-24.** The first cost, the
polymorphic positional input, is unchanged and still stands.

#### 8.7.1 What changed in §7's shape

`Operation` no longer carries a flat `outputs` list. It carries **`results`**:

* `Results::Same(&[Field])` — one shape whatever the arguments. `explain`.
* `Results::PerSubject(fn() -> Vec<Subject>)` — **the first input is a subject
  and its value selects the shape.** Each `Subject` is a name and the field list
  that subject's result carries.

The published catalogue gains a `subjects` array — each subject with its own
outputs, with its own exact descriptions — and that array is now **the
authority**. The flat `outputs` an adapter has always published is *generated*
from it.

**A function rather than a `&'static [Subject]`**, and the reason is the whole
point of the change: `apply`'s three stop subjects come from `Stop::ALL`, which
is already the one place those three words are written. A `const` table would be
a second place, which is what this removes.

#### 8.7.2 What was actually wrong, said plainly

The union was **written by hand**, in two places, and each was guarded by a drift
test in every module that fed it — `apply`, `get`, `log`, `storage`, `trial`,
`stop`. That is `AGENTS.md` §7's staleness in its purest form: a derived fact
maintained by a person, with a test standing over it because everyone knew it
would drift. The tests were not the problem; **needing them was.**

The generated union is assembled twice over:

**The order is a merge, not a concatenation.** Each subject's list is walked
against what has been placed so far — a field already there advances a cursor, a
new one is inserted *at* it. What comes out contains every subject's list as a
subsequence, which is exactly the property the hand-written list was maintained
to have. Two subjects that order the same pair of fields differently produce no
valid union at all, and that is refused loudly rather than papered over with a
duplicate name.

**Each description names who carries the field.** Carried by every subject in the
same words → those words. Otherwise, each distinct description prefixed by the
subjects that give it. So `apply`'s `subject` now publishes

> `` `trial-boot`: What was applied. `power-off`, `restart`, `trial-restart`: Which stop was requested. ``

which the hand-written union could not say at all — it had one sentence for four
subjects and had to be vague to stay true. **The generated catalogue is strictly
more informative than the one a person maintained**, which is not the usual
direction for this kind of change and is the reason to record it.

#### 8.7.3 What this closes and what it does not

**Closed:** the second cost in §8.5.1's and §8.6.1's tables. §8.6.1's *"two
dispatchers and two unions rather than one, and it will be three when `set`
arrives"* is now **two dispatchers and no unions**, and `set` will arrive with a
subject table and nothing to maintain.

**Not closed:** the polymorphic positional input. `apply trial-boot b confirm`
still puts the slot where `apply power-off confirm` puts the confirmation, and
that is still visible to an agent reading the catalogue and invisible to an
operator. The overturn is unchanged and still the maintainer's: key the catalogue
by verb **and** noun, which is an amendment to §6.4.

**Also not closed:** `Field` still describes a *top-level* result field and has
no way to describe a nested one, which `get storage`'s `filesystems` records need
and get by prose. That is §7's shape too, and it is a separate question.

---

### 8.8 The eighth and ninth operations, 2026-08-24: `rivet apply install` and `rivet get install`

**Ruled by the maintainer in conversation, 2026-08-24: `apply install` requests and returns; `get install` reports.** One
noun, two verbs: `apply` asks, `get` reports.

> **`rivet apply install <path> confirm` hands over the path, answers, and
> returns — before a byte is written.** … The write then proceeds without the
> operator's session holding it, which is the reason the maintainer's 2026-08-24 ruling
> made the installer a separate supervised program in the first place: *an
> update writes 640 MiB, and doing that from the login shell means a remote
> session owns a long write.* **An answer that waited would hand the hazard
> straight back.**

**The ruling is stated as it was made, and the operation's ARGUMENT has since
changed.** Since 2026-09-12 it is `rivet apply install <digest> confirm`: the
word after the subject is a BLAKE3 digest, resolved in the machine's own
artifact store, and there is no form of the operation that names a path
(`ADR-052` §7.2). Everything the ruling decided is untouched — the operation
still hands the artifact over, answers, and returns before a byte is written,
for exactly the reason quoted. What changed is *what may be handed over*, and
the shape below is unaffected.

#### 8.8.1 It is `stop`'s shape, and that is the ruling rather than a convenience

§8.5's three stop subjects deliver a request to another process and return.
This does the same, so it carries the same fields and the same warning:
`requested: true`, `installed: false`, and a `next` saying in plain words that
the request was **delivered** and not **completed**. §4.2 is what makes that
enforceable rather than polite — *a request that was not delivered is a refusal,
never a result with a `false` in it* — and the maintainer's reason for reusing
the shape is that an operator then needs only one picture of how the whole verb
behaves. *(`.rulings/` R-264.)*

The difference from a stop is worth stating because it is the one an operator
will feel: a stop's answer races the shutdown that is about to close the
session, and this one races nothing. It returns immediately, and the install
then takes minutes.

#### 8.8.2 The operation does not open the payload, and that is load-bearing

It checks no signature, no digest, no product, no architecture, and **not even
that the file exists**. `ADR-029` §11's argument decides it:

> three formats means three verifiers each with a third of the scrutiny

A check written here would be a *second implementation* of reading the
container, reaching a verdict the installer reaches again with other code.
`ADR-028` §2 puts that out of bounds, and the weaker of two verifiers is the one
that eventually gets believed.

**What the rule forbids is a second implementation, not a second call**, and
since 2026-09-12 the difference is built rather than theoretical: the artifact
store's door in `cmd/rivet-mcpd` verifies every artifact before it is stored, by
calling `rivet_core::package::verify` — the installer's own implementation
(`ADR-052` §7.2.3, `ADR-028` §2's table of where it is consulted). This
operation still does not open the file, because the verdict that governs is the
installer's, reached against the trust store as it is when the install runs.

**The consequence reaches the operator rather than staying in an ADR**: the
plan says the store verified the artifact on arrival, that this operation has
not read inside it, and that the installer checks it again — with, for a system
update, its product, architecture and platform; `next` says that verdict will
appear in the journal.
A payload the installer refuses therefore arrives as a *plan* and then a
refusal a moment later — which is a surprise worth printing rather than
discovering.

#### 8.8.3 What the plan may say about a slot, and what it must not

An `.rvt` carries three kinds (`ADR-029` §11) and only a system update has a
slot. Which kind this one is cannot be known without reading it — which §8.8.2
forbids. So the plan reports the slot an update *would* land in, from
`running_slot()`, and says the answer applies only if this turns out to be one.
On a machine that cannot read its firmware the field is **omitted rather than
guessed**, and that is *not* a refusal: an application install works perfectly
well on a machine with no device tree, and refusing would make the command
unusable everywhere but a board.

#### 8.8.4 Two refusals the CLI does own

| Refused | Why it is not pedantry |
| :-- | :-- |
| the payload word is `confirm` | `rivet apply install confirm` names nothing to install. Taking it literally would hand the installer a file called `confirm` and report a delivered request for a command that said nothing |
| a **relative** path | the program that performs the install is a different process with a different working directory. A relative path resolves somewhere neither the operator nor the CLI chose, and it resolves to *something* often enough that the refusal would name a path nobody typed |

#### 8.8.5 `get install` reads the journal and nothing else

`ADR-011`. The installer is a separate process that has usually already exited;
what it did is `ADR-022` §3's records under one §5 correlation identifier, and
those records are the account. A status file or a marker on `/state` would be a
second answer to *what happened*, and the two would disagree about an install
that was interrupted — which is precisely the case somebody would be asking
about.

**It groups by correlation identifier, not by boot.** Two installs in one boot
is an ordinary thing to do, and a reader following one chain must not have the
other spliced into it. That grouping is also why this is an operation rather
than a nushell pipeline over `get log`: it depends on the order the journal was
appended in, which a shell cannot recover afterwards (`ADR-021` — a Pi has no
RTC, so timestamps do not order records).

**An unfinished chain is never reported as running.** An installer that was
killed, or a machine that lost power mid-install, leaves a chain that never
ends. The authority for what is *running* is the supervisor's cgroup
(`ADR-017` §3.1.3) and this operation cannot reach it, so `finished: false`
says only that no last record was found — and the `next` field says exactly
that, because reading it the other way is how somebody waits an hour for a
process that died with the boot.

#### 8.8.6 A stable token beside the sentence, and why this ADR is where it is argued

`get install` has to answer *did it finish, and how*. Until 2026-08-24 the only
thing that said so was the English in the last record's `msg`, and matching on
that is §4.1's argument inverted:

> Every failure carries a stable machine-readable code, a structured payload,
> and a human string — in that order of authority. **An agent branches on the
> code; a human reads the string.**

So `cmd/rivet-deploy`'s terminal record now carries an `outcome` field with a
closed set of six values, defined once in `rivet_core::deploy`. **It is a name
for what the last line already said** — no record was added or removed, and
nothing anywhere describes the *middle* of an install, because a stage
vocabulary would be a protocol and `AGENTS.md` §1 puts that out of an agent's
reach.

A token this build does not know is reported **verbatim** and left unexplained,
which is what tells a reader that a newer Rivet wrote the record rather than
that the install did something strange.

#### 8.8.7 §8.5.1's first cost, paid a third time

The polymorphic positional input lands again, in both verbs:
`apply install /state/x.rvt confirm` puts a *path* where
`apply trial-boot b confirm` puts a slot and `apply power-off confirm` puts the
confirmation; `get install` takes no second word where `get log` takes two.
§8.7.3 left this open and it is unchanged and still the maintainer's — key the
catalogue by verb **and** noun, which §6.4 says is done by amending this ADR.
The second cost stays paid: both subjects' output shapes are declared beside
their implementations and the union is generated.

#### 8.8.8 What has run, and what has not — `AGENTS.md` §6

~~**`apply install` cannot succeed on any machine**, and that is the honest state
of the system rather than a defect. The maintainer's 2026-08-24 install-request ruling settled the
*shape* of the command and left the **channel** — how the path gets from
`cmd/rivet` to `cmd/rivet-deploy` — unruled. `ADR-029` §14.4 recorded that
gap in the same paragraph as the ruling that created the binary, and
`cmd/rivet/src/deploy.rs` refuses on every platform, naming what is missing
and where the options are written down.~~

**Superseded 2026-08-24 evening**, and struck rather than reworded because
`AGENTS.md` §7 is about this exact sentence. The maintainer ruled the channel —
the maintainer's 2026-08-24 ruling — and `apply install` now **delivers**: it writes a
request naming the payload onto `/state`, returns before a byte is written, and
the supervisor takes it up on its next convergence pass (`ADR-017` §3.2.2).
`cmd/rivet/src/deploy.rs` is the writing half.

**What that changes about §4.2's shape, which is nothing:** the answer is still
`requested: true, installed: false` with a `next` that says *delivered*, not
*completed*. `Ok` from the channel means the request is on the card and nothing
more — the same contract `request_stop` carries, and for a sharper reason, since
what happens next takes minutes and writes 640 MiB.

**`get install` works.** It was run on Linux against a real
`/state/log/journal.jsonl` holding two installers' chains — one refused, one
completed — and reported the second, with the first's warning correctly counted
against the first rather than swept into the answer. Both renderings were
compared, which is §4's own requirement.

What that settles: the journal read, the parse, the grouping, the outcome
token, both renderings, and every refusal on the argument path. What it does
not: ~~**nothing here has run on a Raspberry Pi**~~, and ~~no install has been
requested through this command on any machine, because it cannot be~~ — **an
install has now been requested, under QEMU on `ADR-024`'s own kernel, and the
installer ran and refused for want of a trust store the emulator does not carry
(`ADR-017` §3.2.2). No payload has been installed through this route on any
machine, and the writing half has never touched a real `/state`.**

> **The first clause is struck 2026-08-29, and the way it survived is the whole
> lesson.** This paragraph was corrected once, on 2026-08-24 — the second clause
> was struck and replaced, carefully and in the house style — and the clause
> beside it, in the same sentence, was left alone. `AGENTS.md` §7: *"When
> correcting one, grep for the claim, not the paragraph."* Here it was not even
> the paragraph; it was the clause.
>
> It has run on a Raspberry Pi since 2026-08-25. `PLAN.md` §14.2: *"That refusal
> was also the first time `cmd/rivet-deploy` ever executed on a Raspberry Pi,
> and it exercised the whole channel doing it: `apply install` wrote the
> request, the supervisor **adopted it and created a service instance that did
> not exist at start-up**, the installer read the machine's own trust store,
> verified the signature, and refused on entitlement."* And the third clause is
> false too: `PLAN.md` §19.4 installed a 683 MB payload through this route on a
> board and rebooted onto it.

---

### 8.9 The tenth operation, 2026-08-24: `rivet apply edit`

**Ruled by the maintainer in conversation, 2026-08-24.** A sixth subject of
`apply`, structured and non-interactive, and it is `ADR-012`'s ADR that needed
it rather than this one — see `ADR-012` §8.

```
rivet apply edit /state/home/root/.config/nushell/config.nu \
  --expect '$env.config.show_banner = true' \
  --replace '$env.config.show_banner = false' confirm
```

#### 8.9.1 The gap, stated as it was put

`ADR-012` §2's table says an agent needs structured output, typed errors, a
discoverable surface and plan-before-apply. Every row was true of every operation
in this catalogue and **none of it was true of changing a file**, because there
was no operation for it. There is no `sed` in this image, no `cat`, no editor,
and `rivet` is the login shell, precisely so nothing else
can be run. An agent asked to change a line had exactly one move: improvise in
nushell.

```
open f | str replace "banner = true" "banner = false" | save -f f
```

That has **no atomicity** — the pipeline truncates before it writes — **no check
that it matched what it believed**, no refusal on a projected view, and **no
record that it happened**. That is not a first-class operator; that is
improvising.

#### 8.9.2 The safety primitive is replace-exactly-once

**It refuses if `--expect` matches zero times, and it refuses if it matches more
than once.** That single rule is the whole safety story: an agent physically
cannot blind-clobber a file, because to change anything it must state what it
believes is there and be correct.

**Ambiguity is a refusal, never a guess.** Two matches is not "the first one";
it is a request this operation cannot perform, and the refusal names the line of
every match so the next attempt can be specific. `AGENTS.md` §5's *refuse rather
than guess* bites harder here than anywhere else in this catalogue, for the
reason that section gives: a wrong guess does not fail. It succeeds, and writes
the wrong line.

Zero and many are deliberately **different** refusals, because the next action
differs — and the zero case says so when the *replacement* is already present
exactly once, which is the state an agent reaches by retrying. `apply edit` is
**not idempotent and cannot be**: a second run has no way to tell *already done*
from *the file is not what you think*, so it refuses and says both. That is a
real departure from §6.2's idempotence invariant, and it is the honest one.

#### 8.9.3 Two operations, and the bar for a third is deliberately very high

**AMENDED 2026-08-29 by §8.10, and read that section before this one.** What
follows still stands **for `apply edit`** and is not struck: this operation grows
no third form. What §8.10 amends is the accompanying exclusion of delete, rename
and chmod — which are now sibling **subjects**, each with its own plan and its
own refusals, exactly as *"each is a different act with a different plan"* says.
Ruled by the maintainer, Joel Lindell, 2026-08-29.

**Two things `apply edit` did gain the same day, and neither is a verb.**
`--accept`, which is `ADR-012` §7.3's acknowledgement and part of the
confirmation model rather than of what this operation can express — it exists
here only because §8.10.4.4 made an application's program editable, and that is
the one path where a change stops the machine working *later*. And a **diff in
the plan** (§8.10.7), which is output: a plan saying *"3 bytes will change"* is a
much weaker safety mechanism than one showing the change, and an agent whose
`--expect` matched the wrong one of two similar stanzas cannot tell from a byte
count. **Nothing about what an operator may ask for has changed**, which is the
test this section's rule actually applies.

`--expect`/`--replace`, and `--append`. **This must grow refusals rather than
expressiveness.** Every extra verb — insert-at-line, delete-matching, regular
expressions, replace-all — is a step towards being `sed`, and `sed` is precisely
the tool this exists to avoid: it has no notion of what its caller believed was
there. A literal is what an agent can be *correct* about; a pattern is what it
can be *nearly* correct about, and the two failure modes are not comparable.

**Creation is allowed**, ruled explicitly. A new file is mode **0600** and a new
directory **0700** — private first, widened deliberately, never the reverse.
Missing parents are created and **every one is named in the plan**, so a
creation reads as intent rather than as a silent `mkdir -p`.

#### 8.9.4 What it refuses, and the two that are the same kind of refusal

| Refused | Because | Code |
| :-- | :-- | :-- |
| A **view** — anything `rivet explain` answers for, and the whole of `/etc` | `ADR-007` §3.6.2, §3.6.6 | `E_UNSAFE` |
| A **mechanism path** — `/state/deploy-request` and its temporary, `/state/log`, `/state/app` | the file is consumed by a mechanism; use the mechanism | `E_UNSAFE` |
| A file that is **not UTF-8** | a text edit on bytes that are not text is a guess about an encoding | `E_UNSAFE` |
| A file **larger than 1 MiB**, or text **larger than 64 KiB** | it reads the file whole in the login shell, and a change that size is a file rather than an edit | `E_UNSAFE` / `E_ARGUMENTS` |
| A path that is **relative**, or carries a **control character** | `install`'s reason, and: a plan and a journal record are both lines | `E_ARGUMENTS` |
| `--expect` matching **0 or more than 1** | §8.9.2 | `E_UNSAFE` |
| `--append` text that would **continue a last line with no newline after it** | the text is added exactly as given and nothing is inserted, so the two would join; joined onto a comment, the text vanishes into it while the document still parses | `E_UNSAFE` |

**The append refusal is general, not limited to a trailing comment.** Joining
an appended text onto an unterminated last line is almost never what was meant,
and the one case where it is — continuing a line — is said unambiguously with
`--expect`/`--replace`. Detecting comments would need each file type's comment
syntax and would still let a join corrupt a file this build cannot parse.
Inserting the missing newline instead would be a guess, and the operation adds
exactly what it is given. So the refusal names both unambiguous spellings:
begin the text with a newline, or change the last line in place. It does not
quote the line, because this operation also appends to secret files.

**The mechanism-path refusal is NOT a security boundary and says so in the
refusal.** Same root, same filesystem: anything that can run this command could
write the file with nushell's `save`. It is the same *class* of refusal as a
view — *this file is consumed by a mechanism, use the mechanism* — and claiming
otherwise would be inventing an authority. `ADR-028` §4.5 makes the identical
point about the install request's principal.

The reserved set is deliberately **short**, and two absences are recorded rather
than left to be inferred. `/state/config` is not reserved, because it is the file
an agent most wants to change and §6.4's `set` and `apply <document>` do not
exist — reserving it would leave no way to change it at all. `/state/clock` is
not reserved because `rivet-core` may not name `rivet-store`'s path
(`ADR-014` §3) and a second spelling of it here would be the drift this whole
mechanism avoids. Both are in `PLAN.md`.

#### 8.9.5 Time of check to time of use, answered by the primitive itself

A file can change between the plan and the confirmation, and nothing locks it.
**The confirmed path re-reads and re-counts**, so a file whose `--expect` no
longer occurs exactly once is refused *at* `confirm` — the guard against a stale
plan is the same rule as the guard against a blind clobber, and there is no
second mechanism to get wrong. The plan says so, in as many words, so an operator
is not surprised by it.

`--append` needs no such guard: appending is order-independent, and the only
thing that can change is whether the file had to be created — which the result
reports as it happened.

#### 8.9.6 It is journalled, with the principal — and that is a second writer

`ADR-012` §4 requires actor provenance and `ADR-022` §5 requires an actor on
every record. `rivet apply edit` writes one record per edit, carrying the path,
the change, the byte counts, and `ADR-028` §4.5's principal — the same
`Principal` `apply install` records, read once from `RIVET_CONTROL_PLANE_PEER`
by the same function, so an edit and an install from one session name one
client.

**This makes `cmd/rivet` the second writer of `/state/log/journal.jsonl`, and
`ADR-022` §7 gives that file to the supervisor.** The reason it cannot follow
`cmd/rivet-deploy`'s route — emit on stdout and let the supervisor capture it —
is structural: `rivet`'s stdout **is** the answer travelling back to the
operator, so a record written there would corrupt that answer and reach no
journal. What this does instead is one `O_APPEND` open, one write of one line
under 4 KiB, one close, only when an edit actually happened.

**Two caveats, stated rather than discovered.** Rotation is the supervisor's and
is not coordinated, so a line can land in the file that has just been rotated —
`rivet get log` reads every kept file, so the record is still found. And this
does not rotate, so a machine whose supervisor is not running could grow the
active file past §8's cap one edit at a time.

**Whether `ADR-022` §7 should say "the supervisor and the administration
surface" is the maintainer's**, and `PLAN.md` carries the alternative — a drop
file the supervisor ingests, as `/state/deploy-request` is.

**A journal that could not be written is reported and is not a refusal.** The
file is already replaced by then; saying *nothing was written* about a file that
was would be the worse of the two lies. The result carries `journalled: false`
and an `unrecorded` field saying why.

#### 8.9.7 What this cost §7's shape, and what the alternative was

**`Input` gained one field: `several`.** Every input in this catalogue was
exactly one word until this operation, and `perform`'s arity check said so
structurally — a command with more words than the operation declares inputs was
refused before any operation saw it. `--expect <text> --replace <text>` is
genuinely several words.

The alternative was to declare four positional inputs describing flag words,
which would have been **a catalogue that lies about its own shape** — §7's whole
argument is that a catalogue which is trusted and wrong is worse than none. So
the catalogue says the true thing: this input is several words, the operation
reads them itself, and `several` is published beside `required`.

**What it costs**: the operation declaring one has no arity ceiling and therefore
owns the refusal the dispatcher used to give. `edit` refuses every unknown word
**by name**, which is strictly stronger than the count it replaces, and
`a_verb_with_no_word_ceiling_still_refuses_a_word_it_has_no_meaning_for` in
`rivet-core::operation` is what keeps that true for any future verb that
declares one.

**The alternative that was not taken, and it is the maintainer's:** inputs *per
subject*, which is the symmetric counterpart of §8.7's per-subject outputs and
would also pay §8.5.1's first cost — the polymorphic positional input, now
recorded for the fourth time. `PLAN.md` carries it with a recommendation.

#### 8.9.8 §8.5.1's first cost, paid a fourth time

`apply edit /state/x --expect a --replace b confirm` puts a *file path* where
`apply trial-boot b confirm` puts a slot, `apply install x.rvt confirm` puts a
payload and `apply power-off confirm` puts the confirmation. §8.7.3 left this
open and it is unchanged. The second cost stays paid: `edit`'s output shape is
declared beside its implementation and the union is generated.

#### 8.9.9 What has run, and what has not — `AGENTS.md` §6

**Run, on a Rivet machine under QEMU on `ADR-024`'s own kernel, into a real
nushell session, 2026-08-24:** the plan, the confirmation, the file
changing on the machine, the already-made refusal, the ambiguous refusal, the
view refusal, the mechanism refusal, a creation with two directories named in the
plan and made at 0700 with the file at 0600, the not-UTF-8 refusal, the directory
refusal, the relative-path refusal, and the record read back with `rivet get
log` — with `principal` naming the same remote caller the session's own log line
names.

**That run found three defects and each is fixed:** the plan's re-type line was
not quotable and produced a command that could not be pasted back; the journal's
`directories` field joined paths with a space, which is ambiguous; and the boot
identifier was truncated to eight characters where `cmd/rivet-sysd` writes the
whole one, which would have made one boot look like two to
`rivet get log --boot`.

~~**Not run: a Raspberry Pi.**~~ **RUN on one since 2026-08-28 — struck
2026-08-29.** `PLAN.md` §66.4: *"`rivet apply edit` put `memory = \"32M\"` under
`[services.rivet-mdnsd]` — 1587 bytes to 1602, matched at line 6 exactly once,
written atomically"*, on the board, against an `ext4` `/state` on a card, and
the same section puts the board back with a second `apply edit` and compares
SHA-256. **The rest of this paragraph stands unchanged and is the part worth
keeping:** `ADR-026` §4's right-hand column is only partly closed by
any of this — what QEMU settled here is the filesystem, the syscalls and the
login path, and `/state` under the emulator is a directory on the initramfs
rather than an ext4 partition on a card. **Nothing about SD wear, a card that has
gone read-only mid-write, or `fsync` against real media has been observed.**

---

#### 8.9.10 The refusal was made false by widening `explain` — 2026-08-25

§8.9.4 rules that this operation refuses a view **in `explain`'s own words**, and
that was right. What it did alongside was compose those words with one clause of
its own:

> ...and the next boot would regenerate it over anything written here even if the
> filesystem allowed the write, which it does not.

True of every path `explain` could answer for, because §8.2's scope was `/etc`
and nothing else. **False the moment §8.2.1 widened it.** `/state/deploy-request`
is regenerated by nothing, its filesystem does allow the write, and a file
written there by hand is not a description of an install — it is a **request**
for one that the supervisor will act on (§8.8). The refusal would have told an
operator their write was futile at the exact moment it was dangerous.

**Two corrections, and the smaller one matters more.**

1. **The reserved check runs before the view check.** Both refuse; the question
   is which sentence is read. A reserved path names *the mechanism that consumes
   it*, a view names *what to change instead*, and while the two sets could not
   intersect the order could not matter and so was never chosen. It intersects
   now, and the more specific refusal wins.
2. **The clause is a property of the path**, `Explanation::consequence()`, one
   per `Kind`, living beside the tables rather than inside this operation — so a
   second operation refusing the same path cannot give a different account of
   why. It is also asserted to be a lower-case clause with no full stop, because
   it is spliced after *"and"* into the one message an operator reads at the
   moment they are already wrong.

**What this is an instance of.** `AGENTS.md` §7 usually catches a sentence
somebody forgot to update. This one was *made* false by correctly widening
something else, and nobody wrote it. Composing the refusal from a single source
of truth is what surfaced it at all — a hand-written refusal per path would have
gone on being wrong silently, which is the argument for §8.9.4's design stated
from the other end.

**A view is refused with no exceptions, and the exception list is gone.** This
operation once carried one, for a single path whose only mechanism was a
hand-edit. There is no such path now: every projection this build can name has a
source an operator can change, and `explain` names it. An exception would be an
edit that is accepted, appears to work, and is gone at the next boot — the
failure `explain` exists to prevent, reintroduced by the operation that reads it.
`rivet-core/src/operation/edit.rs` carries a test asserting there is no list.

---

### 8.10 File management: §8.9's exclusion is AMENDED — ruled by the maintainer, Joel Lindell, 2026-08-29

**§8.9.3 is amended, not overturned.** Two texts excluded file management, and
they are not the same text — reading only one of them is how this gets
mis-summarised. §8.9.3 of this ADR rules that `apply edit` *"must grow refusals
rather than expressiveness"*, and it names the verbs it means: insert-at-line,
delete-matching, regular expressions, replace-all. The sentence that excluded
delete, rename and chmod is in the **implementation's own module
documentation**, `rivet-core/src/operation/edit.rs`, in its list of what is
deliberately absent:

> **No delete-file, no rename, no chmod.** Each is a different act with a
> different plan; none of them is an edit.

`docs/design-candidates/Admin-Surface-Survey.md` §5.1 quoted the two together,
which is correct — they were one decision — and then read them the way they are
read here:

> The shape that would honour both: new *subjects* under `apply` — `apply
> remove-file`, `apply rename` — each with its own plan-before-apply and its own
> refusals, rather than new *flags* on `edit`. That is exactly what §8.9 said
> ("each is a different act with a different plan"), so it may be less a
> reversal than a reading of it.

**Read as an instruction, the second sentence is this section's design.** It
does not say the acts must not exist; it says each is *a different act with a
different plan*, which is exactly the shape `ADR-006` §6.5 gives a mutating
operation — one type, one plan, its own refusals.

#### 8.10.1 Who ruled it, when, and on what argument

**Ruled by the maintainer, Joel Lindell, in conversation on 2026-08-29.** Two
statements, and the second is the one that decides the shape of this section
rather than only its existence.

**First: assume that file management will ONE DAY need essentially the full
set — create, meta-info, permissions, update, delete — and even where something
is not implemented today, plan for the day he proves it is needed.**

**Second, on building the whole of it in one pass rather than incrementally: if
the editor is not made fully capable today, it will simply be refactored
tomorrow, and the day after, and the day after that, until it is.**
*(`.rulings/` R-018.)*

**The operational argument that forced it.** `ADR-035`'s last phase leaves the
control plane as the only way in. An update payload is roughly 683 MiB and they
accumulate in `/state`; they had been removed by hand from a general-purpose
session, which that phase takes away. After it,
**nothing on the machine can delete a stale `.rvt`** — the surface would be able
to install a payload and unable to clean up after itself, which is not a
first-class operator, it is the same *improvising* §8.9.1 was written to end,
pointing the other way. `PLAN.md` §92.4 carried this as *waiting on the
maintainer*; this section is the answer.

#### 8.10.2 What survives of §8.9 unchanged, and it is the important half

**`apply edit` grows nothing.** No flag, no mode, no second form, no
`--delete`, no `--chmod`. §8.9.3's rule stands in the words it was written in:

> This must grow **refusals** rather than expressiveness. Every extra verb —
> insert-at-line, delete-matching, regular expressions, replace-all — is a step
> towards being `sed`.

That rule is about `edit`, and it is untouched by this amendment. What this
section adds is **sibling subjects**, each of which is its own type, computes its
own plan, and carries its own refusals. Three consequences worth stating so they
are not re-litigated:

1. **There is no generic file operation.** A single `apply file-op <path> --op
   delete` would be precisely the `sed` §8.9 exists to prevent: one plan
   describing seven acts describes none of them, and one refusal list guarding
   seven acts guards none of them properly. Each subject below has refusals the
   others do not, and that asymmetry is the safety property.
2. **`apply edit`'s permitted paths are NOT re-ruled here.** `edit` keeps its
   deny-list (§8.9.4) and refuses every view with no exceptions (§8.9.10). The
   **destructive** subjects take a stricter posture — an allow-list — for the
   reason given in §8.10.4.
3. **The port may share a shape where the surface must not.** The domain port
   (`rivet_core::operation::machine::Editor`) carries one `can_change_path` /
   `change_path` pair over a **closed enumeration** of acts. That is not the
   generic operation item 1 refuses: the plan, the refusals and the operator's
   vocabulary are per subject and live in the domain, which is where §8.9 put
   them; the port is the seam where a platform does the thing, and it names
   nothing this crate is forbidden to name (`AGENTS.md` §3.1).

#### 8.10.3 The full surface

**Every row is specified here.** The "Built" column records what this build
implements; a row marked otherwise is a design a future implementer can build
from this section, which is what the maintainer's instruction to plan for the
day the need is proved asks for. *(`.rulings/` R-018.)*

| Act | Surface | Kind | Built |
| :-- | :-- | :-- | :-- |
| Read a file's content | `rivet get file <path>` | read | **yes** |
| Meta-info / stat | `rivet get file <path>` — same subject, §8.10.5 | read | **yes** |
| List a directory | `rivet get file <path>` — same subject, §8.10.5 | read | **yes** |
| Compare against supplied content | `rivet get diff <path> <text>` | read | **yes** |
| Update content | `rivet apply edit …` — §8.9, unchanged | write | **yes** (2026-08-24) |
| Create a file | `rivet apply create-file <path> [<text>] confirm` | write | **yes** |
| Create a directory | `rivet apply create-directory <path> confirm` | write | **yes** |
| Permissions — mode | `rivet apply set-mode <path> <octal> confirm` | write | **yes** |
| Permissions — owner/group | `rivet apply set-owner <path> <user>[:<group>] confirm` | write | **yes** |
| Rename / move | `rivet apply rename <from> <to> confirm` | write | **yes** |
| Delete a file | `rivet apply remove-file <path> confirm` | destructive | **yes** |
| Delete an **empty** directory | `rivet apply remove-directory <path> confirm` | destructive | **yes** |
| Install a credential into an application's data directory | `rivet apply install-secret <app> <name> --hex <hex> confirm` | write | **yes** (2026-09-01, §8.10.18) |
| Put one of the machine's own secrets back into `/state/secrets` | `rivet apply restore-secret <name> --hex <hex> confirm` | write | **yes** (2026-09-14, §8.10.19) |
| Delete a directory **and its contents** | `rivet apply remove-tree <path> confirm <count>` | destructive | **NO — specified in §8.10.13, and it needs a ruling** |
| Drift: on-disk against the projection | — | read | **NO — structurally blocked, §8.10.14** |
| Symbolic links: create, retarget, remove | — | write | **NO — §8.10.15** |
| Copy a file | — | write | **NO — §8.10.15** |
| Truncate, append-at-offset, sparse writes | — | write | **NO, and deliberately: that is `sed`** |

#### 8.10.4 The gate every one of these passes, and why an allow-list

Eight checks, in this order, before any subject touches anything. The order is
part of the design for §8.9's reason: everything true of the **request** refuses
before anything is read, so a refusal is reproducible from a workstation.

| # | Refuses | Because |
| :-- | :-- | :-- |
| G1 | a **relative** path | §8.9.4's reason unchanged: the working directory is whatever the session started in, and it resolves to *something* often enough that the answer names a file nobody typed |
| G2 | a path carrying a **control character** | a plan and a journal record are both lines |
| G3 | a `.` or `..` or empty component, or a trailing `/` | **this is what makes G4 mean anything.** `/state/../system/bin/rivet` passes a prefix test and is not under `/state` |
| G4 | anything **outside `/state`** — write and destructive subjects only | §8.10.4.1 |
| G5 | a **view** — anything `rivet explain` answers for, and the whole of `/etc` | `ADR-007` §3.6.2. The `EDITABLE_ANYWAY` exception is `apply edit`'s alone (§8.10.2 item 2) |
| G6 | a **mechanism path** — `edit`'s `reserved_paths` | §8.9.4. Use the mechanism; a file written or removed by hand makes a claim nothing produced |
| G7 | a path the **running machine depends on** — §8.10.4.2 | a surface that can lock an operator out of a headless board has removed its own last channel |
| G8 | a path any component of which **is a symbolic link**, and a target that is one | §8.10.4.3 |

**`rivet get file` and `rivet get diff` pass G1–G3 only.** Reading is not
destructive and refusing to read `/system/bin/rivet-sysd` would make the surface
less able to answer the question it exists for. `get file` **reports** that a
path is a view, in `explain`'s own words, rather than refusing it — which is
strictly more useful and cannot be acted on wrongly.

##### 8.10.4.1 An allow-list for writes, and a deny-list would not have done

`apply edit` guards itself with a deny-list and that is correct for it: an edit
is reversible in the sense that matters — the operator can see what they did and
edit it back, and `--expect` means they had to be right about the old text first.

**None of that is true of a delete.** There is no trash on this machine, no
snapshot, and no undo; a deny-list is only ever as good as its enumeration, and
the failure mode of a missing entry is not a wrong answer but a file that is
gone. So the destructive and mutating subjects take the opposite posture:
**everything is refused except `/state`.**

The root is not chosen for convenience — it is `ADR-018` §4's own division:

| Partition | Why it is not in the allow-list |
| :-- | :-- |
| `/system` | read-only vendor content, replaced whole by an update. Changing it makes a running system that matches no payload, and the next update discards the change |
| `/etc` | `ADR-007` §3.5.1's projection, remounted read-only once rendered. A write is refused by the kernel in the ordinary case and lost at the next boot in the best one |
| the boot partition | FAT the firmware reads. `apply trial-boot` owns the one file anybody has a reason to change |
| `/run`, `/proc`, `/sys`, `/dev` | kernel and supervisor surfaces. Nothing here is a file an operator manages |
| `/state` | **the only writable partition a Rivet card carries.** In the allow-list |

##### 8.10.4.2 The load-bearing set: paths inside `/state` that are still refused

G4 admits `/state`, and `/state` holds things whose removal takes the machine
away from anybody who could put them back. These are refused to **every**
mutating and destructive subject — they are not the mechanism refusal of G6 and
not the view refusal of G5, and they carry their own sentence:

| Path | What removing or moving it costs |
| :-- | :-- |
| `/state` itself | the partition's mount point |
| `/state/config`, and everything under it | the desired-state document. A machine without one converges to nothing, and there is no verb that writes one — `§6.4`'s `set` does not exist. **`apply edit` may still change it**; what is refused is *removing* or *renaming* it, because those are not changes to the document, they are the absence of one |

`rivet_core::operation::filesystem::load_bearing` is the list, and it is
**two entries** rather than a longer one on purpose: `/state/app` and
`/state/log` are already refused as mechanism paths with a better sentence, and a
second entry for them here would be `ADR-011` §3 in the small. The update
payloads under `/state/*.rvt` are deliberately **not** refused — they are the
reason §8.10.1 exists.

**Whether `/state/config` should be removable once a verb exists that can write
one is the maintainer's, and this section does not decide it.**

##### 8.10.4.3 Symbolic links, and why a textual prefix test is not enough

G4 is a test on a **string**. `/state/scratch` may be a symlink to `/system`, in
which case `/state/scratch/bin/rivet-sysd` passes G4 and is not under `/state`
at all. A destructive subject whose only root check is textual therefore has no
root check.

**So the machine refuses when any component of the path is a symbolic link**, and
names the component. That is a fact only the machine can know, so it is checked
in the adapter under `can_change_path` and re-checked at the point of use — never
cached, `AGENTS.md` §5.

**And the target itself being a symlink is refused too**, in this build. Removing
a symlink removes the link and not what it points at, which is the safe
semantics; the reason to refuse anyway is that the useful cases (a stale
`current` link under `/state/app`) are already refused by G6, and *"this is a
link, and the thing it names is not touched"* is a sentence an operator has to
read correctly at exactly the moment they are in a hurry. §8.10.15 records what
widening it would take.

##### 8.10.4.4 The application tree is THREE ZONES, and §8.9.4's blanket refusal is amended

**Ruled by the maintainer, Joel Lindell, 2026-08-29**, after asking whether an
administrative tool needs to care about `ADR-015` §5's sandbox:

> permit, and inform — do not refuse.

**Sandboxing is not the constraint, and saying so first avoids the wrong
answer.** §5's sandbox is a mount namespace the *supervisor* puts an application
in. The administration surface runs as root and outside it, so it can reach any
path — and it should, because a blanket refusal on the application tree
recreates exactly the gap this whole section exists to close: once `ADR-035`'s
last phase lands the control plane is the only way in, and what was done by hand
has to be doable by the surface.

**What actually differs between paths in that tree is trust and persistence**,
and those are matters for a *plan* rather than for a refusal.

| Zone | What is true of it | This surface |
| :-- | :-- | :-- |
| `/state/app` | the application root | **refused** — nothing removes it and a machine without it starts nothing |
| `/state/app/<name>` | one application, whole | **refused** — `apply uninstall` is the act, and it stops the service first, which nothing here does |
| `/state/app/<name>/current` | §7's atomic swap link | **refused** — it is the machine's only statement of which version is live, and a link changed by hand makes it run a version nothing recorded. A path *through* the link is refused too: §8.10.4.3 does not follow links, and the meaning of such a path changes at the next install |
| `…/<version>/bin/…` | **the program.** A signed manifest beside it carries its digest and the supervisor re-checks it at **every launch** (§4.2, §4.2.1) | **permitted, and the plan says**: a changed program does not start, and shows as drift against the digest in the desired-state document. Nothing fails at the time |
| `…/<version>/manifest.toml` | the signed record the launch check verifies against | **permitted, and the plan says**: changing it cannot make a changed program acceptable — the signature is over the manifest — it stops the application starting |
| `…/<version>/…` anything else | **not covered by any digest** | **permitted, and the plan says**: nothing will refuse this and nothing will notice it, and **the next update discards it**, because §7 installs each version into its own directory and swaps a link rather than merging. That is the trap worth naming, because nothing fails at the time |
| `…/<name>/data/…` | the application's own state, mode 0700, owned by its identity (§5.1.1), outside the `current` link | **permitted**, and the plan says it persists. This is where a hand change actually survives |
| `/state/app/<image service>` | the home of an image service that runs under its own identity (`ADR-060`, `rivet_core::service::IMAGE_SERVICE_HOMES`) — part of the operating system, not an installed application | **refused**, naming it as the image's: no uninstall removes it |
| `/state/app/<image service>/…` | the same home's contents. It has **no version directories**, because the program is on the system slot | **permitted, and the plan says it persists**: an update replaces the program and leaves the home as it is. Its `data`, `settings`, `routes` and `dns.toml` are described as an application's are |

**The declarative answer is still the document.** Anything that belongs in the
desired-state document belongs there and not in a hand edit anywhere —
`ADR-011` makes it the single source of authority and `apply edit` plus
`apply reload` is the path. The zone notes above are for what is genuinely a
file.

**This amends §8.9.4, and `apply edit` is amended with the rest of the family.**
That table's `/state/app` entry is `tree: true`, so before this ruling `apply
edit` refused every path under it. Keeping `edit` at the old rule while the six
new subjects took the new one would have produced a surface where a file can be
removed and not edited, which is not a distinction anybody could predict. So the
carve-out is in `reserved_for`, which both use, and the three mechanism paths
above keep their refusal in `edit` too.

~~**One thing deliberately NOT decided, and it is in the report rather than
here.** Whether writing an application's **program** should need something
louder than the ordinary `confirm` — a second word, a named digest — is a real
question, because it is the one case in this family where the machine actively
stops working and does so *later*, at a launch nobody is watching. The plan says
so in capitals and that is this build's answer. **A stronger interlock is the
maintainer's to rule**, and `AGENTS.md` §11.1 is why it is not defaulted here.~~

**RULED THE SAME DAY, before this section shipped — `ADR-012` §7.3.** The
question above was put to the maintainer and he answered it: **anything that can
cause damage gets a second confirmation tier, worded as a `<WARNING>` naming the
hazard and answered `(N/yes)` — default no, and the whole word.** So
writing, moving or removing a **program** or its **manifest** requires
`--accept 'the application will not start until it is reinstalled'` beside
`confirm`, and the acknowledgement echoes the hazard for `--expect`'s reason:
it cannot be produced without having read the plan.

**The mechanism and the doctrine both live in `ADR-012` §7.3 and are not
restated here**, because they govern more than the file surface. What belongs
here is only *which of these paths reach the tier*, and it is two: the program
and the manifest. Everything else — a stale payload, a config file in a version
directory, a `data` file — is the ordinary `confirm` with the consequence in the
plan's prose, because §7.3.3's doctrine is that a warning which fires often is
not a warning. `PLAN.md` §94.2's *"became wallpaper"* is this project's own
evidence for that.

#### 8.10.5 `rivet get file <path>` — read, meta-info and listing, in one subject

**The gap `edit.rs` names against itself**, quoted in the survey §3.2: *"an agent
can change a file it cannot read back."*

**One subject rather than three, and the argument is about who is guessing.** The
alternatives were `get file` / `get stat` / `get directory`, or one subject with a
`--mode` selector. Both make the *caller* say what is at the path before asking
what is at the path — and *"what is this"* is very often the question. A caller
who guesses wrong gets a refusal instead of an answer, and a surface that refuses
a correct question because the caller mis-anticipated the answer is one nobody
can use without already knowing. A `--mode` flag is worse: it is `ADR-006` §6.4's
two-spellings failure, and the catalogue would publish a selector whose values
are the answer.

So: **one subject, and the shape of the answer follows what is actually there.**

| At the path | The answer carries |
| :-- | :-- |
| nothing | `present: false`, and that is `Ok` and not an error — `get desired-state`'s precedent, and *"there is no file there"* is a true answer |
| a regular file | type, size, mode, uid/user, gid/group, modified, and `content` when it can honestly be given |
| a directory | the same meta-info, plus `entries` — name, type and size per entry — and `entry_count` |
| a symbolic link | the same meta-info **of the link**, plus `link_target`. It is never followed |
| anything else | type `other`, and the meta-info. No content |

**`content` is present or explained, never lossy.** It is omitted, with
`content_omitted` saying exactly which of these applied, when the file is larger
than §8.9's 1 MiB read cap, when it is not UTF-8 (with the offset of the first
byte that is not), or when the path is not a regular file. Emitting replacement
characters would be the encoding guess `AGENTS.md` §5 forbids, and emitting
nothing without saying why would be false by omission.

**`entries` is capped and says so.** A directory with more entries than the cap
reports the first N sorted by name, `entry_count` as the true total, and
`entries_truncated: true`. A truncated list that did not say it was truncated is
the same failure in a different costume.

**It reuses `Editor::read_file`** rather than adding a second reader, which is
`ADR-011` in the small: two readers of one file are two answers about whether it
could be read. The meta-info comes from a new `Editor::stat_path`, which is a
*stat* and not a second read.

#### 8.10.6 `rivet get diff <path> <text>` — what would change if this were written

Non-destructive, and the read-side answer to the question `apply edit`'s
`--expect` makes an agent guess at. An agent that can diff before it writes does
not have to be lucky.

* Refuses a directory, a file larger than the read cap, text larger than §8.9's
  64 KiB `TEXT_MAX_BYTES`, and a file that is not UTF-8 — a byte diff is not a
  diff a person reads, and pretending otherwise produces plausible garbage.
* **An absent file is not a refusal.** It reports `present: false` and diffs
  against empty, which is exactly what `apply create-file` would write.
* The diff is **unified**, computed in `rivet-core` (`ADR-014` §3.1: testable
  with no I/O), and **capped** — a diff longer than the cap reports
  `truncated: true` rather than a truncated diff that reads as a complete one.

**No crate was added, and that was decided on `ADR-001` and `ADR-009`'s terms.**
`rivet-core`'s `[dependencies]` table is empty *by construction* — `ADR-014` §3
makes the build refuse rather than a human noticing — so a diff crate could not
go there at all, and putting the algorithm in an adapter would put it where it
cannot be tested without I/O. It is roughly two hundred lines: a common
prefix/suffix trim, then a bounded longest-common-subsequence table. **Above the
bound it degrades and says so** (`algorithm: coarse`), which is the same rule
`get memory` follows for absent pressure — report what was measured, never a
plausible substitute.

#### 8.10.7 The diff belongs in the **plan**, and that is its highest-value placement

`ADR-012` §7.1 makes plan-before-apply the **safety** mechanism. A plan for
`apply edit` that says *"3 bytes will change"* is a much weaker safety mechanism
than one that shows the change: an operator whose `--expect` matched the wrong
one of two similar stanzas cannot tell from a byte count and can tell instantly
from a diff.

**So every subject that changes a file's contents carries a unified diff in its
plan**, capped as §8.10.6's is. This is deliberately **output**, not input:
§8.9.3's rule that `edit` grows no expressiveness is untouched, because nothing
about what an operator may *ask for* has changed.

#### 8.10.8 `apply create-file` and `apply create-directory`

`apply edit --append` already creates a file, and `edit.rs` names its own gap:
*"if the intent was to CREATE an empty file, this build has no operation for
that."* These are that operation, and the directory case, which nothing had.

| | `create-file` | `create-directory` |
| :-- | :-- | :-- |
| Plan says | the path, the byte count of the content (0 for an empty file), mode **0600**, every missing parent named outermost-first at mode **0700**, and the content as a diff against empty | the path, mode **0700**, and every missing parent named outermost-first |
| Refuses | G1–G8; **anything already at the path** — creating is not clobbering, and replacing content is `apply edit`; content over `TEXT_MAX_BYTES` | G1–G8; **anything already at the path**, including a directory that is already there |

**A directory that is already there is a refusal and not a success**, which is a
second deliberate departure from §6.2's idempotence, on §8.9.2's argument: a
subject that reported success for an act it did not perform is asserting
something that did not happen. The refusal says it is already there, which is
what the caller needed to know.

**Modes are §8.9.3's, unchanged and for its reason** — private first, widened
deliberately, never the reverse.

#### 8.10.9 `apply set-mode <path> <octal> confirm`

| | |
| :-- | :-- |
| Plan says | `from 0644 to 0600`, both octal, **both rendered** (`rw-r--r--` → `rw-------`), a **widening note** when the new mode grants access the old one did not, and the application-zone clause when the path is in one |
| Refuses | G1–G8; a **symbolic** mode (`u+x`, `go-rwx`) **by name** — a symbolic mode is a *pattern over the mode that is currently there*, and this family's rule is that a caller states what it believes and is right; a mode that is not 3 or 4 octal digits; a mode **equal to the one already set**, which would cost a journal record for an act that changed nothing (§8.9's `--expect == --replace` rule); a path that does not exist; **any of setuid, setgid or the sticky bit** |

**The setuid refusal is a decision and not a limitation.** Rivet ships no setuid
binaries; a setuid bit set through the administrative surface is a
privilege-escalation primitive that nobody asked this operation for, and the one
place it could ever be wanted is a build-time decision in an image rather than a
runtime one on a board. If it is ever needed, it needs its own ruling and its own
subject — which is this section's whole shape.

**Time of check to time of use**: ~~the confirmed path re-stats and refuses if
the mode is no longer the one the plan described — §8.9.5's rule, reused rather
than re-invented.~~ **Struck the same day it was written, before it shipped —
§8.10.16.** The check is real and the framing was wrong: what it compares
against is *this invocation's* reading, not the plan an operator read. A mode
already equal to the one asked for is refused as a no-op, which catches the case
that matters.

#### 8.10.10 `apply set-owner <path> <user>[:<group>] confirm`

| | |
| :-- | :-- |
| Accepts | a **name or a numeric id** on either side; `:group` alone changes only the group |
| Plan says | `from 0 (root) to 1000 (app)` for each half that changes, with the name **resolved before the confirmation** so an operator is never confirming an identifier they have not seen resolved |
| Refuses | G1–G8; an unknown user or group, naming what is known; a change that is a no-op on both halves; a path that does not exist; **an empty user and group** |
| Does not | **recurse.** A directory's contents keep their owner and the plan says so in as many words. Recursive ownership is `remove-tree`'s question in a different costume — an act whose size the operator cannot see |

Name resolution is the **adapter's**: `ADR-007`'s projected `/etc/passwd` and
`/etc/group` are the authority, and resolving a name is I/O, so `rivet-core`
asks through the port (`Editor::resolve_owner`) rather than carrying a second
copy of the identity table (`ADR-011`).

#### 8.10.11 `apply rename <from> <to> confirm`

| | |
| :-- | :-- |
| Plan says | two effects — `<from>` removed and `<to>` created — each with the size, and for a directory the **entry count** under it, so the operator can see how much is moving |
| Refuses | G1–G8 **on both paths**; `<to>` **already existing** — `rename(2)` replaces silently, and a rename that replaces is a delete nothing planned; `<from>` not existing; `<from>` equal to `<to>`; `<to>`'s parent missing or not a directory — no silent `mkdir -p`, because creating a directory is a different act with its own plan; a **mount point** as `<from>` |
| Reports honestly | a cross-filesystem rename fails with `EXDEV` and is reported as what it is: this build does not copy, and a copy would not be atomic |

**Directories may be renamed** and files may. The act is atomic either way and
destroys nothing; what the operator needs is to see the size of what moves, which
is what the plan carries.

**Time of check to time of use**: the confirming invocation re-checks that
`<to>` is still absent and `<from>` still present, and refuses otherwise —
because it plans again before it acts. **That is not a comparison against the
plan the operator read**, and §8.10.16 is precise about the difference.

#### 8.10.12 `apply remove-file` and `apply remove-directory` — the destructive pair

**This is the pair the ruling was actually for.**

| | `remove-file` | `remove-directory` |
| :-- | :-- | :-- |
| Plan says | the path, the **exact byte count**, the mode, the owner, the modification time, and that **this cannot be undone — there is no trash on this machine** | the path, that the directory **is empty**, and the same irreversibility sentence |
| Refuses | G1–G8; anything that is **not a regular file** — a directory names `remove-directory`, a symlink is §8.10.4.3, and a device, socket or fifo is refused by name; a path that **does not exist** | G1–G8; anything that is **not a directory**; a directory that is **not empty**, naming the entry count and the first few names; a **mount point** |

**`remove-file` removes regular files only, and that single rule subsumes several
hazards** — a mount point is a directory, a device node is not a regular file,
and a directory tree cannot be reached at all. It is one sentence in a refusal
instead of four, which is worth more than the four at the moment somebody reads
it.

**A path that is not there is a refusal, not a success.** Same departure from
idempotence as §8.9.2, same argument: *"nothing was there"* and *"I removed it"*
are different facts, and a caller that got the path wrong needs to know which one
happened.

**Time of check to time of use**: ~~the confirmed path re-stats and refuses if
the size or the modification time differ from what the plan described. It is
exactly §8.9.5's guard — the file that is removed is the file that was
described, or nothing is removed.~~ **STRUCK THE SAME DAY, BEFORE IT SHIPPED.
That sentence was false and §8.10.16 is what replaces it.** What is true: every
refusal above runs on the *confirming* invocation rather than being trusted from
the planning one, so a directory that has become non-empty or a file that has
become a link is refused. What is not true is any comparison against the plan an
operator read — nothing carries one between commands.

**It is journalled with the principal**, on §8.9.6's terms in full: one record
per removal, carrying the path and the size that is now gone, and a record that
could not be filed is reported rather than turned into a refusal, because the
file is already gone by then.

#### 8.10.13 Recursive delete — SPECIFIED, NOT BUILT, and it needs a ruling

**`apply remove-tree <path> confirm <count>` is not implemented and this section
does not rule it.** It is the one act in the family whose danger is not bounded
by the operator's attention: every other subject destroys at most one thing the
plan named, and this one destroys an arbitrary number of things the plan
summarised.

`AGENTS.md` §11.1 says an agent does not decide this. What the design would have
to be, so the day it is ruled the argument is already here:

1. **The plan enumerates.** The exact number of entries, the total bytes, and the
   first N paths in full. A summary that says *"and 4,182 others"* is a plan an
   operator cannot check, which is the property `ADR-012` §7.1 is protecting.
2. **A cap, above which it refuses outright** rather than planning. An act too
   large to describe is an act too large to confirm.
3. **The confirmation carries the count** — `confirm 412` — so that the operator's
   yes is arithmetic they had to have read, and a plan that went stale between
   the read and the confirmation is refused rather than applied to a bigger tree.
4. **G1–G8 apply to every path in the tree**, not only to its root — a symbolic
   link inside a tree is a way out of `/state` and the walk must refuse rather
   than follow.
5. **It re-walks on the confirmed path** and refuses if the count changed.

**The open questions, which are the maintainer's:** whether this subject should
exist at all; what the cap is; whether `confirm <count>` is the right
confirmation or whether a second, differently-spelled word is better; and whether
a partial failure part-way through a walk leaves the operation reporting what it
did remove (the honest answer) or refusing to start unless it can promise the
whole (impossible on a filesystem).

#### 8.10.14 Drift — on-disk against the projection — SPECIFIED, STRUCTURALLY BLOCKED

Diffing what is on disk at `/etc/passwd` against what the current document
*would* generate there is **drift detection**: *"has something hand-edited this
machine away from its declared state?"* It is a question a configuration system
should be able to answer about itself, and this one cannot.

**It is blocked by `ADR-014` §3.2, not by effort.** `rivet explain` answers from
a const table and reads no filesystem (§8.2), and rendering a projection needs
the **document** — `rivet_config::Document`, a type `rivet-core` may not name,
permanently, because `rivet-config` depends on `rivet-core` and that direction
does not reverse.

So the subject cannot live where `explain` lives. Three shapes are available and
choosing between them is a decision about where authority sits, which makes it
the maintainer's:

| Shape | Cost |
| :-- | :-- |
| A domain port that renders a named projection, implemented in `cmd/rivet` where both crates are visible | a port whose vocabulary is *"render `/etc/passwd`"* — the domain naming a projection it cannot describe |
| The subject lives in the adapter, not the core | breaks `ADR-006` §3's co-equality: an operation reachable from the CLI and not from the API |
| `rivet-config` grows the subject and `cmd/rivet` composes it | the cleanest, and it moves an operation out of the one catalogue, which §7 exists to prevent |

**Not decided here.** `get diff <path> <text>` gives the caller the mechanism
today — a caller that can produce the expected content can diff against it — and
that is deliberately weaker than drift detection, because it does not know what
the document says.

#### 8.10.15 What is still deliberately absent

Recorded so it is not mistaken for oversight, and each with what it would take.

| Absent | Why, and the condition for building it |
| :-- | :-- |
| **Creating, retargeting or removing a symbolic link** | G8 refuses links throughout, so a link-creating subject would be building the thing the gate exists to keep out. It needs a use case that is not `/state/app`'s `current` link, which is the installer's (G6) |
| **Copy** | a copy of a large file is an act whose cost the plan can state and whose failure mode is a half-written file; it wants the four-step atomic write `apply edit` already has and a plan that names free space. Cheap, and nothing needs it yet |
| **Truncate, write-at-offset, insert-at-line** | **this is `sed`, and §8.9.3 refuses it.** Not an omission and not a candidate |
| **Recursive `set-mode` / `set-owner`** | §8.10.13's problem exactly — an act whose size the plan cannot show |
| **Extended attributes, ACLs, immutable flags** | Rivet's image has none and a surface for them would be a vocabulary with nothing behind it |

#### 8.10.16 What the time-of-check guard IS and IS NOT — corrected 2026-08-29, before it shipped

**Three sections of this ADR said, in their first draft, that a confirmation is
refused when the path is no longer what the plan described.** That is §8.9.5's
property, it is the right property to want, and **these subjects do not have
it.** The claim is struck where it appeared and replaced by this section, which
is the record showing its own history in the way `AGENTS.md` §2 rule 3 asks for
— and doing it in the same pass rather than leaving a false sentence to be
believed for a while.

##### What is actually true

`rivet` is a command. **The process ends when it answers**, and nothing carries a
plan from one invocation to the next. So:

| | |
| :-- | :-- |
| **True** | the confirming invocation **plans again before it acts** — `crate::operation::plan::resolve` runs `plan()` on both paths — so every refusal in this section is evaluated at the moment of the act. A directory that has become non-empty, a destination that has become occupied, a file that has become a symbolic link, a mode that is already what was asked for: each is refused *then*, not trusted from earlier |
| **True** | `AGENTS.md` §5's *the kernel is the authority, asked at the point of use*, is honoured. Nothing is cached between the two reads |
| **NOT true** | that the machine compares against the plan **you read**. A `confirm` typed a minute later acts on the machine as it is a minute later |

##### Why `apply edit` really does have the stronger property

Because **`--expect` travels in the command.** The operator carries the state
they believe in, in their own hands, from the plan to the confirmation — so the
comparison is between what the caller *believes* and what is *there*, and that
survives the process ending. It is the only mechanism in this catalogue that
does, and §8.9.2 was right to call it the whole safety story.

##### The analogue, and why it is deliberately not built

`apply remove-file <path> --expect-bytes <n>` would close the gap exactly, and
so would an `--expect-mode` for §8.10.9.

**They are not built because of what §8.10.1 is for.** The routine case this
whole ruling exists to enable is an operator — or an agent — seeing that
`rivet get storage` reports a full card and removing a stale payload. Requiring
a byte count in that command puts friction on precisely the act the ruling was
made to make possible, and an operator who has to look up a number will copy it
without reading it, which buys nothing.

**Whether the stricter form should exist, and for which subjects, is the
maintainer's** (`AGENTS.md` §11.1). A defensible middle is to require it only
where §8.10.4.4's second tier already fires, since the acknowledgement is
already being typed there.

##### What the plans say instead

Every plan in this section carries the limit **in the plan itself**, in as many
words: *"this describes the path as it is now, and this process ends when it
answers you."* A plan that over-promises is worse than one that promises less,
because an operator acts on the promise. That is `AGENTS.md` §6 applied to a
sentence rather than to a report.

#### 8.10.17 What has run, and what has not — `AGENTS.md` §6

**Verified on a Windows development host, 2026-08-29**, by `cargo test -p
rivetos-core` — 918 tests, of which about 100 are this section's — and by
`cargo test -p rivetos-cli` and `cargo clippy` over both crates. What that
covers: every refusal above that is a property of the *request* — the path gate,
the traversal and root checks, the argument forms, the mode parsing, both
confirmation tiers — every plan's text, and the acts themselves against in-memory
fakes. The domain refuses these **with no filesystem at all**, which is
`ADR-014` §3.1's standing check and the property that makes them reproducible
from a workstation.

**Every refusal test asserts that the fake removed, moved or wrote nothing**, and
not only that the message was right. A refusal that had already destroyed
something is the worst defect these subjects can have, and a test that checked
the prose would not have caught it.

**NOT run, and this is the whole of what a board would settle:** nothing in this
section has executed against a real filesystem, a real `/state`, a real ext4
partition, an SD card, a symbolic link, a mount point, a device node, or a
`.rvt` payload of the size the ruling was for. The adapter under the port —
`stat_path`, the owner resolution, the symlink-component walk, the mount-point
check, `unlink`, `rmdir`, `rename` — has been exercised **only against a map in
memory**. In particular:

* **the driving case is unproven.** No stale `.rvt` has been removed from a real
  `/state` by this surface.
* **G8's symlink walk is unproven.** A textual test cannot exercise it and the
  in-memory fake's answer is the fake's, not the kernel's.
* **The mount-point check is unproven**, and it is the check whose failure mode
  is the most expensive of the set. On Linux it compares device numbers against
  the parent's; on a development host it answers `false` for everything, so the
  test that exercises it is exercising a boolean the fake set.
* **`lchown(2)` is unproven**, and it is the one call in this section that has
  no Windows equivalent at all — `cmd/rivet`'s implementation of `set_owner`
  refuses outright off Linux, so it has never run.
* **Name resolution against `/etc/passwd` is unproven.** The domain has been
  tested against a two-row table in a fake; the adapter parses the real
  projection and has not been run against one.

`ADR-026` §4's right-hand column is unchanged by any of this.

#### 8.10.18 `apply install-secret <app> <name> --hex <hex> confirm` — added 2026-09-01

**The eighth subject of this surface, and the first that carries bytes and an
owner in one act.** Every subject in §8.10.3 above either writes *text* as root
or changes an *attribute* of a file that already exists. Putting a per-machine
credential where an application can read it needs both at once, and §8.10.9 and
§8.10.10 are the wrong shape for it in a way that is not a matter of
convenience.

##### The two defects it closes

1. **`apply edit` writes as root, mode 0600.** `ADR-015` §5.1 gives every
   application a uid derived from its name, so a credential written by `edit`
   into `/state/app/<app>/data/` is one that application gets `EACCES` on. The
   hand-written repair is `edit`, then `set-owner`, then `set-mode` — three
   subjects, three plans, three confirmations. **Two of the three succeeding is
   worse than none**: the file is at its final name, owned by root, the
   application cannot read it, and nothing reports that state because each
   command succeeded on its own terms.
2. **`apply edit` is text-only.** An Ed25519 seed is 32 bytes of no particular
   encoding and loses its null bytes through a `&str`. `cmd/rivet-fleetd` had
   already recorded this as a gap in its own source and named the operation that
   was missing.

##### What it does

Writes `/state/app/<app>/data/<name>` with the application's uid and gid and
mode `0600`, replacing whatever is there. **The path is composed from the
application's name and is never given**, which is what confines the subject to
one directory: there is no argument a caller can spell that reaches outside it,
so the traversal question is answered by the command's shape rather than by a
check on a string. The name must be a single component matching the same rule a
service name does.

It **replaces** where `create-file` refuses to, because rotating a credential is
the second thing anybody does with one and a subject that could install only the
first would send an operator to `remove-file` holding a live secret. The plan
states two lengths and shows no diff.

##### Hex on the wire, and the asymmetry with `get file`

The control plane is JSON, so bytes must be spelled. **Hex, not base64**, and
the reason is not size — a credential is small and the cap is 64 KiB. It is that
base64 has two alphabets in common use, optional padding, and decoders that
ignore whitespace, so **more than one string decodes to one value and a mangled
paste decodes to something**. That is the "wrong guess succeeds" failure
`AGENTS.md` §5 names, arriving at the one place where a wrong answer is a wrong
credential. Hex has one spelling, `crate::secret`'s decoder already refuses odd
lengths and non-hex characters, and every piece of key material this project
already writes down is hex.

`get file` answers base64 and keeps it: that subject reads whole arbitrary files
and size is real there. **The cost is a conversion step between the two**, and
it is accepted rather than unnoticed.

##### The ordering that makes it one act rather than three

The adapter writes a temporary, **chowns the temporary**, and renames it into
place. So there is no instant at which the final path exists owned by root, and
a power cut leaves either the old credential or the new one. That ordering
cannot be expressed as a sequence of §8.10 subjects at all, which is the
strongest argument that this is a subject rather than a convenience wrapper.

##### The plan does not echo the value

Every subject's plan but this one and `restore-secret` (§8.10.19) closes with
the command to repeat, assembled from the words the operator typed. Here that would print the credential — into a
terminal, into the control plane's response, and into an agent's transcript.
`<the same hexadecimal value, again>` stands in its place and the plan says so.
**Found by a test rather than by review**, which is worth recording: the rest of
the subject was already careful with lengths, and the leak was in the one line
nobody writes.

##### What has run, and what has not — `AGENTS.md` §6

* **The domain, against an in-memory fake**: 21 tests. The binary seed arriving
  whole with its null byte, the ownership, the mode, one act rather than three,
  every refusal above, and that no refusal reaches the machine.
* **The adapter, against a real filesystem, as uid 0**, in the build container:
  the bytes survive, the temporary is gone, the mode is `0600`, `stat(2)`
  reports the uid and gid asked for, and a rotation over an existing secret does
  not give the file back to root. Off root the same test asserts the refusal
  names the temporary, and says so rather than passing quietly.
* **NOT proven by either**: that the file is unreadable at *no* moment — the
  chown-before-rename ordering is read from the code and exercised by its
  outcome, not observed as a race. Closing that needs a second process watching
  the path, which nothing in this repository has a shape for.

#### 8.10.19 `apply restore-secret <name> --hex <hex> confirm` — added 2026-09-14

**The board half of `ADR-030` §5's restore** (`.rulings/` R-183 D5). `cargo
rivet restore-secrets` opens a `backup-secrets` backup with its passphrase on
the workstation (R-185 D1) and sends one of these per file, after it has refused
a different board, a damaged backup and a lock-out `peers` file there.

##### Why a subject, and why this one is narrow

Neither existing subject reaches the machine's store. `install-secret`
composes `/state/app/<app>/data/<name>` and gives the file to that
application's uid — which is its safety property and is not loosened — and
`apply edit` is text-only, so a 32-byte `device.seed` loses its null bytes
through it.

`restore-secret` is **`install-secret`'s act with a different composition**:
the path is `/state/secrets/<name>` by the same one-component name rule, the
owner is root, the mode is `0600`, and the adapter writes a temporary and
renames it into place (`PathChange::InstallSecret`). **No adapter changed**,
and a journal record of it names that act, `install-secret`, with the
`/state/secrets` path, the length and uid 0 — never the content.

##### What it refuses

* A name that is not one file directly inside `/state/secrets`.
* **`/state/secrets` not being there.** `rivet-initd` makes it when it installs
  the device identity; this subject does not make one for a board that was
  never given an identity.
* A directory or a symbolic link at the name — replacing a link would write
  wherever it points.
* **A `peers` file that does not name the key this session was pinned by**, on
  the plan path and the apply path. `rivet-mcpd` admits only the keys in
  `peers`; it declines to load such a file while running (`ADR-063` §3.1) but
  loads it whole at its next start, so the file leaves a headless board
  answering nobody after its next boot. A session with no pinned key — the serial console
  — gets a plan that says the check did not run.
* `--accept`, which is superfluous: nothing under `/state/secrets` reaches
  `ADR-012` §7.3's second tier.

##### What it does not do

It **checks nothing about which board the bytes came from** — that refusal is
`cargo rivet restore-secrets`'s, which compares the key a backup recorded with
the key the handshake proves. It **restarts nothing**: `rivet-mcpd` and
`rivet-netd` read their files at start and are in the reachability set `apply
service-stop` refuses, so the plan says the file is live at the next boot. It
**encrypts nothing**: `ADR-020` §11.2.3 rules the store plain text, and the
passphrase envelope is the workstation copy's, opened before anything is sent.
The plan
states two lengths and never the value, and the line to repeat carries
`<the same hexadecimal value, again>` in its place, as `install-secret`'s does.

##### What has run, and what has not — `AGENTS.md` §6

* **The domain, against the shared in-memory filesystem**: 14 tests in
  `rivet-core/src/operation/restore_secret_tests.rs`.
* **Under QEMU**, `tools/qemu/adr-030-secrets-proof.sh`: the guest refused a
  lock-out `peers` with `E_UNSAFE` naming the session's key, and wrote a
  removed and an overwritten file back with the backup's bytes.
* **NOT proven**: anything on a board, and that `rivet-mcpd` starts from a
  restored identity after a reboot.

### 8.11 RULED 2026-08-29: `get document` becomes `get desired-state`

**Moved here from `PLAN.md` §100 on 2026-08-29, unaltered.** Ruled by the
maintainer, Joel Lindell, the same day. Only the heading numbers changed.
**Bare `§N` references in the body are `PLAN.md` sections** — `§91` is the
session that shipped the subject one day before it was renamed.

**One correction that is not in the moved text, made here rather than by editing
it.** §8.11.3 was written while the rename was still pending; it was performed
the same day (`ef32683`, `967a7dd`), so the subject is `get desired-state`
throughout `rivet-core` and `cmd/rivet` and the tripwire test lists it under that
name. The section is kept as it was ruled, with its heading struck, because it is
the record of a tabled action being carried — which `PLAN.md` §94.7 is about.


**The maintainer, reviewing the thirteen subjects, stopped at `rivet get
document`: he had to confirm that it meant the config document, and said it was
poorly named.** *(`.rulings/` R-019.)*

**Ruled: `desired-state`.** And ruled for the reason that makes it more than a
tidy-up: `config` describes a *category of file*, while **`desired-state` names
the claim** — that there is one declared intent and everything else on the
machine is derived from it. That claim is `ADR-011` in a word, and it is what
distinguishes a Rivet from a box with configuration files on it.

#### 8.11.1 What was considered

| Candidate | Outcome |
| :-- | :-- |
| `desired-state` | **RULED.** Matches the record's own term of art — *"desired-state document"* appears **44 times** across the ADRs — and is unambiguous against every other meaning of "config" on a Unix machine |
| `config` | Not taken. The argument for it was real: `ADR-007` is titled *The Configuration Model*, and since `ADR-011` makes the document the authority and `/etc` a projection, naming the authority `config` claims the word correctly rather than ceding it to the artifacts. It loses on accuracy — it names the file type and not the claim |
| `provisioning` | **Actively wrong.** `ADR-019` already uses that word for the first-boot mechanism through the FAT boot partition — the path §78 built and `ADR-035` §8A.2 names as break-glass |
| leave it | Rejected. It was the one subject name that told a newcomer nothing, and the maintainer wrote the thing and it still read wrong to him |

**There is exactly one document**, so the old name was opaque rather than
ambiguous. `get schema` describes that document's shape and is the only other
subject that could have been confused with it.

#### 8.11.2 One spelling, so this is a one-shot choice

An alias was never on the table. `Stop::as_str` states the rule this project
already follows:

> The wire form, **and the subject an operator types**. One spelling, so a
> result can be fed back in as a request and so no adapter owns a second list of
> these words.

Which is also why it was worth doing **now**: the subject shipped 2026-08-29
(§91), one day old. The same rename in six weeks costs a deprecation.

#### 8.11.3 ~~PENDING: the rename itself is not done~~ — DONE 2026-08-29

Recorded here rather than left to memory, per §94.7 — a tabled action whose
trigger is somebody remembering is how §9.3 went six days unruled.

**The second agent was working inside `rivet-core` when this was ruled**, and
`get.rs`'s four places are exactly what it is editing for `get file`. Two agents
rewriting the same four lines is not worth the merge, so **the rename is the
orchestrator's to perform in the same pass as that merge.** The agent was told
to change nothing and to use the new name only in prose it writes fresh.

What it touches: `operation/document.rs` and its `*_tests.rs` sibling, the
`SUBJECT` constant's value, `Observed::read_document`, `get.rs`'s table,
dispatch, description and tripwire, `operation.rs`'s re-exports, the
`cmd/rivet/` adapter, and `cargo rivet deploy --enable`'s reference to it.
**Prose in the ADRs saying "the desired-state document" stays** — only the CLI
subject is renamed.

#### 8.11.4 Why the name is right, sharper than the ruling first put it

The maintainer pressed on whether any model reading the name could be left in
doubt about what desired state means, and the honest answer corrected the
argument §100 made. *(`.rulings/` R-265.)*

§100 reasoned that a name matching nothing generic **forces a read instead of
inviting an assumption**. That is wrong about this name. `desired state` is a
term of art with a precise, widely-established meaning — Kubernetes, Terraform,
Ansible, Puppet, PowerShell DSC — and any reader, human or model, arrives
carrying it.

**The value is not unfamiliarity. It is that the borrowed meaning is accurate.**
`config` borrows a wrong one: a file, one of several, somewhat imperative.
`desired-state` borrows the right one.

**And it encodes the architecture, which is the part neither of us said at the
time.** A reader who knows only the subject's name correctly infers three things
this project ruled separately and at length:

| Inferred from the name | Where this project actually rules it |
| :-- | :-- |
| there is an **observed** state to compare against, so *drift* is meaningful | `ADR-015` §4.2 — tampering is reported as drift in `rivet plan` and `rivet status`, *"failing to start is a symptom; drift is a diagnosis"* |
| there is a **reconciler** converging toward it | `apply reload` — `ADR-007`, and `ADR-017`'s convergence |
| editing it **declares intent** rather than performing an act | `ADR-017` §6.3.2 (~~`§95.2`~~) — the supervisor's control surface may only ask it to **converge, never diverge** |

**The name teaches the semantics for free**, and `get document` taught nothing
while `get config` would have taught something false about all three. That is
the strongest argument for the rename and it was found after ruling it, which is
recorded here rather than back-fitted into §100.

**The one residual ambiguity, stated honestly:** *desired state of what* — this
machine, or the fleet? `ADR-034`'s register holds fleet-wide intent. On a device
subject the answer is the machine's, and context resolves it; it is mild beside
what the name buys.

### 8.12 RULED 2026-08-29: the wall clock is not `get boot`'s, and `get time` stays its own subject

**Moved here from `PLAN.md` §99.1 on 2026-08-29, unaltered.** Only the heading
number changed. **Bare `§N` references in the body are `PLAN.md` sections**;
*"survey §4"* is `docs/design-candidates/Admin-Surface-Survey.md`.

It belongs in this ADR because §8 is where the subject catalogue is ruled
subject by subject, and this rules a **boundary between two subjects** — which
is the kind of decision that gets made by accident if it is not written down.


The agent left it undecided rather than folding the clock into `get boot`, which
was the right instinct: doing so would have settled survey §4's `get time` by
accident.

**Ruled: separate subject, as the record already wanted.** `ADR-021` makes a Pi
clock wrong until something sets it and then it **jumps**, and survey §4's
argument for `get time` is that *every other timestamp's meaning depends on
knowing that*. A clock reading folded into `get boot` would be a fact about
`boot` when it is really a caveat on every other subject.

So `get boot` reads no wall clock at all, and its `means` prose says that
nothing in it says *when* the machine booted — which on hardware with no RTC is
the true answer rather than a missing one.





---

### 8.13 RULED 2026-08-30: `get catalogue` — the surface says what it is, over the wire

* **Ruled by the maintainer**, Joel Lindell, 2026-08-30: the option for a
  `get catalogue` subject is confirmed. `.rulings/` R-118.
* **The ruling names §6.5 as the home for this.** §6.5 was already *"Dry run is a
  property of the framework, not of each command"*, and the section after it is
  already claimed by four existing citations that mean *how a refusal is worded* —
  `PLAN.md` §102.4.2 names it and queues the writing of it. So this is written
  here instead, in the sequence §8 keeps for one
  operation at a time — §8.10, §8.11 and §8.12 are its immediate neighbours.
  **The placement is the only thing that differs from the ruling; nothing about
  what was ruled does.**

#### 8.13.1 The finding

The second cold-agent run was given this repository and a board and asked to
deploy an application. It reached the machine over `ADR-035`'s control plane and
tried `list`. The machine refused, and its refusal said:

> `rivet --version` on the machine publishes the catalogue, which is generated
> from the types and is the authority for what exists.

**There is no shell on the machine to run that in.** `ADR-035`'s last phase
leaves the control plane as the only way in, and it speaks §6.4's closed verb
set, so `-- --version` is refused before anything is sent. The agent's own words:

> The one thing that tells you what the machine can do is behind the one door
> the system deliberately removed. The refusal did name the three live verbs,
> which saved me — but I discovered the *subjects* by trying words.

That is §7's self-description property failing at exactly the boundary §7 exists
to serve: the surface describes itself into a terminal nobody has.

#### 8.13.2 What was already visible, checked rather than assumed

The report qualified its own finding — *"I never spoke the control plane
directly … that may well invalidate finding #4 — the catalogue might be
perfectly visible one layer down"* — so the first thing done was to check.

**Method:** `rivet mcp` was run over stdio in the build container, given
`initialize` and then `tools/list`, and the reply was read. Half of the finding
survives.

**What `tools/list` does publish**, because `cmd/rivet`'s MCP adapter generates
one tool per catalogue entry and nothing else:

* every implemented verb, as `rivet_get`, `rivet_apply`, `rivet_explain`;
* each one's `summary`, verbatim from the operation;
* each declared input — its name, whether it is required, and its description;
* and therefore **every subject**, because the `subject` input's description
  enumerates them and `the_operations_own_description_lists_every_subject_it_accepts`
  fails the build if it stops doing so.

**What it does not, and cannot:**

1. **What an answer will contain.** §7's per-subject result shapes — the field
   names and what each means — are the largest part of what `rivet --version`
   publishes, and a tool definition has no place to put them.
2. **How an operation refuses.** §4.1's error codes are part of the interface
   contract and appear in no tool definition.
3. **That the vocabulary is nine words.** A tool list shows three tools. A
   caller reading only that concludes the verb set is three, which is a claim of
   absence the catalogue does not make and §6.4 contradicts.

**And the client an operator actually holds never asks.** `cargo rivet run`
sends `initialize` and then `tools/call`; there is no path through it that
prints a tool list. So on the transport that exists, the catalogue was
unreachable — which is the finding, narrowed to what survives checking.

#### 8.13.3 The decision

**`catalogue` is the seventeenth subject of `get`**, and it answers at three
depths:

| Command | Answers |
| :-- | :-- |
| `get catalogue` | §6.4's whole closed vocabulary — all nine verbs, which this build implements, and for each implemented one its summary, whether it confirms, its subjects and its error codes |
| `get catalogue <verb>` | that verb's whole catalogue entry, the same value `rivet --version` publishes for it |
| `get catalogue <verb> <subject>` | that one subject's result fields, and what each means |

**Three depths because the whole thing is 112 362 bytes** — measured on this
build, 2026-08-30. That is a fine thing to print into a terminal and a poor
thing to hand an agent that asked *what can I do here*; for the caller this
surface is designed for (`ADR-012`) it is spent as context, which is
`ADR-022` §9.6's argument about `get log`'s default limit, about a different
answer.

**Every answer also carries a `shape` field** — one sentence saying that a
command is a verb, then a subject, then positional words; that a verb with
`confirms: true` does nothing without the literal last word `confirm`; and how
to drill down. A list of verbs whose grammar the reader still has to guess is
not an answer to *what can I do here*.

**The second and third positions are `get`'s existing polymorphic positionals**,
which §8.5.1 already priced and which `file` and `diff` already reinterpret. The
verb's declared arity is unchanged.

#### 8.13.4 It takes no port, and that is the part worth arguing

`PLAN.md` §102.3 states the recipe for a `get` subject as *"a method on
`Observed`, an implementation at every site (the fakes refuse), an adapter under
`cmd/rivet/`, the subject and its tests, and four edits in `get.rs`"*. **The
first three were deliberately not done.**

`get schema` goes through a port even though its answer is constant per build,
and its own documentation says why: `ADR-014` §3.2 makes `rivet-config` depend
on `rivet-core` permanently, so the schema is **not a type `rivet-core` may
name**. The adapter has both crates and hands the value over. That constraint is
the whole reason for the port, and it does not exist here: `CATALOGUE` is a
`const` in `rivet-core`, and `rivet_core::operation::catalogue()` already
renders it.

Routing it through `Observed` would create **a port with exactly one truthful
implementation and an unbounded number of untruthful ones.** An adapter could
publish a catalogue that differs from the one `perform` actually dispatches on —
which is §7's *"trusted and wrong"* in the one place it would do the most
damage. A machine's account of what it can do must come from the thing that
decides what it can do.

The fleet argument that carried `schema` onto a port does not transfer either.
Two boards running different builds do answer differently here — because each
answers from its own build's `CATALOGUE`. It is a per-machine fact *because* it
is a per-build constant, not in spite of it.

#### 8.13.5 What this does not fix

* **It is answered by the machine, so a board answers it only once a build
  carrying it is installed.** Every check below was run against this workspace's
  own binaries; the board at the time of writing runs an earlier build and
  refuses `catalogue` as an unknown subject, which is the correct answer from
  that build.
* **`cargo rivet run` still never asks `tools/list`.** It does not need to now —
  `-- get catalogue` reaches the same information through the path that already
  exists — but a generic MCP client and this one still see different amounts of
  the surface, and closing that is a client decision rather than a catalogue
  one.
* **It does not make `rivet --version` smaller or reachable.** That command
  remains the terminal form, and this is the wire form of the same generated
  value.

---

## 9. Recorded risk

The failure mode of this ADR is over-building an administrative surface before
there is anything to administer. Stage 0 has no services, no network and no
state worth describing; an elaborate configuration model at that point is
speculation dressed as architecture.

The mitigation is the discipline in section 8: **the phase-0 commitments are all
structural**. Where code lives, what shape output takes, whether errors carry
types. None of them requires deciding what Rivet will eventually be able to
configure, and none of them can be deferred without cost.

---

## History

**2026-09-12 — §8.8.2 read "a second reader of the container, in a process that
is not the one performing the install" as the thing forbidden.** The paragraph
read:

> A check here would be a *second reader of the container*, in a process that is
> not the one performing the install, reaching a verdict the installer reaches
> again independently. `ADR-028` §2 puts that out of bounds, and the weaker of
> two verifiers is the one that eventually gets believed.
>
> **The consequence reaches the operator rather than staying in an ADR**: the
> plan says in as many words that nothing has looked at the file yet …

The maintainer ruled the same day that no unverified artifact enters the
machine's artifact store, and that the check is abstracted into the core so the
store and the installer share one implementation (`ADR-052` §7.2.3). Read
literally, *"a process that is not the one performing the install"* forbade
exactly that. **The argument underneath it never did**: *"the weaker of two
verifiers is the one that eventually gets believed"* is about two
implementations that can disagree, and one implementation called from two
processes cannot. `cmd/rivet-sysd`'s launch check had already been a second
process consulting the verifier since 2026-08-25. The body now says what is
forbidden in the terms the argument supports, and the plan no longer tells an
operator that nothing has looked at a file the store has verified.
