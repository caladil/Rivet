# ADR-032: Asking a Running Machine

* **Status:** **Accepted** — opened as a draft and ruled the same day,
  2026-08-25, in four decisions. §5 records a fifth that arrived with them.
* **Date:** 2026-08-25
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Opened at the maintainer's direction**, out of `PLAN.md` §20.6 and a review
  of what an AI-first operating system would want that Rivet does not have.
* **BUILT 2026-08-26 — all four rulings.** §7 is the account, and it records the
  one place building them corrected the ADR: §6 names the command `rivet status`
  and it is `rivet get status`.

---

## 1. Context

`PLAN.md` §20.6 states the gap in one sentence:

> **Every question an operator or an agent asks this machine costs a
> connection and a process spawn**, and there is no way for the machine to say
> anything unprompted.

`ADR-022` §9.5 answered the cheapest part of it — `rivet get log --follow`
streams the journal, because the journal is a *file* and a file can be tailed.
That closed the observability question and left the harder one untouched:
**nothing can ask the supervisor anything.**

The evidence is in the tree and it is explicit. `cmd/rivet-sysd`'s
`Supervisor` carries four accessors — `phase`, `services`, `journal`,
`state_of` — each marked `#[allow(dead_code)]` under this comment:

> Nothing calls them outside the tests because **no adapter exists to ask** —
> there is no CLI crate and no API — and `ADR-014` §3 says an adapter is where
> that question gets asked from, not something to grow here.

`rivet status` is designed in `ADR-017` §3.3, catalogued nowhere, and
uncallable. A machine can be told what to be (`ADR-007`) and cannot be asked
what it *is*.

### 1.1 A smaller instance of the same shape, already paid for

`PLAN.md` §20.3: the supervisor reads its configuration document once, in
`main`, before it exists as a supervisor. Nothing re-reads it. So a document
edit did not reach a running machine at all, and the fix had to **borrow the
install edge** as a signal — because an install is a discrete event the machine
already notices, and there was no way to simply tell the supervisor that
something had changed.

That is this ADR's question arriving from the other direction. A machine that
cannot be asked also cannot be told.

---

## 2. Two questions, and conflating them is the trap

The request that opened this work was *"build an MCP adapter"*, and MCP is a
plausible answer to **one** of these. They are separable, they have different
costs, and only one of them is on the critical path.

| | The question | Depends on | Blocked by |
| :-- | :-- | :-- | :-- |
| **Q1** | How does **anything** reach the supervisor's live state? | a mechanism inside the machine | nothing — it is the missing piece |
| **Q2** | What does an **agent** speak to, and how does it get there? | Q1, for anything about live state | a contract in the login-shell surface — §5 |

**Q1 is the one that matters**, because every answer to Q2 needs it. An MCP
adapter built today could catalogue and perform every operation
`rivet-core` has — and would answer *"what is this machine doing right now"*
exactly as well as the CLI does, which is not at all.

---

## 3. Q1 — reaching the supervisor

### 3.1 What the constraint actually is

The supervisor is PID 1's only child (`ADR-017` §3.1) and it is a **different
process** from anything that asks. Nothing about that is negotiable: the
accessors are unreachable because process boundaries are real, not because
somebody forgot to wire them.

So this is an IPC question, and Rivet has already answered one of exactly this
shape. `ADR-022` §11 — the journal ingest socket — was ruled 2026-08-24 for the
mirror-image case: *how does a process the supervisor did not spawn get a record
**into** the journal*. This is the same question with the arrow reversed.

**That precedent is the strongest single input to this decision**, and its
reasoning transfers almost line for line: a Unix socket at a known path, no
retries, credentials from the kernel via `SO_PEERCRED`, a bounded request, and a
refusal that says why.

### 3.2 Options

| | | |
| :-- | :-- | :-- |
| **(a)** | **A query socket beside the ingest socket** — `/run/rivet/query`, request/response, the supervisor answers from the accessors it already has. | Symmetric with `ADR-022` §11, which is built and proven. One new listener in a process `ADR-017` §3.1 keeps small. The response is a `rivet_core::Value`, so `rivet status` is an operation like any other and every adapter gets it at once. |
| **(b)** | **Widen the ingest socket to take requests as well as records.** | One socket, one listener. But it makes a socket whose mode is `0666` — because *anything that has a record to file sends it* — also the thing that answers questions about the machine, and those two are not the same authority. Rejected in §3.4. |
| **(c)** | **The supervisor writes its state to a file and readers read it.** | No new listener at all, and it composes with everything that already reads `/state`. But it is `AGENTS.md` §5's *derive, never store* inverted — a second representation of live state, stale by construction, and the stale one will be the one displayed. |
| **(d)** | **Signals.** | `SIGHUP` to mean *re-read your document* is the Unix answer to §1.1 and costs nothing. It cannot carry a reply, so it answers *tell* and not *ask*. |

**RULED 2026-08-25: (a), and (d) as well.** They are not alternatives, and both
were taken. The query socket answers *ask*; the signal answers *tell*, and §20.3
had already shown the project needed the second one — the supervisor could not
be told its configuration had changed, so an install edge was borrowed as a
signal.

