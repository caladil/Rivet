# ADR-034: The Fleet Register

* **Status: Accepted, 2026-08-26.** §10 and §11 ruled
  **all eight** of §8's questions across two further rounds the same day; §8 is
  kept as the record of what was asked, with each item's answer named against
  it. It was *accepted in part* for the earlier half of that day, while §8's
  questions were still the maintainer's.
* **Amended 2026-09-01, and two of its decisions are SUPERSEDED.** Not
  corrected — *decided differently, later, by the maintainer*, which is a
  different thing and the record has to be able to tell them apart:
  * §2.2's **Local mode is dropped** and Remote is no longer the escalation.
    See §2.2.1.
  * §8.1's strike of `rivet-fleetd` is **overtaken by the thing being built**.
    See §8.2.
  §2's own decision — the register is the authority, a fleet server is a way of
  reaching it — is untouched, and building both adapters is what confirmed it.
* **Date:** 2026-08-26
* **Extended by:** `ADR-061` §5 — the fleet server's register holds a second
  kind of record, a paired phone, as `phones/<identity>.toml` beside
  `devices/`. Phones exist only in the fleet server's copy; the workstation's
  register holds none.
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Answers:** `ADR-025` §3's *"the fleet tool"*, which has been a named actor
  with no implementation since 2026-08-22.
* **Gives a home to:** `ADR-007` §3.4's **fleet layer**, which has existed in
  the layering table and nowhere else since the same day.

---

## 1. Context

`ADR-019` ruled that enrollment happens **before first boot**, and `ADR-025` §3
ruled who performs it:

> the **fleet tool** generates the pair, writes it into the provisioning file,
> and **retains only the public key.**

**That tool does not exist.** Its work is done by hand today: a provisioning
document is edited in a text editor outside the repository, a public key is
pasted into it, and the record of which machine holds which credential is in
the maintainer's memory. On a fleet of one this is invisible. `PLAN.md` §9 names
the machines Rivet is meant to replace, and on that fleet it is not invisible.

Four separate things are hand-managed today, and all four are one job:

| Hand-managed today | Ruled to belong somewhere | Where it actually is |
| :-- | :-- | :-- |
| the provisioning document | `ADR-025` §3, "the fleet tool" | a TOML file edited by hand |
| which machine trusts which signing key | `ADR-028` §5.5 | flags on the image build |
| which peer keys a machine's control plane accepts | `ADR-025` §2 | pasted into the provisioning document |
| which services a machine runs | `ADR-007` §3.4 layer 3 | nowhere — every machine is whatever its image was |

This ADR names the thing that owns all four, rules the shape of the two ways of
reaching it, and rules exactly what it may and may not hold.

---

## 2. Decision: the register is the authority; a fleet server is a way of reaching it

**The port is the *fleet register* — the store — and not the *fleet server*.**
`rivet-fleetd` and a workstation application are two **adapters over one
register**, not two programs that must agree.

### 2.1 Why the distinction is load-bearing rather than pedantic

The tempting framing is *"a fleet server could be a daemon, or it could be an
application run as needed."* That is a **deployment** question and it is the
easy half. The question that decides the design is:

> **What is authoritative, and who may write it?**

If a workstation tool and a `rivet-fleetd` may both mutate fleet state, there
are two sources of authority for one fact — which is what `ADR-011` exists to
forbid, and the failure is the one `ADR-011` §3 names: two answers to *"which
key does `kitchen-pi` trust"*, with no vocabulary to describe the disagreement.

Naming the register as the port dissolves it. There is one register. There are
two ways to reach it.

### 2.2 The two modes

| Mode | The register is | Reached by | For |
| :-- | :-- | :-- | :-- |
| **Remote** | a directory owned by `rivet-fleetd` on a Rivet machine | **`ADR-035`'s control plane, and HTTPS for a browser** | **everyone** |

**There was a second mode, Local, and Remote was the escalation.** §2.2.1
records why it went.

