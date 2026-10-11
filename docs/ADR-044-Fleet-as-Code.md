# ADR-044: Fleet as Code — The Desired-State Documents Live in Git

* **Status:** Accepted, and the first increment is built
* **Date:** 2026-09-05
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Ruled:** `.rulings/` R-145, which approves the idea, settles three of the
  four open questions, and leaves ordering across machines open for §7 to argue
* **Implements:** `ADR-034` §2.2's Remote mode gaining an authority for what a
  machine should be
* **Code:** `rivet-core/src/converge.rs` (the decision), `rivet-core/src/leaf.rs`
  (the comparison), `rivet-config/src/leaves.rs` (the reader),
  `rivet-core/src/fleet_as_code.rs` (the repository operations' vocabulary),
  `cmd/rivet-mcpd/src/fleet_as_code.rs` (the git),
  `cmd/rivet-fleetd/src/code.rs`, `cmd/rivet-fleetd/src/repo.rs`,
  `cmd/rivet-fleetd/src/document.rs`
* **Ruled also:** `.rulings/` R-187 D2, which puts the repository's git in
  `rivet-mcpd` and leaves the application sandbox as it is (§5.5)
* **Proof:** `tools/qemu/adr-044-proof.sh`, `tools/qemu/adr-044-fleet-code-proof.sh`

---

## 1. Context

**A git repository, hosted on a Rivet, holding one desired-state document per
machine. A push is what changes the fleet.**

The maintainer's ruling is `.rulings/` R-145 and this document is what it
authorises. The proposal it accepts is `fleet-as-code.md` in the maintainer's
scratchpad; that file is not in this repository and this document is written
to be read without it.

### 1.1 What is bought, beyond tidiness

**Every fleet change gets a diff, an author, a timestamp and a revert.** Today
a change is an `apply edit` over the control plane. The journal records that it
happened and who asked, but not what the document looked like before, and there
is no way to put it back except by remembering.

**A new workstation clones the fleet.** A stored profile remembers one
machine's address for one operator on one workstation; a repository *is* the
fleet's definition.

**The web interface gets much smaller.** `ADR-043`'s portal has to be a form
that validates every key, refuses the impossible ones and is the authority for
what a machine should be. A viewer that renders state and shows what a pending
change would do is a fraction of that. `cmd/rivet-fleetd`'s own principle —
the register and the machine are two authorities kept visibly apart — survives
intact, because git becomes the third and the most human-readable of them.

**And it is the natural home for the agent.** `ADR-012` makes an agent a
first-class operator. An agent that proposes a fleet change as a commit is
reviewable in a way that an agent calling `apply edit` is not.

### 1.2 Why it is small, which is the surprising part

Every piece was already built and proven. What follows is wiring, and the two
places where it is not are §6.2's drift rule and §7's argument.

| piece | state before this |
| :-- | :-- |
| a git server over smart HTTP | proven, `ADR-032`, running on the board |
| one TOML per device | already the shape of the fleet register, `ADR-034` |
| pushing a document to a machine | `apply edit` over `ADR-035`'s control plane |
| a machine refusing a bad document | `rivet-config` refuses unknown keys with no carve-out |
| comparing two documents leaf by leaf | `ADR-034` §6.2's refusal, built 2026-08-29 |
| an application's own settings | `ADR-038`, built 2026-09-02 |

---

## 2. What is settled, and is not re-argued here

`.rulings/` R-145.1 adopts three positions with the idea, on the ground that
each is a safety posture this project already takes elsewhere.

### 2.1 The fleet server POLLS. There is no `post-receive` hook

A hook on the machine serving the repository is the obvious answer and it puts
a **fleet-wide actuator inside the git server** — a service that already execs
`git-http-backend` as root, which the audit of 2026-09-03 flagged. Polling is
slower and duller and keeps the git server a git server.

The repository is a bare repository on the fleet server, served by that
server's control plane, and `rivet-fleetd` asks the control plane for what it
holds (§5.5). Nothing is ever checked out and `rivet-fleetd` keeps no copy, so
a repository that cannot be reached is a pass that does not happen and the next
cycle asks again — a quiet non-event rather than a fleet-wide outage. The
control plane answers questions and makes the one commit `POST
/api/commit/<device>` asks for; it acts on no machine, so the git server stays a
git server.

The cost is latency: a push takes effect within one poll interval, one minute
by default. `rivet-fleetd converge` makes a pass immediately for an operator
who does not want to wait, which is the whole of what a hook would have bought.

### 2.2 A push that would overwrite out-of-band drift REFUSES

`apply edit` must keep working. **A fleet you cannot touch when the repository
is unreachable is worse than no fleet**, so drift is possible by design — and
`R-40`'s reasoning applies: the dangerous direction needs a word. §6.2 is the
rule and §6.3 is where the word lives.

### 2.3 A document in git carries a REFERENCE to a secret and never a secret

**This needed no code, and that is the finding rather than a convenience.**
R-145.1 asked that the guard which refuses an unknown key be the guard that
refuses a literal. It already is: `ADR-007` §7.3 gives the schema **no key a
credential could be written under**, and `rivet-config`'s own
`there_is_no_key_for_a_credential` asserts it for `psk`, `passphrase`,
`password`, `pmk` and `key`.

So the fleet server runs `rivet_config::Document::parse` over every file in the
repository **before it contacts any machine**, and a push carrying a credential
is refused there. §9's stage 3 is that, watched: the file is pushed, the pass
refuses it naming `psk` and pointing at `apply install-secret`, and the
machine's document is byte-identical before and after.

A wireless passphrase reaches its machine by `apply install-secret`
(`ADR-020` §11.2.3) and the repository never sees it.

---

## 3. The authorities, and there are now three

`ADR-011` requires that every fact have one owner, so this has to be stated
plainly rather than left to be inferred.

| | owns | read by |
| :-- | :-- | :-- |
| **the repository** | what a machine *should* be | the fleet server |
| **the register** (`ADR-034`) | how to *reach* a machine — its address, its pin, its identifier | the fleet server |
| **the machine** | what it *is* | everybody, over the control plane |

**The repository does not replace the register and cannot.** A document says
nothing about where the machine is or what key to trust at the far end, and a
repository that carried those would be a repository of credentials in the sense
§2.3 refuses. The filename is the machine's **label**, and
`Register::resolve` turns a label into a record — refusing an ambiguous one
rather than choosing, which is `ADR-034` §10.2's rule and matters more here
than it did there.

**The machine remains the authority for its own state.** The fleet server never
answers a question about a machine out of the repository; §6.1's comparison
reads the machine every cycle.

---

## 4. The repository's shape, and the bootstrap

```
fleet/
  machines/
    rivet-01.toml       the desired state, exactly the document that machine reads
    kitchen-pi.toml
README.md
```

`examples/fleet/README.md` is the README a real fleet repository carries, and
`tools/qemu/adr-044-proof.sh` copies it into the repository it builds rather
than describing a shape nothing produces.

**The format needs no invention and that is the point.** A file under
`fleet/machines/` is the same TOML the machine keeps at
`/state/config/machine.toml`. There is no second format and no translation
layer — if the two could diverge in shape, they would.

### 4.1 A machine is adopted by copying its document in verbatim

`rivet get desired-state` prints exactly the file the repository holds. Commit
that, push, and the first cycle finds the machine already agrees: it is
adopted, the baseline is recorded, and **nothing is written to the machine.**

**A composed file is refused.** A fleet server that has never converged a
machine will not write a document it has not first seen that machine agree
with — `Verdict::Unadopted`. Adopting a machine onto a file nobody compared is
the blind overwrite the whole mechanism exists to prevent, and it would arrive
disguised as the ordinary first use.

### 4.2 The repository is authoritative for VALUES, not for BYTES

Read off the live board on 2026-09-05: `rivet-01`'s document is 1,351 bytes,
carries ten consecutive blank lines and two tables with no blank line between
them, and its own header still says it was canonically serialised. Every
`apply edit --append` that has ever run against it left its formatting behind.

A fleet server that compared **bytes** would rewrite that machine on every
cycle for ever and bury its own journal. So every comparison here is over
`rivet_core::leaf`'s leaves, and two documents that differ only in comment,
order and whitespace are the same document. That text is a test fixture in
`rivet-config/src/leaves.rs`, labelled as captured from hardware rather than
composed — `AGENTS.md` §4.

---

## 5. Where the code is, and why the decision is in the core

```
rivet-core/src/leaf.rs        the types, and the leaf-by-leaf comparison. No dependencies.
rivet-core/src/converge.rs    THE DECISION. Three sets of leaves in, a verdict out.
rivet-config/src/leaves.rs    a document's text, as those leaves. Needs toml_edit.
rivet-core/src/fleet_as_code.rs   the repository operations: routes, formats, path rule, refusals
cmd/rivet-mcpd/src/fleet_as_code.rs   the git, as root, against the server's repository
cmd/rivet-fleetd/src/repo.rs      asking this server's control plane for those operations
cmd/rivet-fleetd/src/document.rs  get desired-state and apply edit, over ADR-035
cmd/rivet-fleetd/src/code.rs      the wiring, the baseline, and what gets said
```

### 5.1 The comparison moved, because it acquired a second caller

`ADR-034` §6.2's diff — *"REFUSES if the write would remove anything the
register does not account for"* — is the same question a drift report asks, and
two implementations of *how do these two documents differ* would eventually
disagree about the one case that matters.

The split is by dependency, not by who asked first. `rivet-core` depends on
nothing that touches the world, so it holds the **types and the comparison**;
`rivet-config` already owns the schema and already carries `toml_edit`, so it
holds the **reader**. `cmd/rivet-fleet/src/diff.rs` keeps `remedy`, `cost` and
`refusal`, which are about that tool's own commands, and re-exports the rest.

`leaf::additions` is new. `between` is deliberately one-directional — its first
caller asks *what would this write take away* — so on its own it cannot see a
table somebody added to a machine by hand, which is exactly the drift most
likely to be silently overwritten.

### 5.2 The decision is in `rivet-core`, and the reason is testability

`AGENTS.md` §3 puts logic out of composition roots, and this is the strong form
of that argument rather than an application of the letter: **a rule that lives
beside a `git fetch` is a rule only ever exercised against a board.**
`rivet_core::converge::decide` takes three sets of leaves and returns a verdict.
`cargo test -p rivetos-core` exercises every branch of it with no network, no
repository and no machine.

### 5.3 A dependency was added, and here is its price

`cmd/rivet-fleetd` now depends on `rivetos-config`, which puts `toml_edit` in
the binary. That is the honest cost and it buys the thing that matters: **the
fleet server refuses a document with the machine's own parser.** A second TOML
reader with its own opinions is how a fleet server and a machine come to
disagree about a document neither of them wrote.

`rivetos-remote` was the other candidate — it already composes exactly the two
control-plane calls `document.rs` makes, with the same compare-and-swap and the
same read-back. It was rejected: it is a **host** library that links a whole TLS
client stack the image does not otherwise need. Pulling that onto
a card to reuse two functions is a trade that would be hard to reverse. What is
shared instead is the part that must not diverge — the subject's word, the
verb's word and the confirmation word are `rivet-core` constants, so a rename in
the catalogue breaks the build rather than a board.

### 5.4 Where the configuration lives

`fleet-repository`, `fleet-branch` and `fleet-poll-seconds` are `ADR-038`
settings, written under `[applications.rivet-fleetd]` in the fleet server's own
desired-state document. **A fleet server's configuration is itself
fleet-as-code**, which is pleasing and also correct: there was no argument for
inventing a configuration file for one feature when the mechanism exists.

**`fleet-repository` names a repository on the fleet server itself** — the name
of a `[repos.<name>]` table in the server's own document, which `rivet-sysd`
creates under `/state/git` and `rivet-mcpd` serves (`ADR-035` §4). It is a name
and never a URL or a path. It may not be `fleet`: that repository is `ADR-052`'s,
its `main` is the operator's workstation's text pushed with `cargo rivet fleet
push`, and a commit made into it would be undone by the next push. The two
repositories stay apart.

**`fleet-repository` unset is the off switch.** A server with it unset grows no
poller; turning it on is one setting, a declared repository, and no flag.
Provisioning declares `[repos.fleet]` on a server and does not turn fleet-as-code
on.

### 5.5 The git runs in `rivet-mcpd`, as control-plane operations

**`rivet-fleetd` runs no program.** It is an ordinary application confined by
`ADR-042`'s policy, which grants no execute right on `/system/bin` and no access
to `/state/git`, and that policy is not widened for it (`.rulings/` R-187 D2).
`cmd/rivet-mcpd` already runs git as root for smart HTTP and for `ADR-052`
§7.4's repository store, so it runs these too — the same shape as that store:
routes on 7777, a caller named by its pinned key, and a `permits` table.

```text
POST /fleet-as-code/head       repository, branch                  the commit the branch names, and its message
POST /fleet-as-code/machines   repository, commit                  every machine document that commit holds
POST /fleet-as-code/file       repository, commit, path            one machine document, as that commit has it
POST /fleet-as-code/commit     repository, branch, expected-head,  one machine document committed onto the branch,
                               path, message, document             only if its tip is still expected-head
```

They are exactly what the poller and `POST /api/commit/<device>` ask:

| | |
| :-- | :-- |
| **who** | a key in the machine's `peers` — the operator's identity and the fleet server's own operator identity, both written by provisioning. A connection with no pinned key is refused `403`; a key not in `peers` completes no handshake. `rivet_core::fleet_as_code::permits` is the rule as a table, so a member that later fetches from its server is a new caller that the table grants nothing |
| **where** | `rivet_core::git::repository_dir(<name>)`, for a name the request's parser accepted as a repository name and not `fleet`. The caller never supplies a filesystem path |
| **which files** | `fleet/machines/<label>.toml` and nothing else: no absolute path, no `..`, no deeper directory, no other file in the repository. Refused `400` by the parser, before git is given anything |
| **how git runs** | its environment replaced, `safe.directory` naming the one repository, pathspecs literal |
| **how a commit is made** | plumbing on the bare repository, touching no checkout: the document as a blob, a private index read from the expected tip with that one path replaced, a tree, a commit on the tip, and `update-ref <branch> <new> <expected>` |
| **a stale head** | refused `409` with the word `stale-head`, and nothing moves — whether the tip had moved before the request arrived or moves between the check and `update-ref` |

**The commit is two compare-and-swaps, and each catches a different arrival.**
`POST /api/commit/<device>` carries what the browser believes the document is.
`rivet-fleetd` reads the branch's head and the document at it and refuses a
mismatch, which catches a change composed against a document somebody has since
changed. It then asks for the commit on the head it read, which `rivet-mcpd`
refuses as a stale head if the branch moved in between — the race the first
comparison cannot see, where two operators, or an operator and a push, arrive
together. Both refusals say nothing was committed.

**`rivet-fleetd` finds its own control plane in its register**: the one record
whose `role` is `fleet-server`, which provisioning writes as the record a
server's card carries (`ADR-034`). That record's pin is the machine's key, and
its `listen` says where `rivet-mcpd` binds — an unspecified or loopback address
is reached on loopback, a specific one there. None, or more than one, is
refused by name rather than chosen between. This is §7.5's second candidate, a
flag on the server's own record, used for reaching its own control plane; §7.4's
ordering does not rest on it.

`cmd/rivet-sysd/src/landlock.rs`'s `rivet_fleetd_confined_by_its_own_policy_cannot_run_git`
builds that application's ruleset and asserts a confined child is refused git
at `execve` with `EACCES`, which a policy row granting it would turn red;
`cmd/rivet-fleetd`'s `no_source_file_in_this_crate_starts_a_process` asserts
nothing in that crate needs it.

---

## 6. The rule, in full

For each machine the branch names, at the commit `HEAD` points at:

* **`desired`** — the document in the repository now.
* **`machine`** — the document the machine holds now, read over the control
  plane.
* **`baseline`** — the document the repository held at the commit this server
  last converged this machine onto, or nothing if it never has.

| machine vs desired | baseline | machine vs baseline | verdict |
| :-- | :-- | :-- | :-- |
| same | — | — | **Converged.** Do nothing, say nothing |
| differs | none | — | **Unadopted.** Refuse — §4.1 |
| differs | some | same | **Apply.** Write it |
| differs | some | differs | **Drifted.** Refuse, and name it |
| differs | some | differs, and the push carries the word | **Overwrite.** Write it, and report what was lost |

### 6.1 Why there are three inputs and not two

**Without the baseline, drift is indistinguishable from a push.** Both are
*the machine and the repository disagree*, and the correct answer is opposite in
the two cases. Two inputs cannot tell them apart.

The baseline is stored as **a commit id per device**, under the device's
identifier, in the fleet server's own data directory — not as a copy of the
text. Git already holds every version with an author and a date; a second copy
of the text would be a second authority for what the repository said, and it
would be the copy nobody could diff. A baseline naming a commit that is no
longer in the repository reads as *never converged*, which is the safe
direction: a history rewritten under a fleet server is exactly when it should
stop believing what it wrote down.

It is filed under the **identifier** and not the label, unlike the repository,
where a human reads the filenames. A label is mutable (`ADR-025` §4.1) and a
baseline that moved when somebody renamed a machine would silently become
*never converged* — a refusal an operator would then override.

### 6.2 The refusal names the keys and both values

*"Refused: drift"* is useless, and the natural wrong move from a report like
that is to overwrite without looking. `ADR-034` §6.2 already argued this shape
for the register's own refusal and the reasoning carries. What the proof
produced, verbatim:

```
rivet-fleetd: fleet-as-code: adr044-guest: REFUSED. NOTHING HAS BEEN WRITTEN
and the machine is exactly as it was.
  why  this machine has been changed since the repository last wrote to it, and
       the repository is now asking for a different document. Applying it would
       silently undo the change below, which nobody recorded a reason for.
  hostname
      repository  converged-by-a-push
      machine     changed-by-hand
  kiosk.auto-run
      repository  (not there)
      machine     changed by hand, out of band
  fix  either put the change into the repository - it is the fleet's definition,
       and a value only a machine knows is a value the next apply deletes - or,
       if the machine is the one that is wrong, add the line
         Fleet-Override: overwrite-drift adr044-guest
       to the commit message of the push that should win.
```

Both shapes are there and the second is the one an implementation forgets: a
value that moved has two values to print, and a table added out of band has
none on the repository's side.

### 6.3 The override word is a COMMIT TRAILER, and it names one machine

```
The kitchen pi goes back to the fleet resolver

Fleet-Override: overwrite-drift kitchen-pi
```

Three candidates were weighed:

| where the word lives | | |
| :-- | :-- | :-- |
| a key in the machine's document | rejected | it would be a **second document format**: the file under `fleet/machines/` is the machine's own document and a key the machine's schema does not have would be refused by the very guard §2.3 relies on |
| a sibling file in the repository | rejected | a file is a **standing permission**. It authorises the push nobody has thought about yet, which is the one the refusal is protecting |
| **a trailer in the commit message** | **chosen** | it belongs to one push, it is in the history with an author and a date, and whatever signs the commit signs it |

**It must name the machine.** A blanket override clears the drift on a machine
the operator was not thinking about, which is the failure the refusal exists to
prevent arriving through the door marked *fix*. One trailer may name several,
because one push often changes several.

**A label containing whitespace, a `"` or a `\` is double-quoted**, with `\"`
and `\\` as the only escapes: `Fleet-Override: overwrite-drift "kitchen pi"`.
A label may contain a space, and splitting the line on whitespace alone would
read `kitchen pi` as two machines. A bare word keeps its meaning, so a trailer
written before quoting existed still names what it named. The refusal prints
the form the parser reads, and a test pins the two together (`.rulings/`
R-210).

**It cannot adopt.** The word is spelled *overwrite the drift*, and an
unadopted machine has no drift to overwrite — there is no baseline, so there is
nothing the operator can be said to have looked at. `decide` consults it on the
drift branch alone.

### 6.4 A machine that is off is not a failure

The document is intent; convergence happens when the machine is reachable.
**Reporting an unreachable machine as a failed apply is wrong and would train an
operator to ignore the report**, which costs more than the report is worth.

`rivet-fleetd converge` exits non-zero for a refusal and zero for an
unreachable machine, because those are different things and a script that could
not tell them apart would retry the one that needs a person.

The same distinction runs through the summary: a machine that refused is
`REFUSED x`; a document the repository got wrong is `THE REPOSITORY IS WRONG
ABOUT x`. One is fixed by looking at a machine and the other by editing a file.

### 6.5 Quiet, or the journal is worthless

**A machine that already agrees produces no record at all**, and a pass in
which every machine agreed produces no record at all. A poller running every
minute against a fleet that is already right must be silent, or the one cycle
that mattered cannot be found in what it wrote. `ADR-022` §8's journal is a
rolling megabyte; a chatty poller is a journal that holds a few hours.

### 6.6 The write is a compare-and-swap, and that is not a workaround

There is no *write this file whole* operation. `apply edit --expect <a>
--replace <b>` refuses unless `<a>` occurs exactly once, so replacing a
document means stating what you believe is there. **If the document changed
between the read and the write, the machine refuses** rather than clobbering
somebody else's edit — which for a poller is exactly right: the drift check ran
against a document read seconds earlier, and this is what stops a change that
landed in between being silently undone.

The document is read back afterwards and compared. `rivet_remote::tls` does the
same and says why: one end of this channel escapes bytes it cannot read, and a
document that is *nearly* right is the one that gets left in place.

---

## 7. Ordering and failure across machines — `R-145.2`, argued and not built

**The ruling leaves this open, says the first increment does not need it, and
asks that this document argue it.** Nothing in §5's code implements any of what
follows; `converge::decide` answers about one machine and cannot be the place a
fleet-wide policy is smuggled in.

### 7.1 The requirement, restated precisely

*A fleet-wide change that bricks the first machine must not proceed to the
second.* The word doing the work is **bricks**, and a design that does not
define it observably is a design that cannot be built.

### 7.2 What a fleet server can actually observe

| what happened | can it be seen? | is it a brick? |
| :-- | :-- | :-- |
| the document was refused by the schema, on this server | yes, before anything is sent | no — nothing was written |
| the machine refused the edit | yes | no — the machine is unchanged |
| the write succeeded and read back | yes | no |
| **the machine answered before the write and is silent after it** | **yes** | **yes** |
| the machine is doing the wrong thing but still answers | no | not by this definition |
| the machine will fail at its next boot | no | not by this definition |

**The fourth row is the whole of the observable signal, and it is enough.** It
is cheap, it needs no new mechanism — it is a second `Machine::read`, which the
write already performs — and it is the failure that actually matters: *a machine
you cannot reach cannot be fixed.* The two concrete disasters `ADR-034` §6.2
measured on real hardware are both exactly this row: losing
`control-plane.listen` drops the listener to loopback, and losing
`services.rivet-mcpd.enabled` stops the control plane starting.

**The last two rows are out of scope and stay out.** A health endpoint, a soak,
a canary percentage, waiting out a `TakesEffect` reboot — each of those is a
rollout system, and R-145 is explicit that this is not one.

### 7.3 So the rule should be: serial, with a reachability gate

Converge one machine; read it again; if it answered before and does not answer
now, **stop the pass** and report which machines were not attempted.

A refusal does not stop the pass, and that asymmetry is deliberate. A refusal
wrote nothing, so one machine's refusal says nothing about the next machine's
document — and the first increment already behaves this way. A *brick* is
different: a fleet-wide change usually arrives as several files changed in one
commit, so the same mistake is very likely in the next file, and continuing
would spend the fleet one machine at a time.

The distinction the code would need is one it already draws — between
`Outcome::Refused` (nothing was written) and `Outcome::Unreachable` (could not
be reached). What it does not yet do is tell *was off before I started* from
*went silent because of me*, and that is one boolean: the machine answered the
read at the top of the pass.

### 7.4 The fleet server goes LAST, and `ADR-035` §5 is scoped rather than overturned

`ADR-035` §5 says **the server updates itself first**, and that is right for
what it is about: an **update payload**. A server that distributes an update
should have survived it, and the recovery if it has not is `ADR-018`'s A/B —
the server reboots into the other slot and comes back.

**For a document it is exactly backwards, and the reason is a difference in
kind rather than a difference of opinion.**

| | an update payload | a desired-state document |
| :-- | :-- | :-- |
| where it lands | the inactive slot | `/state/config/machine.toml` |
| what undoes a bad one | A/B: boot the other slot | **nothing** |
| so self-first is | prudent | unrecoverable |

`/state` is the partition an update never replaces. The machine says this
itself, in `apply edit`'s own refusal — *"ADR-018's A/B DOES NOT UNDO THIS:
/state is the partition an update never replaces, so a rollback leaves this same
document"*. A fleet server that applies its own bad document loses the ability
to apply anyone's, **including the correction**, and there is no slot to fall
back into. The recovery is a person at the machine.

There is a second, weaker reason pointing the same way: a fleet server
converging itself mid-pass may be restarted by its own change, and ordering it
last means every other machine's outcome is already recorded before that can
happen.

**So: payloads first-on-self, documents last-on-self.** `ADR-035` §5 is not
wrong and is not amended; it is about a different artifact and this states the
boundary.

### 7.5 The order among the others should stay git's, and the one open mechanism

Git lists a tree sorted, so the order is stable and a human can predict it. That
is enough, and inventing a rank field or a dependency graph would be inventing a
policy nobody has needed — `ADR-017` §3.2 declined exactly that for services and
the argument is the same one.

**The open mechanism is how a fleet server recognises itself for ordering.**
§7.4's rule needs the server to know which entry in the repository is its own
machine. Three candidates were recorded: match the kernel hostname against the
register's labels; give the fleet server's own record a flag; or have it ask
each machine `get status` and recognise the answer that describes itself.
**The second is what §5.5 reads** — the record provisioning writes with
`role = "fleet-server"` — to reach the server's own control plane. Whether
§7.4's ordering should rest on the same record, which a hand-edited register
could give to two machines or to none, is not decided, and ordering stays
unbuilt.

---

## 8. What this does NOT do, stated so it is not oversold

* **It is not a rollout system.** No staged rollouts, no canaries, no health
  gates, and — until §7 is built — no ordering.
* **It does not remove the need for the control plane.** It is a producer of
  document writes over it.
* **It does nothing for a machine that is off.** The document is the intent, and
  the gap between the two is what a viewer has to show.
* **It does not make the register redundant** — §3.
* **It does not stop `apply edit`**, and must not. §2.2.

---

## 9. Proving it, and why one half of the proof is worthless alone

Two proofs, both in the container.

`tools/qemu/adr-044-proof.sh` is **the rule**. Seven stages, one boot: an
aarch64 guest under `tools/qemu/run.sh` with `ADR-035`'s control plane and a
declared `[repos.machines]`, the guest's own document pushed into that
repository with a stock git over the control plane, and a host build of
`rivet-fleetd converge` driven against the pair. Its register names the guest as
`role = "fleet-server"`, so every repository operation is asked of the guest's
`rivet-mcpd` (§5.5).

| stage | what it settles |
| :-- | :-- |
| 1 | a verbatim copy is adopted, nothing is written, and nothing is said |
| 2 | a second pass with nothing changed is silent as well — §6.5 |
| 3 | a credential in a document is refused **on the fleet server** and the machine is never contacted — §2.3 |
| 4a | `--dry-run` names the change and touches nothing |
| **4** | **a commit pushed to a repository changed a machine**, and the machine's own `get desired-state` says so |
| 4b | and the cycle after it is silent again |
| **5** | **a push over out-of-band drift is REFUSED**, names both shapes of drift with both values, and the machine is byte-identical afterwards |
| 6 | the commit trailer naming that machine lets the push win — §6.3 |
| 7 | a machine that is switched off — a second register record where nothing listens — is reported and exits zero — §6.4 |

**Stage 5 is not decoration.** With the drift branch of `converge::decide`
deleted, stages 1 to 4 all still pass and stage 5 fails with *"the push over
drift was ACCEPTED. This is the guard missing."* That was watched before the
branch went back — `AGENTS.md` §4, a regression test that cannot fail proves
nothing.

Every assertion about the machine is a second read over the control plane rather
than the poller's own word for it. The refusal says NOTHING HAS BEEN WRITTEN;
what the proof believes is the document the machine hands back afterwards.

`tools/qemu/adr-044-fleet-code-proof.sh` is **the fleet server as it runs**: one
guest that is its own fleet server, carrying the vendored git and
`[repos.machines]`, with the real `rivet-fleetd` installed, enabled and started
by the supervisor under its Landlock ruleset, its certificates, operator
identity and one-record register staged as a card stages them.

| stage | what it settles |
| :-- | :-- |
| 1 | the supervisor's `ENFORCED` record names `rivet-fleetd`, and 4443 serves the operator |
| 2 | the guest's document, pushed verbatim, is adopted by the confined poller: the baseline, read with `get file`, names the pushed commit, and `rivet-mcpd`'s journal holds the head, listing and document it answered for the fleet server's own operator key |
| **3** | **`POST /api/commit/<device>` with the operator's certificate commits a renamed document**; the commit is `main`, a child of the pushed one, by `rivet-fleetd`; the poller converges the guest onto it, and the guest's own `get desired-state` carries the new hostname |
| 4 | the same POST again is refused as a stale document, and a commit sent to 7777 with a stale `expected-head` is refused `409 stale-head`; `main` does not move |
| 5 | another machine's front-door certificate is refused `403` at 4443, and a control-plane identity not in `peers` completes no request on 7777 |

### 9.1 What the proofs do not settle

`ADR-026` §4's right-hand column, entirely, and two of their own:

* **One guest is the fleet server and the machine.** The confined proof
  converges the fleet server itself; a second machine converged by a confined
  server, over a network, is not exercised. The host-build proof covers the
  rule's other branches against the same guest.
* **The repository's size is a handful of commits**, pushed and read over
  QEMU's loopback forwarding.

---

## 10. Consequences

* `rivet-core` gains two modules with no dependencies and a rule that is now
  unit-testable without a machine.
* `cmd/rivet-fleet`'s `apply document` refusal and the fleet server's drift
  refusal are one comparison. A defect found in one is fixed for both.
* `cmd/rivet-fleetd` carries `toml_edit` — §5.3.
* `rivet-fleetd` gains a second invocation, `converge [--dry-run]`, which needs
  no listening socket.
* **The poller is coupled to the browser side, and that is stated rather than
  hidden.** The daemon exits before it starts polling if the supervisor handed
  it no socket, or if its server certificate is missing — so a fleet server
  whose HTTPS side will not come up does not converge anybody either. It is one
  program with one purpose and that is defensible; it is written down because a
  fleet that stopped converging for a reason about a certificate would be a
  puzzling silence. `converge` from a command line is unaffected and is the way
  round it.
* `rivet_core::operation::DESIRED_STATE` is exported, and `rivet-remote`'s
  literal copy of that word reads it.
* **A fleet server's poller depends on its own control plane.** `rivet-fleetd`
  asks this machine's `rivet-mcpd` for every repository operation, over TLS,
  with its operator identity, so that key must be in the server's own `peers`
  (provisioning writes it) and the register must hold exactly one record with
  `role = "fleet-server"`. A server whose control plane is down converges
  nobody, which it could not have done anyway: the same control plane is how it
  reaches itself.
* `cmd/rivet-mcpd` gains four routes that write to a repository as root, bounded
  to one directory of one declared repository (§5.5).
* A fleet repository is a thing that exists and has a shape,
  `examples/fleet/README.md`.

---

## 11. What is still unproven, stated plainly

* **No board has run this.** Both proofs are QEMU. Nothing here has converged
  `rivet-01`, and `rivet-01` is the maintainer's dashboard. What closes it: a
  fleet server provisioned with a declared repository and `fleet-repository`
  set, a machine's document pushed into it, a Save in the portal, and the
  machine's own `get desired-state` read afterwards.
* **A confined server has converged only itself.** A second machine reached
  over a real network by a confined poller, and the drift, override and
  unreachable branches under confinement, are the host-build proof's and not
  the confined one's.
* **The poller loop has run in a guest for minutes**, not for days against a
  journal with a megabyte limit (§6.5).
* **§7 is an argument and not a mechanism.** Ordering and the reachability gate
  are open; §5.5 recognises the server's own record only to reach its own
  control plane.

---

## History

### §11 and §5.5 — the git ran in `rivet-fleetd`, and Landlock refused it

Until `.rulings/` R-187 D2, `rivet-fleetd` cloned `fleet-repository` as a
remote into its data directory with `rivet_core::git::BINARY`, fetched it every
cycle, and committed through a throwaway `git worktree` before pushing. §11
recorded why that could not run as built:

> **Known defect: Landlock refuses the git this program runs.**
> `rivet_core::confinement::policy` grants `rivet-fleetd` no execute right
> beneath `/system/bin`, so a confined fleet server cannot run
> `rivet_core::git::BINARY` — and the poller and `POST /api/commit/<device>`
> both need it. `cmd/rivet-sysd/src/landlock.rs`'s
> `rivet_fleetd_confined_by_its_own_policy_can_run_git` builds that policy's
> ruleset in the container and execs git: unconfined it runs, confined
> `execve` answers `EACCES`, with the binary at `/system/bin/git` as on a
> machine. The test is `#[ignore]`d and fails when run.

The two ways out were widening the application sandbox to give an application
a system program, or moving the git to the program that already runs git as
root. R-187 D2 chose the second, so the repository became one the fleet server
declares and serves, `fleet-repository` became its name rather than a remote,
and §5.5 is the result. The probe was inverted rather than deleted: it now
asserts the refusal, so a widened policy fails it.
