# ADR-038: Application Settings

* **Status:** **Accepted, 2026-09-01**, and accepted *reluctantly* — see §10,
  which records the maintainer's objections rather than only his answers,
  because they are the reasons to revisit this. **§§3, 4, 6 and 8 are BUILT as
  of 2026-09-02 and have NOT run on a board**; §9's open question is unbuilt and
  restart is what happens meanwhile, which is its own recommendation. §12 says
  what runs and by what method. **Amended 2026-09-04: §8.1 — the settings file
  leaves `RIVET_APP_DATA` for a root-owned file beside `current`, and the
  application is told where it is.** §8's original recommendation is struck in
  place beside it.
* **Date:** 2026-09-01
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Answers:** `ADR-036` §8.5's finding that *"an application on a Rivet has no
  configuration surface at all"*, and `PLAN.md` §110's second open question.
* **Depends on:** `ADR-034`'s register for the fleet half, and
  `apply install-secret` (`ADR-006` §8.10.18, built 2026-09-01) for the half
  that must not be written down.

---

## 1. Context

**An application on a Rivet cannot be configured.** Not awkwardly — at all.
Verified 2026-09-01 rather than assumed:

* `rivet_core::package::application`'s `KEYS` is twelve long — `kind`,
  `restart`, `ready`, `network`, `state`, `memory`, `listen`, `provides`,
  `connects`, `library`, `devices`, `title` — and **none of them is
  configuration**.
* `cmd/rivet-sysd`'s supervisor spawns an application with `args: Vec::new()`.
  There is no argv.
* The environment an application receives is the platform's own —
  `RIVET_APP_DATA`, `RIVET_DISPLAY`, `LISTEN_FDS` — and nothing an operator can
  add to.

An application may write whatever it likes into `RIVET_APP_DATA`. **The gap is
the other direction: there is no way for a value an operator chose to get in.**

### 1.1 The case that found it

`PLAN.md` §110 rewrites Aura as Rivet's built-in demo. The Aura that exists
today reads its own `config.json` — `PLAN.md`'s own transcript of its first run
on a Rivet shows `brightness=60 screensaver=solid timezone=America/Chicago` —
and exposes brightness, screensaver, display sleep and schedule through an HTTP
portal.

**A Rivet application can do none of that**, so the port either loses every
setting or reinvents a private channel. `ADR-036` §8.5 recorded this as the
largest gap between what Aura is and what a Rivet application can be.

It is not an Aura problem. A departures addon needs a stop id; a weather
producer needs a location; anything with a threshold needs the threshold.

### 1.2 Why this looked harder than it is

**Four separate concerns were being argued as one.** Naming them apart is most
of the design:

| Concern | The question | Ruled in |
| :-- | :-- | :-- |
| **Declare** | which settings does this application have? | §3 |
| **Store** | what is authoritative for a value? | §4 |
| **Write** | who may change one, and how? | §7 |
| **Deliver** | how does it reach the running process? | §6, §8 |

Every unsatisfying answer considered before this ADR collapsed two of these
into one — a config file that was authority *and* delivery, or a control-plane
call that was write *and* delivery.

---

## 2. What is NOT in scope

* **Secrets.** §5 sends them a different way on purpose.
* **Typed or structured settings.** §9 keeps values textual for a first
  version, and says what that costs.
* **A settings user interface.** The fleet page could show what §3's
  declaration makes discoverable; that is downstream and is not decided here.

---

## 3. Decision: an application DECLARES its settings, in its manifest

**Proposed by the maintainer**, and it is the piece that makes the rest
checkable rather than merely possible: an application that wants state passed
in **says so in its manifest**, as part of its contract with the platform — and
the mechanism that carries it is the SDK's. *(`.rulings/` R-122.)*

The manifest gains a key naming the settings this application understands.

### 3.1 Why a declaration, rather than an opaque table

Without it, a per-application table in the machine document is **a bag of bytes
nobody validates**, and every mistake is silent: a misspelled key is accepted,
stored, provisioned to every machine in the fleet, and produces an application
that behaves as though it were never set. Nothing reports it, because nothing
knows what the application expected.

A declaration turns three silent failures into loud ones:

1. **A key the application does not declare is REFUSED when the document is
   composed** — at provisioning, on a workstation, in front of the person who
   typed it. Not at runtime, on a board, in a hallway.
