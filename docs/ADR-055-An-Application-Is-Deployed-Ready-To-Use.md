# ADR-055: An Application Is Deployed Ready To Use

* **Status:** Accepted in part. **§3 (with §3.5) and §5.2 are BUILT, with
  tests, and none of it has run against a machine** — §8 says what is proved
  and by what method. §4 is built in part: the interview offers a switched-off
  dependency's enable before the install, `install` prints it among the steps
  after (§3.5), and the library pre-check still runs on the publish route only.
  §6.1 and §6.2 are ruled. §6.2's table of stale readers is built, with the
  proxy as its only entry (`.rulings/` R-224). What it has been proved
  against is its tests; §8 says what that does and does not cover. §5.2 did not wait on
  it, because nothing was ever running.
* **Date:** 2026-09-15
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Answers:** what happens between `cargo rivet install … confirm` and an
  application somebody can actually use — which of those steps a tool can
  take, and how the rest are handed to whoever runs it.
* **Scope:** deploying to a machine that is **already running**. It changes
  nothing about `ADR-054`'s provisioning arc.
* **Builds on:** `ADR-049` (the staging bundle — the same material, minted at
  provisioning time, for the applications a card carries), `ADR-050` (what a role's card carries, and the
  ask-or-decide razor), `ADR-052` (the fleet repository, and why a secret is
  never in it), `ADR-054` (provisioning selects), `ADR-015` (the manifest as
  the permission request, the data directory, `enabled`), `ADR-046` (the proxy,
  its derived routing table, and what a backend must serve), `ADR-036` (a
  declaration is two-sided), `ADR-017` (no dependency graph; every service
  retries)
* **Supersedes:** nothing. Every document above stands unamended.
* **Code (§3):** `rivet-core/src/package/application.rs` (`Material`, the
  `material` key), `cmd/cargo-rivet/src/crate_meta.rs` (what an author may
  declare), `cmd/cargo-rivet/src/fleet.rs` (`refuse_if_material_missing`),
  `rivet-remote/src/deploy.rs` and `rivet-remote/src/tls.rs` (what deploy asks
  the machine, and the wait for its own install)
* **Code (§3.5):** `cmd/cargo-rivet/src/fleet_deploy_plan.rs` (the one
  planner), `cmd/cargo-rivet/src/fleet_deploy.rs` (the interview, and the two
  readers both front ends share), `cmd/cargo-rivet/src/fleet_next_steps.rs`
  (what `install` prints), `cmd/cargo-rivet/src/fleet.rs`
  (`publish_then_send`)
* **Code (§5.2):** `cmd/rivet-sysd/src/material.rs` (the check),
  `cmd/rivet-sysd/src/supervise.rs` (`State::WaitingForMaterial`,
  `Supervisor::start`), `cmd/rivet-sysd/src/service.rs`
  (`ServiceDef::material`, `Limits::material_poll_ms`),
  `cmd/rivet-sysd/src/applications.rs` (carrying it from the manifest)

---

## 1. Context

A fleet **member** — a kiosk machine, provisioned from a card carrying
`rivet-proxyd` and nothing else — was given four applications with
`cargo rivet deploy <device> --artifact <digest> --enable confirm`:
`aura-controller`, `aura-renderer`, `tempmon-rivet` and one more adopter. Every
install reported success. **Every one of the four then needed a step the
install process does not perform, before it was usable**, and none of those
steps is named anywhere in the deploy that preceded it.

In the order they were met:

1. **Three of the four crash-looped on missing per-machine TLS material.**
   `aura-controller`, `tempmon-rivet` and that adopter each read `tls-cert`,
   `tls-key` and `tls-root` out of their own data directory, and the payload
   carries none of the three by design (§7, item 1). The repair, per
   application, is `rivet-provision ca server <hostname> <ca dir>` once and
   then `cargo rivet secrets <application> … confirm`. **Provisioning performs
   exactly this work for the applications a card carries** — `ADR-049` §8's
   table stages the same three files under the same three names, delivered into
   the data directory by the supervisor pass that creates it — and it does not
   happen for an application deployed afterwards.
2. **`tempmon-rivet` declined to start on a dependency that was switched off.**
   Its manifest declares `connects = ["rivet-bluetoothd:advertisements"]`, and
   that system service ships with `enabled = false` because whether a machine
   listens for advertisements is the operator's policy (`ADR-050` §2.1). The
   supervisor's refusal is correct and names its own fix — *the fix is
   `[services.<name>] enabled = true` in the configuration document rather than
   an install*. It arrives after the payload was sent, from a journal, on a
   machine with no screen.
3. **`aura-renderer` drew nothing until `rivet-launcher` was restarted.** The
   launcher asks the supervisor once, at start, what this machine can offer.
   The supervisor re-runs discovery after every install (`ADR-049` §5.1), so
   the machine's own answer was correct within a second of the install
   finishing; the program drawing the screen was holding the answer it got when
   it started.
