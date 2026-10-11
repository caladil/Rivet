# ADR-012: The Agent as a First-Class Operator

* **Status:** **Accepted** — ruled 2026-08-22. §4, §5 and §7 all adopted as
  recommended. **§8 added 2026-08-24**, when the maintainer ruled
  `rivet apply edit` — the one thing §2's table did not cover and an agent
  could not do at all.
* **§7.3 added 2026-08-29 — a SECOND confirmation tier.** The maintainer ruled
  that `confirm` alone is not a safety mechanism for the exceptional case,
  because it is the same word typed for a restart and for every routine edit.
  `--accept '<the specific hazard>'` echoes what is about to break, and
  §7.3.3 carries the doctrine that decides when it fires — WARNING is
  reserved for the exceptional case. *(`.rulings/` R-030.)* **§7.1 is
  unchanged and is not superseded**: the second tier is above it, not instead
  of it.
* **Date:** 2026-08-22, amended 2026-08-24 and 2026-08-29
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

**The maintainer's statement of what the administration surface is for: whoever
wants to be a command-line jockey has it; whoever wants a GUI can have that; and
if what you want is Claude connecting remotely to configure the device, that is
no problem either.** *(`.rulings/` R-028.)*

The first two follow from `ADR-006` section 3.1 and need nothing further. The
third is the goal that was stated at the outset of this project's design —
*AI-friendly ways of configuring and administration* — and it is the least
specified part of the record.

It is also the one that is not quite free, and this ADR is about the gap.

---

## 2. Most of it is already done

Worth stating first, because the remaining work is small and it is easy to
mistake this for an unsolved problem. An agent administering a Rivet host
needs, in descending order of importance:

| Requirement | Where it already exists |
| :-- | :-- |
| Machine-readable output, not prose to scrape | `ADR-006` section 4 |
| Errors it can branch on, not English to match | `ADR-006` section 4.1 |
| A surface it can discover without prior knowledge | `ADR-006` section 7 |
| Predictable command construction from the schema | `ADR-006` section 6.4 |
| The ability to see a change's consequence before committing | `ADR-006` section 6, `ADR-007` |
| Idempotent operations, because agents retry | `ADR-006` section 6.2 |
| A single answer to any question | `ADR-011` |
| Transport and authentication | the control plane, per `ADR-006` section 5 |

**None of that was designed for agents specifically.** Every row is there
because it is what makes the system honest, and an agent is simply the operator
least able to compensate for dishonesty. That is the useful framing: Rivet is
not being made AI-friendly, it is being made *legible*, and agents are the
strictest test of legibility.

---

## 3. Plan and apply is the safety mechanism, not the friendliness mechanism

Worth separating, because they get conflated and only one of them is load-bearing
here.

* Structured output and a typed catalogue make an agent **able** to operate the
  system.
* `plan` before `apply` makes it **safe** to let one.

An agent driving imperative commands cannot preview consequences, cannot verify
what it did, and cannot roll back. An agent that must produce a diff — which a
human can read, and which is machine-checkable — is a fundamentally different
risk proposition. The reviewability is the entire point.

This is the strongest single argument for ruling `ADR-007` in, and it is worth
making explicitly: **`plan`/`apply` is what makes "let Claude configure the
device" a reasonable sentence rather than an alarming one.**

---

## 4. Decision: provenance records the actor, not only the layer

**Ruled 2026-08-22: adopted, from the start.**

`ADR-007` section 3.4 makes every effective value name the **layer** it came
from. Once more than one kind of operator can write, that is no longer enough.

> `net.wlan.country = GB`
> from: machine configuration
> **set by: `agent:claude` at 2026-08-22T14:02Z**
> overrode: image default (`US`)

Kubernetes solved the same problem with server-side apply's field managers, and
for the same reason: once several actors write to one document, "which layer"
stops being the interesting question and "who" starts.

**Why now rather than later:** the layering machinery is being designed anyway,
and an actor field is one more column while it is being built. Retrofitting
provenance onto values already written means every existing value has an unknown
author for ever.