**Remote mode is reached the way everything else is** — `rivet-mcpd` over TLS
with pinned Ed25519 keys, plus HTTPS with a client certificate for the browser
half, because a browser cannot pin (`ADR-025` §2, `ADR-035`, and
`cmd/rivet-fleetd`'s own module note).

### 2.2.1 SUPERSEDED 2026-09-01: there is one mode

**Local mode is dropped**, and this is not a correction of an error — it is a
later decision. The maintainer ruled on 2026-09-01 that Local mode goes because
it no longer makes sense, and that the provisioning tool is the only
command-line tool left for a human. *(`.rulings/` R-279.)*

The argument for Local was that *"a service whose only client is the person
sitting at the machine is a component with no job."* **That argument was sound
and its premise stopped holding.** The fleet server's clients are now a browser,
an agent, and `rivet-provision` — three, none of which is a person at a
terminal — and the register has to be somewhere all three can reach. A directory
on one workstation is not that place.

#### What follows from having one mode

* **The register lives on a fleet server**, in `rivet-fleetd`'s own application
  data directory. Built 2026-09-01.
* **§2.3's "no sync" stops being a rule about the escalation and becomes the
  rule.** The workstation is a client, always. There is no second writer and
  nothing to reconcile, which is the same protection §2.3 was already giving —
  now with no configuration in which it does not apply.
* **`rivetos-register` is a library, not a module of the workstation tool.** It
  was `cmd/rivet-fleet/src/register.rs` and moved on the day a second consumer
  appeared, exactly as `rivet_core::identifier` moved out of that crate on
  2026-08-26.
* **The durability argument in that module's I/O half no longer holds.** It
  justified whole-file, non-atomic writes on the grounds that *"a register lives
  on a workstation, is version-controlled by the operator"*. On `/state` on a
  machine that can lose power, neither clause is true.
  **CLOSED 2026-09-01 — it borrows.** `rivet_store::atomic::commit` became
  public for a third caller, on the argument its own module already makes: the
  sequence is four calls whose *order* is the guarantee, so a copy is a place
  for a step to go missing with nothing failing until the power is pulled.
  `rivet-register` writes through it on Unix, which is where the register
  lives, and falls back to a plain write on Windows **saying so** — because
  `rivet_store::StdFilesystem` is `cfg(unix)` on purpose and a faked `sync_dir`
  is the one lie only a power cut discovers. The maintainer's follow-on
  question — whether an *application* should be able to do this — is `ADR-036`
  §8.5 and is open.

**§3 is untouched by any of this.** The register still holds no signing key and
no device private key, and §3.4's payoff — committable, reviewable,
agent-readable — is what let it move onto a machine at all.

The operator's machine is **Windows 11, Linux or macOS**. `ADR-002` already
makes the development host Windows, and the register must not be the component
that forces a Linux workstation on somebody. This is the first Rivet component
with a genuine three-platform requirement, and §4.3 records what it costs.

### 2.3 What this forecloses, stated so it is not re-litigated

* **Two writers.** In Remote mode the workstation application is a *client*. It
  does not open the register's files, and **there is no "sync"** — a sync is two
  authorities plus a reconciliation policy, which is `ADR-011` §3 wearing a hat.
* **A register that is discovered rather than named.** Which register a tool
  talks to is configuration the operator sets, not something inferred from the
  network. A tool that found a fleet server by broadcast is a tool that can be
  answered by somebody else's.

---

## 3. Decision: the register holds no signing key and no device private key

**The register holds public halves, device records, policy and history.** It
never holds a signing seed, and it never holds a device private key at rest.
**There is no exception to this and §7 is not one** — §7 is about a different
category of secret that this decision never covered.

This was asked as a question and it survives being pushed on.

### 3.1 It is what `ADR-015` §4 already says

The signing key never leaves the build machine. In **Remote** mode the register
is explicitly *not* the build machine — that is what the mode is for — so a
register that held the seed would move it, by design, the first day anybody used
the feature.

### 3.2 The deployment symmetry breaks on this and on nothing else

Everything in §2.2 is symmetric between Local and Remote **except private key
material**, and there the asymmetry is severe rather than awkward:

* `ADR-025` §5.2 already rules that **a stolen Pi is a compromised identity**.
* If the system signing seed lived in the register and the register were
  deployed in Remote mode, a stolen Pi would be a **compromised fleet** — the
  thief could sign a system update that every machine in `keys/` installs.

A Raspberry Pi on a shelf is the least physically defensible machine in the
estate. It is the right place for a register of public facts and the worst
available place for the key that signs system updates.

### 3.3 The blast radii must not share a failure domain — the decisive argument

| Loss | Consequence | Recoverable? |
| :-- | :-- | :-- |
| **The register** | the bookkeeping is gone | **yes** — `ADR-028` §5.5.6 makes a machine able to say what it trusts and `ADR-032`'s query socket is being built to ask it. A register can be rebuilt by asking the fleet. |
| **The signing seed, lost** | every card ever built refuses every future update, permanently, and cannot be repaired over the wire (`ADR-028` §5.5.5) | **no** |
| **The signing seed, leaked** | anyone may sign a system update for every machine that trusts it | **no** |

One of these is an inconvenience and two are terminal. **Putting them in one
container converts the recoverable failure into an unrecoverable one for free**,
and buys nothing but one fewer path to remember.

### 3.4 Being secret-free is what makes the register useful

A register of public facts can be put in `git`, reviewed as a diff, synced
between machines and read by an agent. A register holding a 32-byte seed could
be none of those — `.gitignore` refuses `*.seed` as a backstop and `AGENTS.md`
§8.3 binds history rather than the working tree.

**Its security requirement is integrity, not confidentiality.** It must be hard
to change without anyone noticing; it does not need to be hard to read.

§7's credential store is confined to a separate file for exactly this reason.

### 3.5 The one place private key material is touched, and it is transient

`ADR-025` §3 requires the fleet tool to **generate** a device keypair and write
the private half into the provisioning document. The tool therefore handles a
private key. The distinction has to be implemented rather than assumed:

```
the tool GENERATES a device key   -> the private half goes into the provisioning document
the tool RETAINS the public half  -> the register records the public half and the key id
the private half exists           -> in memory, then on the card, then blanked at first
                                     boot by ADR-019 section 3.1
```

**The register is never written with the private half at any point in that
sequence.** An implementation that writes it "temporarily" and removes it later
has failed this decision, because a deleted file is not an absent one.

`ADR-025` §3's alternative — device-generated keys with a one-time registration
token — removes even the transient handling, and remains available and unbuilt.

### 3.6 The keys a device ACCEPTS cost nothing here, because they are already public

The request that the register manage the peer keys needs no exception: the
entries in a device's `control_plane_keys` **are public keys**. The register
owning the set and rendering the right subset into each device's provisioning
document involves no secret at all.

The **operator's own** private half stays where it is, in the operator's
identity directory. `PLAN.md` §19.1 ruled this for `cargo rivet deploy` and the
reasoning transfers unchanged: which key an operator presents, and from where,
stays theirs.

The register records **which public keys a device accepts**. It has no opinion
about which private key the operator presents, and no access to it.

---

## 4. Decision: two surfaces, one operation set

**The register is administered through a full-screen, human-oriented
application *and* through a command-line surface, and these are two renderings
of one set of operations.**

Ruled 2026-08-26. The maintainer's framing, which is the ruling: the human
interface should be **human-oriented rather than a CLI**, and a command-line
surface is wanted **as well**, because `ADR-012` makes an agent a first-class
operator and an agent wants a command line.

### 4.1 The failure this prevents

"A TUI, and also a CLI" is two implementations of one thing — `ADR-011` §3
again. They drift, and the drift is discovered by a human and an agent
disagreeing about what was configured.

**So the ruling is not "build both". It is:**

> **Every keystroke in the human interface resolves to a named operation, and
> there is no operation the human interface can perform that the command-line
> surface cannot.**

The precedent is in the project twice already. `ADR-007` §3.6.2 makes `/etc` a
generated read-only **view** rather than a second place configuration lives, and
`cmd/rivet` already renders one operation set as JSON or as text with neither
being the real one.

### 4.2 The human interface shows its work

Because every action is a named operation, the application can **display the
command it is about to run** — and that is worth building deliberately rather
than as a debugging affordance:

* it teaches the command-line surface to a human who did not set out to learn
  it;
* it is an audit surface — what was done is expressible in one line, which is
  the line `ADR-028` §4.5's attribution already wants;
* it is where `ADR-012`'s agent operator and a human operator are demonstrably
  doing the same thing rather than two similar things.

### 4.3 Why `ADR-005` §5.5's rejection of `crossterm` does not bind here

**Recorded because §5.5 will otherwise be cited at this decision, and it does
not apply.** §5.5 rejected `crossterm` for `femto`, and its reasoning is
explicitly scoped:

> `signal-hook` installs signal handlers **in a binary that ships in the base
> image** of a system where signal disposition is PID 1's and the supervisor's

The register's application **does not ship in the image**. It runs on the
operator's Windows, Linux or macOS workstation, where there is no `ADR-018` §4
slot budget, no PID 1 whose signal disposition can be trampled, and no
`ADR-001` §4.1 applet scoreboard to disturb. `ADR-001`'s austerity is a rule
about **what reaches a device**; a workstation tool is on the other side of that
line — the same side `cargo rivet` is already on.

A terminal library is also the only practical way to meet §2.2's three-platform
requirement without writing a Windows console backend, which would be
`ADR-009` §3.1's reinvention with none of its justification.

### 4.4 The shape is `raspi-config`, not a dashboard

Ruled 2026-08-26: full-screen and menu-driven, task-oriented, aimed at a human
who wants to accomplish something rather than one who wants to watch numbers.
A dashboard is a different product, and `ADR-032` is the design for asking a
machine what it is doing.

### 4.5 BUILT 2026-09-01: the operator's certificate is filed, and the authority is not

The browser half of §2.2's Remote mode needs two things in the operator's
computer, and they are **not** the same kind of act:

| | What it changes | Who does it |
| :-- | :-- | :-- |
| **The authority**, into Trusted Root | what this computer **believes** | the person, deliberately |
| **The operator's identity**, into Personal | files something its owner **already has** | `rivet-provision` |

**The maintainer drew that line and it is kept as a rule, not a convenience:**
the root is a step a person can reasonably be asked to take by opening the file,
and the browser identity is not, because it has to go in one particular store
and nothing tells the person to go and change it. *(`.rulings/` R-280.)*

He is right about both. A person can be asked to install a root; they cannot be
expected to know that a client identity belongs in **Personal, under the current
user** rather than the machine — that is the one choice Windows puts in front of
them and the one they have no way to get right. Getting it wrong fails in the
worst way available: the certificate exists, and the browser never offers it.

**So `rivet-provision` never writes to a trust store.** That is a property worth
stating rather than an accident of scope: a tool that quietly installed a
certificate authority would be changing what its operator's computer trusts, and
the whole reason §3.3 keeps blast radii apart is that some acts deserve to be
deliberate.

#### What it does

`rivet-provision ca` now writes `browser.pfx` itself — `p12`, a pure-Rust
PKCS#12 encoder — and files it with `certutil -user -importpfx My`. **This
removes OpenSSL from the setup path entirely.** Before it, the operator had to
install OpenSSL and run `openssl pkcs12 -export`, inventing a password to
satisfy a tool he had not asked for.

The bundle carries **the leaf and the key, and no authority**. A PKCS#12 may
carry its chain, and including it would push the root into whichever store the
identity lands in — which is how a trust store quietly acquires a CA nobody
decided to install.

The file is **kept** after import. Windows imports the key as non-exportable, so
the bundle is the only copy that can travel to a second computer, and an
operator with two machines is the ordinary case.

A password is prompted for, without echo, and an empty one is accepted. The
maintainer's permission was explicit: a password prompt from `rivet-provision`
at any point is acceptable to him *(`.rulings/` R-280)* — and echo is
suppressed through three `SetConsoleMode` calls rather than a crate, because
this binary holds `root.key` and `ADR-009` §5's gate is what that section is
for. **A console whose echo cannot be silenced is one this says so about**
rather than reading a secret while the person believes it is hidden.

#### Windows only, and the other two are NAMED

`ADR-002` makes the development host Windows and §2.2 records that the
operator's machine may be *"Windows 11, Linux or macOS"*. **Only Windows is
built**, by the maintainer's instruction to build the Windows path and document
the other two *(`.rulings/` R-280)* — and the others report themselves unbuilt
rather than silently doing nothing:

| Platform | What it would take | State |
| :-- | :-- | :-- |
| **Windows** | `certutil -user -importpfx My` | **built**, verified 2026-09-01 |
| **macOS** | `security import browser.pfx -k login.keychain` | one command; unbuilt |
| **Linux** | **there is no single store** | unbuilt, and the reason is below |

Linux is the one that keeps this from being three easy commands. Chrome and
Chromium read the NSS database at `~/.pki/nssdb`; **Firefox keeps its own, per
profile**, on every platform including Windows; and neither is the system trust
store `update-ca-certificates` manages. A tool that installed into one of the
three and reported success would be right about a third of the time, which is
worse than a tool that says it does not do this yet.

#### Verified, and by what method

Run against a throwaway authority on 2026-09-01: the certificate appeared in
`Cert:\CurrentUser\My` with `HasPrivateKey: True`, **no root was installed**
(checked, and the count was zero), and the operator's real identity was
untouched. The test certificate was then removed. What is **not** verified by
that: no browser has yet been offered a certificate this path installed —
`rivet-01`'s working identity predates it and was imported by hand.

---

## 5. Decision: turning services on and off is the register's everyday operation

Ruled 2026-08-26, when the services then scheduled were `rivet-mdnsd`,
`rivet-proxyd`, a git server and `rivet-smbd`, with mdnsd **starting disabled**.
DHCP arrives on the same axis. (Git is no longer a service of its own: it is
smart HTTP inside `rivet-mcpd`, `ADR-035` §4.)

**Whether a machine runs a service is fleet configuration, not an image
property.** The alternative — that each machine is whatever its image was — is
how it works today, and it means enabling a service requires building and
installing a system update, which is absurd for a boolean.

This matters more than it sounds, because it is what stops the service family
from turning into a family of images:

| Without the register | With it |
| :-- | :-- |
| a machine that should run `proxyd` needs an image built with `proxyd` enabled | one image; the register says which machines run it |
| turning mdnsd on is a 683 MB system update | it is a configuration change |
| "which machines run SMB" is answered by remembering | it is a query |

This is `ADR-007` §3.4 layer 3 doing the job it was defined for, and it is the
operation an operator will perform most often.

### 5.1 The first case is provisioning, and it needs no running machine at all

**Clarified by the maintainer 2026-08-26**, correcting an earlier draft of this
section that treated the live case as the primary one. **The project will be
provisioning systems on which these services need to be enabled, disabled or
configured — settings fixed at the provisioning point, which is what implicates
the fleet register.** *(`.rulings/` R-050.)*

A service's enabled state and its configuration are **fields of the provisioning
document** — §7 of this ADR's framing, applied: the register's first message to
a machine it cannot yet reach. `ADR-004` §6.1 already carries network
configuration this way; services are the same mechanism a third time, and it is
the mechanism `ADR-004` §6 keeps calling *"the same mechanism again"*.

**Nothing about this needs a signal, a socket or a running machine.** A board
that has never been powered on has no process to notify. This is the common
case and it is available as soon as the register and the document schema exist.

### 5.2 For a running machine, "requires a reboot" is an accepted answer

**Ruled by the maintainer 2026-08-26: where applying a change requires a reboot,
so be it — this environment may perfectly well have settings that can be
switched on and off and that need a reboot to take full effect.**
*(`.rulings/` R-051.)*

**`ADR-032` §3.2's `SIGHUP` is therefore not a prerequisite for §5** — it is an
optimisation that removes a reboot, and it can arrive whenever `PLAN.md` §24.2
schedules it. A change that lands in the persisted document and takes effect at
the next boot is a complete implementation of this decision.

That is also the honest posture for a system with `ADR-018` §4's A/B slots: a
machine that reboots safely and rolls back automatically is one where requiring
a reboot is a small cost rather than an outage.

### 5.3 What a reboot-required setting must do, and this is not optional

The one hazard, and the project has already been bitten by its shape.
`PLAN.md` §0.0.4 names the state that produced the A/B rollback evidence: a
configuration that is wrong in a way that is **indistinguishable from success
until the next boot**.

So a setting that does not take effect immediately must **say so at the moment
it is changed**. An operator who disables `mdnsd`, sees a success, and finds the
responder still answering has been told something false by omission.

**`requires-reboot` is a property of the setting, in the schema** — not a note
in a message and not something the operator is expected to remember. That makes
it available to three consumers at once:

* `apply` states it in the plan, which is `ADR-006` §6.5's typed diff
  (`PLAN.md` §24.2 item 1) carrying one more field;
* the register's human interface shows it before the operator commits (§4.2);
* an agent can branch on it structurally rather than parsing prose, which is
  `ADR-032` §4.3's whole requirement.

This is cheap, it composes with work already scheduled, and it is what makes
§5.2's answer honest rather than merely acceptable.

### 5.3.1 BUILT 2026-08-26, as three states rather than a `bool`

`PLAN.md` §28.5 Phase 1 item 3, alongside `ADR-006` §6.5's typed diff — which is
where §5.3 said it would land, *"carrying one more field"*.

**One type, `rivet_core::operation::plan::TakesEffect`, for all three consumers
this section named.** `Immediately`, `OnNextRestart`, or `NotAsserted { because }`.
A second spelling for the schema and the plan would have been `ADR-011` §3 with
the two halves a crate boundary apart, and an agent would have had to know both.

**Why three states.** **Three of `apply`'s SEVENTEEN subjects** — a count
corrected twice on 2026-08-29, the second time when `ADR-017` §6.3.2's
`service-restart` landed — cannot answer honestly —
`apply edit` writes a path the operator chose and this build does not know what
reads it, `apply install` has deliberately not opened the payload, and
`apply create-file` joined them on 2026-08-29. A `bool`
would have forced each to claim *live* or *needs a restart*, which is exactly the
omission this section exists to stop, pointing the other way. `requires_reboot()`
therefore answers `Option<bool>`: a caller cannot read *not asserted* as *no*.
`ADR-006` §6.5.2 carries the argument in full.

**Where each consumer reads it:**

| Consumer | How |
| :-- | :-- |
| `apply`'s plan | a field of `Plan`, rendered as `takes-effect` and `requires-reboot` in the refusal's structured payload |
| the register's interface (§4.2) | **Built 2026-08-26 and run on a real terminal 2026-08-27 (§9, `PLAN.md` §55). It does NOT read this field**: nothing under `cmd/rivet-fleet/src/` mentions `takes_effect`, `TakesEffect` or `requires_reboot`. The row said *not built — Phase 3* until 2026-08-29: its conclusion was right and its reason was wrong, which is worse, because it reads as scheduled work rather than as a gap |
| an agent | both, and **the two spell it identically**, which was the point |

**In the schema**, per §5.3's *"a property of the setting"*:
`rivet_config::schema::Property` carries a `TakesEffect`, and the emitted JSON
Schema puts `takes-effect` and `requires-reboot` on every property as extension
keywords.

**This section said every setting in the build answered `on-next-restart`, and
that has been FALSE since 2026-08-26 — corrected 2026-08-29. The way it survived
is worth more than the correction.** `SIGHUP` landed, the guard test
`every_setting_publishes_when_a_change_to_it_takes_effect` fired exactly as the
paragraph had predicted it would, and **`rivet-config` struck its own copy of
the sentence in place, in the house style, on the day**
(`rivet-config/src/schema.rs`, `Property::takes_effect`'s doc comment). The
ADR's copy was left. That is `AGENTS.md` §7's named pattern — *"fixing the
quoted sentence and leaving its three siblings"* — with the sibling one file
away, and it is the clearest case in this audit of the record and the code
disagreeing while both were maintained carefully.

**What is true now:** `services.*.enabled` is `TakesEffect::NotAsserted`,
because a reload applies it to most services immediately and **withholds it
for `rivet-mcpd` and `rivet-netd`** — a running machine may not stop the two
things that are the means of undoing the change (`ADR-032` §3.2.1). So the
answer depends on which service the key sits under, and this schema
deliberately does not know service names. Every *other* setting still answers
`on-next-restart`, and the three readers named above are still the three.

---

## 6. What the register contains

Ruled in outline; the on-disk shape is §8's question.

| | Holds | From |
| :-- | :-- | :-- |
| **Logical device** | opaque stable identifier, human label, role, location | `ADR-025` §4, §4.1 |
| **Current credential** | the device's **public** key and its key id | `ADR-025` §4 |
| **Trust** | which signing key ids this device trusts, and for which kinds | `ADR-028` §5.5 |
| **Access** | the peer keys this device's control plane accepts | `ADR-025` §2, §3.6 |
| **Network** | addressing per interface, the resolver (`nameservers`, `search`, as its own field — never an interface), and the wireless network to join | `ADR-004` §6.1 |
| **Services** | which services this machine runs | §5 |
| **Fleet configuration** | values shared across machines | `ADR-007` §3.4 layer 3 |
| **History** | what was provisioned or installed, when, under which key id | `ADR-028` §4.5 |
| **Credential store** | §7 — **a separate file**, and the only secret here | `ADR-020` |

### 6.1 This is `ADR-007` §3.4's fleet layer, given a home

`ADR-007` §3.4 has carried a four-layer table since 2026-08-22 in which **layer
3 is "Fleet — shared across the operator's machines — set by the tools"**, and
it records plainly that layers 2 and 3 do not exist and nothing merges layers.

**The register is where layer 3 lives.** This ADR introduces no new concept; it
gives an existing one the store it always implied. It also means `ADR-007`
§3.4's deferred precedence engine is a prerequisite for the fleet layer being
*effective* and **not** for the register being *useful* — a register that only
composes provisioning documents is worth building before any merging exists.

---

## 7. Decision: shared network credentials are a different category, and the register holds them

**Ruled 2026-08-26 by the maintainer: the register may remember the wireless
credential for the purpose of provisioning systems.**

### 7.0 It is not an exception to §3, and calling it one is the error

**The maintainer's correction, 2026-08-26, and it is the reason this section is
shaped the way it is: a Wi-Fi password and a private key are FUNDAMENTALLY
different things.** *(`.rulings/` R-052.)*

An earlier draft of this ADR wrote §7 as a *bounded exception* to §3. That
framing is wrong, and wrong in a way that would have caused real damage — it
implies the two are the same kind of secret with one being tolerated, which
invites the next request to argue for tolerance too. They are not the same kind
of secret. §3 is a rule about **private keys** and it never reached this at all.

| | **Private key** | **Shared network credential** |
| :-- | :-- | :-- |
| What possession grants | **authority** — the right to act, sign, or be someone | **access** — the right to join a medium |
| Cryptographic shape | asymmetric; the private half is the identity | symmetric; a shared secret by construction |
| How many copies should exist | **exactly one**, and that is the security property | **one per participant**, and that is how it works at all |
| Recoverable by the owner | **no** — a lost seed is lost | **yes** — read it off the router, or change it |
| Blast radius if leaked | everything that trusts it, indefinitely | one LAN, until the operator changes it |
| Revocation | remove the public half from every device (`ADR-025` §5) | change the passphrase once |

A secret whose correct number of copies is *one* and a secret whose correct
number of copies is *every device on the network* are not two points on a scale.
**§3 governs the first category. §7 governs the second. Neither is a carve-out
from the other.**

The practical consequence: a future request to put some *other* secret in the
register is decided by asking **which category it is in**, not by asking whether
it is "important enough to be an exception". §7.3 makes that checkable.

### 7.1 What is stored is the PMK, not the passphrase

`ADR-004` §5 already rules that **the plaintext passphrase is never written to
persistent storage** and that Rivet performs the PBKDF2 derivation itself,
keeping only the 32-byte PMK. **That rule extends to the register unchanged.**

The derivation happens **once, in the register, when the human types the
passphrase**, rather than once per device at first boot. The PMK is per-SSID and
not per-device, so one derivation provisions the whole fleet.

**Why this is a real reduction and not a fig leaf.** Possessing the PMK is
sufficient to join the network, so this buys nothing against an attacker who
wants your LAN. It buys something specific and worth having: **a human's WiFi
passphrase is frequently a passphrase they have used elsewhere**, and the PMK
is not reversible to it. The thing that is never persisted is the thing whose
loss reaches beyond this network.

### 7.2 It also removes a window that exists today

`ADR-004` §6's sequence writes the **plaintext passphrase** into `rivet.toml` on
the FAT boot partition, derives at first boot, and then blanks it. A register
that derives first hands the provisioning document a PMK, so:

* plaintext never reaches the card at all;
* the blanking step in `ADR-004` §6 becomes unnecessary **for documents the
  register composed** — and must remain for hand-written ones, which stay
  supported.

**This is a security improvement over the hand-written path, not a concession
against it.**

### 7.3 The test, in checkable form

§7.0 gives the categories; this is how to sort a new secret into one without
arguing about it. Ask the questions in order — the first is decisive and the
second is a cross-check that catches a mis-sort:

> 1. **Does possession grant authority, or access?** Authority → §3, refused.
> 2. **Does a provisioned device already hold this secret?** If a device holds
>    it, the register holding it creates **no new class of exposure**. If no
>    device ever holds it, the register holding it creates one.

The two agree, and that is the point: a credential that grants authority is
precisely one Rivet works to keep *off* devices.

| Secret | Grants | Does a device hold it? | In the register? |
| :-- | :-- | :-- | :-- |
| WiFi PMK | **access** to one LAN | **yes** — every wireless machine has it in `/state/secrets` | **yes**, §7 |
| device private key | **authority** to be that device | no — blanked at first boot (`ADR-019` §3.1) | **no**, §3.5 |
| system signing seed | **authority** over every machine that trusts it | **no, ever** — that is what `ADR-015` §4 is | **no**, §3 |
| application signing seed | **authority** to ship software | no | **no**, §3 |

The signing seed fails both questions in the strongest available way: it grants
authority over the whole fleet, and **no device has ever held it — its never
being on a device is the entire security property.**

The WiFi PMK passes both, and the second is worth stating concretely: a stolen
Pi already yields the PMK today, by `ADR-025` §5.2. **Remote mode therefore adds
no new exposure for §7's secret, and would add a categorical one for §3's** —
which is §3.2's asymmetry, arrived at from the other direction.

### 7.4 It lives in a separate file, so §3.4 survives

The credential store is **not** part of the register's public records. It is a
separate file, obfuscated at rest (**§10.4 supersedes** — it is NOT encrypted and must not be called so), excluded from version control, and a `Secret`
in `ADR-020`'s sense — so it cannot be rendered into `plan`, `export` or a log.

**The public register stays committable, reviewable and agent-readable**, which
§3.4 argues is most of its value. One tool, two stores, one of which is a
secret and is treated as one.

### 7.5 What is deliberately not decided here

How the credential store is encrypted, and by what key, is **open** and is
listed in §8. `ADR-020` is the precedent on a device; a workstation has a
different threat model and different facilities (a keychain, DPAPI, a
passphrase), and choosing among them across three platforms is its own
decision.

---

## 8. ANSWERED — the maintainer's questions, listed so nobody answers one by accident

**All eight were ruled on 2026-08-26**, in §10 (items 1–4 and 6) and §11 (items
5, 7 and 8). The list is kept because what was asked is part of the record, and
each item now names where it was answered. **Nothing below is open.**

1. **What the application is called, and whether it is one binary or two.**
   `cargo rivet` administers a *crate*; `rivet` administers a *machine*; a third
   name administers a *fleet*. Whether the human surface and the command-line
   surface are one binary with two entry points, or two binaries over one
   library, is undecided.
   **ANSWERED — §10.1: one binary, `rivet-fleet`. §11.1 completed it — `clap`, `ADR-006` verbs.**
2. **The on-disk format of the public register.** TOML is the project's document
   format everywhere else (`ADR-007`), and one file per device is the obvious
   git-friendly shape. Not ruled.
   **ANSWERED — §10.2: TOML, one file per device named by the opaque identifier.**
3. **How the §7 credential store is encrypted**, across Windows, Linux and
   macOS. See §7.5.
   **ANSWERED — §10.4: it is NOT encrypted — obfuscated, restrictively permissioned, and named honestly.**
4. **Whether the register is version-controlled by the tool or by the
   operator.** §3.4 makes it committable; it does not rule that the tool runs
   `git`.
   **ANSWERED — §10.3: by the operator. Ignition decides where it lives; the tool writes files that are good to commit.**
5. **Whether `rivet-fleetd` is a base-image service (`ADR-017`) or an
   application (`ADR-015`).** This matters more than it looks: an application is
   installed and updated through the path `PLAN.md` §16 proved; a base-image
   service is not.
   **ANSWERED — §10.3: neither — `rivet-fleet` is an IGNITION APPLICATION, `cargo install`ed on a workstation.**
6. **How a tool learns which register to use.** §2.3 rules it is named rather
   than discovered; it does not say where the name is written.
   **ANSWERED — §10.3: Ignition's configuration, at Ignition's platform-correct path.**
7. **Whether the terminal-interface layer belongs in Ignition.** `ADR-013`
   §2.1's placement test requires naming a **second existing project** that
   wants it. Olympus is an orchestration console and is the plausible
   candidate — a claim to check rather than assert. Until it is checked the code
   belongs in Rivet.
   **ANSWERED — §11.3: not yet — `ratatui` is adopted and the layer stays in Rivet until `ADR-013` §2.1's test is actually run against Olympus.**
8. **Whether the register ever writes to a machine directly**, or only ever
   composes documents and hands the work to `cargo rivet deploy`. The second is
   smaller and is the recommendation.
   **ANSWERED — §11.4: it ACTS — composing is a given, and deploying a document to a member and trial-booting it is in scope. §11.5 rules it calls a library rather than shelling out.**

---

## 9. Consequences and recorded risks

* **The signing seed's single point of failure is not addressed by this ADR and
  is not created by it.** As of 2026-08-26 the system seed exists in one copy
  with no backup; §3.3 explains why the register must not become that backup.
  **The custody question is separate, real and open** — `ADR-028` §5.6's root
  key is the designed answer and is deliberately deferred.
* **`cargo rivet deploy --device <name>` becomes possible**, resolving address,
  access and trust from the record instead of `--host root@<address>`. That is
  the concrete payoff and it is small once the register exists.
* **§5 depends on no unbuilt mechanism.** The provisioning case (§5.1) needs no
  running machine, and the running case is complete with a reboot (§5.2).
  `ADR-032` §3.2's `SIGHUP` removes the reboot and is not on the critical path.
* **§5.3 adds one field to `ADR-006` §6.5's typed diff** — `requires-reboot` —
  which is `PLAN.md` §24.2 item 1 carrying slightly more, not a second piece of
  work. It is the thing that stops a deferred setting from looking like an
  applied one, which `PLAN.md` §0.0.4 records as a state the project has
  already been bitten by.
* **The register raises the value of `ADR-032`'s query socket**, because §3.3's
  recoverability claim — that a lost register can be rebuilt by asking the
  fleet — depends on machines being able to answer. Today that rests on
  `ADR-028` §5.5.6 being implemented; it is a claim, not a demonstrated
  recovery.
* **§7 means the register is no longer uniformly safe to copy.** The public half
  is; the credential store is not. A tool that offers to "back up the register"
  must distinguish them, and an operator who copies the directory wholesale to a
  shared location has moved a secret. This is the cost of §7 and it is accepted
  rather than unnoticed.
* **Built 2026-08-26, the same day it was
  written** — `PLAN.md` §37. Two crates: `rivet-remote` (§11.5's extraction) and
  `cmd/rivet-fleet`. §4.1's rule is structural rather than aspirational, because
  both surfaces are generated from one operation table; §15's acceptance test
  passes **byte for byte** against the board's own document; §10.4's honesty is
  enforced by a test that scans the module for the word. What is **not** built
  is named in §37.6. This ADR is still the design, and
  **the section it must be read beside is `PLAN.md`
  §102.5.**

  > **Both halves corrected 2026-08-29.**
  >
  > * **The interface was run on a real terminal on 2026-08-27** — `PLAN.md`
  >   §55, *"The terminal interface was run, and it was wrong three ways"*,
  >   under `script(1)` at 40×120. It found three defects and they were fixed.
  >   §55 opens by saying the *previous* hand-off carried this same claim, that
  >   it was wrong, and that finding out cost twenty minutes. It then went on
  >   costing, here, for two more days.
  > * **`PLAN.md` §24.5 is a frozen 2026-08-25 snapshot**, not a standing list —
  >   its own item 1 is already struck and closed. The standing list is `§102.5`,
  >   which `§102.4` requires every hand-off to restate in full. A pointer to a
  >   *"standing list"* that is actually a dated snapshot is the same defect as
  >   a stale claim, one indirection out: the reader follows it and gets a
  >   confident answer from six days ago.

---

## 10. Ruled 2026-08-26, second round — four of §8 closed

### 10.1 One binary, `rivet-fleet` — §8 item 1

**One binary over one library.** Bare `rivet-fleet` opens the full-screen
interface; `rivet-fleet <verb> …` performs the operation and prints.

**The maintainer's reason is the ruling and it is stronger than the convenience
one: this is critical to ensuring that the human and agentic interfaces are
actually co-equal.** *(`.rulings/` R-053.)*

**One binary makes §4.1's rule structural instead of aspirational.** Two
binaries would let the two surfaces drift and nothing would catch it; one
binary cannot offer the human an operation the command line does not have,
because there is only one set of them.

(`jackhammer` was strongly considered and is recorded because a name that was
nearly taken is worth knowing about.)

### 10.2 TOML — §8 item 2

Confirmed. The project's document format everywhere else (`ADR-007`).

**One file per device, named by the opaque identifier**, with the human label
as a field inside it. `ADR-025` §4.1 makes the identifier authoritative and the
name a mutable label, so a register that filed devices by *name* would rewrite
history on a rename and break every reference to the device. The directory
listing is ugly and no human reads it — the interface shows names.

### 10.3 `rivet-fleet` is an Ignition application — §8 items 4 and 6, and it changes §2.2

**Ruled: `rivet-fleet` is installed with `cargo install` into `~/.cargo/bin`
and behaves like any other Ignition consumer** — CLI shape, logging,
configuration location, self-update.

**This is the first binary in the project where `AGENTS.md` §8.4's default
actually lands.** Every Rivet binary so far has excluded Ignition and recorded
why, and reading those reasons together shows they are all one reason wearing
different clothes:

| Binary | Why Ignition was excluded |
| :-- | :-- |
| `cmd/rivet` | it is a **login shell**; everything linked into it is linked into what a login session execs, and `ignition::update` cannot be compiled out — a self-updater reachable from a device whose update model is A/B images is a second updater |
| `cmd/rivet-netd`, `-initd`, `-sysd`, `-timed` | the same, one level down |
| `cmd/rivet-deploy` | the same |

**Every one of those reasons is about being on a device.** `rivet-fleet` is not
on a device: it has configuration to load, it has somewhere to log, and
self-update is a feature rather than a hazard because it is not competing with
`ADR-018` §5. `cmd/rivet/Cargo.toml`'s revisit condition — *"when the CLI grows
its first operation that reads configuration"* — is met here on the first day.

**Consequences that follow rather than being chosen:**

* **The register's default location is Ignition's**, not this ADR's. §2.2 said
  *"a directory on the operator's own machine"* and left it open; Ignition owns
  the platform-correct path and the register sits with the configuration it
  belongs to.
* **§8 item 4 is answered without the tool running `git`.** Ignition decides
  where things live; version control stays the operator's, and the register's
  job is to write files that are *good* to commit — stable ordering, no
  churning timestamps, one device per file.
* **Two layer stacks now exist and must not be conflated.** `ignition::config`
  layers **the tool's own configuration** (defaults ← file ← env ← locked).
  `ADR-007` §3.4 layers **a machine's desired state** (schema ← image ← fleet ←
  machine). They are different stacks about different things and the register is
  the *source* of one while being *stored under* the other.

### 10.4 The credential store is obfuscated, not encrypted, and must say so — §8 item 3

**Ruled: no platform keychain. The maintainer's reason: until the store holds
more than a Wi-Fi password, keychains and secure stores are simply TOO MUCH —
his Wi-Fi password is not a state secret.** *(`.rulings/` R-054.)*

That is the correct call and §7.0's own table is the argument for it: a shared
network credential is not a private key, and building three platform keychain
integrations to protect one is a cost with no matching risk.

**What this obliges, and it is not optional.** `AGENTS.md` §3 — *stubs refuse,
they never fake success* — applies to names as much as to functions. **A file
that is obfuscated must not be called encrypted**, in the code, in the
documentation, or in anything the tool prints. An operator who believes a store
is encrypted will treat it as safe to copy, and §9's warning about backing up
the register wholesale is exactly that mistake.

So: restrictive file permissions, obfuscated at rest so it does not appear in a
casual `grep`, and **named for what it is.** §7.4's phrase *"encrypted at
rest"* is superseded by this section.

**The revisit trigger is named** so this is inherited deliberately rather than
by default: **the first credential in the store that is not a shared network
credential.** At that point §7.0's table has a second row on the wrong side and
the decision is a different one.

---

## 11. Ruled 2026-08-26, third round — §8 items 5, 7 and 8, and the shape of the crate

### 11.1 Verbs from `ADR-006`, inputs as flags, `clap` doing the parsing — §8 item 1 completed

**Ruled: `rivet-fleet` speaks `ADR-006` §6.4's closed verb vocabulary, with
inputs as `clap` flags.**

```
rivet-fleet get device --device kitchen-pi
rivet-fleet add device --name kitchen-pi --role proxy
rivet-fleet export provisioning --device kitchen-pi
```

*(This block spelled the first as `get device kitchen-pi` and the third as
`apply provision …` until 2026-08-29.)*

> **Two of these three did not work, and were corrected 2026-08-29.** They are
> the only commands in this document a reader would copy.
>
> * **`get device kitchen-pi`** — there are no positionals.
>   `cmd/rivet-fleet/src/cli.rs` builds **every** declared input as
>   `Arg::new(input.name).long(input.name)`, which is exactly what this
>   section's own heading says (*"inputs as `clap` flags"*). The example
>   contradicted the sentence above it by one line.
> * **`apply provision`** — **`apply` has no `provision` subject.** It takes three:
>   `document`, `repo-delete`, `trial-restart`. The provisioning
>   document is reached by `export provisioning`.
>
> `add device --name … --role …` was correct and is unchanged. Note that
> `claims.py` cannot see either defect: a command spelling is not a citation,
> not a path and not a count, and checking one means knowing the CLI's grammar.
> Both were found by reading `operation.rs`'s table against the page.

**The maintainer's reasoning, and it settles the alternative that looked
tempting: the alternative would be adding Rivet's vocabulary to Ignition, which
he judges the worse solution. He feels strongly that `rivet` and `rivet-fleet`
must speak the same language even where the machinery under the hood differs.**
*(`.rulings/` R-055.)*

**That last clause is the ruling.** `cmd/rivet` hand-tokenises because it is a
login shell that must refuse anything a shell would interpret; `rivet-fleet` has no such duty and uses `clap`, which
`ADR-014` §3's forbidden-edge table permits in `cmd/*` and forbids only into
`rivet-core`. **Two parsers, one vocabulary** — and pushing Rivet's vocabulary
*up* into Ignition was rejected because Ignition is a general library for five
projects and Rivet's verb set is Rivet's.

This is `ADR-034` §4.1 restated at the grammar level: an agent that learned
`rivet get storage` can read `rivet-fleet get device` without being taught
twice, and an agent is one of the two co-equal surfaces §10.1 exists to protect.

### 11.2 It lives in the Rivet workspace, and the published crate count gets a denominator

**Ruled: `cmd/rivet-fleet`, inside this workspace.** The maintainer thought
putting it outside the Rivet workspace made little sense. *(`.rulings/` R-058.)*
And it needs `rivet-config`'s document schema,
which is the thing it composes.

**The consequence the maintainer named:** its dependencies become this
repository's dependencies. Two things follow and both are work rather than
notes:

1. **`README.md`'s `Third-party crates` row acquires a denominator.** It reads
   *"6 direct, 30 total"* and carries `ADR-001`'s austerity argument. That
   argument is about **what reaches a device**, and `rivet-fleet` reaches none —
   so the row must report *crates in the image* separately from *crates in the
   workspace*, or it will silently start measuring something else while looking
   unchanged. The image number does not move.
2. **`deny.toml`'s target list is wrong for a host tool** — see `ADR-010` §9.

### 11.3 `ratatui` — §8 item 7 answered in the direction of adoption

**Ruled: adopt `ratatui`.** `ADR-009` §3.1's textbook case — a solved problem
someone else maintains — and §2.2's three-platform requirement makes
hand-rolling actively bad, because the part nobody wants to write or test is a
Windows console backend.

§4.3 already established that `ADR-005` §5.5's rejection of `crossterm` does not
bind here, because that argument was explicitly about a binary shipping in the
base image.

**`ADR-013` §2.1's placement test is not yet run**, so the terminal layer stays
in Rivet. Whether Olympus wants it is a claim to check when Olympus is next
touched, not an assertion to make here.

### 11.4 It composes **and** it acts — §8 item 8

**Ruled: composing a provisioning document is a given, and deploying an updated
document to a member and trial-booting it is in scope** — in the maintainer's
judgement that combination should absolutely be available.
*(`.rulings/` R-057.)*

### 11.5 It calls a library; it does not shell out to `cargo rivet`

**The maintainer's correction, and it is right: the project would not shell out
to call code it can simply reach as a crate — that would be silly.**
*(`.rulings/` R-056.)*

**No, and the refusal generalises** — §11.5.1 rules that the tool shells out to
nothing at all to reach a machine:

| Shelling out to | Verdict | Why |
| :-- | :-- | :-- |
| **`cargo-rivet`** | **refused** | it is a crate in this workspace. Calling it as a subprocess re-parses our own output and loses every type. |
| **any remote transport** | **refused — §11.5.1** | there is one way to reach a machine and it is a library call over the control plane |

**The work this implies, found 2026-08-26:** `cmd/cargo-rivet` is **binary-only**
— no `[lib]` target — and its push-verify-install logic is a module inside the
binary crate. `ADR-014` makes `cmd/*` composition roots, so the logic does not
belong in a `cmd/` crate at all. **It is extracted into a library crate that
both `cargo-rivet` and `rivet-fleet` depend on**, which is `ADR-011` applied:
one implementation of "put a payload on a machine", not two.

#### 11.5.1 RULED 2026-08-29: every remote human tool is an MCP/TLS client

**On 2026-08-26 this table still allowed a second way to reach a machine, and
the maintainer closed it on 2026-08-29** — the same day `ADR-035` removed
the last of the alternative from the image. His reasoning is a one-line proof
rather than a preference: **there is nothing left at the far end.**
*(`.rulings/` R-101.)*

**The ruling, stated generally because it is more than a fix to one row:**

> **Every remote human-facing tool is an MCP/TLS client.** `rivet-fleet`, and
> anything like it, reaches a machine over `ADR-035`'s control plane — mutually
> authenticated with pinned Ed25519 keys on port 7777 — and by no other route.

**This was already the only possible answer and the ADR had not noticed.**
`ADR-035` phase 2 moved `rivet-remote`'s entire transport to TLS and proved it on
hardware; phase 3 did the same for git. §11.5's table was written before either
landed, kept its second row for a reason that was true then, and **survived by
nobody re-reading it** — which is `PLAN.md` §94's finding exactly, and `AGENTS.md` §7's
*a claim that was true and rotted*: nothing overturned it, so there was nothing
to strike it with.

**Found by an agent migrating quotations**, reading `ADR-034` closely for an
unrelated reason, and reported as a contradiction it would not rule on. That is
the correct behaviour and it is why the contradiction surfaced at all.

##### What this changes, and what it does not

* **`rivet-fleet` takes `--identity` and `--pin`**, the same pair `cargo rivet
  run` and `cargo rivet deploy` take. It is `PLAN.md` §102.4.1 item 8's other
  half.
* **The extraction §11.5 already requires becomes larger and more clearly
  right.** The library both tools depend on is now *the control-plane client*
  rather than *the payload pusher*, and `rivet-remote` is where it already lives.
* **It does not settle where the operator identity lives** for a fleet of more
  than one machine. `ADR-025` §2.1 is still open and `§102.4.1` item 3's
  secrets-at-rest gap is still unbuilt underneath it.

#### 11.5.2 RULED 2026-08-29: the Monitor moves onto the control plane, and the pane must show it is alive

§11.5.1 made every remote tool an MCP/TLS client. **The Monitor pane was the
first thing that ruling broke and the first that had to be fixed**, because its
only `telemetry::Source` implementation spawned a subprocess per request against
a transport the board no longer served. *(`.rulings/` R-102.)*

**The pane's DATA PATH does not change** — this said *the pane itself does not
change* and was amended 2026-08-29 when it was built, because the same section goes on to
require an indicator the pane did not have, and the two sentences together were
not consistent. `crate::telemetry::Source` is a trait and its own documentation
already said why: *"when the fleet tool grows an identity, one new
implementation of this trait moves the whole pane onto the control plane and
nothing above it changes."* This section is that sentence being cashed, and it
was: `Ssh` was replaced by one new implementation of the trait and no renderer
of a number was touched. What the pane gained is the indicator below.

##### The two things in the way, and neither is a design problem

1. **`rivet_remote::tls::Tls::call` is private.** `Tls::ask` deliberately refuses
   a word list — *"MCP is the CATALOGUE, not a command line"* — so a `get`
   subject over TLS needs a `tools/call`.
2. **`rivet-fleet` holds no operator identity.** `crate::main`'s deploy path
   passes `identity: None` and says so.

Both were cleared when this section was built. The deploy path named in the
second was later removed outright — History, *In §11.1 and §11.5.2 —
`rivet-fleet apply deploy`*.

##### RULED: the cost model, measured against what a connection actually costs

**The tiering stays; the arithmetic under it changes.** Under the old subprocess
transport a request cost a process, a TCP connection and a handshake. Under TLS
it costs a TCP connection, a TLS handshake, and **a `rivet mcp` process spawned by
`rivet-mcpd`** — which `PLAN.md` records is spawned **per CONNECTION, not per
request**, a distinction found on 2026-08-29 by reading the listener rather than
inheriting the claim.

**That distinction is the whole optimisation.** The fast tier asks three subjects
(`memory`, `process`, `cpu`); one connection per subject is three processes per
tick, and one connection carrying three `tools/call` messages is **one**. So:

> **One connection per poll CYCLE, carrying every subject due on that tick** —
> not one per subject. The module header's property 3 — *one worker, one
> transport* — becomes true of the build and not only of the shape.

**And the fast tier's default moves from 2 s to 5 s.** The maintainer offered it
and it is the right trade now that a poll spawns a process on a 926 MiB board.
`FAST_INTERVALS` keeps its 1/2/3/5/10 range, because the interval is still a
matter of taste and the argument for tiering — *how fast can this fact actually
change* — is unaffected by the transport.

##### RULED: the pane must show that it is waiting, and the indicator is a claim

A five-second refresh is only honest if the screen says so while it waits.
Otherwise a still pane is indistinguishable from a dead one — **which is this
pane's founding failure**, *"a stale number rendered as a live one"*, in its
other direction.

**The indicator is derived from the poller's state, never from a wall clock.** A
countdown that keeps cycling while the transport is dead is an animation that
says *alive* about something that is not, and it would be a worse defect than
having no indicator at all. Four states, and they must be distinguishable:

| State | What it means | Drawn as |
| :-- | :-- | :-- |
| **waiting** | counting down to the next due poll | a clock face filling: `○ ◔ ◑ ◕ ●` |
| **in flight** | a request is on the wire | an indeterminate spinner — **never a countdown**, because the remaining time is unknown and the honest statement is *waiting on the board* |
| **failed / backing off** | the last attempt failed and the interval has widened | the error style, naming the back-off it is now at |
| **no transport** | no identity, or never started | said in words, not animated |

**`Glyphs::ascii()` gets an equivalent** — the pane already refuses to require a
font (`RIVET_FLEET_GLYPHS`), and an indicator that only works in one mode is a
liveness signal that vanishes exactly where a terminal is least capable.

##### What this does NOT settle

* **Where the operator identity comes from for a fleet of more than one
  machine.** `ADR-025` §2.1 is open and `ADR-020`'s secrets-at-rest is
  ruled-and-unbuilt beneath it. This section needs one identity for one
  operator; it does not answer the fleet question.
* **Whether `Tls` should keep a connection across cycles.** The listener closes
  after a response, so a kept connection is a listener change and not a client
  one. Out of scope, and the batching above is most of the benefit anyway.

#### 11.5.2.1 BUILT 2026-08-29 — what was made, and where it differs

This is §11.5.2 implemented, recorded in the same section rather than in a new
one, and it differs from the specification above in five places. Four of them
are the specification being more specific than it could be before there was
code; the fifth is a **gap this build cannot close on its own** and is the one
to read.

**1. `Tls::call` stayed private.** §11.5.2 named it as one of the two things in
the way and did not say to make it public, which turned out to be the right
silence. The public surface is `Tls::get(subject)` and
`Tls::get_each(subjects)`: the caller names a catalogue **subject**, and the
tool name and argument object are composed by `rivet_remote::command::translate`
from `rivet_core::operation::CATALOGUE`. A caller holding that cannot compose a
word list, so `Remote::ask`'s refusal — *"MCP is the CATALOGUE, not a command
line"* — is intact rather than routed around.

**2. `cmd/cargo-rivet/src/endpoint.rs` moved to `rivet_remote::endpoint`.**
§11.5.1 requires `rivet-fleet` to take *"`--identity` and `--pin`, the same pair
`cargo rivet run` and `cargo rivet deploy` already take"*. Taking the same pair
means reading it with the same code: `ADR-014` makes `cmd/*` a composition root
and §11.5 already ruled that a shared implementation does not live in one, so
the three flags moved into the library both tools depend on. `rivet-remote`
gained `rivetos-config` as a dependency with them, for the default port.

**3. The pin comes from the REGISTER, and `--pin` overrides it.** `ADR-034` §3.5
has the register hold the public half of every device key and `add identity`
writes it there, so the value `cargo rivet` makes an operator type is already
recorded. Asking for it again would be asking a person to copy a number out of
one of this tool's screens into one of its flags. `--pin` is still taken,
because a device whose identity has been replaced has a record that is briefly
wrong and *the record is wrong* has to be survivable without editing it first.

**4. The back-off state is EVERYTHING failing, not the last attempt failing.**
§11.5.2's table says *"the last attempt failed and the interval has widened"*.
Taken literally that would put the whole pane in the error style whenever one
subject a build cannot answer failed, next to eight that are current — and each
section already says so on its own heading. So the indicator shows
`BackingOff` only when every subject that has been attempted is failing, which
is what *the board has stopped answering* actually looks like: a cycle is one
connection, so a dead transport fails all of them at once.

**5. THE GAP — the register records no control-plane PORT.** `rivet-fleet`
resolves a member's address from the record (§9) and the record has no field for
where `rivet-mcpd` listens, so the port is
`rivet_config::control_plane::DEFAULT_PORT`. That is read from the crate that
owns `[control-plane]` rather than written in the fleet tool, so it cannot
become another spelling of a number a cold agent already found in three places
(`PLAN.md` §92) — **but it is still a value this tool chose rather than read.**

> **A device whose document moves `rivet-mcpd` to another port cannot be
> reached by `rivet-fleet` at all**, and there is no flag that would help,
> because the address is deliberately not typed. Closing it is a register field
> — §6's table gaining a control-plane `listen` per device, composed into the
> document by `crate::compose` so the register and the board cannot disagree
> about it. Not built, and not ruled.

**And the four states are four variants, with one carrying its words.**
`crate::telemetry::Liveness` is `Waiting`, `InFlight`, `BackingOff` and
`Stopped(&'static str)`. The fourth is §11.5.2's *no transport* — *"said in
words, not animated"* — and **paused reuses it**, because a person who pressed
`p` has stopped the poller and a clock face counting down to a poll that is not
coming is the same lie in a smaller font.

##### What has and has not been proved, 2026-08-29

`AGENTS.md` §6. **No board.** None was available to this work, and nothing below
changes that.

* **Proved by test**: the wire format of a batch and the matching of answers to
  calls by id; the poller sending one cycle to the source; the fast tier's new
  default; each of the four indicator states in both glyph modes; and — the one
  the ruling is for — that a poller whose every subject has failed shows a
  back-off and never a countdown across ten minutes of simulated wall clock.
* **Proved on a real terminal**, driven under `script(1)` at 40×120 and replayed
  through `pyte`: the monitor pane opening with no `--identity` and drawing the
  no-transport indicator and the refusal beneath it, in **both** glyph modes.
* **NOT proved**: that any of this reaches a Raspberry Pi. No connection has
  been opened, so the `waiting`, `in flight` and `backing off` states have been
  rendered from constructed poller states and not from a request that went out;
  no `rivet_get` composed here has been answered by `rivet mcp` over TLS; and
  the claim the batching rests on — that `rivet-mcpd` spawns one process per
  connection and answers nine `tools/call` messages on it — was read off the
  listener rather than measured against a running one.
#### 11.5.3 RULED 2026-08-29: the minting is extracted, and there is no bounded exception

§11.5 forbids `rivet-fleet` shelling out to `cargo-rivet`. **It does it anyway —
running our own binary and re-parsing our own stdout to mint an identity — and
the code cites §11.5 as its warrant for doing so.** Three options were put:
extract the minting into a library both tools use, write a bounded exception
into §11.5 naming this one case, or leave it. **Extract.** *(`.rulings/` R-109.)*

**A rule that its own violation cites is corrosive** beyond the fragility it
excuses: it teaches the next reader that the rules here are decorative, which is
expensive in a repository whose whole method is that the record can be trusted.
And the fragility is real rather than aesthetic — re-parsing our own stdout
means every output-format change is a silent breakage in a second program.

**This is the third thing now waiting on one extraction.** §11.5's original
requirement, §11.5.1's control-plane client, and this. They are one library, and
`rivet-remote` is where it already lives — the work has grown a clearer shape
each time rather than growing larger.

### 7.1.1 RULED 2026-08-29: add PBKDF2-HMAC-SHA1, so an operator can type a passphrase

§7.1 says the register derives the wireless PMK. **The function always refuses**,
because deriving one needs PBKDF2 and SHA-1 and neither is in this workspace — so
`ADR-019` §2's *"wake up already on WiFi"* has never been deliverable and no
human could type a passphrase into this tool.

Three options were put: add the primitive; have the operator paste a
pre-computed PMK from `wpa_passphrase`; or shell out to `wpa_passphrase`.
**The recommendation was the second. The maintainer ruled the first.**
*(`.rulings/` R-110.)*

#### The recommendation was wrong on the technical merits, and the correction matters

It argued that adding SHA-1 — *"a broken hash we'd carry forever"* — to save one
paste was a bad trade against `ADR-001`'s austerity. **That framing was
misleading and should not have been written.**

**SHA-1's break is collision resistance, and WPA2's PSK derivation does not rest
on it.** The PMK is `PBKDF2-HMAC-SHA1(passphrase, ssid, 4096, 256)` — SHA-1
inside HMAC, where the relevant property is pre-image and PRF behaviour, neither
of which the collision attacks touch. **It is also not a choice**: IEEE 802.11i
specifies exactly this construction, so any implementation that interoperates
uses it. Declining it would not have avoided SHA-1; it would have moved it into
`wpa_passphrase` on the operator's machine and called that austerity.

#### What the ruled option buys that the recommended one did not

* **`ADR-019` §2 becomes deliverable.** An operator types a passphrase into the
  fleet tool and a card wakes up on WiFi. That was the promise; pasting a
  64-hex PMK is a different, worse product.
* **No external tool on the provisioning path**, which was the objection to the
  third option and applies to the second in weaker form — `wpa_passphrase` ships
  with `wpa_supplicant`, which a Windows operator does not have.
* **The register keeps deriving what it says it derives.** §7.1's sentence
  becomes true rather than becoming a documented aspiration.

#### The cost, stated plainly

**A PBKDF2 and a SHA-1 implementation enter the workspace**, and `ADR-001` §2's
count moves. They are **host-side only** — the fleet tool derives the PMK and
writes it into a provisioning document; nothing on a card needs either — so the
image's dependency count, which is the number `ADR-001`'s austerity claim is
actually about (`ADR-034` §11.2), does not move at all. **That distinction is
what makes this cheap, and it is the one the recommendation failed to make.**

**Not built.**

### 8.1 RULED 2026-08-29: `rivet-fleetd` is struck, and remote mode is written up as open

§8 item 5 asked what `rivet-fleetd` is. **It is named four times in this
document and exists nowhere else in the repository** — and this ADR's status
block says *"nothing in this ADR is open"*, which cannot both be true.

**Ruled: strike the four references AND write remote mode up as an open
question.** Both, not either. *(`.rulings/` R-112.)*

**Striking alone would have been the tempting half and it is not enough.** It
removes a phantom component and leaves a status block still claiming nothing is
open, while §2.2's Remote mode — a designed, documented, unbuilt deployment —
sits behind it. **A record is not made honest by deleting the evidence that it
is incomplete.**

So §2.2's Remote row stands, because the *design* is real, and this section is
the statement that **no daemon implements it, none is designed, and none is
scheduled.** A reader reaching §2.2 now learns that Remote mode is a shape
rather than a thing.

**And the status block changes**, because *"nothing in this ADR is open"* was
false the day it was written and is the claim that hid this. `AGENTS.md` §7:
a claim of completeness is a claim, and it rots like any other.

### 8.2 OVERTAKEN 2026-09-01: `rivet-fleetd` was built, so there is nothing to strike

§8.1 ruled on 2026-08-29 that `rivet-fleetd` be struck from this document
because **it was named four times here and existed nowhere else in the
repository**. That ruling was right on the day it was made, and its reasoning is
the part worth keeping:

> **A record is not made honest by deleting the evidence that it is
> incomplete.**

It was never carried out. Three days later the maintainer chose the other way of
closing the gap, observing that the HTTP side could itself be `rivet-fleetd`,
speaking MCP to any `rivet-mcpd` it holds keys for *(`.rulings/` R-281)* — and
`cmd/rivet-fleetd` was built: a signed `.rvt` application holding `tcp:443`, terminating TLS,
requiring a client certificate, serving the register, and forwarding JSON-RPC to
any machine in it over `ADR-035`'s control plane.

**So the strike is withdrawn rather than performed.** The four references are
correct now. Striking them would remove true statements about a running
component, which is the same fault R-112 was guarding against, arriving from the
other direction.

#### This answers §8 item 5, which §10.3 answered about a different program

§8 item 5 asked *"whether `rivet-fleetd` is a base-image service (`ADR-017`) or
an application (`ADR-015`)"*, and §10.3's answer — *"neither — `rivet-fleet` is
an **Ignition application**"* — is about `rivet-fleet`, the workstation tool.
The two names differ by one letter and the question was never actually answered
for the daemon.

**It is an `ADR-015` application, and that was the maintainer's point rather
than an implementation detail:** it leaves `rivet-fleetd` as nothing but a
standard network-service `.rvt` application, which he named a pleasant side
effect. *(`.rulings/` R-281.)*

The side effect is the best part. **Exposure is limited by not installing it**:
a managed node does not carry the web stack, because it does not carry this
program. There is nothing to disable and nothing to configure off.

#### Its data directory holds the fleet's objects, and its `/api/*` gains no route for them

`.rulings/` R-184 D1 puts a fleet server's repository objects in a
digest-keyed store inside **this program's own data directory**,
`repository-objects/`. **The routes that fill, list and prune it are the
control plane's, not this program's**: `PUT /repository-objects/<name>`,
`GET /repository-objects/` and `POST /repository-objects/prune` on
`rivet-mcpd`'s 7777, and `rivet-fleetd`'s `/api/*` on 4443 is unchanged.
`ADR-052` §7.4 carries the argument, which is this program being an ordinary
application: confined, it can read neither the machine's trust store nor the
pushed `fleet` repository, and the store's door and its prune need both. Every
entry is handed to this program's uid, so it can read what the store holds.

**Fleet-as-code's repository is reached the same way.** `rivet-fleetd` runs no
git: its poller and `POST /api/commit/<device>` ask this machine's own
`rivet-mcpd` for `POST /fleet-as-code/{head,machines,file,commit}` over the
control plane, with the fleet server's operator identity (`ADR-044` §5.5,
`.rulings/` R-187 D2). It finds its own control plane in this register — the
one record with `role = "fleet-server"`, which provisioning writes onto a
server's card — so that record's pin and `listen` are what the poller dials,
and a register holding none or two of them is refused by name.

**Who may use them is R-184 D2's operator, as a key in the machine's `peers`.**
The 4443 handshake says less than that. It admits any certificate the fleet
authority issued, and `cmd/rivet-provision/src/stage.rs` issues one to every
fleet machine's front door from that same authority; `rivet-fleetd` serves only
the one that names no host, which is the operator's browser certificate
(`ADR-043` §5.3, `.rulings/` R-187 D1). That is an inference from the leaf's
shape rather than a key the machine was told to expect, until the re-mint gives
the operator's certificate an explicit marker (`ADR-052` §10), and whoever
designs `ADR-052` §9 step 5 starts from it.

### 6.1 RULED 2026-08-29: the register records the control-plane port, and one operation writes both

§11.5.2.1 found the gap: **§6's device record has no field for where
`rivet-mcpd` listens**, so `rivet-fleet` composes `<address>:DEFAULT_PORT` and a
device whose document moves the listener cannot be reached by the fleet tool at
all. No flag helps — §9's whole payoff is that the address is *resolved from the
record* rather than typed.

**Ruled: §6's device record gains `listen`.** And the maintainer added the half
that matters more. *(`.rulings/` R-115.)*

#### The requirement is concurrency, not a field

A field alone would create the thing `ADR-011` exists to forbid: **two places
that say where a machine listens, and no vocabulary for their disagreement.**
The register would hold one port and the machine's document another, and the
first symptom would be a fleet tool that cannot reach a board it can see.

> **The tools that change the listener are ours. So the operation that changes
> it writes the register and the document together — both, or neither.**

That is stronger than nominating an authority and letting the other side
converge. Converging would be consistent with the desired-state model
(`ADR-007`) and is **wrong here for a specific reason**: the value being
converged is *how the tool reaches the machine to converge it*. A register
corrected on the next `apply document` is a register that cannot deliver the
`apply document` that corrects it. **This is the one field where drift is
self-sealing**, and that is what takes it out of the ordinary model.

#### What that implies, and where the residue is

* **Changing the port is a fleet operation**, not a document edit that the
  register learns about afterwards. `crate::compose` writes the value into the
  document from the record, which it already does for every other field.
* **A board-side `apply edit` of `[control-plane]` is still reachable**, because
  the board does not know a register exists, and refusing it would need the
  machine to enforce a fleet-layer rule (`ADR-007` §3.4's layering says it must
  not). **So drift remains possible from that direction and cannot be prevented
  at the machine.**
* **It can be DETECTED, and that is what the fleet tool owes.** `rivet-fleet` is
  already reading the board; comparing the register's `listen` against
  `get desired-state`'s is one comparison at the point it is already connected.
  A disagreement is reported, naming both values and which one the tool used to
  get there — which is the only honest report available, since the tool is
  necessarily talking to the machine on the value that worked.

#### Not settled

* **What `listen` defaults to when absent.** `rivet_config::control_plane::DEFAULT_PORT`
  is the obvious answer and keeps every existing record valid; recorded as
  obvious rather than ruled, because an absent field and a field equal to the
  default are different states and nobody has said whether they should be.
  **ANSWERED by the build, 2026-08-29 — see §6.1.2.**
* **BUILT 2026-08-29 — §6.1.2.**

---

### 6.1.1 AMENDED 2026-08-29: the PIN is the same ruling, and it was found on a board

§6.1 above ruled one field. **It should have ruled two**, and the second was
found the way the first was — by running the tool against the real board rather
than by reading it. *(`.rulings/` R-116.)*

> `rivet-fleet --help` says of `--pin` that **"The register already holds one per
> device and that is what is used; this overrides it"** — and I had to pass
> `--pin` explicitly or nothing connects.

**The maintainer's ruling: the device record carries the machine's pin, on §6.1's
terms exactly.** A pin and a listen address are the same *kind* of fact — both
are **how the tool reaches the machine** — and both are self-sealing when they
drift, which is what took `listen` out of the ordinary converging model. A
register holding the wrong pin cannot deliver the document that would correct
the pin.

#### What the build found underneath it, which is narrower than the report

The field **existed**, as `Device::public_key`, and `add identity` wrote the
minted key into it. Two things made it useless in practice and both are fixed:

1. **`add identity` MINTS.** It is the only thing that wrote the field, and it
   replaces the machine's identity — which needs a new card and locks out every
   peer pinning the old key. A board provisioned by a card build already has an
   identity, minted by `cargo rivet identity new`, and **there was no operation
   that recorded its public half.** So the help string described a field nothing
   could populate for the ordinary case, which is `ADR-012` §7.4's *confident
   sentence a reader cannot test*. **`set pin` is the missing verb** — it records
   and mints nothing.
2. **The name said nothing.** Beside `control_plane_keys` — hex, and the keys
   the machine **accepts** rather than the one it **presents** — a second
   key-ish field called `public_key`, whose own documentation said *"the public
   key, and its key id"*, naming two things and committing to neither, was read
   wrongly **by this crate's own test**, which put a whole line of base64 in it.
   It is `Device::pin` now, with `serde(alias = "public_key")` so every register
   on disk still loads.

#### Absent is a REFUSAL, and that is where it differs from `listen`

`listen` has a sensible default. **A pin has none**, and inventing one would
mean connecting without pinning — the single failure `ADR-025` §2's trust model
exists to prevent. So an absent pin refuses, names the device, and names both
ways to fix it. That refusal already existed and now points at a verb that
exists.

#### And the concurrency rule does NOT reach it, for a reason worth stating

§6.1's *both, or neither* binds `listen` because the listener lives in the
machine's **document**, which this tool writes. **A pin lives in the machine's
identity files**, which reach a board on a card — `ADR-019` §3.1 blanks the
private half at first boot. There is nothing on the wire for `set pin` to write,
so it is not a concurrent operation; it only ever catches the register up with a
fact that is already true of the board. Making it concurrent would mean
inventing remote identity replacement, which is a separate ruling.

---

### 6.1.2 BUILT 2026-08-29 — both fields, and the one thing that was picked rather than ruled

* **`listen` absent and `listen` equal to the default are ONE state.** §6.1 left
  this open and the simple reading was taken: an absent field composes **no
  `[control-plane]` table at all**, so the machine reads `DEFAULT_LISTEN` for
  itself and the tool dials `DEFAULT_PORT`. What that buys is worth more than the
  distinction: **every register written before this field existed composes
  byte-identical documents**, and `apply document` compares the bytes it sent to
  the bytes read back. What it costs is that an operator cannot record *this
  machine is deliberately on the default* as distinct from *nobody has said*.
  Nothing reads that difference today.
* **The port is the port half of `listen`, and the address half is not used to
  dial.** What a machine BINDS and what a workstation DIALS are different facts:
  a board listening on `0.0.0.0:7777` is reached at its own interface address,
  which is §9's payoff. Only the port is shared.
* **A finding this closes that nobody had written down.** `crate::compose` never
  emitted `[control-plane]` at all, so **every document this tool composed left
  the board on `127.0.0.1:7777` — loopback — and unreachable by `rivet-fleet`
  from a workstation.** The gap §11.5.2.1 recorded was *a device that MOVES its
  listener cannot be reached*; the larger truth was that a device provisioned by
  this tool could not be reached either, until somebody edited its document by
  hand. Recording a routable `listen` is what makes the control plane usable.
* **`set listen` writes both, and the register is written FIRST.** Compose (so a
  record the parser would refuse is caught while both sides are untouched), save
  the register (proving the file is writable while nothing has been sent), reach
  the machine **on the old value** — it is the one currently answering — write,
  and **put the register back if the machine refuses.** Machine-first was
  rejected because its failure has no local recovery: a board that took the new
  document while the register held the old value has moved somewhere this tool
  cannot dial. Writing the register first makes the rollback a local file
  operation, which is the one kind that can be relied on at the moment the
  network has just failed. A crash *between* the two is still possible and is
  not preventable without a transaction spanning two machines.
* **Detection is at `apply document` and at `set listen`**, and it costs no extra
  round trip in the second case: both read the machine's own document before
  overwriting it, which is the last moment the machine's answer exists. The
  report names the register's value, the machine's, **and the address the tool
  reached it on** — reachable at all because a `listen` change does not take
  effect until `rivet-mcpd` restarts, so a board whose document was edited is
  still answering at the old address.

---

### 6.1.3 The reach probe, which was the site worth thinking about

`ADR-034` §11.5.1 struck the subprocess shell-out and four sites moved onto the
control plane by translation. **The reachability probe could not be translated
and this section records why**, because the literal translation was available,
looked correct, and would have been *worse than the thing it replaced.*

The old probe opened a TCP connection to a fixed port and called that **up**.
That was defensible then, because under the transport it replaced *can I open a
session* and *do I trust this machine* were the same question asked in two
places: a changed host key was refused at the moment a session opened.
**The control plane's trust is a pin, and a TCP connection tests none of it.**
So a probe that connected to 7777 and said *up* would have put a green dot
beside a machine presenting a key this operator does not pin, which is either a
device whose identity was replaced or something else wearing its address.

> **A probe that reported "reachable" for a board that answers TLS but presents
> an unexpected key would be worse than the one it replaced.**

So it completes a handshake — `rivet_remote::tls::Tls::handshake`, which sends no
application data and therefore **spawns nothing on the board**, since
`rivet-mcpd` reads a connection's opening bytes before it decides which endpoint
it is. Four answers, not two:

| | Means | Why it is its own state |
| :-- | :-- | :-- |
| **up** | answered, and presented the pinned key | the only one that is good news |
| **STRANGER** | answered, and this end will not trust it | needs a person to find out why, not a power cycle |
| **down** | nothing answered | needs the machine switched on |
| **not asked** | no identity, or no pin in the record | a fact about THIS END, drawn faint rather than as trouble |

**What `up` does not claim, stated because it is easy to assume:** TLS 1.3 lets a
client finish its side before the server has judged the client certificate, so a
board that will refuse this operator still completes the handshake. *Up* says the
machine is the one the register describes. It does not say the machine will talk
to you.

#### What running it found that no type-check could

The interface was driven under `script(1)` at 120×40 and replayed through
`pyte`, against a scratch register and a listener that accepts a connection and
never speaks TLS. Three defects, none of them visible to the test suite as it
stood:

1. **The Status pane said `"Reachable means the address answered on port 22"`**,
   on screen, to an operator, on a board — `AGENTS.md` §7's *a claim that was
   true and rotted*, still being rendered after `ADR-035`.
2. **A long refusal was clipped at the pane border**, losing the half that says
   what to do — `PLAN.md` §55's defect arriving in a second pane. It stopped at
   *"can reach no m"*.
3. **`Trusts signingnone`** — a fourteen-character label ran into its value,
   because `{name:<14}` pads a short name and does nothing for one already that
   long.

And a fourth that the register itself surfaced: the System pane's **"Trusted
keys"** showed `Device::trusts`, which is `ADR-028` §5.5's *signing* key ids, so
a board holding two control-plane peers read as `none`. Two different trusts and
the label named neither; they are `Trusts signing` and `Accepts keys` now, and
the second is counted from the **composed** set rather than the record's half.

### 6.2 RULED 2026-08-29: `apply document` refuses to erase what the register does not know

§6.1 added `listen` and §6.1.1 added `pin`. **Neither makes `apply document`
safe**, and the maintainer instructed that it must be: close the gap in what
compose emits, so that applying a composed document cannot take a board away.
*(`.rulings/` R-117.)*

*(His own words were quoted here until 2026-08-30, when `tools/docs/rulings.py`
reported it. `ADR-029` §16.6: a verbatim quotation lives in `.rulings/` and
nowhere in `docs/`, because `docs/` ships as a card's manual. Stated in this
document's own words and cited, which is what that ruling asks for.)*

#### What the gap actually is, measured on the test board

`crate::compose` is a pure function of the register. Anything on a machine that
the register does not hold is **absent from the composed document**, and
`apply document` writes that document — so applying it **removes** the machine's
value. Compared against the live `/state/config/machine.toml` on 2026-08-29, the
composed document was missing two things:

| Missing from compose | What applying it would have done |
| :-- | :-- |
| `[control-plane]` | the listener falls to `DEFAULT_LISTEN`, **`127.0.0.1:7777`** — the board becomes unreachable from any workstation |
| `[services.rivet-mcpd]` | `enabled` reverts to the image default, which on that card is `false` — **the control plane does not start at all** |

**Both at once.** One `apply document` would have disabled the control plane and
bound it to loopback, on a headless board, with recovery only through the boot
partition or a card rewrite. That is not a hypothetical: it was true of the only
Rivet in existence, and it was found by a tool that had to compose one of those
two fields going looking for where it belonged.

#### The ruling: the refusal goes at APPLY, not in compose

> **`apply document` reads the machine's current document, diffs it against the
> composed one, and REFUSES if the write would remove or change anything the
> register does not account for** — naming each loss and the command that would
> record it.

**Compose stays a pure function of the register.** It is offline, deterministic
and byte-comparable, `export provisioning` depends on that, and §6.1's *absent
and default-valued are one state* keeps every existing record composing
identically. **Making compose guess at a machine it cannot see would trade all of
that away to solve a problem that only exists where a machine IS in reach** —
which is exactly where `apply document` already is.

So the danger is addressed where the danger is, and the unknown becomes a
refusal with a name, which is `ADR-012` §7.4's preference order: **structure,
not warning.**

#### What the refusal must say

Each loss by name, what the machine currently has, and **the command that
records it** — `set listen`, `set service`, `set pin`. A refusal that named the
loss without the remedy would leave an operator to work out that the register is
the thing to change, and the natural wrong move from there is to edit the
machine instead, which widens the drift the refusal exists to catch.

#### Adopting, rather than transcribing by hand

**Recommended and not required**: a verb that reads the machine and records what
the register lacks, so clearing the refusal is one command rather than one per
value. It is the ergonomic half of the same idea and it is how the existing board
gets its record filled. If it is not built, the refusal must still name every
loss individually, because an operator who has to discover them one at a time
will stop before the last.

#### What this does not settle

* **Whether a CHANGE should refuse as loudly as a REMOVAL.** A register that
  holds a different value than the machine is a deliberate reconfiguration most
  of the time. Removal is unambiguous; change is the operator's intent more often
  than not, and treating them alike would make the refusal routine — which is how
  a refusal stops being read. **ANSWERED by the build, 2026-08-29 — see §6.2.1:
  a change is reported and written; only a removal refuses.**
* **BUILT 2026-08-29 — §6.2.1.**

---

### 6.2.1 BUILT 2026-08-29 — a REMOVAL refuses, a CHANGE is reported, and `add document` is the adoption

§6.2 ruled the refusal and left one question open. This section answers it,
records what was made, and names **every place the build differs from the
specification above** — which is four places where §6.2 could not be specific
before there was code, and three findings underneath it that nobody had written
down.

#### The open question, answered: only a REMOVAL refuses

> **A change is reported and then written. A removal refuses.**

§6.2 gave both halves of the argument and did not choose. The build chooses
removal-only, and the reason is not that a change matters less — it is that
**the two are different kinds of statement**:

* **An absence in the register is never a request.** `crate::compose` emits what
  the register holds, so *undeclared* and *deliberately removed* compose the
  identical document. **There is no way for an operator to spell "take this
  away" in the register at all** — which means a missing value can only ever be
  ignorance, never intent. That is what makes a removal unambiguous, and it is
  the whole asymmetry.
* **A differing value is a request, and it is the request `apply document`
  exists to carry.** A register that says `enabled = false` where the machine
  says `true` is an operator turning a service off. A guard that stopped there
  would fire on every ordinary reconfiguration, and §6.2 names that as the
  failure mode itself: *"treating them alike would make the refusal routine —
  which is how a refusal stops being read."*

So changes are **listed in the report and then written** — the operator sees
what moved without having to confirm past a refusal to get it. The one thing
this loses is the operator who mistyped a value into the register and would
rather have been stopped; they are told, in the report, before the reload.

**And `listen` keeps its own louder treatment**, which is not an exception to
this: §11.5.2.1's `listen_drift` fires on a *change* to that one field and stays
exactly as it was. It is narrower on purpose — one field, reported with the
address the tool actually dialled — and §6.1 gives the reason that field is
special. This section did not widen it and did not fold it in.

#### What was built

| | |
| :-- | :-- |
| `crate::diff` | flattens two TOML documents to leaves, compares them, and composes the refusal |
| `crate::guard` (in `main`) | the check itself, between reading the machine's document and writing over it |
| `add document` | the adoption §6.2 recommended, built |

`apply document --confirm` now reads the machine's document, compares, and
refuses before `member::put_document` is reached. **The order is: drift report
(§6.1), then this refusal, then the write** — and a refusal carries the drift
report with it, because the process prints an error and drops the report.

#### Where it differs from the specification, and why

**1. §6.2 says *remove or change*; the build refuses only on remove.** That is
the answer to §6.2's own open question and is argued above. Stated here as a
difference as well, because the ruling's own sentence reads as though both
refuse and the code does not.

**2. §6.2 says "anything"; the build had to decide what a *thing* is.** The
comparison is over **leaves of the document**, and a leaf is one scalar *or* one
list of scalars. **An element of a list is compared on its own**, which is the
consequential half: losing one entry of `network.resolver.nameservers` is losing
that server, and a comparison keyed only on paths would have called it a
*change* to `network.resolver.nameservers` and let it through. An empty table is
also a leaf — a table `rivet-config` renders deliberately to state that
something is declared and empty, and flattening it to nothing would make
dropping it invisible here.

**3. §6.2 requires "the command that would record it". For three kinds of loss
there is no such command, and the build says so rather than naming one that
would do something else.** These are findings, not omissions in the refusal:

| Loss | Why no command | What is named instead |
| :-- | :-- | :-- |
| `repos.<n>.id` | **`add repo` MINTS.** It is §6.1.1's `add identity`-versus-`set pin` again: the verb that exists makes a *new* identifier, and the machine already serves a directory under this one | the field, and `add document` |
| `network.<i>.*` | no verb writes an interface | the field in the device record |
| `volumes.*` | **the register has no field for volumes at all.** §6's table does not list them | that growing the record is a ruling, not a fix |

A fifth is narrower: `services.<n>.memory` *does* have a command, and
`set service` requires `--enabled`, so the memory cap cannot be recorded on its
own. The refusal writes the flag with a `<true|false>` placeholder rather than
guessing which the operator wants — `enabled` is the more consequential half and
this tool does not know it.

**4. The PLAN cannot check, and says so.** `apply document` without `--confirm`
opens no transport, deliberately — *"an `--identity`-less run must still be able
to see what would land"* — so there is no machine to compare against. The check
therefore happens only on the confirmed run, and the plan now carries a line
saying that in as many words. **This is residue and not a fix**: an operator can
still be surprised by a refusal after a clean-looking plan.

#### The adoption, and the two lines it will not cross

§6.2 recommended a verb and did not require one. **It was built, as
`add document`** — `ADR-006` §6.4 has nine verbs and no *adopt*, and inventing a
tenth for one operation would be a worse trade than using the one whose gloss
already reads *put something into the register*. It is `apply document` read
backwards, and the symmetry is the point: **`apply` pushes the register's
document at the machine; `add` pulls what the machine holds back into the
register.**

It writes only what the register has **no answer for at all** — every value it
records is one the composed document did not carry. **A value the register holds
and disagrees about is never touched**, which is what keeps §2's *the register is
the authority* true: a tool that copied a machine's answer over a recorded one
would have quietly made the machine authoritative.

One thing it refuses to adopt, and the refusal is about which side gets to
decide:

* **A hostname**, because the composed hostname *is* the device's label
  (`crate::compose::hostname`) — so adopting it would rename the record to
  whatever the machine answers to, and let the machine decide what the register
  calls it.

Its plan **reads the machine**, which no other plan in this tool does. There is
nothing to plan otherwise: what it would record is a fact about the machine and
cannot be composed from the register. Reading is not changing, and `get status`
already reaches a machine with no confirmation.

#### Two smaller things the build picked

* **A machine holding no document passes.** That is a freshly-imaged board and
  the first `apply document` onto it must not be made harder by a guard about
  erasure; there is nothing there to erase.
* **A machine document that does not parse is compared as TEXT**, and the report
  says so. The comparison normally goes through `rivet-config`'s own parser and
  its canonical rendering, so it is over what the document *means* rather than
  over how somebody typed it. When that parser refuses the machine's file,
  comparing the raw text is worse than comparing meaning and much better than
  refusing to look — and a silent downgrade would be worse than both.

##### What has and has not been proved, 2026-08-29

`AGENTS.md` §6. **No board.** None was reached by this work.

* **Proved by test**: the measured case — a register lacking `listen` and
  lacking `services.rivet-mcpd`, against a machine document carrying both,
  refuses and names both losses, both remedies and both costs; an ordinary
  complete register applies with no refusal; a changed value is reported and does
  not refuse; one lost element of a list is a loss of that element rather than a
  change to the list; an empty table is a leaf; three spellings of one key
  flatten to one path; every
  command a remedy names exists in the operation table; and adopting what the
  guard found makes the guard pass. The refusal was **broken deliberately and
  watched to fail** before it was committed (`AGENTS.md` §4).
* **Proved by running the binary**: `add document` is reachable from the command
  line, carries the table's own help, and refuses without `--identity` — the
  refusal an operator meets first.
* **NOT proved**: that any of this has met a Raspberry Pi. No document has been
  read off a board through this path, so the guard has never compared a document
  a *machine* wrote against one this tool composed — only documents this tool
  composed on both sides. Closing it needs one `apply document --confirm`
  against the test board with a register that lacks `listen`: it must refuse and
  name both losses, and `add document --confirm` must then clear it.

---

### 6.3 BUILT 2026-09-09: a resolver modelled as an interface is retired, and the register gets a field for one

**`Device` now holds `[network.resolver]`'s two keys — `nameservers` and
`search` — as `resolver: Option<Resolver>`, a field of its own rather than an
entry of `Device::interfaces`.** `crate::compose::render` (`cmd/rivet-fleet`)
composes it as `[network.resolver]` with `nameservers = [...]` and
`search = [...]`, handed to `rivet-config`'s own parser exactly as every other
table this crate composes is — §2's whole method, applied to the one table
that was not going through it correctly.

#### The defect this retires

A resolver has no `method`, no `address` and no `gateway` —
`rivet-config`'s `Resolver` declares exactly two keys, `nameservers` and
`search` (`rivet-config/src/resolver.rs`). Before `Device::resolver` existed,
the only place a per-network fact could live on a `Device` was `interfaces`,
keyed by name, and one build of `add document`'s adoption
(`cmd/rivet-fleet/src/main.rs`'s `adopt`) reached for that map when it met
`[network.resolver]` on a machine's document — its match arm bound on any
two-segment path under `network` regardless of what the second segment
actually was. It wrote `interfaces["resolver"] = { method: "dhcp", address:
None, gateway: None }` as a **side effect of looking the entry up**, before
checking whether the field it had been handed — `nameservers` — was one it
knew what to do with, and only then discovered it was not, and returned an
error. **The error did not undo the insertion.** `add_document` writes its
record to disk regardless of which particular losses it could and could not
record, so the bogus interface reached the register anyway, and
`crate::compose::render` faithfully rendered it back out as
`[network.resolver] method = "dhcp"` — a document `rivet-config` correctly
refuses, `unknown configuration key network.resolver.method`, because that
table takes `nameservers` and `search` and nothing named `method`. Because the
insertion happened before the field name was even checked, **the nameservers
the loss was carrying were never written anywhere** — the record lost the
only copy of them the register held.

#### What was built

* `rivet_register::Device` gained `resolver: Option<Resolver>`
  (`rivet_register::Resolver`: `nameservers: Vec<String>`, `search:
  Vec<String>`, both raw strings — validated where every other document rule
  is, at compose time, by the parser the composed document is actually handed
  to). Both `serde(default)` and `skip_serializing_if`, so a record that says
  nothing about a resolver renders exactly the bytes it always did.
* `crate::compose::render` composes `[network.resolver]` from that field,
  silent when the field says nothing — matching
  `rivet_config::resolver::Resolver::is_silent` — and never again walks
  `Device::interfaces` for an entry named `"resolver"`; that loop skips the
  reserved name outright as a second line of defence, in case some other
  caller ever builds a `Device` that still carries one.
* `adopt`'s match gained a dedicated arm for `["network", "resolver", field]`,
  ordered ahead of the generic interface arm, that appends into
  `Device::resolver` instead of touching `Device::interfaces` at all.
  `crate::diff::remedy` gained the matching arm, so a refusal naming a
  resolver loss now tells an operator that `add document` will actually
  record it, rather than pointing at `[interfaces.resolver]`, which has no
  field for either key.
* **A record still carrying the retired shape is read, not refused.**
  `rivet_register::read_device` drops a legacy `[interfaces.resolver]` entry
  on the way in. There is nothing in `method = "dhcp"` to migrate into a
  nameserver, so dropping is the whole of the repair, and refusing outright
  would turn every register the retired adoption ever wrote into one this
  tool could no longer open — which would have blocked the exact recovery
  path (`export card-plan` against an existing register) the finding that
  named this defect exists to keep working.

#### What has and has not been proved, 2026-09-09

`AGENTS.md` §6.

* **Proved by test**: a device with nameservers composes `[network.resolver]`
  in the shape the real parser (`Document::parse`) accepts and reads back; a
  device with none composes without that table at all; an interface literally
  named `"resolver"` is never rendered, even when constructed directly rather
  than through a register file; a legacy `[interfaces.resolver]` record is read
  without error, its entry is gone from `Device::interfaces`, and `resolver`
  stays `None` rather than inventing a value the record never stated;
  adopting a machine's two nameservers lands both in `Device::resolver` and
  never creates an `interfaces["resolver"]` entry — the exact case that
  produced the defect. The regression test was **broken deliberately and
  watched to fail** before it was committed (`AGENTS.md` §4): reverting the
  dedicated match arm reproduces the bogus interface entry.
* **Checked against the maintainer's own register, read-only.**
  `export card-plan --device rivet-01` composes and the document inside the
  plan parses, where it refused before. The maintainer's real register still
  carries the retired `[interfaces.resolver] method = "dhcp"` entry on disk —
  this work did not write to it, by instruction — so the composed document
  carries no `[network.resolver]` at all; that is a card that boots and has
  no DNS, which is a real gap and not a defect of this fix. The finding that
  reported this defect separately named the machine's actual nameservers,
  read off it on a different occasion, as `192.168.50.1` and `1.1.1.1`; this
  work did not re-read the machine and takes no position on whether that
  reading is still current. Recovering them into the register needs `add
  document --confirm` run against the real machine, deliberately left undone
  here — see the finding's own instruction to leave the register alone.
* **NOT proved**: that any of this has met a Raspberry Pi. No card built from
  a composed document carrying `[network.resolver]` has been imaged or
  booted, so nothing has confirmed the rendered table is one `/etc`'s
  generator (`rivet_core::etc`) actually turns into a working
  `/etc/resolv.conf` on real hardware — only that `rivet-config`'s own parser
  accepts the bytes.

### 6.4 BUILT 2026-09-09: two more register-only verbs, and `add document` reports what it will not adopt

§6's field table gained a home for `SPEC-05` §5.1's kiosk fact, and two gaps
`SPEC-05` §5 had already named as open closed with it: a device record could
only ever acquire a timezone by adopting one from a machine that had already
answered it, and a kiosk machine's record had nowhere to keep the one thing
§5.1 says it additionally declares.

**`rivetos_register::Device` gained `kiosk_auto_run: Option<String>`**, on
`Device::timezone`'s terms exactly: absent composes no `[kiosk]` table at
all, so every register written before this field existed still composes the
bytes it composed yesterday. `crate::compose::render` (`cmd/rivet-fleet`)
writes it back out as `[kiosk] auto-run = "…"`, and `add document`'s `adopt`
gained a `["kiosk", "auto-run"]` arm — the same shape its `["timezone"]` arm
already had.

**Two register-only `set` verbs, `set pin`'s and `set fleet-role`'s reason
exactly.** Neither `timezone` nor `kiosk.auto-run` is *how the tool
reaches the machine*, which is the property §6.1's both-or-neither binds — a
wrong `[control-plane] listen` is self-sealing, and neither of these is. So
`set timezone --device <name> --timezone <zone>` and `set kiosk-auto-run
--device <name> --application <name>` only ever change the register, and take
effect the next time a document is exported, built onto a card, or written to
a machine. `set timezone` checks its value against `rivet_core::timezone`,
the same grammar `rivet-config`'s own parser applies, and says plainly that it
checks grammar only: whether a zone actually exists needs a real zoneinfo
database, and `cmd/rivet-provision`'s copy of one lives in a binary crate with
no library target — nothing outside that crate can call the function that
checks it, and duplicating its six-hundred-name table here would be a second
copy of a fact `ADR-011` gives one authority. `set kiosk-auto-run` checks only
that the name is not empty, because `rivet-config`'s own parser does not
check the value at all and `SPEC-05` §5.1 forbids checking whether the name
will resolve to an installed application.

**`add document`'s report now names a CHANGE beside every LOSS it finds**,
where before it named losses only. §6.2.1 already rules that a change is
reported and then written by `apply document`; what had not been built is
that `add document` — which never writes a change, only a loss — still owed
the same report, because a register that disagrees with a machine over a
value both sides hold is exactly as real a finding as one the register has no
answer for at all, and a plan showing one kind and dropping the other reads
as complete when it is not. `crate::main`'s `add_document` split its
report-building out of the transport-reaching wrapper into `adopt_report`,
testable without a fake transport the way `set_listen`'s `write_both` already
was, and both its PLAN and its confirmed report call `diff::changes` beside
the loss report.

#### What has and has not been proved, 2026-09-09

`AGENTS.md` §6.

* **Proved by test**: `set timezone` records a well-formed zone and refuses a
  value that is not zone-shaped, naming the check as grammar-only; `set
  kiosk-auto-run` records a name and refuses an empty one; adopting
  `kiosk.auto-run` lands on `Device::kiosk_auto_run` and nowhere else; the
  composer emits `[kiosk] auto-run` when the record names one and no table at
  all when it does not; and `add document`'s report — both the PLAN and the
  confirmed write — names a loss and a change together, constructed to the
  same shape as the register/machine mismatch that went unseen. `cargo
  clippy --all-targets`, diffed with `git stash` rather than eyeballed,
  reports identical findings before and after on both crates.
* **Checked against the maintainer's own register, read-only.** A dry
  `add document --device rivet-01` now offers `kiosk.auto-run` as adoptable
  where it previously said the register had no field for it, and shows the
  `listen` difference alongside the losses it already reported.
  `export provisioning --device rivet-01` still refuses, naming `timezone`
  and `[applications.rivet-proxyd] upstream-name` — unchanged, because
  running `add document --confirm` is the maintainer's own to make.
* **NOT proved**: that either verb, or the changed report, has met a
  Raspberry Pi. No card has been built from a record carrying
  `kiosk_auto_run`, and no `apply document` carrying a timezone this verb
  recorded has reached a board — every write to `rivet-01` and to the
  maintainer's real register stayed out of this pass, on purpose.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### Status block — accepted in part, and nothing open

**Both superseded 2026-08-26.** The superseded text read:

~~**Accepted in part** — ruled 2026-08-26. §2, §3, §4 and §5 are
ruled. §8 is a list of questions that are the maintainer's and are
deliberately not answered here.~~

~~Nothing in this ADR is open.~~

**The second is the one that matters.** §8.1 records it: *a claim of
completeness is a claim, and it rots like any other.* It was false the day it
was written and it is what hid the `rivet-fleetd` references §8.1 went on to
find.

### In §2.2 — Local mode

**Superseded 2026-09-01 — §2.2.1**, and not a correction of an error: it is a
later decision. The superseded table cells read:

~~**Local**~~ | ~~a directory on the operator's own machine~~ | ~~direct file
access~~ | ~~one operator, one workstation~~

~~several operators, or an operator who is not always present~~

and the paragraph under the table:

~~**Local is the default and Remote is the escalation.** A home LAN with one
administrator needs no daemon, and `ADR-009` §3.1's argument applies here as
much as anywhere: a service whose only client is the person sitting at the
machine is a component with no job.~~

### In §2.2.1 — the register's durability was open

**CLOSED 2026-09-01 — it borrows `rivet_store::atomic::commit`.** The superseded
half of the bullet read:

~~Recorded in `rivet-register/src/store.rs` and **open**: the fix is either to
borrow `rivet-store`'s write-and-rename or to keep the register somewhere that
already has those guarantees, and that is a decision rather than a patch.~~

### In §5.4 — how many of `apply`'s subjects cannot answer

**Corrected twice on 2026-08-29.** The superseded counts read:

~~Two of `apply`'s seven subjects~~ ~~Three of `apply`'s sixteen subjects —
corrected 2026-08-29~~

### In §5.4 — the register's interface was said to be unbuilt

**Corrected 2026-08-29.** The superseded cell read:

~~not built — Phase 3.~~

Its conclusion was right and its reason was wrong, which is worse, because it
reads as scheduled work rather than as a gap.

### In §5.4 — every setting answered `on-next-restart`

**FALSE since 2026-08-26 — corrected 2026-08-29.** The superseded paragraph
read:

~~**Every setting in this build answers `on-next-restart`**, and that is one fact
rather than one per key: **nothing re-reads the configuration document while it
is running.** PID 1 reads it at boot, the supervisor reads it when it starts and
compiles its service set then, and `rivet-netd` is handed the path when it is
started. The schema's root description says so once and names all three readers.
`ADR-032` §3.2's `SIGHUP` is what will move some of them to `immediately`, and
`rivet-config`'s `every_setting_publishes_when_a_change_to_it_takes_effect` is
written to **fail** when it lands — so the settings that move are moved in the
same change rather than discovered by an operator whose service did not restart.~~

**How it survived is worth more than the correction**, and the body says so:
`rivet-config` struck its own copy of the sentence in place, in the house style,
on the day the guard test fired. The ADR's copy was left.

### In §8 — the section heading

**All eight questions ruled 2026-08-26.** The superseded heading word was
~~Open~~.

### In §8.1 — the four `rivet-fleetd` references were not yet removed

**Overtaken 2026-09-01 — §8.2**, when `rivet-fleetd` was built and there was
nothing left to strike. The superseded line read:

~~**Not done.** The four references still stand and the status block still says
it.~~

### In §9 — nothing was built, and the interface had not been seen

**Built 2026-08-26; the interface was run on a real terminal 2026-08-27.** The
superseded text read:

~~**Nothing here is built.**~~

~~the largest of them is that the full-screen interface has never been looked at
on a real terminal.~~

~~the section it must be read beside is `PLAN.md` §24.5 — the standing list of
what is not proved.~~

### In §11.1 — two of the three example commands did not work

**Corrected 2026-08-29**, and they are the only commands in this document a
reader would copy. The superseded spellings were ~~get device kitchen-pi~~ and
~~apply provision …~~.

### In §11.5 — a second remote transport was allowed to stay

**REFUSED 2026-08-29 — §11.5.1.** The row's verdict was ~~**stays**~~, argued
from `PLAN.md` §19.1: that reusing an operator's already-configured client was
worth more than replacing it with a library. The argument was sound while the
board served that transport, and stopped meaning anything when it did not.

### In §11.5.1 — the Monitor pane itself did not change

**Amended 2026-08-29** when it was built, because the same section goes on to
require an indicator the pane did not have, and the two sentences together were
not consistent. The superseded sentence read:

~~The pane itself does not change.~~

### In §11.1 and §11.5.2 — `rivet-fleet apply deploy`

**Removed 2026-09-12**, by the maintainer's ruling recorded in
`PLAN-Feature-Set-2026-09-12` §2 and §3.4: one deployment surface. The
superseded list in §11.1 read:

~~`apply` takes four: `document`, `deploy`, `repo-delete`, `trial-restart`.~~

`apply deploy --device <name> --rvt <file>` sent an already-signed `.rvt` to the
machine the register resolved, through the same `rivet_remote::deploy::run`
`cargo rivet deploy` performs. **It went because it was a second way in.** The
fleet repository (`ADR-052`) makes deployment select from the repository, and a
second surface that takes a path to any file is how software kept reaching
machines without passing through it. The register's job is the fleet's records,
not shipping software; `cargo rivet deploy` is where a deploy happens, and
§11.4's ruling — composing a document and applying it to a member, and
trial-booting it — is untouched.

### In §6.1 and §6.2 — two rulings were unbuilt

**Both BUILT 2026-08-29 — §6.1.2 and §6.2.1.** The superseded bullet, twice,
read:

~~**Not built.**~~

### `Fleet::greeting` — removed 2026-09-13

**Removed, `.rulings/` R-168.1.** The fleet's `greeting` field composed a
`[greeting]` table into every machine's document; `add document` refused to
adopt one from a machine, on the same footing this ADR still gives a
hostname. Both the field and the refusal are gone with `[greeting]` itself,
by the maintainer's ruling that a removed feature owes no accommodation to a
document that no longer exists. SSH and login sessions were removed long
before the field was, so nothing was left to greet.

### In §8.2 — every fleet machine passed as the operator at 4443

**Changed 2026-09-14, `.rulings/` R-187 D1.** The superseded sentence read:

~~That is a known defect in every `/api/*` and `/mcp/*` route this program
serves, proved over real TLS and carried by `ADR-043` §5.3~~

It was true: the handshake was the whole check. The maintainer chose to tell
the operator apart by the property the two kinds of leaf already differ in —
a machine's names its host, the operator's names none — enforced in
`rivet-fleetd` and `rivet-proxyd` with no certificate reissued. §8.2's point
that this is an inference and not a pinned key stays in the body.