4. **Nothing was reachable at `https://<host>/` until `rivet-proxyd` was
   restarted.** The proxy reads its routing table once at start. `rivet-sysd`
   rewrites that table at each finished install, so an application is routable
   without a reboot (`ADR-046` §5) — and the running proxy served an index
   saying nothing on this machine asks to be proxied, with four applications
   installed and three of them declaring a `proxy-path`.
5. **Changing `[kiosk] auto-run` took a reload and then a restart, in that
   order.** The supervisor re-reads the `[kiosk]` table on reload; the launcher
   re-asks for the menu only when it starts. Restarting first restarts the
   launcher against the value the supervisor parsed before the edit.
6. **Adjacent, and named here because it looked identical from the outside:**
   the machine had no resolver at all. Weather and the reading card failed with lookup
   errors that read like application defects. That one belongs to the
   interview, not to deploy — §8.2.

### 1.1 The unifying sentence, and the two kinds under it

**Each application has setup steps the install process does not automate.**

They fall into two kinds, and the distinction is what the rest of this record
is organised around:

| | what it is | what is missing |
| :-- | :-- | :-- |
| **material the machine needs that the payload deliberately does not carry** | items 1 and 2 | something has to be **provided**, and nothing says what |
| **a running machine not re-reading what it already knows changed** | items 3, 4 and 5 | the authority is correct and a **second in-memory copy** of it is stale |

The second kind is `ADR-011`'s ordinary shape: one fact, two readings, and no
invalidation on the second. Naming it that way matters because it decides the
repair — the machine gains no new source of truth, only the message that says
*read yours again*.

**And item 2 is the first kind even though nothing is installed by it**, which
is why the column says *provided* rather than *minted*: what the application
needs is a switch in the operator's own document, and the thing missing is
still that nobody was told.

---

## 2. Decision

Three parts. Each has its own reasoning and its own cost. The one that changes
how a running machine behaves is ruled in §6.2: the deploy offers it, and the
operator decides.

1. **A manifest declares the per-machine material its application needs, and
   deploy REFUSES — naming the commands — when the machine does not hold it**
   — §3. Built. Deploy mints nothing and opens no authority.
2. **A declared dependency that is switched off is named in the deploy plan,
   before the payload is sent** — §4. Unbuilt.
3. **The machine converges what it already knows changed** — §5, by the
   deploy offering to restart each stale reader, defaulting to No (§6.2).

---

## 3. Part one: the manifest declares the material, and deploy refuses without it

### 3.1 An implied requirement is one no tool can act on

Today the requirement exists and is written nowhere a program can read it.
`proxy-path` implies the application is reached over HTTPS; `ADR-046` §4.2
requires such a backend to *serve a certificate for the name the proxy verifies
it against* and to *require a client certificate chaining to the fleet
authority*; and satisfying both means three files, under three exact names, in
the application's own data directory. Not one link in that chain is in the
signed artifact.

Where the chain **is** written down is a comment beside a constant in
`cmd/cargo-rivet/src/secrets.rs` — *"an operator installing three secrets into
five applications types the same name each time"* — and that comment is honest
about why: four of the five readers are foreign programs that link nothing of
Rivet's, so there is no crate the names can be imported from. The agreement is
a convention. **A convention is a thing an operator must already know. A
declaration in a signed manifest is a thing a tool can act on**, which is the
whole difference between the two.

**Inferring it from `proxy-path` would be wrong as well as implicit.** The same
pair is also a *client* certificate (`ADR-046` §4.2) — an application that calls
another presents it — so a program that serves no route can need the three files,
and a program that serves one can be satisfied without them the day it is
reached only from the machine itself. An inference that is right four times out
of five produces the failure `ADR-015` §6.1 forbids in the other direction: the
system asserting something about an application that the application never said.

**The precedent is the routing table itself.** `ADR-046` §5: *nothing in it is
configured* — every row comes from an installed application's signed manifest,
and an operator cannot add a route without shipping a new artifact somebody
signed. The routes derived from `proxy-path` are already declaration-driven.
The credentials without which those routes reach a program that will not start
are derived from nothing at all.

`ADR-036` §8A generalised this into a rule already: a new subsystem is a new
declaration, never an ambient capability, and a declaration is two-sided — the
platform provides plumbing and the application takes on obligations in
exchange. TLS material is a subsystem that has the second half and is missing
the first.

### 3.2 What the declaration looks like

One optional key, a list of tokens, each naming a **kind** of per-machine
material rather than a file:

```toml
[package.metadata.rivet]
proxy-path = "/tempmon/"
material   = ["fleet-tls"]
```