2. **What an application can be configured with is discoverable without running
   it**, from a signed artifact. A fleet page, `rivet explain`, or an agent can
   answer "what does this take?" from the payload.
3. **An operator who sets something the application never reads is told**,
   rather than left to conclude the feature is broken.

### 3.2 It is the manifest already doing its job

`ADR-015` §6.1 makes the manifest a **permission request** — what an
application needs, declared, and granted by the platform rather than taken.
Settings are the same shape: a declaration of what the application expects to
be handed. The Rivet SDK's first article (`ADR-036` §2) is *"an application declares what
it needs and is HANDED it. It does not go and take one"*, and a program reading
a configuration file it went and found is the exact pattern that sentence
rejects.

### 3.3 The division that keeps payloads stable

**Keys ship with the application; values do not.**

| | Lives in | Changing it costs |
| :-- | :-- | :-- |
| **Which settings exist** | the manifest, inside the signed `.rvt` | a new signed payload |
| **What they are set to** | the machine document | an edit and a reload |

This is the right way round. Adding a setting **is** a change to the
application's contract and should ride with the application under its
signature. Changing a value is an operator's act on one machine and must not
require rebuilding anything.

---

## 4. Decision: the machine document is the authority

Settings live in the machine's configuration document, under a table named for
the application.

### 4.1 What that buys, and none of it is available otherwise

* **Provisioned before first boot.** `ADR-034` §5.1 is explicit that the common
  case is a machine that has never been powered on — *"nothing about this needs
  a signal, a socket or a running machine"*. A setting in the document is set
  by `rivet-provision` at card-write time. A setting in a data directory cannot
  be, because there is no filesystem to put it in yet.
* **In the register, therefore reproducible.** `ADR-034` §6's table has a
  **Fleet configuration** row — *"values shared across machines"* — which has
  had no contents since it was written. This is its contents. Set a value for
  every machine in a fleet, once, and have it survive a card rewrite.
* **One authority.** A value the document does not hold is a value the register
  cannot mirror and provisioning cannot reproduce.

### 4.2 The cost, stated rather than buried

**`rivet-config` refuses keys it does not know, deliberately.** That rule
was re-ruled to make an unrecognised key an outright refusal rather than
a warning, on the argument that a document silently carrying something nothing
reads is worse than one that will not load.

An application settings table asks that parser to hold **bytes it does not
interpret**, which is a real weakening of a property this project chose on
purpose. It must therefore be bounded, and the bound is §3's declaration:

* the pass-through region is confined to one namespace, never top-level;
* a table is only accepted for an application the machine actually has;
* within it, keys are checked against that application's declaration.

So the document does not become extensible. It gains exactly one region whose
schema is supplied by a signed artifact rather than by `rivet-config`.

**This is the decision in this ADR most likely to be wrong**, and it is the one
to argue with first. The maintainer accepted it while objecting to it on two
grounds: that it puts foreign data in the most critical file the system has,
and that the platform cannot fully validate what it is carrying. *(`.rulings/`
R-123.)* The second half is answered below; the first is the cost.

### 4.3 Validation is COMPLETE on the machine and best-effort when composing

The objection above is right about composition and wrong about the machine, and
the difference is worth stating precisely because it decides the blast radius.

| Where | What is available | What can be checked |
| :-- | :-- | :-- |
| **On the machine**, at reload | the installed application's manifest | **everything** — the name, every key, against the declaration |
| **At composition**, on a workstation | the payload only if the operator has it | the name and the keys **if** the `.rvt` is to hand; otherwise nothing |

So a mistyped key is not carried silently to a fleet. It is caught at
provisioning **when the payload is present**, and caught on the machine
otherwise — the check moves, it does not vanish.

### 4.4 A bad setting must NEVER refuse the document

**And this is the containment that makes §4.2 survivable.** Everywhere else in
this document an unrecognised key is fatal. **Inside the
application region it must not be**, and the asymmetry is deliberate rather
than a weakening of the weakening:

* `PLAN.md` §104.19 records that a document this machine will not parse can
  **strand it**. Refusing to boot over a misspelled brightness key would be a
  catastrophic response to a trivial mistake.