**`SIGHUP` means re-read the document**, with one rule that is part of the
ruling rather than an implementation note: **a document that will not parse
leaves the supervisor running on the last good one, and says so loudly in the
journal.** A reload that takes a machine down is worse than one that never
happened, and `ADR-017` §3.1 makes this process PID 1's only child — there is
nothing underneath it to catch a fall.

The honest limit of a signal, stated because it is why (a) is not replaced by
(d): **it cannot carry a reply.** The operator who sends `SIGHUP` learns what
happened from the journal, after the fact, rather than from an answer. That is
acceptable for *reload* and would not be for anything else.

#### 3.2.1 BUILT 2026-08-26 — and a reload applies everything but a reachability set

`PLAN.md` §28.5 Phase 1 item 4. `cmd/rivet-sysd`'s `reload` module is the
receiving half: `SIGHUP` joins the `signalfd(2)` mask, arrives as
`Event::Reload`, and the document is read again.

**Blocking it is the load-bearing half rather than a detail**, and it is written
down because getting it wrong fails in the worst possible place: `SIGHUP`'s
default disposition is to **terminate the process**. A supervisor that read it as
an event and left it at its default would die the first time an operator asked it
to reload — and `ADR-017` §3.1 makes this process pid 1's only child, so that is
a machine losing its supervisor in answer to a request to reconfigure it.
`BLOCKED_SIGNALS` is now one list with a test on it.

##### The question the ruling settles: what may a running machine take?

`ADR-018` §8's argument — **a machine that is reachable is one somebody can
repair** — had already been made twice in this codebase, and neither time as a
general rule. `crate::document::load` refuses to stop on an unreadable document
for it, and `Supervisor::set_application_enabled` refuses `Origin::Image` for it,
saying in as many words that re-applying the document to the service that carries
the administration surface *"would let a badly edited document take a machine
away from its operator mid-session"*.

That second refusal was reasoned about an **unrelated install** triggering the
re-read. `SIGHUP` is not that: it is the operator saying *apply my document now*,
explicitly. So the filter could not simply be reused, and three shapes were put
to the maintainer.

**RULED 2026-08-26: a reload applies everything EXCEPT a named reachability
set.**

| | |
| :-- | :-- |
| **Applies** | `enabled` for every service outside the set — including `Origin::Image` ones such as `rivet-timed`, which the install edge's narrower re-read never touched |
| **Withheld** | `enabled` for `rivet-mcpd` and `rivet-netd` |

`cmd/rivet-sysd`'s `definitions::REACHABILITY_SET` is the list and the test for
membership is **not** "is this a system service" — `rivet-timed` is one and
stopping it costs a clock. It is **whether stopping this service takes away the
means of undoing that**, and exactly two do: the control plane, which is the
administration surface (`ADR-006` §5, `ADR-035` §3), and the interface bring-up,
without which there is no address for it to listen on.

**Membership is a claim about the present and it has moved once.** The set was
ruled on 2026-08-26 against a different administration surface and went on
naming it for three days after that stopped being true. A stale member is worse
than a missing one, because withholding is how this set works: the supervisor
would be defending a channel against the operator.
`the_reachability_set_is_the_control_plane_and_the_interface` is what pins it
now.

**`memory` is withheld on a reload too**, for a different and simpler reason:
`ADR-017` §3.1.3 writes the cap into a cgroup at the spawn that creates it, so
moving the number on a definition would report a limit the running process does
not have. That is §5.3's hazard again, exactly.

##### Everything withheld is reported, never discarded

`ADR-007` §3.6 — the system never accepts an input it will discard. Each withheld
value is journalled at `Severity::Warn`, naming the service, the value, and
`rivet apply restart confirm` as the way to take it now. It applies at the next
boot, when `crate::main` reads the document with a console attached and
`ADR-018` §5.1's rollback still underneath.

##### What is still missing, and it is the half an operator needs

**Nothing on a Rivet board can send this signal.** There is no shell and no
`kill`, and `cmd/rivet` executes nothing — which is the
state `ADR-017` §3.1.10 found the three stop signals in, and described as
*"receivable"*. The sending half is a separate change and is not built here.

`ADR-017` §3.1.10 already reserves this signal's meaning, in the paragraph that
chose `SIGUSR1` for a trial boot:

> `SIGHUP` conventionally means *reload configuration*, and a system that
> overloaded it would eventually meet something that sends `SIGHUP` meaning the
> conventional thing

