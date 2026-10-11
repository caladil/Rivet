# The Cold Agent Test — notes, not a proposal

* **Status:** **Notes.** Too early to propose, deliberately kept anyway
* **Date:** 2026-09-07
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Why this file exists at all:** the idea is good and the design is not ready.
  Written down so it does not quietly become something everybody always meant to
  do, and short so it does not pretend to be settled.

---

## 1. The idea, as the maintainer put it

He is sorely tempted to have Gemini run a Cold Agent test: stand up a Rivet OS
server and an Android client that pass data, just to exercise the SDK and Girder.
Perhaps the agent working with him should create that test instead, after the fleet
server is deployed and verified. *(`.rulings/` R-319.)*

**A different vendor's agent, given the repositories and nothing else, builds a
Rivet application and a phone client that pair and exchange data.**

## 2. The one thing that is settled

**THE FRICTION IS THE TEST.** Ruled 2026-09-07.

An agent *succeeding* proves less than an agent getting **stuck**, because the
stuck points are the documentation defects. So the deliverable is a record of
every place it guessed, backtracked, or read source because prose did not say —
and a working application is a pleasant side effect rather than the result.

**This inverts how the run is judged**, and it is why the test is worth building
rather than just performing. A run that produces a paired client and twenty
friction entries is a better outcome than one that produces a paired client and
none, because the second means nobody was watching.

### 2.1 The subject is the OS's resources, not the application

**Ruled 2026-09-07:** the actual look and feel of the application is unimportant
next to whether the agent was able to use the resources from the operating
system that are to be measured. *(`.rulings/` R-319.)*

**What the agent is told, close to how it should be worded:**

> You are not graded on the visual appearance of the solution. You are graded on
> the ease — or the friction — you encountered implementing your plans, and on
> whether you were forced to **change** those plans because of a limitation the
> operating system **informed you about**, or one you were forced to **discover
> on your own**. The latter is particularly note-worthy.

**Not "make it plain".** An earlier draft of this section said the application
should be plain to the point of ugly, and that instruction is worse than
useless: an agent that knows appearance is assessed will spend its budget
deciding whether its solution is ugly *enough*. **Anything named as graded gets
performed at you** — so appearance is named as *not* graded, and nothing more is
said about it.

**And the sharpest measurement is in the last clause.** A limitation the
platform *told* the agent about is the platform working — a refusal that names
what it wanted, a manifest key rejected with the reason. A limitation the agent
had to *discover* is a documentation defect, and it cost somebody a detour to
find. **The ratio between those two is the closest thing this test has to a
score.**

### 2.2 The finding worth catching, with a worked example

The maintainer's illustration: an agent making a web app (a reasonable thing)
reaches for React (normal) and discovers that React is somehow incompatible with
Rivet OS — that is a finding. *(`.rulings/` R-319.)*

**And it is not hypothetical.** `ADR-046` §8 rules that an application never
learns its own base path, so every URL a page asks for must be relative and a
leading slash is a 404 behind the prefix. A mainstream front-end build emits
**absolute** asset paths by default. Mounted at a prefix, that is a 404 for
every asset — a blank page with a clean console. The smaller version of this has
already been met: a bare prefix without its trailing slash made every relative
link resolve against the machine root, and the page loaded unstyled with dead
buttons.

**Why this class of finding is the valuable one.** It is not a bug and it is not
the agent being wrong. It is the platform's deliberate design meeting the
ecosystem's default, and **somebody then has to decide** — should Rivet tell an
application its base path after all, or should the record say plainly which
tools this rules out and why? **Nobody makes that decision unprompted**, because
everybody already here knows to write relative URLs.

**So the test's real output is a list of decisions somebody now has to take**,
and a run that produces none has probably been run by somebody who already knew
the answers.

### 2.3 Weight the finding by how reasonable the reach was

The counter-example, and the log needs it or it fills with noise: if an agent
reaches for an obscure GitHub library and it does not work, that is much less of
a finding. *(`.rulings/` R-319.)*

**A mainstream default failing indicts the platform. An obscure library failing
indicts the library.** The friction is real in both cases and only one of them is
evidence about Rivet, so a log that records them at equal weight is a log
nobody can act on.

**`ADR-009` §3's ladder is already the arbiter of what counted as a reasonable
reach**, and reusing it here costs nothing: a dependency the ladder would have
approved, which then fails, is a finding about the platform. One the ladder
would have refused, which then fails, is the ladder working — and the agent
having reached for it at all is a separate and smaller note.

**The second-order case, worth one line so it is not lost.** An agent that
reaches for something obscure *when a mainstream option exists* may itself be
the finding — if the documentation is what pushed it there. That is a defect in
the record rather than in either library, and it is invisible unless the log
captures **why** a choice was made and not only that it failed.

**And the friction log is therefore a SCORECARD OVER SURFACES, not an essay.**
One row per resource the platform hands an application — did the agent find it,
use it correctly, and do so without reading Rust source? A free-form list of
complaints has no owner; a row against `journal` or `settings` has one.

The surfaces worth a row, which is also the argument for what the test
application must be made to *do*: handed sockets and the non-blocking
conversion; the journal; settings delivery; the data directory it is handed
rather than resolves; secrets; readiness; the manifest as a permission request;
the proxy route and its authentication; the display, for a kiosk; and pairing.

**The application's function follows from the scorecard rather than the reverse.**
It has to be shaped so that every surface is touched at least once, or the run
reports nothing about the ones it skipped — which is how a test comes back
green about a thing it never exercised.

## 3. Why a different vendor, specifically

`AGENTS.md` opens by claiming it orients *"any AI coding agent working in this
repository, whichever vendor's tool reads it."* **Nothing has tested that
claim.** An agent from this lineage cannot test it — it has absorbed the habits,
the traps and the shorthand from sessions the documentation never carried.

An outside agent does not know that the `cargo-rivet` on a PATH is stale, that a
handed listener arrives blocking, or that an emulator answers `unknown host` to
an mDNS name. **Everything it gets right came from the record**, which is the
subject under examination.

## 4. Why it is too early, and what would make it ready

* **The platform is still moving.** Testing documentation against a target that
  changes weekly measures the wrong thing.
* **The repository is private**, so an outside agent cannot be given it. The
  move to public is a precondition rather than a convenience.
* **No adopter consumes the SDK yet** in the converted sense (§3.4-WHY). A cold
  agent asked to use a library nothing uses would be the first consumer, which
  is a different and harder test than the one intended.

**Ready when:** the fleet server is deployed and verified, the repository is public, and at
least one adopter has been converted rather than rolling its own.

## 5. The questions to answer THEN, kept open on purpose

Recorded so the design starts from them rather than rediscovering them, and
**not answered here** — answering them now would be inventing a test for a
platform still being built underneath it.

1. **What is it given?** The repositories and nothing else is the obvious
   answer; the moment anybody helps, the run measures our helpfulness.
2. **What does *pass* mean, decided in advance?** *"It worked"* is not a result.
   Something closer to *"reached a paired client without reading Rust source"*
   is falsifiable.
3. **Where does it run?** Nowhere near the maintainer's phone, his data, or
   `rivet-01`. An emulator and QEMU or a spare board.
4. **How long before it is stopped?** An agent flailing for six hours reports
   the same thing as one flailing for one.
5. **Who reads the friction log, and against what?** A list of complaints with
   no owner changes nothing.

## 6. What it is NOT

**Not a benchmark between vendors.** The subject is the documentation and the
SDK, and the agent is the instrument. A run that says more about the model than
about the record has been designed wrong.