`fleet-tls` is `tls-cert`, `tls-key` and `tls-root`, exactly as
`cargo rivet secrets` installs them and exactly as `ADR-049` §8 stages them.

Four properties, each chosen against an alternative:

* **A kind, not a file list.** The three files travel together and an
  application holding two of them serves nothing. A per-file declaration lets a
  manifest ask for a key with no certificate, which is a state nobody designed.
  It also keeps the *contents* of a kind the platform's: the day a fourth file
  joins the set, every manifest already declaring `fleet-tls` gets it.
* **Optional, and appended to the manifest's key order.** `rivet-core`'s key
  array is the canonical order and a key added to its end cannot make an older
  manifest out-of-order — the same move `library` and `devices` were added by.
  An absent `material` therefore means what it has always meant: this
  application asks for nothing, and deploy provides nothing and says nothing.
  A required key would refuse every artifact already published.
* **Declared by the application, carried in the signed artifact.** §7 item 2 is
  why it cannot be a table inside `cargo rivet deploy`.
* **The names are the platform's, and `ADR-049` §8's exception stays where it
  is.** `rivet-fleetd` reads `server.crt`, `server.key` and `ca.crt` — three
  naming schemes in one directory, because each program is the authority on the
  names it reads. That application is staged at card time, its files survive a
  redeploy (`ADR-049` §6 keeps a file the data directory already holds), and
  widening this key to carry per-application names to satisfy one case would
  make every other manifest able to ask for a private key under a name of its
  choosing. The one case that needs it already has a mechanism; this key
  deliberately serves the other five.

### 3.3 What deploy does with it

**Deploy reaches no key and installs nothing. It asks, and it refuses.** §6.1
is ruled: minting is an operator act, and what deploy owes is the refusal
rather than a crash-loop the operator diagnoses from a journal.

Before the payload is sent, deploy reads the manifest it is about to send.
For each declared kind it asks the machine what that application's data
directory holds — `get file /state/app/<name>/data`, the read the declared
library closure already makes one directory along (§4) — and compares it
against the kind's files.

**A directory that is not there is not an empty one.** The installer makes it
(`ADR-015` §3.2), so its absence means this application is not installed on
this machine at all.

### 3.3.1 What is refused, and what is only reported

**A deploy that would put the application INTO SERVICE without its material is
refused, and nothing is sent.** That is `--enable` on the command line, or a
document that already says `[services.<name>] enabled = true`. It is the
failure §1 found: a service started without credentials it cannot invent.

**A deploy that only installs is reported, and proceeds.** Installing is not
running (`ADR-015` §3.2.2), so an idle application holding no credentials harms
nothing — and refusing it would be a wall rather than a refusal. `apply
install-secret` refuses a data directory that is not there rather than creating
one, *"creating one for a name nobody installed would put a credential where
nothing is going to read it and report that as a success"* — so installing the
application is what makes the place its material goes. A refusal whose own
instructions cannot be carried out is worse than the crash-loop it replaces.

The two together give the operator one order that always works: **deploy,
install the material, enable.**

### 3.3.2 The refusal's text is the deliverable

A refusal that named a problem and left the operator to find the commands would
be the runbook §7 item 3 rejects, printed at a different moment. So it names,
every time:

* **the application**, and the directory the files are missing from;
* **each missing file**, with the kind that asked for it;
* **`rivet-provision ca server <hostname> <authority>`**, with the hostname
  read out of the machine's own document — and the sentence that this is once
  per MACHINE rather than once per application, because the leaf carries the
  machine's name and every application on it is handed that one pair
  (`ADR-046` §4.2);
* **`cargo rivet secrets <application> … confirm`**, with the host, identity
  and pin this invocation is already holding written into it;
* **that installing the material and running the deploy again is the way
  forward** — and, where the document did not say what the machine is called,
  which command answers that.

**A machine that will not answer is refused rather than read as holding
nothing** (`AGENTS.md` §5), at the one point where guessing means either a
crash-loop or a refusal nobody needed.

### 3.4 What it costs

* **An operator types two commands per machine and one per application.** That
  is §6.1's ruling stated as a cost: the tool reaches no authority, and the
  evening `ADR-049` §1 describes is shortened rather than abolished.
* **A manifest key that every HTTPS application must gain.** Until an adopter
  republishes, its deploy behaves exactly as today — which is a migration with
  no flag day, and also a period in which the defect is named for some
  applications and not others.
* **One control-plane round trip per deploy that declares material**, and a
  second — the document — only on the path that has something to say.
* **Deploy reads the manifest before sending.** It already does this for the
  declared library closure (§4), so the cost is one more use of a read that
  exists rather than a new one.

### 3.5 One planner drives both front ends