So the name is free and the convention is honoured. What is undecided is **who
delivers it**: pid 1 forwarding to its child (which needs a fourth row in
§3.1.10's table, and pid 1 already knows the pid), or `cmd/rivet` finding the
supervisor in `/proc` and signalling it directly (which needs new machinery and a
new answer about signalling an arbitrary process). That is the maintainer's, and
`PLAN.md` §31 carries it.

---

**One obligation this acquires from elsewhere, recorded 2026-08-26.** `ADR-034`
§5.3 was built that day and puts a `takes-effect` on **every property of the
configuration schema** — and in this build every one of them answers
`on-next-restart`, for the single reason that **nothing re-reads the document
while it is running.** `SIGHUP` is what makes that sentence false. So the change
that implements (d) must, in the same commit, move to `immediately` every setting
the reload actually applies. `rivet-config`'s
`every_setting_publishes_when_a_change_to_it_takes_effect` is written to fail
until it does, which makes this an obligation rather than a note: a `SIGHUP` that
reloaded a setting the schema still called `on-next-restart` would be
`ADR-034` §5.3's hazard produced by the very change meant to remove it.

**Discharged the same day, and the test fired.** §3.2.1 landed and
`every_setting_publishes_when_a_change_to_it_takes_effect` refused it until the
schema said something true — which is the whole reason for writing a test that
way. **It moved exactly one key**, and not to `immediately`:
`services.*.enabled` is now `not-asserted`, because a reload applies it to most
services and withholds it for the reachability set, and the schema deliberately
does not know service names (`Kind::NamedTables` has one entry list for every key
an operator names). `immediately` would be false for two services and
`on-next-restart` false for the rest; the third state with its reason is the only
one of the three that is true. `ADR-006` §6.5.2's argument, reaching the schema.

### 3.3 What a query socket would have to decide

Listed because each one is a place a design like this goes wrong, and because
the ingest socket has already answered three of them.

1. **The request vocabulary.** *Recommendation: it is not a new one.*
   `ADR-006` §6.4's verbs and `rivet_core::operation` already describe every
   question this machine answers. The socket should carry an operation and its
   arguments, and the supervisor should perform it — not grow a second command
   language with `status`, `services` and `phase` in it.
2. **Who may ask. RULED 2026-08-25: `0600`.** The ingest socket is `0666`
   because filing a record is something any process legitimately does
   (`ADR-022` §11.4). **Reading the supervisor's state is not the same act.**
   `0600` grants nothing to nobody new — root can already read `/state`
   directly — and `PLAN.md` §11.2's still-open question about what the
   permissive mode next door costs is reason enough not to open a second one.

   **Widening later is a decision; narrowing later is a break.** The trigger to
   revisit is a **named requester that is not root**, and if a service ever
   needs to ask the supervisor about itself, that is the moment — not before.

   *(§5's original candidate was `rivet-gitd` running as its own account. Both
   halves went on 2026-08-30: the crate was purged when `ADR-035` §4 moved git
   to smart HTTP inside `rivet-mcpd`, and the SSH half of the account model went
   with it. **The trigger is still unfired** — no non-root requester exists
   yet.)*
3. **Whether it may change anything.** *Recommendation: no. Read-only, in this
   ADR.* `ADR-011` gives the document sole authority over what a machine should
   be, and a socket that could start a service would be a second one. `ADR-017`
   §6.3.2 later made this socket the supervisor's control surface, limited to
   converging toward the document, and §6.5 added a stop held on one
   application that the next boot releases; neither writes the document.
4. **What it costs the supervisor.** A listener, an accept loop, a bounded
   read, and a response built from accessors that already exist. `ADR-022` §11
   priced the same thing at one non-blocking socket in the poll loop, and that
   is the shape to copy — **not** a thread.
5. **What happens when it is not there.** A machine whose supervisor is down is
   exactly when somebody asks. *Recommendation: the refusal names the socket and
   says the supervisor is not answering* — which is a true and useful statement,
   and is what `crate::journal::send` already does on the other side.

### 3.4 Why not widen the ingest socket

Stated separately because it is the tempting shortcut and it is a real trap.

The ingest socket's mode is a **ruled consequence** of what it is for: `ADR-015`
§5.1 gives applications their own uids, `ADR-008` §7.3 rules full Unix
multi-user, and a journal only uid 0 can write answers the question for one
account. None of that reasoning applies to reading the supervisor's state — and
a socket that does both inherits the more permissive answer for both halves.

`PLAN.md` §11.2 already flags what the permissive half costs and recommends
revisiting it *when the first application ships*. Applications shipped on
2026-08-25 (`PLAN.md` §14), so that trigger has fired and is a separate open
item. Widening the same socket in the meantime would make that decision harder
rather than easier.

---

## 4. Q2 — what an agent speaks to

### 4.1 Most of this is already ruled, and it is cheaper than it looks

`ADR-006` §3.3 has the table, and the MCP row is the least expensive one in it:

| Adapter | Transport | Authentication |
| :-- | :-- | :-- |
| MCP | **TLS, `ADR-035`** | **`ADR-025` §2's pinned Ed25519** |

This row named a **tunnelled** transport until `ADR-035` on 2026-08-28 — on the
argument that an MCP adapter is a program speaking JSON-RPC on stdin and stdout,
that something already gave a caller a program's stdin and stdout on this
machine, and that reusing it therefore needed no listening socket, no TLS, no
identity system and no new attack surface.

**SUPERSEDED 2026-08-28 — `ADR-035`.** The transport is now TLS and MCP is the
primary control plane rather than a projection tunnelled through something else.

**That argument is the honest price of the pivot**, and it is restated here
rather than dropped because `ADR-035` §3.1 quotes it as exactly the bill: the
pivot buys legacy-freedom and pays in a listening socket, TLS, an identity
system and a new attack surface — four costs this section itemised in advance
as reasons not to. Two of them turned out to be cheaper than they read here:
`ADR-025` §2 had already ruled the identity system in 2026-08-22 (pinned
Ed25519, no CA), and `ADR-019` §2 had already ruled enrollment (a machine
arrives enrolled). The listening socket and the attack surface are real and are
accepted.

**Stdio does not go away.** `ADR-035` §8A makes the same MCP reachable over the
serial console as break-glass — no network, no TLS, no identity, because
physical presence is the credential.

And `ADR-006` §3.1 makes the content of it a **conformance property** rather
than a design: every operation in `rivet-core` is reachable from every
adapter, the catalogue is generated from the core's types, and *"an operation
that exists in only one interface is a defect in that interface"*. An MCP
adapter is a projection of `rivet_core::operation::CATALOGUE` into tool
definitions. It is a small amount of code and almost no decisions.

### 4.2 The finding: today, nothing can reach it

**`root`'s login shell is `rivet`, and `cmd/rivet` executes nothing.**
That is the login-shell contract, enforced by this binary's `invocation` module:
a remote invocation arrives as `-c <one string>`, and the only thing that string may be is a
`rivet` command. It is not a shell and will not exec a program.

So a request naming any *other* program is refused, and an MCP adapter shipped
as its own binary in the image would have been **unreachable by the transport
its own ADR row named**. This had not been noticed before because nothing had
needed to run a second program remotely.

**RULED 2026-08-25, and it splits by *who is logging in* rather than by *what
they want to run*.** That split is the maintainer's correction and it is the
important half of this section.

### For `root`: a declared table of internal handlers

`rivet mcp` is a **handler the login shell runs itself**, so `ADR-005` §5.4's
*"it executes nothing"* is untouched. What was ruled is not one more bare word
but the **table**: `schema` and `measure` become the first two entries of a
declared set, and `mcp` is the third.

The reason is `AGENTS.md` §3's, applied to a pattern rather than a boundary: the
fourth special case is where somebody adds a fifth without noticing it has
become a pattern. As a table it is a list that `ADR-006` §7's catalogue can
publish — so an agent can *discover* what the login shell will answer to — and
a test can assert the table and the dispatch agree, which is the anti-drift
shape this project uses everywhere else.

### For `git`: not root at all — its own account

This section proposed the `-c "git-receive-pack …"` case as a fourth entry in
the same table, and it was withdrawn the day it was written.
**The maintainer's correction: git servers do not
run privileged, so it should be running under a service account — why is anybody
connecting as `root` in this context rather than as `git@<server>`?**
*(`.rulings/` R-049.)*

**He is right, and the record already said so.** `ADR-008` §7.3 rules **full
Unix multi-user**, and §7.3.1 records that **service identities are BUILT** —
one uid and gid per installed application, enforced by a compile-time table
plus the `/state/app` projection, so that *"service accounts arrive with the
services that need them"* is enforced rather than remembered. Every earlier
draft of this section treated multi-user as pending and reached for a
root-shaped answer because of it. That was a misreading of this project's own
ADRs, and it is struck here rather than quietly corrected.

So a `git push` authenticates as **`git@<host>`**, not `root@<host>`, and:

* it reaches an account whose login shell is a **restricted handler** that
  accepts `git-receive-pack` and `git-upload-pack` and refuses everything else
  — the same shape `git-shell` has had for twenty years, and the same shape
  `cmd/rivet` already is for `root`;
* a compromise of it is a compromise of **one repository directory**, not of
  uid 0;
* nothing about it needs the internal-handler table, because it is not `rivet`.

#### 4.2.1 The table is BUILT — 2026-08-26, with one entry, and the ruling named two

`PLAN.md` §28.5 Phase 1 item 5, §32. `cmd/rivet/src/handler.rs`.

**`schema` was retired the same day this section was written.** §4.2 names it as
the first of two entries; `ADR-006` §7.4 made it `rivet get schema` — a subject
of a verb, where the operation catalogue can see it — so it is not a bare word at
all any more and `cmd/rivet`'s parser refuses its old spelling by name. `mcp` is
not built, and `AGENTS.md` §3 keeps it out of the table rather than in it
answering *unimplemented*: a listed handler is a published claim about a surface
this build would not have.

So the table ships with `measure`, alone. **That is not a reason to have skipped
it**, and the reason is worth recording because a one-row table reads like
over-engineering until you ask what it fixed:

> **`measure` was invisible.** `rivet --version` publishes the operation
> catalogue, which is keyed by §6.4's verbs — and `measure` is not one. An agent
> reading everything this machine says about itself found no mention of a word
> the login shell answers to.

That is `ADR-006` §7's *"trusted and wrong"* with a quieter failure mode: not a
document that says something false, but a complete-looking one that omits. The
table is published beside the operations under `handlers`, and it is the first
time this surface has described itself completely.

**Each entry is declared by the module that implements it** — `measure::HANDLER`
— never beside the table. That is `rivet_core::operation::CATALOGUE`'s own rule,
and this is its adapter-side counterpart: a description kept next to a list
instead of next to the code is the thing §7 says will be wrong within a month.
`every_handler_is_reachable_and_every_reachable_word_is_listed` asserts both
directions, and it asserts the **absence** of `mcp` specifically, so that a
declared-but-unlisted name cannot quietly start working.

**Each entry also publishes `why-not-a-verb`.** A field rather than a comment,
because it is the question an agent will have: §6.4's verb set is closed, and a
word outside it needs a reason rather than an exemption. Publishing the reason is
what stops the table becoming the place special cases go to be legitimised —
which is the failure §4.2 named when it ruled a table instead of a fourth bare
word.

**It also names a case this project's record has no word for.**
`ADR-008` §7.3.1 splits identities into *service* (built, derived from an
application's name, nothing logs in as one) and *human* (deferred, a `users`
section projected into `/etc/passwd`). **`git` is neither**: it is a service
identity that **humans authenticate as, by key**. Nothing in the document
carries a per-identity key set today. Whatever
carries `git`'s is the first piece of the deferred human half to be built, and
it should be recognised as that rather than bolted on beside it.

**That is the one thing this ADR leaves open**, and it belongs to `ADR-008`
rather than here: *what is the document's shape for an account that is not an
application and is not a person, but that a person logs in as?*

### 4.3 What an MCP adapter would need that does not exist

* **Q1**, for anything about live state. Without it the tool list is the
  catalogue and `status` is missing from it.
* **A confirmation story.** `ADR-012` §7.1 makes confirmation per-*change*, and
  `PLAN.md` §21's `E_CONFIRMATION_REQUIRED` plans are **prose**
  (`ADR-006` §6.5's typed diff does not exist). An agent asserting *"this
  change is the one I intended"* against a sentence is doing string matching.
  That is the scheduled work in `PLAN.md` §22, and it is a prerequisite for an
  agent driving mutations rather than reads.
* **A decision about whether the machine may push.** MCP has notifications, and
  `--follow` proves the machine has something to say. Out of scope here; noted
  so it is not discovered late.

---

## 5. The adjacent ruling: single-quoted strings — `ADR-012` §8.3 option 2

Ruled by the maintainer 2026-08-25 and recorded here because it arrived in the
same conversation. **It is not part of this ADR's question** and does not wait
on it; `ADR-005` §5.4 is where the amendment belongs and `ADR-012` §8.3 is where
the finding is.

The short form: `cmd/rivet`'s tokeniser refuses any string a shell would
interpret, so a remote `rivet apply edit … --expect '…'` cannot be typed —
and neither can most edits worth making, because a value with a space in it
needs quoting. **Option 2 accepts POSIX single-quoted strings and nothing
else**, because a single-quoted string in `sh` has no escapes and no expansion,
so splitting one is *provably* the same answer a shell would give. It is not an
approximation of a shell; it is one more exactly-reproducible construct, which
is the test §5.4's tokeniser already applies to itself.

---

## 6. What was ruled, and what is left

**Ruled 2026-08-25, all four, the day it was opened.** This section said
*everything* was left, because the document was a draft opened to hold the
question while it was being answered.

| | Decision | Where |
| :-- | :-- | :-- |
| 1 | **A read-only query socket** at `/run/rivet/query`, request-and-response, carrying an operation rather than a second command language | §3.2, §3.3 |
| 2 | **`0600`** — root only, widened when there is a named requester that is not root | §3.3 item 2 |
| 3 | **A declared table of internal handlers** for `root`; **`git` gets its own account** and is not in it | §4.2 |
| 4 | **`SIGHUP` means re-read the document**, and a document that will not parse leaves the machine on the last good one | §3.2 |

**What is still open, and it is one thing:** `ADR-008` §7.3.1 has a word for a
*service* identity and a word for a *human* user, and `git` is neither. The
document's shape for an account that a person logs into by key, which is not a
person's account, is `ADR-008`'s to rule and is the first piece of §7.3.1's
deferred half.

**All four are built, 2026-08-26 — §7.** The order did
matter in the one place this predicted: the query socket and `get status` landed
together, and neither would have been testable alone.

## 7. BUILT 2026-08-26 — and §6's own spelling was wrong

`PLAN.md` §28.5 Phase 1 items 4, 5 and 7, §31, §32 and §35. All four of §6's
rulings are implemented; what follows is what building them changed about them.

### 7.1 `rivet status` is `rivet get status`

**Ruled by the maintainer 2026-08-26.** §6 names the command `rivet status`, and
that was written the same day `ADR-006` §7.4 retired `rivet schema` for being a
bare noun — *"a bare noun is not a command; it is a subject of `get`, which is
where the operation catalogue can see it"*. The same argument applies here
unchanged, and it is stronger for §3.3 item 1's sake: the socket is ruled to
carry **an operation**, and `get status` already is one.

`rivet_core::operation::status` is the subject; the socket carries
`get status\n` and nothing else in this build.

### 7.2 What crosses the socket, narrowed

§3.3 item 1 says the socket *"should carry an operation and its arguments, and
the supervisor should perform it"*. **Ruled 2026-08-26: only operations the
supervisor is the sole authority for cross it, which today is exactly one.**

The wider reading — the supervisor performs any read-only operation — was
considered and refused for a reason §3.3 item 5 already implies: it would put
the journal and the storage table behind a process `ADR-017` §3.1 explicitly
permits to be dead, and duplicate two paths that work without it. What makes
`status` different from every other `get` subject is that **no file holds the
answer**: which services are running, their pids and their restarts exist only
in the supervisor's memory, by `AGENTS.md` §5's *derive, never store*.

### 7.3 The wire format, and why it is not JSON

Recorded because it is the decision an implementer would otherwise re-litigate.
**This workspace has no JSON parser and that is a ruling.** `rivet get log`
sidesteps it by returning the journal's lines as strings for `nu`; that is right
for records an operator pipes and wrong for a result `--output text` renders.

So the format is tab-separated lines, and **both halves are one definition in
`rivet-core`** with the round trip asserted. A service name cannot contain a tab,
so there is no escaping — which is the property a JSON encoding would have cost
a parser to buy.

### 7.4 What the emulator settles here, and it is more than for §3.2's signal

`tools/qemu/smoke.sh` asserts the whole exchange: the CLI connects, the
supervisor answers from its own memory, and the CLI parses it back. **Nothing
about this path identifies pid 1**, which is what makes it reachable in the
emulator where `apply reload` is not (`ADR-026` §4.4). The assertion that makes
it a live answer rather than a shape is that a service the caller depended on to
get there appears in it as `started` — the exchange asserting the very thing
that carried it.

---

## 8. Two rulings about what a read may do — 2026-08-29

### 8.1 The query socket became the supervisor's control surface — `ADR-017` §6.3

**Ruled 2026-08-29 and recorded in `ADR-017` §6.3, not here.** §3.2 wrote that
the socket carries *a whole operation and not the bare subject*, and closed with
*"the day a second one is ruled across it needs no new grammar."* That day
arrived: service control crosses it, and the authority rule is **converge-only**
— the surface may ask the machine to converge toward the document, never to
diverge from it.

**It is recorded in `ADR-017` because the ruling is one ruling** — it covers
`rivet-sysd`, `rivet-initd`, `rivet-netd`, `rivet-timed` and the rest in one
pass, and splitting it between two ADRs would give it two homes and eventually
two answers (`ADR-011`). This section is the pointer, and the widening §3.4
demands be *argued* rather than assumed is argued at `ADR-017` §6.3.1.

**BUILT the same day.** `rivet apply service-restart <name> confirm` crosses this
socket, so §3.2's *"the day a second one is ruled across it needs no new
grammar"* is now a fact rather than a prediction: the request is
`apply service-restart <name>`, spelled the way an operator types it, and §7.2's
*"only operations the supervisor is the sole authority for cross it, which today
is exactly one"* is **two** — the second for the same reason as the first, since
whether a service may be restarted depends on state that exists only in the
supervisor's memory. What was built, what it departs from, and what no board has
run is at `ADR-017` §6.3.2.

**`apply service-stop` and `apply service-start` cross it too**, for the same
reason: a hold on one application lives only in the supervisor's memory, and
only the supervisor can say what stopping or releasing it would do from the
state it holds. `ADR-017` §6.5 is the rule, including why `rivet-mcpd` and
`rivet-netd` — this ADR's §3.2.1 reachability set — are refused.

**One thing this ADR says is corrected there rather than here**, and is named
so a reader of §3.2.1 finds it: the *code* implementing this ADR's withheld
memory cap told an operator it applied *at the next start of the service*, and a
per-service restart made that false. The ruling in §3.2.1 was never wrong — it
says *at the next boot* — and it is the two runtime sentences that were fixed.

### 8.2 RULED: a read may not `mount(2)` — 2026-08-29

**Moved here from `PLAN.md` §99.2 on 2026-08-29, unaltered.** Only the heading
number changed. **Bare `§N` references in the body are `PLAN.md` sections** and
resolve through that file's pointer table; `§95.1`, which it names as the way to
close the gap, is `ADR-017` §6.3.1 and §8.1 above.

This belongs in this ADR because it is a **port contract on the read path**, and
this ADR's subject is what may be asked of a running machine. `Observed` is the
port; the ruling says what an implementation of it is allowed to do to the
machine while answering — which is nothing.


`get slot` reports which slot is running and whether this boot is a trial. It
does **not** report what is armed as the default, because that lives in
`autoboot.txt` on the boot partition and reaching it needs a mount. The agent
left it unread rather than deciding on its own that reads may mount —
`AGENTS.md` §11.1, and correctly.

**Ruled: `Observed` reads what is already visible — `/proc`, `/sys`, `/state` —
and mounts nothing.**

A mount is a change to the machine, however transient, and a read port that
mounts fails in ways no other read can. That is not hypothetical here:
`cmd/rivet-deploy/src/install.rs` carries a load-bearing comment about exactly
this, because `Kernel::mount` treats *"already mounted here"* as the convergent
answer, so **a failed unmount leaves a read-only mount in place and the next
writable mount converges onto it** — every subsequent write then answers `EROFS`
on a perfectly healthy card. A read subject that could leave that behind would
break the installer from the outside.

**The cost is real and is stated rather than hidden: nothing can currently ask a
board what it will boot next.** `get slot` names the armed default as not read
instead of guessing.

**The way to close it, when it is worth closing, is `ADR-017` §6.3.1's** — the supervisor
already reads the boot medium and is the natural authority for A/B state, so the
answer belongs on the query socket beside `get status` rather than in a CLI that
mounts things. Recorded as a named gap rather than rediscovered later (§94.7).




---

## 8. `get status` says what the machine is FOR — added 2026-09-01

§7.2 rules that **only operations the supervisor is the sole authority for cross
the socket**, and gives one test for it: *no file holds the answer*. Kiosk mode
appears to fail that test — the `[kiosk]` table is in a file — and it is here
anyway, because the test is about *authority* rather than about storage, and
this answer was already crossing in the same shape.

### 8.1 What was wrong, and it was the worst class

`cmd/rivet-fleetd`'s page decided whether a machine was a kiosk with:

```js
const kiosk = typeof doc.contents === "string" && doc.contents.includes("[kiosk]");
```

— a substring test on the raw text of `get desired-state`. It is **true** for a
comment that mentions the table and for a table with no keys in it, and
**false** for a machine whose document will not parse, where it then answered
`headless`: a confident claim about a document nobody had read. `AGENTS.md` §5's
first rule is refuse rather than guess, and this guessed in the direction that
succeeds.

### 8.2 Why the supervisor, and not a new port

`rivet-core` cannot read the table. Its `[dependencies]` table is empty by rule
(`AGENTS.md` §10.1), so it has no TOML parser and could only have string-matched
— the same defect one crate further in. `cmd/rivet-sysd` **already parses the
document**, and — this is the part that decides it — **it already reports
document-derived facts through this same answer**: `services.*.enabled` is one,
and §5.2 above is a whole section about that field's semantics. So the fact is
not a new kind of thing crossing the socket. A new method on `Observed` would
have been, at a cost of twenty-two implementors.

### 8.3 Three values, and `locked` is not one of them

**`mode` decides; `[kiosk]`'s presence is only ever its fallback** —
`.rulings/` R-164, R-165. A document's own `mode` key, where it names one, is
the switch: `headless` answers `headless` even when a `[kiosk]` table is
present, and `kiosk` answers `kiosk` from the table when one configures it. A
document naming no `mode` reads exactly as it always has — a `[kiosk]` table
makes it a kiosk, no table makes it headless — which is what keeps every
document written before `mode` existed, `rivet-01`'s included, meaning what it
always meant. A `[kiosk]` table's presence is never itself a claim that a
kiosk is active, and nothing clears it when `mode` names something else: R-165
leaves the same room for a future `[desktop]` table on identical footing.

| `mode` | when |
| :-- | :-- |
| `kiosk` | the document names `mode = "kiosk"`, or names none and has a `[kiosk]` table |
| `headless` | the document names `mode = "headless"` (whether or not `[kiosk]` is present), or names no `mode` and has no `[kiosk]` table, or there is no document — **a real reading**, not an absence |
| `unknown` | there is a document and it could not be read. `mode_unknown_because` carries the machine's own sentence |

**`mode = "kiosk"` with no `[kiosk]` table is tolerated and reported, not
refused.** It still answers `kiosk` here — that is the fact worth surfacing —
while `locked` and `auto_run` carry the same values an empty or absent table
always has, and `kiosk_unconfigured_because` names why: a kiosk was asked for
and nothing configures it. Silently downgrading the report to `headless` would
hide exactly the disagreement an operator needs to see; silently honouring the
table's absence as if it were the true answer is what §8.1's defect already
was, one key over.

**`locked` is a separate boolean beside the mode, not a third word for it**, and
that is a decision rather than a layout:

* They answer different questions with different consequences. An unlocked kiosk
  still holds the display and still keeps the console off it; a locked one
  additionally refuses `long-back` (`PLAN.md` §108).
* A word like `kiosk-locked` would make every caller that wants *is this a
  kiosk* match a prefix — which is the string-matching this whole change
  removes, moved one level in.
* It would have to grow again the next time a kiosk property is ruled.

`locked` and `auto_run` are **absent on a machine that is not a kiosk**. A
`locked: false` beside `mode: headless` would invite the reading *a kiosk
somebody left unlocked*, and absence is the convention `pid` and `reason`
already follow in this answer.

A machine that is `locked` with **no** `auto_run` is locked to an application it
was never told, and its launcher puts a fault screen up. The two fields say so
together; nothing here predicts the launcher.

### 8.4 The version skew, and the one thing it must not answer

The mode crosses as a `kiosk` line, second, before the services — so a truncated
read loses services rather than the mode. **A response with no `kiosk` line
parses as `unknown`, never `headless`**, because the only way to see one is a
supervisor older than the CLI asking, and filling the gap with a default would
reintroduce the defect through a version skew instead of a string match. Both
halves ship in one image, so this takes an A/B slot older than the caller.

### 8.5 A reload does not overwrite a known mode with `unknown`

`crate::reload` re-reads the document on `SIGHUP` and sets the mode from it —
**except when the document will not parse**, where the last good answer stands.
That is the same rule §3.2.1 already applies to everything else a reload
touches: nothing on the machine changed, and replacing `kiosk` with `unknown`
would describe a machine still running its application as one nobody can
describe. The read failure is in the journal at `Severity::Error`, put there by
`document::load`, which is where the honesty about it lives. **Boot is the other
case and takes the other answer**: there, nothing stands, so `unknown` is the
only true one.

### 8.6 `get status` also carries the image's own version — added 2026-09-13 (`.rulings/` R-163, R33)

No `get` subject published the running image's version string, and the fleet
portal's Overview page had to say so — plan §8's Wave 6 built the card that
asked for it and found nothing on the machine to answer it with. **`version`
is now a field of `get status`, `String`-typed and absent rather than `""`.**

It is read from `cmd/rivet-sysd`'s own `CARGO_PKG_VERSION`, passed into
`Status` once, at the same point `phase` and `machine` are. There is exactly
one build number a running image can report, so this is a second reader of an
existing fact, not a second source of one: `rivet --version` and `get status`
cannot disagree about which build is running, because both ultimately trace to
the one `version.workspace = true` every crate in the workspace takes.

**Absent, never empty, on a supervisor older than this field** — the same
convention `kiosk` and `display_master` already use for the same reason
(§8.4): the only way to see no `version` line is an A/B slot whose supervisor
predates it, and a default would report *this build has no version* about a
machine that simply said nothing.

**On the wire, after `display` and before the services** — not among the
first two lines `phase` and `machine` occupy, because a build's version is
fixed for the life of the boot and a truncated read losing it has lost
nothing that changes tick to tick, unlike everything ahead of it.

### 8.7 `phase` says `running`, and the field explains itself

**`phase` is one of three words: `running`, `shutting-down` or `finished`.**
`running` is the supervisor's normal state for as long as the machine is up.
On every pass it reads the desired-state document, observes what is actually
running from the cgroups, and starts or stops services until the two agree
(`ADR-017` §3.1.2, §3.2). That loop never finishes, so there is no `converged`.
Whether one service is where the document wants it is answered by that
service's own `enabled` and `state`, not by `phase`. `shutting-down` means every
service is being brought down. `finished` means all of them are down; it is
never seen over the control plane, which is down by then.

**The word names the condition, not the mechanism.** The supervisor's own type
keeps the loop's name, `Phase::Converging`, because in the code that is exactly
what it is. An operator reading `converging` in an answer takes it to mean the
machine has not got there yet, and acts on that. *(`.rulings/` R-373.)*

**A build older than this says `converging` for the same state**, and every
reader treats the two alike. Nothing compares the value: `cargo rivet run` and
`rivet-fleet` print it, Rivet Monitor stores answers verbatim, and the facts an
application is handed (`ADR-057`) carry it through as the supervisor said it,
never rewritten. A reader that glosses it, as the fleet server's page does,
glosses both words the same.

**The field's own catalogue description is where an operator reads this**:
`rivet get catalogue get status` prints it beside every other field, from the
same constant that names the field, so the two cannot drift apart.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### Status block — nothing in the document was built

**BUILT 2026-08-26 — all four rulings, §7.** The superseded bullet read:

~~**Nothing in this document is built.** `PLAN.md` §24 carries the work.~~

### In §3.1 — the recommendation before the ruling

**Ruled 2026-08-25: (a), and (d) as well.** The superseded sentence read:

~~**Recommendation: (a), with (d) as a separate and much smaller decision.**~~

### In §4.1 — MCP's transport was stdio over an existing remote-shell channel

**Superseded 2026-08-28 by `ADR-035`.** The row named that channel as both the
transport and the authentication, and the argument under it was that this cost
no listening socket, no TLS, no identity system and no new attack surface. §4.1
carries what that argument turned into, because `ADR-035` §3.1 quotes it as the
price of the pivot.

### In §4.2 — git as a fourth entry in the handler table

**Struck the day it was written**, on the maintainer's correction that git
servers do not run privileged. The superseded sentence read:

~~The `-c "git-receive-pack …"` case is a fourth entry in the same table.~~

### In §6 — everything was left open

**Ruled 2026-08-25, all four, the day the document was opened; built 2026-08-26.**
The superseded text read:

~~Everything. It is a draft opened to hold the question while it is being
answered…~~

~~**Nothing here is built.**~~

### In §8 — the closing pointer lived in `PLAN.md`

**Changed 2026-08-29**, when `PLAN.md` was split. The superseded pointer was
~~`§95.1`~~; it is `ADR-017` §6.3.1.