* Everywhere else, `rivet-config` owns the schema, so an unknown key means the
  document is from a future it cannot understand. Here the platform explicitly
  does **not** own the schema, so an unknown key means something much smaller:
  one application was told something it does not understand.

**Ruled: an unrecognised or unparseable key under the application region is
REPORTED and ignored, never fatal.** It appears in `get status` and in the
reload's answer so an operator can see it, and the machine keeps running. The
strictness of the rest of the document is untouched.

That is the honest answer to the objection: the part that cannot be fully
validated is also the part that cannot break the machine.

### 4.5 A refusal names the table the key belongs to — 2026-09-05

**Reported and ignored is only half of a good refusal. The other half is that a
reader can act on it, and for one key both refusals were true and neither was
actionable.**

`enabled` is where an operator's hand goes first, and `[applications.<name>]` is
the table named after the application, so that is where it gets written. What
came back was, in order:

1. from `rivet-config`, *"a setting's value must be a quoted string"*, whose
   suggested repair works — quote it and the document loads;
2. from the supervisor, *"this application's manifest does not declare that
   setting, so it would never be read. Check the spelling"*, which sends the
   reader to verify a spelling that is already correct.

An application is enabled under `[services.<name>]`. Neither sentence said so,
and the second port to adopt the SDK lost a boot between them.

**Ruled: where a key written under `[applications.<name>]` is one
`[services.<name>]` declares, the refusal names `[services.<name>]` and gives the
line to write.** Two layers, because the two know different things:

| Layer | Sees | What it can say |
| :-- | :-- | :-- |
| `rivet-config` | the document only | a **non-string** value under such a key. It has no manifest, so it cannot judge a quoted one |
| `rivet_core::settings::render` | the document **and** the manifest | the key is not declared by this application **and** belongs to the other table |

**The schema is not widened and nothing new is accepted.** Both keys were
already refused; only the sentence changed. §10.2's text-only rule stands, and
the second layer consults the manifest **first**, so an application that
genuinely declares a setting called `enabled` keeps it and the platform has not
narrowed a region §4.3 puts outside it.

**Two spellings of one intent was the alternative and it is worse.** Accepting
`enabled` in both tables would give a machine two answers to whether an
application runs, which is `ADR-011` exactly; and the two would then have to be
reconciled at every read, in a region whose whole design is that the platform
does not know what the keys mean.

`rivet_core::service::OPERATOR_KEYS` is the one list both layers read, moved down
from `rivet_config::Service::DECLARED_KEYS` in the same change so that a key
added to `[services.<name>]` is covered on the day it is added.

**The same pass repaired what §10.2's refusal says**, for the friction Trends
reported: it named the rule and not the repair, so an operator with an integer
setting was told that a bare number is not carried and not that the fix is to
write `max-readers = "3"`.

---

## 5. Decision: settings and secrets travel different channels

| | Goes | Ends up in the register? |
| :-- | :-- | :-- |
| **A setting** — a stop id, a brightness, a zone name | the machine document | **yes**, and that is the point |
| **A secret** — an API token, a device seed | `apply install-secret` | **never** |

`ADR-034` §3 rules that the register holds no signing key and no device private
key, and §3.4 explains what that buys: *"a register of public facts can be put
in `git`, reviewed as a diff, synced between machines and read by an agent."*

**A token in the document is a token in git.** So the channel is chosen by one
test: *could this value be committed to a repository the operator shares?* If
not, it is a secret and goes to one machine only.

`apply install-secret` already exists and already writes into an application's
data directory with that application's ownership, which is why this ADR does
not need to invent the second channel — only to say where the line is.

**Each channel has its own SDK call, named for what it is** — `.rulings/`
R-169. `rivet_sdk::settings::get` reads the delivered settings file; `rivet_sdk
::secrets::get` reads an installed secret by name, straight out of the
directory `apply install-secret` already writes into (`RIVET_APP_DATA`),
which needed no new handed variable because that directory was already
handed over. An addon author still does not have to reason about the
register's threat model to read a stop id — the two calls have the same
shape, `Option<String>` for an unset value never an error — but a value's
channel is visible in which call reads it, which is what makes the next
paragraph enforceable rather than only advisory.