`cargo rivet deploy` is the interview, for a person at a terminal: it takes no
arguments, picks the artifacts and hosts from menus, and asks before each step.
`cargo rivet install` is the scripted verb, for an agent or a script. **Both
plan from one function, `deploy_plan::plan`**, over the same two inputs read by
the same two readers — the artifact's signed needs, and the machine as it
stands before anything is sent. A step taught to the planner for the person
therefore reaches the agent in the same change. Two front ends each writing
their own advice is the failure this prevents: the install verb's hand-written
advice had already stopped mentioning the proxy restart a routed application
needs, while the interview offered it.

What `install` does, in order:

1. **Publish or select** the artifact, and refuse a declared library the
   machine does not hold (§4 — on the publish path).
2. **Read what the planner reads**: the artifact's needs, the machine's
   document and — when material is declared — its data directory, and each
   unmet `connects` dependency, resolved exactly as the interview resolves it.
3. **Refuse to put the application into service without its material**
   (§3.3.1). The missing files are the planner's material step, and the two
   commands are printed by the same function that prints that step after an
   install, so the refusal and the step cannot name different things.
4. **Send, install, and wait for that install.** `rivet apply install` returns
   before the installer writes a byte, and the next command an agent types —
   `cargo rivet secrets` — writes into a data directory only the installer
   makes. `get install` reports the LATEST install, so `finished` on its own
   describes whichever install came last, however long ago. So `install` reads
   `get install`'s `correlation` immediately before the request that is
   accepted, waits for it to change and then for that chain to finish, and
   exits non-zero, naming the installer's outcome, when it did not succeed.
   `--no-wait` returns once the request is accepted.
5. **Print what remains**, each step as the exact command the interview's
   executor runs for it: a dependency's steps first, then material
   (`cargo rivet secrets`), the enable (`apply edit`), a service restart, the
   proxy restart, and one converge (`apply reload`) when anything is to be
   enabled. **On a fleet server** the last step is `cargo rivet fleet sync`, so
   `rivet-fleetd`'s store holds what was installed; it is printed and never
   run, because it changes a second store on a second decision.

What the install itself did is taken off the list rather than re-offered:

| the install… | so |
| :-- | :-- |
| installed | the install step is done |
| enabled with `--enable` before sending | the enable is done, and the supervisor re-reads the document when an install finishes, so no converge is owed for it |
| was refused as already running this artifact | nothing changed, so no restart is owed; an enable `--enable` wrote is re-read by nothing, so the converge is |

**The `--artifact` route reads the manifest too**, with the application
signing seed, so the material refusal covers the route an agent uses most. A
workstation without the seed still installs by `--artifact` — the install
needs no seed — and says the steps that remain could not be planned, and why.

**`cargo rivet deploy` refuses an argument** rather than ignoring it: the
interview chooses its hosts itself, and a word it would have dropped reads to
an agent as a host it took. The refusal names `install`.

The interview's own install step waits for its install in the same way, for
the same reason: its next step can be staging material.

**The proxy is an image service** (`ADR-060`). Neither front end offers to
install a proxy payload — the machine's install door refuses one — and the
planner offers to restart `rivet-proxyd` after a routed application as the
first entry of §6.2's table of stale readers, defaulting to No.

---

### 3.6 A machine that holds its fleet's root issues the material itself

Everything above is about a fleet whose root rests on a workstation. **A
fleet-root machine issues each `fleet-tls` application its certificate itself,
at install and at every boot** (`ADR-063` §4.5), so there is nothing for an
operator to stage and nothing a workstation's authority could sign.

So when an application declares material, the planner also asks the machine
`rivet get fleet-root`. A machine that holds its root is planned with **no
material step**: `install` prints no `rivet-provision ca server` and no
`cargo rivet secrets` line, `--enable` is not refused for the files the machine
is about to write, and the interview offers nothing to approve. A machine that
cannot answer that question is planned as a workstation-root machine — the
material step it always printed, which is wrong in a way an operator can see,
rather than no step, which would be wrong in a way nobody would.

A `fleet-tls` application held for missing material on such a machine says
that the machine issues it and that the journal's issuance record says why it
is not there, rather than naming a command.

## 4. Part two: a dependency that is switched off is named in the plan

`connects` already carries the fact. `tempmon-rivet`'s manifest says it wants
`rivet-bluetoothd:advertisements`; the supervisor resolves that against the
services this machine has enabled and refuses to start an application whose
connection cannot be resolved (`ADR-015` §5.3). **The machine's answer is
already right, and it arrives at the wrong end of the operation** — after the
transfer, after the install, in a journal, on a headless box.

**Deploy already asks a question of exactly this shape and asks it first.**
`cargo rivet deploy` reads `/state/lib` before sending and refuses an
application whose declared library closure the machine lacks, naming each
missing reference and the command that deploys it (`SPEC-02` §5; `.rulings/`
R-157 R15). `SPEC-02` §5.1 states the principle in its own title — refusal at
install, never a loader error at start. Part two is that check's sibling, one
rung softer.