The accountability property this buys is what makes agent administration
acceptable rather than merely possible — every change can name who made it, and
`ADR-011`'s honesty principle extends naturally from *which authority* to *which
actor*.

---

## 5. Decision: capability scoping is a filter over the catalogue

**Ruled 2026-08-22: the catalogue is shaped so scoping is possible; enforcement
waits for Stage 3.** Nothing needs enforcing while there is one operator and no
agent connected, but the catalogue's shape must permit it without redesign.

A human administrator has judgement. An agent has a prompt. The two should not
necessarily hold the same credential.

The useful observation is that Rivet does not need to invent a permission
system to fix this. `ADR-006` section 7 already generates an **operation
catalogue** enumerating every administrative operation. A scoped credential is
therefore a *subset of that catalogue* — a filter, not a parallel model.

Sketch, not a specification:

| Scope | Might permit |
| :-- | :-- |
| `read` | every `get`, `list`, `explain`, `plan` |
| `configure` | the above, plus `set` and `apply` within named schema subtrees |
| `full` | everything, including image roll and `/state` operations |

An agent given `configure` can bring up wireless and cannot roll back an image
or wipe machine state. That is a meaningful reduction in blast radius for
roughly the cost of a field on a credential — **but only because the catalogue
exists.** Without it this would be a bespoke permissions subsystem and would not
be worth building at this stage.

### 5.1 The honest limit

**A prompt is not a security boundary.** An agent holding a credential can do
whatever that credential permits, and no amount of instruction changes that.
Two things are actual controls and should not be confused with instructions:

* **The credential's scope**, per this section.
* **Human review of a `plan` before `apply`**, per section 3.

Everything else — careful prompting, agent instructions, this repository's own
`AGENTS.md` — is guidance that improves behaviour and constrains nothing. Saying
so plainly here is itself an instance of `ADR-011`: the credential is
authoritative for what an agent may do, and the system should not imply that
anything else is.

---

## 6. The adapter

`ADR-006` section 3 already makes an MCP server one thin adapter over
`rivet-core`, and section 3.3 notes it inherits the transport's authentication.
Nothing further is decided here.

The test of whether this ADR and `ADR-006` were right is concrete and should be
stated so it can be checked later: **if the MCP adapter is more than a thin
mapping from the operation catalogue onto tool definitions, the boundary was
drawn in the wrong place.**

---

## 7. Ruled 2026-08-22

1. **Actor provenance — from the start.** §4.
2. **Capability scoping — designed for now, enforced at Stage 3.** §5.
3. **Confirmation before apply — required by default; unattended apply is an
   explicit grant on a credential.** §7.1.
4. **A second tier above it, for the exceptional case — ruled 2026-08-29.**
   §7.3. It adds to this list rather than amending item 3.

### 7.1 Confirmation is per *change*, not per machine

The obvious objection to requiring confirmation is that it destroys automation,
and `ADR-006` §5.1 has already committed to a dashboard that pushes changes to
several machines. Confirming each one would be intolerable.

It does not follow, because **the unit of confirmation is the change, not the
target.** The dashboard computes one plan for a fleet-wide change, the operator
confirms once, and it applies to every machine in scope. That is both coherent
and what an operator would actually want — the interesting question is "what is
about to happen", and it has one answer regardless of how many machines it
happens to.

**Unattended apply exists**, for scheduled drift correction and for an agent
trusted to work alone. It is a **scope on a credential** (§5), never a default
and never a flag anyone can simply pass. An agent may be trusted to apply
without supervision only because that trust was deliberately granted.

### 7.2 Two different protections, both retained

Worth keeping distinct, because they sound similar and guard opposite failures:

| Mechanism | Protects against | Ruled in |
| :-- | :-- | :-- |
| `--confirm-within` auto-revert | a change that **works** and locks the operator out | `ADR-007` §4.1 |
| Confirmation before apply | a change that **should not have been made** | §7.1 |