**`settings` refuses a name that reads like a credential, so the boundary
cannot be crossed by mistake.** `rivet_core::package::application::
is_credential_like_name` checks a declared setting's whole hyphen-separated
words against `token`, `secret`, `password`, `passwd`, `credential` and
`key` (which also refuses `api-key`, since it splits into `api` and `key`) —
enforced wherever a manifest can be produced: `cargo rivet package`'s
builder, and the manifest parser the supervisor reads an installed
manifest's bytes through, since that path never goes through the builder. A
name that merely contains one of those words as a stray substring —
`keyboard-layout`, `monkey`, `upstream-name`, `sensor-labels` — is
unaffected; only a whole word matches. The refusal names the rule and points
at `apply install-secret` as where a credential belongs instead. TempMon's
own `write-token` — found written as an ordinary document setting on a real
machine — is exactly the shape this refuses; moving it into an installed
secret and reading it through `rivet_sdk::secrets` is TempMon's own change,
in its own repository, not this one's.

---

## 6. Decision: the SUPERVISOR delivers, not the launcher

The maintainer asked whether the launcher might pass settings in. It should
not.

**The launcher's subject is the display lease** (`PLAN.md` §106). Routing
settings through it would leave every application that does not draw — the
fleet server, a controller, a network addon, anything headless — with no path
at all, and the majority of Rivet machines are headless (`ADR-036` §8.4).

The supervisor already hands an application `RIVET_APP_DATA` and its listening
sockets. Settings are that mechanism a fourth time, which is what `ADR-004` §6
keeps calling *"the same mechanism again"*.

---

## 7. Decision: the control plane writes the DOCUMENT, never the process

An operator changes a setting by changing the document — an `apply` operation
that edits it and validates against §3's declaration — and then a reload. The
supervisor hands the new value over.

**The control plane must not push a value into a running application.** If it
could, the document and the process would be two authorities over one fact with
no vocabulary for their disagreement, which is precisely what `ADR-011` exists
to forbid and precisely the failure `ADR-034` §6.1 names: *"two places that say
where a machine listens, and no vocabulary for their disagreement."*

The payoff is that a setting survives everything. Restart, power cut, A/B
update, card rewrite, reprovisioning — because it was never only in the
process.

---

## 8. Recommendation: delivered as a file, not as environment

| Mechanism | Why not |
| :-- | :-- |
| **Environment variables** | Fixed at `execve`. A changed setting would require a restart, and `apply reload` exists precisely to be the lighter answer. Also visible in `/proc` to anything that can read it. |
| **A socket** | Live updates, at the cost of a protocol, a connection state and a failure mode, for values that change a few times a year. |
| ~~**A file in the application's data directory**~~ **A file the application is handed** | Survives a restart, is rewritten on reload, is readable with one SDK call, ~~and is the same shape `install-secret` already writes~~. **Recommended.** The struck words are superseded by §8.1 — a file, yes; in `RIVET_APP_DATA`, no. |

The SDK gains one call that returns the settings this application was handed.
An application that finds none must behave as though every value were unset —
which is `ADR-036` §8.5.2's rule arriving from another direction: **unreadable
and absent are the same answer.**

### 8.1 Superseded 2026-09-04: the file leaves `RIVET_APP_DATA`

**The recommendation above put the file in the one directory an unprivileged uid
owns, and the reason it gave — *the same shape `install-secret` already writes* —
turned out to be the defect rather than the argument for it.**

`ADR-015` §3.2A gives an application `<volume>/app/<name>/data`, mode `0700`,
under its own uid. The supervisor runs as root and wrote the settings file
**into that directory, by path, on every convergence.** The security audit of
2026-09-03 found what that means: the application deletes the file, puts a
symlink there — at the machine's document, at a host key, at another
application's data, at the journal — and root truncates the target and writes
settings text into it. A FIFO at the same path blocks the open and freezes the
supervisor. It was the audit's only CRITICAL.

**That instance is fixed and this is not a second fix of it.** `O_NOFOLLOW` plus
link and FIFO checks closed it (`PLAN.md` §123.2 item 2), and the guarded write
still exists and is still called. What the fix did not do is remove the *shape*:
root writing by path below a directory an unprivileged uid controls. The review
of 2026-09-04 put it plainly — the pattern **will be copied by the next feature
that needs to hand an application a file**, and it already has two users.