**Softer, and the reason is which document the fix lives in.** A missing
library is a program that cannot execute: the machine's own installer refuses
it too, and there is no order of operations that makes the deploy correct. A
provider that is installed and switched off is one line in the operator's own
desired state — `[services.rivet-bluetoothd] enabled = true` — and the operator
may legitimately be deploying a consumer before its provider. **Refusing would
turn a sequencing preference into a rule**, which is the dependency graph
`ADR-017` §3.2 rules this system does not have. So the plan **names** it: the
declaration, the provider, whether the machine has it installed, whether the
document enables it, and the exact edit that would.

Deciding it for the operator was considered and rejected on `ADR-050` §7's
razor. Whether this machine listens for advertisements is a policy only the
operator knows, and `rivet-bluetoothd`'s starting value is *the operator's
decision, and only theirs*.

**The pre-flight runs on every route into deploy, `--artifact` included.** The
material half and the dependency steps do: `install` reads the manifest on
both routes (§3.5). The library closure check still runs on the publish route
only, so a payload selected from the repository reaches the machine's own
installer — the backstop — without it; that check reads the same manifest at
the same moment, and joining it to the other two is the remaining repair.

### 4.1 What it costs

* **One control-plane round trip before a send**, beside the one the library
  check already makes.
* **A report, not a refusal, when the tool cannot resolve a declaration.** A
  provider this workstation cannot reason about is reported and the deploy
  proceeds, on `ADR-046` §5's containment rule: the machine is reached through
  these tools, and a pre-flight that can refuse a correct deploy is worse than
  one that occasionally says less than it wanted to.
* **The plan grows.** Against `.rulings/` R-193 D4 that is a debt and not a
  feature, and it is paid the same way: the verdict line is the last thing
  printed, and the pre-flight's findings sit below it.

---

## 5. Part three: the machine converges what it already knows changed

Two programs hold a copy of a fact the machine has already updated, and neither
is told.

| reader | what it holds | what updates the authority |
| :-- | :-- | :-- |
| `rivet-proxyd` | the routing table, read once at start | `rivet-sysd` rewrites it at each finished install and uninstall (`ADR-046` §5) |
| `rivet-launcher` | what this machine can offer, asked once at start | the supervisor re-runs discovery after every install (`ADR-049` §5.1) |

**Neither is a second authority, and that is what makes this cheap.** The
proxy's table is a file the supervisor owns and writes beside `current`, where
the proxy's own uid cannot reach it; the launcher's menu is a question answered
by the supervisor over the kiosk socket, deliberately so that the launcher is
not a second reader of the document (`ADR-046` §5, `ADR-042` §8.2). In both
cases the machine's answer is correct within a second of the install finishing.
What is missing is the sentence *read it again*.

**So the decision is invalidation, not a new mechanism**, and it is the
operator who performs it (§6.2). When a pass changes what a reader holds, the
deploy offers to restart that reader: a pass that changed the routing table
offers `rivet-proxyd`, and one that installed an application declaring
`devices = ["display"]` is where `rivet-launcher` would be offered, once it is
an entry.

**Item 5 of §1 is the same defect with a different trigger, and it is fixed by
the same sentence.** The supervisor re-reads `[kiosk]` on reload — that is why
reload must come first — and the launcher re-asks only at start, which is why a
restart before the reload reads the old value. A launcher that re-asks when the
supervisor's answer changes removes the ordering rather than documenting it.

### 5.1 This is a behaviour change on a RUNNING machine, and it costs an outage

**Stated plainly because it is the part that has to be ruled.** Today a deploy
transfers bytes, installs them, and stops. After this, a deploy stops and
restarts a program that was serving.

* **The proxy is the machine's front door.** Every page under it is unreachable
  for as long as the restart takes. `rivet-fleetd` listens on 4443 and is
  deliberately not a route (`ADR-046` §7), so the administration surface
  survives the outage — which is the same argument `ADR-050` §4.2 makes about
  `rivet-mcpd` and is why this is survivable rather than merely brief.
* **The launcher is a person's screen.** Converging it takes the display back
  from whatever is drawing and redraws the menu. On the maintainer's own
  machine the launcher is the `[kiosk] auto-run` arbiter for a soak that is
  gathering frame-cost records, and restarting it costs that run its continuity
  (`AGENTS.md` §9.1).

**A brief outage nobody asked for** is therefore the honest description, and
the two readers do not carry equal cost. §6.2 rules it: the deploy offers the
restart, defaulting to No, and nothing restarts a reader by itself.