Neither substitutes for the other. The first is about reachability, the second
about intent.

### 7.3 A SECOND confirmation tier, for the exceptional case — ruled 2026-08-29

**Ruled by the maintainer, Joel Lindell, 2026-08-29**, while `ADR-006` §8.10's
file-management subjects were being built. **For anything that can cause damage
he wants the form `<WARNING> This could cause damage? (N/yes)`.**
*(`.rulings/` R-029.)*

Default **no**, and the whole word `yes` rather than `y`.

#### 7.3.1 What is wrong with one tier

§7.1 gets the plan in front of the operator, and that part is right and is not
what this corrects. What it terminates in is `confirm` — **the same single word
for `apply restart`, for `apply install`, and for every routine `apply edit`.**
A word typed by reflex is not a safety mechanism for the dangerous case, because
it carries no information about *what is being accepted*: an operator who typed
it for the last nine commands types it for the tenth.

#### 7.3.2 One mechanism, two renderings — because the operator may be an agent

This ADR's whole subject is that a human and an agent are co-equal on one
surface, and `ADR-006` §3.1 makes that structural. **So the mechanism cannot BE
a TTY prompt**: there is no terminal over `ADR-035`'s control plane, and a
protection that only exists for the human half is not a protection.

| Where | How it renders |
| :-- | :-- |
| a terminal | `<WARNING> <the specific hazard>? (N/yes)` — the maintainer's own phrasing, default no, the whole word |
| the API, and any adapter | the caller supplies `--accept '<the specific hazard>'` beside `confirm` |

**The acknowledgement echoes the hazard, and that is the whole design.** It is
`ADR-006` §8.9.2's `--expect` idiom, applied one level out: `apply edit` makes a
caller state what it believes is *there* and be right, and this makes a caller
state what it believes it is about to *break*. Neither can be produced without
having read something. A fixed second token — `--yes-really` — could be appended
blindly by an agent's template and would protect nothing.

**A superfluous acknowledgement is refused by name.** `--accept` on a change with
no warning is an argument error, not an ignored word, because a caller that
appends it to everything has turned the second tier back into the first.

#### 7.3.3 The doctrine: WARNING means the machine is objecting

**The doctrine itself is ruled in §7.4**, moved there from `PLAN.md` §98 on
2026-08-29 — including the preference order this section does not carry:
structure, then refusal, then WARNING, then plain plan prose. This subsection
applies it to the tier; §7.4 is where it is ruled.

The mechanism is the easy half. **When it fires is the part that decides whether
it is worth anything**, and the maintainer ruled that too, in the same
conversation: **WARNING is reserved for the case where something exceptional is
happening — it is the operating system's one opportunity to object before the
operator wrecks something.** *(`.rulings/` R-030.)*

**The tier's entire value is its rarity**, and this project has its own evidence
for what happens otherwise. `PLAN.md` §94.2 found `ADR-029` §9.3's F5 warning
firing on **every** install targeting slot A — a standing condition rather than
an exception — and recorded that it *"became wallpaper."* That finding predates
the doctrine and is exactly what the doctrine forbids: **a warning that fires
every time is not a warning**, and a second word that fires often is worth
precisely as much as the first one.

So the test for reaching this tier is **not** *"is this important"* and **not**
*"would I want to know"*. Both of those are answered by the plan's prose, which
is where almost everything belongs. The test is:

> **Is something exceptional happening, and is this the machine's chance to
> object before the operator does themselves harm?**

A useful sharpening, from applying it in `ADR-006` §8.10: **the harm is not
undone by repeating what you just did.** An edit discarded by the next update is
retyped; a deleted payload is re-deployed; a program that no longer matches its
signed digest needs a rebuilt package from a workstation, which on a headless
board at `ADR-035` phase 5 may be a long way away.

#### 7.3.4 Where it is in force today

**Two paths in the whole catalogue**, and the shortness of that list is the
point rather than a sign the work is unfinished:

| Reaches the tier | Why |
| :-- | :-- |
| writing, moving or removing an application's **program** (`ADR-015` §4.2.1) | the machine stops working, and it stops **later**, at a launch nobody is watching |
| the same for its **signed manifest** | the same effect by a different route — the launch check has nothing valid to verify against |
| `apply remove-tree`, if it is ever ruled (`ADR-006` §8.10.13) | an act whose size the operator cannot bound by reading |

**Deliberately NOT in the tier**, and each is a decision rather than an
oversight: removing a stale `.rvt` payload from `/state` (the routine case
`ADR-006` §8.10.1 exists to enable — friction there would defeat the ruling);
editing a file inside an installed version directory (nothing is hosed, the edit
is discarded at the next update, and that belongs in the plan's prose); anything
in an application's `data` directory; and every existing `apply` subject, none of
which was changed by this ruling.

#### 7.3.5 What is NOT built, stated so it is not assumed

**The terminal rendering is text in the plan, not an interactive prompt.** The
warning appears as `<WARNING> …? (N/yes)` in the plan an operator reads, and the
command the plan says to repeat already carries the `--accept` clause so it can
be pasted back. **Nothing in this build reads a reply from stdin.** `rivet` is
the login shell invoked as `-c` over a session whose stdout
is load-bearing, and making it read from a terminal is a change to that contract
rather than to this mechanism. Whether it should is the maintainer's; the
mechanism above is complete without it and identical for both kinds of operator,
which is §7.3.2's whole point.

### 7.4 DOCTRINE: what `WARNING` is for — ruled 2026-08-29

**Moved here from `PLAN.md` §98 on 2026-08-29, unaltered.** Ruled by the
maintainer, Joel Lindell, and set out below. Only the heading numbers
changed. **Bare `§N` references in the body are `PLAN.md` sections** — `§97` is
the ruling §7.3 above was written from, `§97.3`'s *"middle row"* is the config
file inside a version directory that §7.3.4 lists as deliberately outside the
tier, and `§94.2` is the F5 warning that became wallpaper.

**Its relationship to §7.3 is deliberate and is not a duplicate.** §7.3 is the
**mechanism** — a second tier, its two renderings, and where it is in force
today. §7.4 is the **doctrine that decides when the mechanism may be reached
for**, and it governs more than the file surface: §7.3.3 states the test and
cites this ruling; this section is the ruling itself, with §7.4.4's preference
order, which §7.3 does not carry.

**A note on `ADR-011`, because this section is about not saying things twice.**
§7.3.3 and §7.4.2 both quote `PLAN.md` §94.2's *"wallpaper"* finding. That is one
finding quoted from one place by two sections with different jobs, not two
authorities: §7.4 is where the doctrine is ruled, and §7.3.3 applies it.


**Ruled by the maintainer, and it governs more than the file surface. The
official position is this: WARNING is reserved for the case where something
exceptional is happening — it is the operating system's one opportunity to
object before the operator wrecks something.** *(`.rulings/` R-030.)*

#### 7.4.1 What it decides immediately

* **§97.3's middle row is settled.** A config file inside `/state/app/<name>/<version>/`
  does **not** get the second tier. Nothing exceptional is happening and nothing
  is being hosed — the edit simply will not survive the next update. That is
  **plan prose**, terminating in the ordinary `confirm`.
* **The tier's whole value is its rarity**, and the doctrine is what protects
  it. A WARNING on merely-notable operations makes the second word worth exactly
  as much as the first, which is the failure it exists to prevent.

#### 7.4.2 It applies retroactively, and `PLAN.md` §94.2 is the evidence

**This was measured before it was stated.** §94.2 found `ADR-029` §9.3's F5
warning firing on *every* install targeting slot A — a standing property of the
direction, not an exception — and named the consequence: it **became wallpaper**,
which is how the mitigation went six days unruled while the machine said so each
time.

Corrected the same day: that record is now `info`, its text no longer claims the
mitigation is unruled, and `warn` on that path means **the selector was actually
damaged** — which `repair_autoboot` emits and which is genuinely exceptional. The
test was renamed with it, and its assertion was already on the *content* rather
than the severity, which is why saying it stayed required while how loudly it is
said remained a judgement.

**The general rule:** a condition that holds on every run of a normal operation
is not an exception, whatever its consequences would be if things went wrong.

#### 7.4.3 A warning must name the specific consequence

**Raised by the maintainer as a joke that carried a real requirement — a warning
that asks whether you REALLY want to overwrite `rivet-initd` with a "Hello
World", and tells you it is not going to turn out the way you think.**
*(`.rulings/` R-031.)*

**That is what a warning should read like**, and *"this could cause damage"* is
not. A generic hazard class is the phrasing that becomes wallpaper fastest,
because it is identical every time and therefore carries no information.

It is also why §97.2's acknowledgement **echoes the hazard**: a caller cannot
meaningfully echo a generic string, so requiring the echo forces the plan to say
something specific enough to be worth echoing. The two rulings hold each other
up.

#### 7.4.4 The best warning is the one that is structurally impossible to need

**The maintainer answered his own example: that filesystem, he noted, is
read-only in any case.** *(`.rulings/` R-032.)*

**Correct, and it is stronger than a mount flag.** `ADR-018` §3 is titled
*`/system` is read-only by format, not by mount flag* — the slots are `erofs`,
so there is no write path for a root process to take, rather than a flag it
could remount away. Confirmed on the live board 2026-08-29: `/` is `ro` on
`/dev/root`.

So the joke's premise cannot happen, **by design and not by warning**, and that
is the preference order this doctrine implies:

| | |
| :-- | :-- |
| 1. **Structure** | make it impossible — `/system` is `erofs` |
| 2. **Refusal** | make it require a different act — `apply edit` refuses a view, `--expect` refuses a wrong belief |
| 3. **WARNING** | the machine objects, and the operator overrides deliberately |
| 4. **Plan prose** | say what will happen, and proceed on the ordinary `confirm` |

**A WARNING is what is left when the first two are unavailable.** Reaching for
one where structure would do is a design failure wearing a safety mechanism's
clothes.

**The honest scope**, so this is not read as more comfort than it is: `/system`
being unwritable protects the *running* system's binaries. It does not protect
`/state`, it does not protect the inactive slot — which the installer writes by
construction — and it does not protect an application's own files. Those are
where the file surface can do harm, and where §97's tier applies.



---

## 8. The row §2 did not have: changing a file — ruled 2026-08-24

§2's table said *"most of it is already done"*, and every row of it was true.
**There was one thing an agent could not do at all**, and it went unnoticed
because the table is about *qualities* of the surface rather than about the
things an operator does.

An agent asked to change a line of a file on a Rivet machine had no tool. There
is no `sed`, no `cat`, no editor; `rivet` is the login
shell precisely so that nothing else can be run, and `ADR-006` §6.4's verb set
had no operation over a file. So an agent would improvise in nushell —
`open f | str replace … | save -f f` — with no atomicity, no check that what it
matched is what it believed was there, no refusal on a projected view, and no
record that anything happened.

**That is not a first-class operator. That is improvising, in a system whose
whole argument is legibility.** §2's closing sentence is the one that makes it a
defect rather than a missing feature: *"an agent is simply the operator least
able to compensate for dishonesty."*

**Ruled 2026-08-24: `rivet apply edit <path> …`.** `ADR-006` §8.9 is the
operation, its refusals and what it cost §7's shape. Three things about it belong
in *this* ADR rather than that one.

### 8.1 It is §3's argument in the smallest available form

§3 separates the two mechanisms and says only one is load-bearing: *"`plan`
before `apply` makes it safe to let one."* `apply edit` is the first operation
where a plan is a **diff** rather than a sentence, and it is therefore the
strongest case yet for `ADR-006` §6.5's typed dry run — which does not exist, so
this operation's plan is prose in the message of an `E_CONFIRMATION_REQUIRED`
refusal, exactly as every other mutating subject's is.

**An agent cannot read that plan structurally**, which is the one place this
operation falls short of §2's first two rows. It is recorded in `PLAN.md` as a
question rather than answered here, because §6.5 is `ADR-006`'s.

### 8.2 §4's actor provenance is met for the first time by an operation

§4 was ruled *"from the start"*, and every record on a Rivet machine has
carried `ADR-022` §5's actor since the journal existed — but the actor has always
been *the process doing the work*: `supervisor`, `update`, `time`. §4 asks a
different question, *who set this*, and the only operation that had begun to
answer it was `apply install`, whose principal reaches the supervisor's record of
taking the request up and stops there (`ADR-028` §4.5).

**`apply edit` is the first operation whose own record names who asked** — the
caller that authenticated, beside what was changed, in one line. It is
attribution and not authorisation, and the record says so: nothing checked it,
and anything running as uid 0 could have written the file another way and left
nothing at all.

That last clause is the honest form of §5.1's *"a prompt is not a security
boundary"*. What an edit through this operation buys is not a control. It is a
**record**, and one whose caller can be matched against the key the machine
pinned to admit that connection — a forgery that leaves
evidence rather than a blank where the evidence would be.

### 8.3 The finding: an agent cannot type this command remotely today

**Found while building it, 2026-08-24, and it is a gap between the ruled surface
and the machine rather than a defect in either.**

A remote `rivet apply edit … --expect '…' …` arrives at the login shell as
`-c <one string>`. `cmd/rivet`'s tokeniser **refuses** any
string containing a character a shell would interpret — `ADR-005` §5.4's
*"complete or absent, never partial"*, and the reasoning behind it is sound: an
incomplete shell fails in ways attributed to the caller rather than to the
environment. Quotes, `$`, `#`, `!`, `*`, `~`, `;`, `&`, backslash and backtick
are all in that set.

**So the maintainer's own example cannot be typed over a non-interactive
session**, and neither can most edits worth making, because a value with a space
in it needs quoting and quoting is refused. What works today is an interactive
`nushell` session on the machine (`ADR-005` §5.1) — nushell does the quoting and
execs an `argv`, which never reaches the tokeniser. That is the path this
operation was exercised through, and it works.

`a_dash_c_command_carrying_quoted_text_is_refused_and_that_is_the_gap` in
`cmd/rivet` pins it as a test, so the day it is ruled the sentence fails rather
than going stale.

**Three options, in `PLAN.md`, and the choice is the maintainer's** because every
one of them amends `ADR-005` §5.4:

1. **Leave it.** `apply edit` is reachable from an interactive session and from
   any future MCP or network adapter (`ADR-006` §5.1), which bind named inputs
   and never see a shell at all. The cost is that the *ordinary* way an agent
   drives a machine today — one command in one string — cannot reach it.
2. **Accept POSIX single-quoted strings, and nothing else.** A single-quoted
   string in `sh` has no escapes and no expansion, so splitting one is
   *provably* the same answer a shell would give — which is the exact test
   §5.4's tokeniser already applies to itself. It is not an approximation of a
   shell; it is one more construct that is exactly reproducible.
3. **A blob spelling** — `--expect-base64 <blob>` — which is *complete rather
   than partial* in §5.4's own terms, carries no shell metacharacter at all, and
   also solves passing a trailing newline. It is a second spelling of an input,
   which §6.4's one-spelling rule is against.

The recommendation is **2**, with **3** as the thing to add if a trailing newline
turns out to matter in practice.

### 8.3.1 **RULED 2026-08-25: option 2.** Single-quoted strings, and nothing else

The maintainer took the recommendation. `ADR-005` §5.4 is amended by §5.4.2
there; this is the finding's own record of its outcome.

**The reason, kept because it is what bounds the next request:** a single-quoted
string in `sh` has no escapes and no expansion. Splitting one is therefore
*provably* the same answer a shell would give — which is the exact test §5.4's
tokeniser already applies to itself, so this adds a construct rather than
loosening a rule. It is **not** the beginning of accepting shell syntax: double
quotes, `$`, backticks, `\`, `;`, `&`, `*`, `~`, `#` and `!` stay refused, and
each of them stays refused for the same reason as before — an incomplete shell
fails in ways attributed to the caller rather than to the environment.

**Option 3 was pressed for by a reviewing agent on 2026-08-25 and is still not
taken.** A base64 blob carries no metacharacter at all, which is a real
advantage, and it remains what this section already said it was: a **second
spelling of one input**, against `ADR-006` §6.4's one-spelling rule, and the
thing to add if a trailing newline turns out to matter *in practice*. That is a
question a board answers, not an argument.

**BUILT 2026-08-25, the same day it was ruled.**
`cmd/rivet/src/invocation.rs` carries the splitter, and
`a_dash_c_command_carrying_quoted_text_is_refused_and_that_is_the_gap` was
**inverted rather than deleted** — it is now
`a_dash_c_command_carrying_single_quoted_text_now_reaches_the_operation` and
asserts that this section's own example produces its plan.

`PLAN.md` §22.1 records what building it cost. One consequence is worth having
here: **a newline inside single quotes is literal in `sh` and is therefore
accepted**, which is most of what option 3 was wanted for — arriving as a
consequence of option 2 rather than as a second spelling of an input.

**Not proved on hardware.** The board this was ruled for runs a build that
predates it; `cmd/rivet` is in the system filesystem, so proving it over a
real remote session needs another signed update. The test drives the same
`-c` string such a session delivers, which is the shape of the invocation and not the
transport.

---

## 9. The row §2 did not have: pressing a button — ruled 2026-09-01

### 9.1 The claim this ADR makes was not true

This document is called **The Agent as a First-Class Operator**, and §2 says
most of it is already done. On 2026-09-01 the gap was found by running into it:

**an agent built the display-lease mechanism and could not test it**, because
`select` is the launcher's to send, the launcher sends it when somebody presses
a button, and there was no way to produce one. The maintainer pressed ENTER.
Everything else that day — the image, the update, the trial boot, the document
edits, the diagnosis — went over `ADR-035`'s control plane.

On being told it was not a small change, the maintainer's response was that this
inability violates a principle of Rivet. *(`.rulings/` R-268.)*

He is right, and the principle is this ADR's title. An operator that can read
every fact on a machine, install to it, reconfigure it and reboot it, and cannot
press a button, is an operator with a hand tied.

### 9.2 What was actually missing, which was not input

The first diagnosis was *a kernel rebuild* — `/dev/uinput` is absent and
`CONFIG_INPUT_UINPUT` is not in the kernel configuration. That is true and it is
not the gap:

> **An agent has no write channel into a running program on this machine.**
> Everything it can do is `get`, which reads, or `apply`, which performs
> operations this build defines. **There is no verb that hands bytes to a
> process.**

That is a statement about the control plane rather than about input, and it is
why enabling `uinput` closes nothing on its own: something would still have to
create the device before `rivet-launcher` enumerates, and something would still
have to let an agent write to it.

So the kernel flag is **not taken**. The operation is the mechanism.

### 9.3 Decision

> **`apply input <action>` delivers one navigation action to the launcher.**
>
> The vocabulary is the human vocabulary and nothing more:
> `up`, `down`, `left`, `right`, `enter`, `back`, `long-back`, `capture`.

Four questions were put to the maintainer and ruled the same day.

#### The grant is `apply install`'s, not a new one

The maintainer confirmed that the grant for `apply input` is the grant
`apply install` already uses. *(`.rulings/` R-268.)*

A caller holding the pinned Ed25519 key that may replace the program a machine
runs may also press a button on it. **The install is the larger power by a wide
margin**, and a separate narrower grant for the smaller one would be ceremony
without safety.

#### An agent's press is distinguishable from a person's

The maintainer asked that an agent's inputs be tagged differently from his own,
if that is feasible. *(`.rulings/` R-268.)*

**Feasible, because the two arrive by different roads and each already carries
its origin.** A person's press comes off `/dev/input/event*` and the launcher
already records which node — *"/dev/input/event5 - ENTER"*. An agent's arrives
over the control plane, where `ADR-028` §4.5 already stamps the caller's pinned
key.

This is not a preference about logs. **An injected press that read like a
pressed one would break §4's provenance**, which this ADR made a property of
every request. The record says which road it came by.

#### With no launcher, the request is refused

The maintainer's ruling is that input belongs to kiosk applications, and kiosk
applications exist only under a launcher. *(`.rulings/` R-268.)*

**The ruling dissolves the question rather than answering it.** Input exists in
the context of a launcher: it is the launcher that has a menu, a selection and a
vocabulary. With none there is no consumer and nothing an input could mean.

So the operation **refuses and says so**. It does not queue, and it does not
drop silently — the second being the failure this project keeps naming.

#### Navigation only, and that is parity rather than restriction

The maintainer's ruling is that an agent is a first-class citizen of the machine,
which settles the vocabulary by arithmetic: whatever a person has at the kiosk,
an agent has. *(`.rulings/` R-268.)*

**The vocabulary is the human vocabulary — no more and no less.** A person at a
Rivet kiosk has a directional pad, ENTER, BACK, a long BACK and a screenshot
button. That is what an agent gets.

There is deliberately **no general text entry**, and not as a limit on agents:
**no human has one either**, because there is no keyboard surface on a Rivet
kiosk to be equal to. The rule is parity, and **parity is the ceiling as well as
the floor** — the day a person gets text entry, so does an agent, and the
argument will already have been made.

### 9.4 How it travels, and why nothing new was invented

The platform's own idiom — declare, be handed:

```text
agent --control plane--> rivet-mcpd --> rivet-sysd --unix socket--> rivet-launcher
       apply input enter                          provides = ["socket:input"]
```

* `rivet-launcher` declares `provides = ["socket:input"]`. The supervisor binds
  it and passes the listening descriptor, exactly as it does for every other
  socket an application declares (`ADR-015` §5.2, §5.3).
* The socket is `0600` and **root-owned**, because no application declares
  `connects` on it. The supervisor is root, so it can reach it and nothing
  unprivileged can — the grant mechanism doing its job by *not* granting.
* The action joins the same pipeline a real button feeds, so a selection made by
  an agent and one made by a person are the same code path from that point on.

### 9.5 What this does not do

* **It does not make the agent a user of the display.** It presses buttons; what
  is drawn is still the launcher's and the application's.
* **It does not survive without a launcher**, by §9.3.
* **It is not a general remote-control surface.** `rivet_core::kiosk` is three
  verbs wide because a launcher that could install payloads would be a much
  larger thing to trust (`ADR-036` §5.4); this adds one operation with a
  closed vocabulary and no way to grow one accidentally.


### 9.6 Implemented, 2026-09-01

Shipped in `0.1.58` and **proved on hardware**, not merely compiled. Every
action in the vocabulary was sent from a workstation over `ADR-035`'s control
plane to a Raspberry Pi: `enter` selected `kiosk-demo` and the launcher leased
it the display; `long-back` stopped it and revoked the lease; `capture` wrote a
frame that was fetched, decoded and looked at; the four navigation actions
moved the focus; an unknown word was refused with the vocabulary. `PLAN.md`
§107.8 carries the log lines.

Two adjustments the implementation forced, both recorded there:

* **§9.4's "same pipeline" had to be built, not just described.** The action
  match lived inside the device-read loop and the long-press timer held its own
  copy of the return-from-application work. Both now drain one queue, so the
  identity §9.4 claims is a property of the code rather than of two things
  kept in step by hand.
* **The launcher now reports its own focus.** Nothing did, and an agent reading
  the screen instead reached a confident wrong conclusion. §4 makes provenance
  a property of every request; this is the same argument applied to state.