**Ruled: the settings file is `<volume>/app/<name>/settings` — root-owned, mode
`0644`, beside `current`.** That directory is the installer's, at mode `0755`
(`rivet_core::package::install::DIR_MODE`), so an application cannot create
anything in it and there is no link for root to follow. The class is gone rather
than guarded.

#### 8.1.1 What this costs, stated rather than discovered

| | Before | After |
| :-- | :-- | :-- |
| **The application can overwrite its own settings** | yes, and §8 named it as a cost | **no.** `ADR-007` §3.6.2: *a file that can be written and have effect is not a view*, and this is a view of the document |
| **Who else on the machine can read it** | that application and root | any uid — see below |
| **How the application finds it** | joined the filename onto `RIVET_APP_DATA` | **told**, in `RIVET_APP_SETTINGS` |
| **Lifetime** | survives an update, goes with an uninstall | unchanged — both are siblings of the version directories |

**`0644` reads like a widening and is not one.** §5 chooses a value's channel by
one test — *could this value be committed to a repository the operator shares?* —
and everything that answers yes is in the machine document, in the register, in
`git`. The `0700` the file used to inherit was a property of where it happened to
live, not a promise this ADR ever made about settings. A value that answers *no*
is a secret and goes through `install-secret` to `data/` at `0600`, unchanged.

#### 8.1.2 The application is TOLD the path, and that is not incidental

`RIVET_APP_SETTINGS` is a fourth handed variable (`rivet_core::service`,
`rivet_sdk::app`). It names a **file**, not a directory, because there is exactly
one and naming the directory would leave the filename to be agreed in two places.

The alternative was to let the SDK compose `$RIVET_APP_DATA/../settings`, and it
is rejected for the SDK's first article (`ADR-036` §2) — *an application declares what
it needs and is HANDED it. It does not go and take one* — and for the concrete
reason this section is an instance of: **this file has now moved once.** Every
program that had composed the path would compile, pass, and read a file nothing
writes.

**Using a variable for the PATH is not §8's rejected environment mechanism.** §8
rejects environment for the *values*, because a value fixed at `execve` cannot
change on `apply reload`. The path does not change while the process runs; the
contents do, and they are still in a file.

#### 8.1.3 `install-secret` is NOT moved, and here is exactly why

It is the second user of the shape and the review named it. Leaving one behind
risks the next author copying whichever they read first, so the decision is
argued rather than assumed.

**It stays, on three grounds, and none of them is "it is more work".**

1. **The application must be able to WRITE it, and that is the subject's own
   ruling.** `install_secret::SECRET_MODE` is `0600` and not `0400`,
   deliberately: *"several programs rewrite their own state file beside a key,
   and a mode that refuses that would be this subject deciding how an
   application manages its own directory."* A root-owned file beside `current`
   cannot be rewritten by the application at all. Moving it would silently
   overturn that ruling, which is `AGENTS.md` §1's line — a decision that
   changes the shape of the software.
2. **The exposure is a different one.** Settings are written **unattended, at
   boot, on every reload, and the moment a newly installed application is
   discovered** — corrected 2026-09-05 from *"on every convergence and every
   reload"*, which named a rate rather than the events that actually cause a
   write (`PLAN.md` §134.3, §8.1.4 below) — so an attacker chooses one of a
   few known moments rather than a continuous one. A secret is installed by an
   operator, once, behind a plan and a typed confirmation, at a moment
   somebody is watching. That does not make settings unsafe — it makes the
   window a short, named list of events against a guarded write, not a
   standing one.
3. **`0644` is not available to it.** Everything §8.1.1 says about settings being
   register-public is the opposite of what a secret is.