**The better end state, named and not chosen here:** a reader that re-reads
without stopping — the proxy re-reading its table on a signal while keeping its
listener, the launcher re-asking `offer` when the supervisor says the set
changed. It removes the outage from both rows. It is also code inside two
programs rather than a decision about when to send an existing message, and
§6.2's table is built to take it: an entry's action can become a reload
instead of a restart without changing what triggers it. A reader that reloads
without an outage could then default to Yes.

### 5.2 A machine that starts with material still missing converges toward it too

§3.3.1's refusal stops a `cargo rivet deploy` from putting an application into
service without its material. It cannot stop a document written by hand, or a
card whose deploy predates the check, from enabling one anyway — and on a real
board that produced the failure this whole record opens with a different
version of: an application crash-looping several times a second, each crash
landing mid-load of its add-ons.

**So the supervisor carries the identical check, at the one place a deploy
cannot reach: the moment before every spawn.** `rivet-sysd` asks the
application's data directory whether every file its declared material names is
there; missing any, it holds the service `waiting-for-material` instead of
spawning, names what is missing and both commands that install it as the
service's `reason`, and checks again on a poll — nothing on this machine wakes
it when `apply install-secret` writes a file. It is convergence in exactly
§5's sense: a fact about the machine that changed (the file arrived) without
anything telling the reader, met by asking again rather than by remembering
what was true at the last look.

**It costs nothing §5.1 already costs**, because nothing was ever running:
there is no outage to weigh and no `AGENTS.md` §1.1 third-row question behind
it, unlike §5's other two readers. `ADR-017` §3.5.2 is the supervision-side
record of the state itself.

---

## 6. What the maintainer ruled, and what is still his

Both questions are `AGENTS.md` §1.1's third row, and both are ruled.

### 6.1 Deploy does not touch the fleet authority — ruled

**The CA steps stay manual, and deploy owes the refusal.** Minting a machine's
leaf is an operator act: `cargo rivet deploy` opens no authority, reads no key,
and installs nothing (`.rulings/` R-194 D1). On a fleet-root machine there is
no workstation authority to open: the machine issues the leaf itself (§3.6),
and deploy still opens, reads and installs nothing.

**Why that is the shape and not merely a smaller feature.** A tool that reads
the authority on an ordinary run is a tool that reads the authority, and the
only way back is to take a capability away from something that had it —
against `ADR-052` §4.1, which makes the authority the most sensitive thing on
the workstation, ruled to the point that even *backing it up* takes a directory
named on the day. What the defect in §1 actually cost was not the typing: it
was that nothing said what to type, and the account arrived from a journal on a
headless machine after the install had reported success. **The refusal is the
repair, and it is the whole repair** (§3.3.2).

**No flag turns it into a mint**, and none is proposed: the option that had one
is in this record's History, and adding it later would give an ordinary deploy
the capability this ruling declined.

**Two things follow that are not this repository's to do alone**, both ruled
with it:

* **Each application's own README says it** — an application serving HTTPS
  needs `tls-cert`, `tls-key` and `tls-root` installed per machine, by the
  operator, with the commands. Aura, TempMon and Reliquary each own
  that sentence; Rivet does not own it for them.
* **The rebuild-from-scratch instructions carry it as a step**, in the order it
  falls — after the deploy and before the application is expected to work.
  `skills/bringing-up-a-new-machine` is where that lives.

### 6.2 Changing how a running machine behaves without an operator asking — ruled

**A stale reader is offered, never restarted by itself** (`.rulings/` R-224).
When a pass makes a running service stale, the deploy offers to restart it as
one of its steps, and the prompt defaults to **No**. The interview asks; the
scripted `cargo rivet install` prints the command among the steps that remain;
the supervisor never restarts it on its own.

**The mechanism is a table, not a special case.** The planner holds one entry
per reader of §5's kind. Each entry names the service, the change in a pass
that makes it stale, and what a restart costs, which the prompt says out loud.
Teaching the deploy a new reader is one more entry, and the prompt, the
default, the printed command and the tests come with it.

**The proxy is the first entry.** It is offered whenever a pass installs or
changes a routed application, which covers a plain route appearing or
disappearing, since a plain route needs a route. When the artifact declares a
plain route, the prompt says that port 80 is what changes (`ADR-046` §7). **The
launcher is the natural second**, and it is not an entry yet.

**Why No, and why never automatic.** A restart is an outage for everything
behind that reader: the proxy drops 443 for every application on the machine,
and has been seen to hold its whole ten-second shutdown grace before it is
killed. The launcher's restart takes a person's screen and a running
measurement. In both cases whether now is a good moment is precisely what only
the operator knows (`ADR-050` §7's razor). And **a machine that is briefly
wrong is recoverable by a command it names**: the supervisor journals the
change and the exact restart, and the deploy's own plan puts the same command
in front of whoever is deploying. A restart nobody asked for on a machine
nobody is watching is not.

---

## 7. Rejected alternatives

1. **Carry the certificates in the payload.** They are per-machine and they are
   secret, and a payload is one signed artifact every machine gets a copy of —
   `ADR-049` §1.1 states it exactly: material that is *about this machine*
   cannot be in a signed payload every machine holds. A machine's private key
   would be in the fleet repository, which `ADR-052` §4 refuses as a check
   rather than a convention because the failure is silent and unrecoverable.
2. **Teach `cargo rivet deploy` a list of which application needs what.** The
   knowledge belongs in the manifest that ships with the application. A table
   inside the tool is a second representation of a fact the artifact already
   carries (`ADR-011`), and it goes stale the day an adopter publishes a version
   that changed its mind — the adopters are separate repositories this tool
   neither builds nor reads. It is also the thing `ADR-046` §5 already refused
   for routes: every row in the routing table comes from a signed manifest
   precisely so that changing one takes a new artifact somebody signed.
3. **Write an operator runbook.** One exists —
   `skills/deploying-an-application` carries the TLS step and the proxy restart,
   from a rehearsal that met them before this evening did — and the evening
   still spent its four deploys rediscovering two more that the skill does not
   carry. **A runbook is the record of a defect, not its repair**; that is
   `ADR-049` §1's own standard applied to the layer above it. Every step in a
   runbook is also a step whose omission is silent, which is the property that
   makes the four failures in §1 look like application bugs.
4. **Re-provision the card to add an application.** `ADR-050` §4 rejected baking
   infrastructure into the image for a cost measured on the day: it makes the
   fix of the software most likely to need a fast fix the slowest one available.
   Re-provisioning a running machine to add an application is that cost plus a
   card write that needs administrator and a board that goes down — to install a
   program the control plane can install in ninety seconds.

---

## 8. Consequences

**Built: §3, §3.5 and §5.2.** The `material` key is parsed, carried in the
signed manifest, declarable in `[package.metadata.rivet]`, and refused by name
at both gates when it holds a kind this build does not define. `cargo rivet
install` asks the machine for the declared kinds' files before it sends,
refuses an install that would put the application into service without them,
reports one that only installs, and reaches no key at any point; it then waits
for its own install and prints the steps that remain from the interview's
planner. `rivet-sysd` carries the
identical check at the one place a deploy cannot reach — the moment before
every spawn — and holds a service `waiting-for-material` rather than starting
it without what it declared (`ADR-017` §3.5.2). `aura-controller` is the
in-repo application that declares it.

**What the unit tests prove, and what only a machine can.** The tests assert
that the bytes parse and render, that the pre-flight fires and does not fire in
the right cases against a fake, that the supervisor's own gate holds, journals
once and releases itself with no restart, and that its text names what §3.3.2
says it must. They cannot assert that a real board's `get file` answers the
shape this reads, or that an operator who follows the two commands the
refusal prints ends up with a working application — that takes a run against a
machine (`AGENTS.md` §6). For §3.5 the tests tie the steps `install` prints to
the steps the interview's own functions plan for the same fixture, and prove
the wait ignores an earlier finished install; they cannot prove that a real
board's `get install` answers in the shape the wait reads, or that the printed
commands, run in order, leave an application a person can use.

**What an install does, end to end**, for an application whose manifest
declares what it needs: read the manifest, ask the machine for the material
and its dependencies, refuse or report, publish and send, install, enable when
asked, wait for that install to finish, and print what remains as commands
from the planner the interview runs (§3.5). A stale reader is one of those
steps, offered and defaulting to No (§6.2), never something the machine does by
itself. §5.2 is not a step at all, because nothing was ever running and there
is no outage to weigh.

### 8.1 What still needs a human afterwards

* **`enabled` stays the operator's.** Installing is not running (`ADR-015`
  §3.2.2), and `--enable` remains something typed rather than assumed.
* **Switching on a system service the machine does not run** — §4 names it and
  stops there, on `ADR-050` §7's razor.
* **Whatever the application's own first run needs.** `SPEC-05` §10.6 is the
  evidence that this is a short list: every application on that machine created
  its own configuration, database and secrets unaided. The platform's
  data-directory contract works; what was missing is only the per-machine
  material the platform is supposed to hand an application before it starts.
* **Minting and installing the material.** §6.1's ruling makes this the
  operator's, permanently. What the platform owes is that nobody discovers it
  from a journal.
* **A proof.** None of this is working until a real deploy to a real board
  refuses in the operator's own terms, the two commands it prints do what it
  says, and the deploy after them produces a usable application —
  `AGENTS.md` §6. Reading the machine back with `get status` and fetching the
  application's own page through the proxy is the check, and neither a passing
  unit test nor a plan that prints correctly is a substitute for it.

### 8.2 What this does NOT solve — the resolver gap belongs to the interview