**What it would take**, so this is a decision and not a deferral: a ruling that
an application may not rewrite its own installed credential (overturning
`SECRET_MODE`'s note); a `chown` to the application's gid with `0640` in the
install plan and in `OUTPUTS`, which brings a new failure mode — the chown
fails, the file is root-only, and the application reports `EACCES` from inside
itself at a moment nobody is watching, which is the exact failure
`crate::service::app_identity` exists to prevent; a `RIVET_APP_SECRETS` variable
beside `RIVET_APP_SETTINGS`; and a migration for `cmd/rivet-fleetd`'s node seed
and `cmd/rivet-proxyd`'s certificates, both of which read out of
`RIVET_APP_DATA` today.

**Until then the guarded write is what stands between it and the audit's
finding.** `Kernel::write_file_nofollow` is therefore kept and documented as the
correct call for *any* write below an application-owned directory — not as
something the settings file needed.

#### 8.1.4 Delivery is by EVENT, not by rate — settled 2026-09-05

The recommendation above and §8.1.3 item 2 both said the file is written *"on
every convergence and every reload"*, and `PLAN.md` §134 found that the build
matched half of that sentence: a reload delivered settings, `crate::main`
delivered them once at boot, and neither the ADR nor the code had ever asked
what a continuous reading of *"every convergence"* would cost. `runtime::
declared_settings` reads every installed application's manifest off disk; a
supervisor's convergence pass runs on every event a machine produces — a
process exiting, a timer firing — not on a document change, so a literal
reading would mean a filesystem read and a settings write per installed
application on every one of those, for no reason tied to a setting having
changed at all.

**The sentence named a rate where it meant a guarantee, and the guarantee is
what the code now gives: every EVENT that can make settings stale delivers
them, and nothing else does.** Three events, and each was chosen because
without it a value written to the document would sit unseen:

1. **Boot** — `crate::main`, before the first convergence pass, so the first
   thing every application reads is current.
2. **Reload** — `crate::reload`, `SIGHUP` (`ADR-032` §3.2), inside the same
   `!Unreadable` guard as `enabled`, because an operator who changes the
   document and asks for it to be taken must not wait for a reboot.
3. **An application newly discovered while the machine runs** —
   `crate::installed::Watch::look`, the same moment `PLAN.md` §19.2 already
   makes a newly installed application into a service this supervisor knows.
   Closed 2026-09-05: before this, `cargo rivet deploy --enable` against a
   running machine started an application that had never been handed the
   settings already sitting in the document for it, and nothing but a second,
   separate reload would deliver them. `Watch::poll`'s own guard — it acts
   once per finished install, not once per pass — is what keeps this an event
   rather than the wasteful rate the sentence used to describe.

**What is deliberately still absent: a setting changed on an application that
is already running and was not reinstalled.** That is `crate::reload`'s job,
and it already covers it — a document edit reaches a running application
through `apply reload`, same as `enabled` does, and this section is only about
the moment an application starts knowing nothing yet.

`a_reload_hands_an_application_the_settings_the_document_now_carries` (reload)
and `an_application_installed_while_the_machine_runs_is_handed_its_settings`
(installed) are the two tripwires — each watched to fail with its delivery
call removed before it was trusted (`AGENTS.md` §4).

---

## 9. Open: what happens to a RUNNING application when a setting changes

The honest options, none of them free:

1. **The supervisor restarts the service.** Simple, always correct, and visibly
   heavy for a brightness change.
2. **The application is signalled and re-reads.** Cheap, but every application
   must implement it correctly or the setting appears not to apply.
3. **The application polls.** No platform work; wastes wakeups on a device that
   should be idle.

**Recommendation: the manifest says which**, defaulting to restart. That is
`ADR-034` §5.3's precedent — it ruled that a setting requiring a reboot must
*say so* rather than appear applied, and made it three states rather than a
`bool` because "unknown" is a real answer. The same honesty is owed here: a
platform that reported a setting as applied when the process had not re-read it
would be the *"applied when it is not"* state §5.3 exists to prevent.

---

## 10. Ruled, 2026-09-01 — with the objections, because they are the reasons to revisit

The maintainer answered all five and disliked three of the answers. **His
objections are recorded rather than smoothed over**: an ADR that keeps only the
verdict loses the reason somebody would reopen it.

### 10.1 The application region in the document — ACCEPTED, unwillingly

Accepted, and recorded as accepted **against his instinct** — he judged the
reasoning correct and the conclusion distasteful, and took it as the best
option available rather than a good one. *(`.rulings/` R-123.)* **What would reopen it:** any second mechanism that can be
provisioned before first boot, survive a card rewrite, and be mirrored by the
register. Nothing available today does all three, which is the whole of the
argument — not that this is good, but that the alternatives lose all three.

§4.4's containment was added in response to the second half of his objection
and is part of what makes this acceptable.

### 10.2 Text-only values — ACCEPTED, unwillingly

Accepted for a first version. A value is text; the application parses it.

**The cost, restated so it is not a surprise:** `stop-id = "twelve"` is refused
by the application at runtime, on the machine, rather than by the platform at
provisioning, in front of the person who typed it. That is exactly the failure
§3 exists to prevent, tolerated here because a type system in the manifest
needs a schema language and the manifest's grammar is a strict subset of TOML
by rule.

**What would reopen it:** the first setting where a wrong *type* rather than a
wrong *name* reaches a fleet.

### 10.3 Defaults in the manifest — NO. Absent is simply absent

The maintainer disliked both directions and said why. Against having no
defaults: nobody should be forced to set a value in a configuration file merely
because the software declined to choose a sensible one. Against having them:
defaults breed edge cases and edge cases breed problems. He ruled absent for
now, without enthusiasm. *(`.rulings/` R-123.)*

**Ruled: absent. And the tension he named dissolves on inspection, which is
worth writing down because the ruling reads more expensive than it is.**

Rejecting a default *in the manifest* does not mean an application has no
sensible default. It means the default lives **in the application's own code**,
which is the better home for it in every respect:

* it is versioned and signed with the code that reads it;
* it is testable by the application's own test suite;
* it is not a second place a value can come from — a manifest default plus a
  document value is precisely the two-authorities problem `ADR-011` forbids,
  and diagnosing *"which of these two is winning"* is the failure it produces.

So the operator's promise is kept — you do not have to set a value to get
sensible behaviour — without the platform learning what a default is.

**What would reopen it:** an application where absent must mean *broken* rather
than *use the sensible thing*, and where the platform rather than the
application should say so.

### 10.4 When a setting changes, does the application restart? — OPEN

**The question in plain words**, because the first draft of this list asked it
as a section reference and the maintainer rightly refused to answer that:

You change a brightness value and reload. The application is already running.
**Does the platform stop and restart it so it reads the new value, or does the
application stay up and re-read on its own?** Restarting always works and is
visibly heavy for a small change. Re-reading is cheap but only works if every
application implements it correctly, and one that does not will look like the
setting silently failed to apply.

**Recommendation: the application says which, in its manifest, and the default
is restart** — the safe answer when nothing is declared. §9 has the three
options and the `ADR-034` §5.3 precedent for being honest about when a setting
actually takes effect.

### 10.5 Should the fleet page let you see and change settings? — OPEN

**Also restated.** §3's declaration means the fleet server can read, out of a
signed payload, what each application on a machine can be configured with —
without running it, and without anyone writing that list down twice.

**So the page could show every setting a machine's applications accept, with
their current values, and let you change one.** Nothing in this ADR makes that
happen; it only makes it possible. The question is whether it is wanted, and
whether editing belongs there or only viewing.

---

## 11. Consequences

* `rivet-config` gains a bounded region whose schema comes from outside it.
  This is the first time anything in that document is not fully described by
  `rivet-config` itself.
* `ADR-034` §6's **Fleet configuration** row acquires contents for the first
  time since it was written.
* The manifest becomes a capability declaration as well as a permission
  request. Every existing manifest stays valid — the key is optional, and
  appended to `KEYS` rather than inserted, so the canonical ordering rule does
  not renumber anything.
* `rivet-provision` gains the ability to be wrong in a new, loud way: it can
  refuse a document for naming a setting an application does not have.
* Aura's port stops needing a private configuration channel, and its
  `brightness`, screensaver and schedule settings acquire a home.
* **Nothing here is built.** This ADR is a proposal; `PLAN.md` §110 carries the
  work.

---

## 12. What is built, and what has run

Built 2026-09-02, the day after this was accepted. **Nothing has run on a
board.**

| | State | Proven by |
| :-- | :-- | :-- |
| §3 — the manifest declares | **built** | an OPTIONAL key, appended last. 6 tests, including that a manifest from before it **parses and renders byte-identically** — which is why no existing `.rvt` was invalidated, unlike `library` and `devices` |
| §4 — the document carries | **built** | `[applications.<name>]`, `rivet-config`. **7 shapes asserted not to refuse the document** — a number, a boolean, a float, an array, a nested table, an undeclared key, and an application that is not installed |
| §4.4 — never fatal | **built and asserted** | what cannot be carried is reported with its application, its key and a sentence; the document loads |
| §6 — the supervisor delivers | **built** | 5 tests. The declaration is checked here, where the manifests are, which is §4.3's *"the check moves, it does not vanish"* |
| §8 — as a file | **built** | `rivet_core::settings` renders, `rivet_sdk::settings` reads, and **4 agreement tests** prove the two halves cannot drift — including that both sides agree on what a setting NAME is, because a disagreement there means the supervisor writing a line the application silently skips |
| §8.1 — out of `RIVET_APP_DATA` | **built 2026-09-04** | the path is `<volume>/app/<name>/settings`, root-owned `0644`, and `RIVET_APP_SETTINGS` hands it over. Four tests were broken and watched to fail first (`AGENTS.md` §4). **NOT run on a board** — see §12.2 |
| §8.1.4 — delivered on every event that can make settings stale | **built 2026-09-05** | boot (already had it), reload (`PLAN.md` §134), and a newly discovered application (this session) all call `runtime::deliver_settings`. Two tripwires, each watched to fail with its call removed. **NOT run on a board** — see §12.2 |
| §9 — a changed setting on a running application | **not built** | restart is what happens, which is §9's own recommendation for the default. The manifest key that would say otherwise does not exist |
| §5 — secrets, read through the SDK | **built** — `.rulings/` R-169 | `rivet_sdk::secrets::get`/`get_bytes` reads `RIVET_APP_DATA/<name>`, exactly where `install-secret` writes; no supervisor change needed. `aura-controller`'s `credential::read` adopts it. **NOT run on a board** — nothing has installed a secret and read it back through this call on real hardware |
| §5 — `settings` refuses a credential-shaped name | **built** — `.rulings/` R-169 | `is_credential_like_name`, enforced by `cargo rivet package`'s builder and by the manifest parser the supervisor reads bytes through. Unit-tested against the words a substring check would have wrongly caught (`keyboard-layout`, `monkey`) and the real fixture that found the gap (`write-token`) |
| §10.5 — the fleet page | **not built** | still open, and downstream |

### 12.1 The one thing checked before the key was added rather than after

`library` and `devices` are REQUIRED manifest keys, and their arrival
**invalidated every existing package** — deliberately, because a capability
that defaulted to absent would be indistinguishable from one an author never
considered.

Adding `settings` the same way would have invalidated the three `.rvt`s built
that morning and waiting to go to a board. It did not have to: a setting is not
security-relevant in that sense — **declaring none accepts none**, which is the
closed end of the axis (`ADR-015` §3.3) and is what every application did before
the key existed. `title` was the precedent for an optional key and the rule that
separates them.

So the key is optional, written only when non-empty, and
`manifest_text_is_exactly_this` still passes untouched.

### 12.2 What is NOT proven

**No application has read a setting.** The chain is asserted end to end in
tests — manifest → document → supervisor → file → SDK — and every link is
checked against the next, but the whole of it has run on a workstation.

`aura-renderer` still carries `BACKGROUND` and `BRIGHTNESS_PERCENT` as
compile-time constants. **Making it read them is the demonstration**, and it is
the next piece: it closes `SPEC-04` §11 (2), (3) and (9) together, and it is
the first time an operator will be able to change what is on the wall without
rebuilding anything.

**Corrected 2026-09-04: one application does read a setting.** `cmd/rivet-proxyd`
takes `upstream-name` through `rivet_sdk::settings::get`, and the board is
carrying `[applications.rivet-proxyd]` in its document today. So the chain has
run on hardware end to end, and the sentence above was true when it was written
(2026-09-02) and stopped being true within a day — `AGENTS.md` §7 in the
direction it usually is not.

**What §8.1 asks of that machine, stated because it can be got wrong once.** The
image and the application must move together: a supervisor from before §8.1
writes `data/settings`, and a `rivet-proxyd` built against the SDK after it reads
`RIVET_APP_SETTINGS`, which that supervisor does not set. Either mismatch ends in
the same place — no settings, and `rivet-proxyd` refusing to start with the
message that names the document table. **Loud, and recoverable by redeploying the
other half.** There is deliberately no fallback to the old path: two places a
setting can come from is `ADR-011`'s two authorities, for a transition that is
one deploy long.