Item 6 of §1 is **not** deploy's and is not absorbed here. A machine with no
`[network.resolver]` gets no nameservers, every outbound lookup fails, and the
failure surfaces inside whichever application made the call — which is why it
sat in the same list as the four above and is a different defect entirely.

It is the timezone's shape exactly. `SPEC-05` §5.0 ruled that the timezone must
be **asked and never defaulted**, because a wrong or absent one is silent: every
timestamp is simply wrong. A resolver's absence is the same class of failure
wearing an application's face. `ADR-050` §7's razor puts it on the ask side
without ambiguity — only the operator knows what this network resolves through.

**The interview asks it.** A static machine is asked for up to three
nameservers, defaulting to its gateway, and its document carries them as
`[network.resolver]`; a DHCP machine is not asked, and its lease fills the
resolver (`SPEC-05` §4.5). A static machine provisioned before the question
existed still has none, and `skills/bringing-up-a-new-machine` step 7 is its
repair.

**And there is a second half of it that is §5's shape a third time:**
`/etc/resolv.conf` is written by the supervisor's `/etc` projection, which runs
once before the first convergence (`ADR-008` §5.3.1), so setting
`[network.resolver]` on a running machine and reloading leaves the file as it
was. Named here, owned by the interview and by whatever rules the projection,
and repaired by neither part of §2.

---

## 9. Scope, stated once

**This record covers a deploy to a machine that is already running**, and the
boundary is deliberate. `ADR-054` §2's eight stages are unchanged: provisioning
still selects a signed image and a role's applications from the fleet
repository, still composes and stages `ADR-049`'s bundle for what the card
carries, and still never compiles, signs or publishes anything. `ADR-050`
§5.1's table of what a role's card carries is unchanged.

What changes is what happens to an application the card did not carry — which
`ADR-049` §5.1 already names as the ordinary case rather than the exception:
none of the applications a fleet server runs is on the card, and the same is
true of every application a member gains after it boots.

---

## History

### 2026-09-16 — §3 was written as *deploy provides the material*, and §6.1 asked whether it should

The maintainer ruled §6.1 option B (`.rulings/` R-194 D1): he accepts doing the
certificate-authority steps by hand himself. So §3 is now a refusal rather than an
install, and §6.1 states the ruling rather than the question. The reasoning
that survived the change is in the body — it is why the refusal exists at all —
and what is kept here is the shape that was chosen against, because the
recommendation was **C** and the ruling was **B**, and an agent reading only
the body would otherwise wonder where the flag went.

~~**Recommended: C.** B is honest, cheap, and leaves the defect exactly where
it was found — a well-signposted refusal is still a person doing the system's
job, which is the standard `ADR-049` §1 was written against. A is one command,
and the property it buys is permanent: a tool that reads the authority on an
ordinary run is a tool that reads the authority, and the only way back is to
take a capability away from something that had it. C is the one shape that has
both halves — the default reads no key and says exactly what to type, and the
flag is a place where *this run will open your fleet authority* can be said out
loud before it happens.~~

~~| | what deploy does | what it costs |~~
~~| **A — mint automatically** | reads the authority whenever a manifest
declares `fleet-tls` | every deploy becomes a key-using command, for ever |~~
~~| **B — refuse, naming the commands** | refuses before sending, printing
`rivet-provision ca server …` and `cargo rivet secrets … confirm` verbatim |
reaches no key at all |~~
~~| **C — refuse by default, mint on request** | B, plus a flag that turns the
refusal into the work | one more word to type; one more place the tool can be
wrong |~~

~~Before the payload is sent, deploy reads the manifest it is about to send.
For each declared kind it establishes what the machine needs — for `fleet-tls`,
a leaf for this machine's fleet name from the workstation's authority, and the
authority's own root — and after the install, before the service is enabled, it
installs the three files into the data directory the install just created,
owned by the application, mode `0600`.~~

**What that costs, and it is the part a later reader should weigh before
reopening this:** `ADR-049` §2's fourth step — the ownership hop, the mode, and
the never-overwrite discipline of `ADR-049` §6 — was going to be inherited
unchanged at install time. None of that machinery is built here, so an
application whose material was staged from a card and one whose operator
installed it by hand now reach the same state by two routes rather than one.
The refusal makes the second route visible; it does not make it automatic.

### 2026-09-16 — the refusal binds a deploy that would RUN the application, not one that installs it

Not in the original record, and decided while building §3 rather than ruled:
`AGENTS.md` §1.1's second row, recorded here so it is cheap to overturn.

An unconditional refusal deadlocks. `apply install-secret` refuses a data
directory that is not there, and only an install makes one — so an
application's first deploy would be refused for want of material that could not
be installed until that deploy had happened. Gating the refusal on *would this
application be running* (`--enable`, or a document that already enables it)
leaves exactly one order that works and never blocks it: deploy, install the
material, enable.
