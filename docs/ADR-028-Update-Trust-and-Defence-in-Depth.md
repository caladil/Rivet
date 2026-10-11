# ADR-028: Update Trust and Defence in Depth

* **Status:** Accepted
* **Date:** 2026-08-23
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`ADR-015` §4 signs artifacts, and `ADR-035`'s control plane carries them.
Between the two sat a sentence that made the transport sound like a
convenience:

> **Note what this does *not* rest on.** `ADR-015` §4 signs images, so an
> update's authenticity comes from the signature and not from the pipe. The
> transport buys confidentiality and saves downloading garbage; it may be
> replaced later without weakening the thing that makes an update trustworthy.

**That sentence is struck.** It is not false — a signature really is what proves
the bytes are ours — but it answers *authenticity* and was being used to dismiss
**confidentiality**, **freshness** and **authorisation**, which are three
different properties a signature does not touch.

**Raised by the maintainer, 2026-08-23**, after an agent asserted the sentence
three times in one afternoon; his point was that defense in depth is a real
thing. *(`.rulings/` R-273.)* He was right, and the framing was in this
repository's own record, so an agent repeating it was repeating us.

This ADR spans `ADR-015` (signing), `ADR-018` (updates), `ADR-019` (enrollment)
and `ADR-025` (fleet identity), which is why it is its own file rather than a
section in any of them.

---

## 2. What a signature does not give you

| Property | A signature… | Consequence if nothing else supplies it |
| :-- | :-- | :-- |
| **Authenticity** | **provides it.** The bytes are ours. | — |
| **Freshness** | does not. A signed *old* image is still validly signed. | A replayed, correctly-signed, known-vulnerable image installs cleanly. §6. |
| **Confidentiality** | does not. | An observer learns which version every machine runs — a fleet-wide inventory of which boxes are unpatched. Some of those boxes hold their owner's personal data. |
| **Authorisation** | does not. It says nothing about *who may say "install it"*, which one, or when. | The artifact is genuine and the order to apply it is anyone's. §4. |
| **Availability** | does not. | An on-path attacker who simply withholds updates leaves a machine vulnerable indefinitely, reporting perfect health. |
| **A safe verifier** | does not — it *is* the thing being protected. | Everything upstream of the check parses attacker-controlled bytes. |

**The last row is the one that inverts the original sentence.** "The signature
makes it trustworthy" treats verification as a wall. It is a *parser*, and it is
the most security-sensitive parser on the machine. Authenticating the channel
first is what keeps hostile input away from it, which makes the transport part
of the defence rather than an optimisation on top of it.

**So there is one implementation of it, and it may be consulted wherever an
artifact must be judged.** `ADR-029` §11 draws the consequence — *"three formats
means three verifiers, each with a third of the scrutiny"* — and what that rules
out is a second **implementation**, because two can disagree and the weaker is
the one that eventually gets believed. A second **call** of the one
implementation cannot disagree with the first about the same bytes and the same
trust store. On a machine it is consulted at three points:

| Where | When | Against the trust store as it is… |
| :-- | :-- | :-- |
| `cmd/rivet-mcpd`'s bulk endpoint | before an artifact is named in the machine's store — `ADR-052` §7.2.3, since 2026-09-12 | when the artifact arrives |
| `cmd/rivet-deploy` | before an install writes anything — `ADR-029` §9.2 | when the install runs |
| `cmd/rivet-sysd` | before every launch of an installed application — `ADR-015` §4.2, header only | at that launch |

The first two call `rivet_core::package::verify`, the sequence that builds the
trust set from `/system/trust` and reads the header and every member through
`rivet_core::package::read`. The third calls `read::open_installed` directly,
because at launch there is no file to stream — only the retained header — and
still builds its own trust set from the store; converging that onto
`verify::trust_set` is owed. **Each verdict is true of the trust store it was
reached against**, which is why an earlier consultation never excuses a later
one: `§5.5.4` makes revocation an update that replaces the file.

**Both ends of the channel are authenticated before any of the three runs** —
the control plane's mutual TLS for the first, a supervised process reading
`/state` for the other two — so the parser's input is never simply whatever
arrived off a network.

---

## 3. The layers, and what each one is for

Defence in depth is not "more security is better". It is that **each layer
covers a failure the others cannot**:

| Layer | Covers the case where… | Exists? |
| :-- | :-- | :-- |
| The control plane — mutually authenticated TLS, pinned Ed25519 keys, `ADR-025` fleet identity | the **signing key leaked**. An attacker holding a valid signature still cannot deliver, and cannot read the version inventory off the wire. | **Built** — `ADR-035`, proven on hardware |
| Artifact signature — `ADR-015` §4 | the **channel is compromised**. A machine-in-the-middle cannot forge an image. | Ruled, not built |
| Authority for the instruction — §4 | the **artifact is genuine and the order is not**. | Ruled here. **Not built** — §4.5 built *attribution* for the instruction and no layer checks authority |
| Root of trust outside configuration — §5 | the **device's own configuration was written**. | Ruled here |
| Version monotonicity — §6 | **every layer above held and the artifact is simply old**. | **Deferred, §6** |

**Note the second row is the argument for signing, and the first row is the
argument for an authenticated transport, and they are not the same argument.**
Reading either as sufficient is how a layer gets dropped for being redundant.

---

## 4. Ruled: authority and authenticity are different, and come from different places

**The maintainer's ruling, 2026-08-23:** an authorized user, locally or
remotely, provides the authority, and the manifest verifies the authenticity.
*(`.rulings/` R-274.)*

* **Authority** comes from an **authenticated principal** — a person or an agent
  — acting through the administration surface, at the console or over the
  control plane.
  `ADR-012` already requires actor provenance from the start, and `ADR-006` §6
  already requires a `plan` the principal confirms. An update is that, not a
  special case.
* **Authenticity** comes from the signature over the manifest and the artifact
  as one unit (`ADR-015` §4.1).

**Neither substitutes for the other**, and the failure of either is survivable
because the other still holds. A stolen signing key produces artifacts nobody is
authorised to install. A compromised operator account can order the installation
of nothing it cannot also sign.

### 4.1 The line is where authority comes from, not when it is exercised

**An earlier draft of this section said "there is no autonomous update path".
That was wrong twice and is corrected rather than quietly reworded**, because
the error is instructive: it conflated *acting without authority* with *acting
without a human present at the moment of action*.

* It **contradicted `ADR-012` §7.1**, ruled 2026-08-22, which says the unit of
  confirmation is *"the change, not the target"* — one plan for a fleet-wide
  change, confirmed once, applied to every machine in scope — and states plainly
  that **"unattended apply exists"** as a scope on a credential.
* It **contradicted §6.2 of this same ADR**, which rules monotonicity as a fleet
  *policy*. A policy is a standing instruction. Forbidding standing instructions
  three sections earlier was incoherent.

**The maintainer's objection, 2026-08-23:** if the rule meant he could not tell
the fleet server to automate the updates of its members on an administrator's
instruction, he asked why not. *(`.rulings/` R-275.)* There is no good answer,
and the record already agreed with him.

**What §4 actually forbids** is a machine acting on authority **nobody granted**
— a fleet server that polls an upstream and installs what it finds is deciding,
and no principal is answerable for the result. **What §4 permits**, and what
`ADR-012` §7.1 already ruled, is a grant made once and executed many times
without a human in the loop for each machine.

### 4.2 What a standing instruction has to carry

The risk of a standing grant is not that it is unattended. It is that a grant
made in March is still executing in December. Four requirements, three of which
`ADR-012` already supplies:

1. **A scope** — what was granted, not merely that something was.
   `ADR-012` §5's capability scoping. *"Roll this plan out to these members"* and
   *"install whatever appears, indefinitely"* are different grants and must not
   be spelled the same way.
2. **Attribution that survives the delay.** An unattended rollout's journal
   records name **the principal whose grant it was and when the grant was
   given** — never "the fleet server did it". `ADR-012` §4's actor provenance,
   applied to an actor who is not present.
3. **Revocation that takes effect before the next execution**, not before the
   next rollout is planned.
4. **A bounded life.** This is the one nothing currently supplies, and it is the
   *authorisation* analogue of §6's artifact-freshness problem — a grant, like a
   signature, does not go stale on its own. Expiry, a version ceiling, or
   re-confirmation when the scope changes; §8 item 5.

### 4.3 Automation does not weaken `PLAN.md` §9.4, and it is worth saying why

§9.4 rules that the administrator instructs the fleet server to **update itself
first, and then the fleet**, because a bad image then breaks one machine and
rolls back before reaching the others.

**That protection is the ordering plus `ADR-018` §5's rollback — not the
presence of a human between the two steps.** A fleet server that fails its own
update reverts on the next boot and therefore never reaches the step that
touches members. The safety property survives automation intact, which is why
automating it is a scheduling decision rather than a security one.

### 4.4 What this buys on instruction replay, for free

An instruction that arrives inside a mutually authenticated TLS session cannot
be replayed without replaying the session, which the handshake already prevents.
**The freshness problem for *instructions* is therefore solved by the channel** — and
solved by the same layer the struck sentence called replaceable.

**Three different freshness problems, and only one of them is solved:**

| What can go stale | Solved by | State |
| :-- | :-- | :-- |
| The **instruction** — an order replayed later | the authenticated session | **Solved**, and free |
| The **artifact** — a signed but older image | nothing yet; §6's fleet policy when it lands | Open |
| The **grant** — a standing authority still firing months on | nothing yet; §4.2 item 4 | Open |

They are separate because they fail separately: a fresh instruction can carry a
stale artifact, and a stale grant can issue a perfectly fresh instruction to
install it.

### 4.5 Built 2026-08-24: the install request names who asked, and that is attribution rather than authority

**The maintainer's instruction, 2026-08-24, on being shown that
`rivet_core::deploy::ACTOR` documented the gap rather than closing it: *fix
it*.** This section is what was built, and — more importantly — the boundary of
what it means. Read §4.5.2 before quoting §4.5.1 anywhere.

#### 4.5.1 What it is

the maintainer's 2026-08-24 ruling made `/state/deploy-request` the channel: `rivet
apply install <digest> confirm` writes it and the supervisor consumes it on its
next convergence pass. As first built it carried **one absolute path and nothing
else**, so the only answer the machine had to *"who asked for that install"* was
*"somebody who was root"*.

It now carries a principal, on the same single line:

```text
from=pinned-3f9a…c21b -- 8c4e9b21f0a37d5e…39ad84f2c
```

**The payload half became a DIGEST on 2026-09-12** — `ADR-052` §7.2. It was an
absolute path, and that is what made this file worth more to an attacker than it
is now: a request could name *any* file on `/state`, so writing one bought the
ability to name something the fleet repository never saw. A digest resolves in
the machine's own artifact store and nowhere else. **The trust boundary is
unchanged and is still root on `/state`** — §4.5.2 below is untouched, and this
is a narrower capability rather than a new wall.

There is **one** field, and a marker. `rivet_core::deploy::Origin` is the closed
set of things it may say:

| Token | Where it comes from | What it is |
| :-- | :-- | :-- |
| `from=pinned-<hex>` | `RIVET_CONTROL_PLANE_PEER`, set by `cmd/rivet-mcpd` for the CLI it spawns | the **Ed25519 public key the caller was pinned by** — `ADR-025` §2. The strongest thing a request can say about who asked |
| `from=serial-console` | `ADR-035` §8A.1.2's break-glass | **somebody was physically at the machine.** Physical presence is the credential, so there is no key to record |
| `from=no-session` | nothing in the environment named a caller | *nothing was visible*, said rather than guessed at (`AGENTS.md` §5) — and explicitly **not** "somebody was at the console", which is what the token above means |
| `-- <digest>` | the operator's argument | the artifact, by its BLAKE3 digest, resolved in this machine's own store (`ADR-052` §7.2). Last because it was once the only field that could contain anything, and the order is kept: the grammar being positional is what refuses the pre-2026-08-24 format with one comparison |

`rivet_core::deploy` owns the format — one writer, one reader, and after this
change still exactly one of each. The reader is
`rivet_core::deploy::parse_request`; the record is written by
`cmd/rivet-sysd`'s `deploy_request` module, as a `principal` field and a
sentence.

**The chain of custody, and why it needs no new cryptography.** The pinned key
**is** the identity: nothing has to be matched against a log line to reach a key,
because the key is what was written down. `ADR-025` §2's map turns it into a
device or an operator without anything here inventing cryptography or carrying
any secret.

**`no-session` was renamed on 2026-09-06** and this is the one rename a wire
token has had. A token is part of the interface contract precisely because a
reader may be matching it against a record years old, on a machine whose journal
is the only account of what happened (`ADR-022` §8). It was renamed anyway
because it had been named after a mechanism this system no longer has, and a
token naming something absent is not a contract but a fossil. **The cost is paid
out loud rather than hidden:** a request written by a build older than that date
is `RequestProblem::BadPrincipal`, which says in a record that the value was not
one this build accepts — rather than being installed unattributed.

#### 4.5.2 What it is **not**, and this is the load-bearing half

**It is attribution, not authorisation. Nothing checks anything.**

* **The trust boundary is unchanged**: root on `/state`. Anything running as uid
  0 can write `/state/deploy-request` by hand and put any principal in it, and
  nothing at this layer could prevent that — `ADR-015` §7 already gives that
  attacker `/state/app`. The unprivileged boundary is the one being held, and it
  is held by `cmd/rivet-image` creating `/state` as `root_owner=0:0`.
* **`RIVET_CONTROL_PLANE_PEER` is an environment variable.** Anything that can
  run the CLI as root can set it to whatever it likes. The CLI's own uid is root
  on every invocation, so it distinguishes nothing on its own. The record is
  **self-reported and says so**; `rivet_core::journal`'s `Peer` — filled by the
  kernel from `SO_PEERCRED` and unforgeable by the sender — answers the
  *different* question of which process on this machine wrote it, and the two are
  kept apart deliberately.
* **Holding the private half is not the same as being authorised by it.** The
  key names whoever holds it, which is a fact about the connection rather than
  about a person.
* **§4's first half is still unbuilt.** §4 wants a principal whose authority is
  *checked*. Nothing here is checked. What this supplies is `ADR-012` §4's actor
  provenance and `ADR-022` §5's actor for an event that had neither.

**What is genuinely gained is falsifiability.** A claim that can be contradicted
is different in kind from no claim at all: a request naming a session that never
authenticated is refuted by the same journal it was written into. Before this,
there was nothing to refute.

#### 4.5.3 A request that names nobody is REFUSED, and the argument for it

`rivet_core::deploy::RequestProblem::NoPrincipal`. A file that is one bare
absolute path — the format Rivet wrote until this change, and also what an
operator's `echo` produces — is refused, removed, and reported, and no install
starts.

The case for accepting it and recording *"nobody was named"* is real: refusing
costs an install, and an attacker who is root can forge a principal anyway, so
refusing stops no attack. It is rejected for three reasons:

1. **There is exactly one writer of this format**, and after this change it
   always writes a principal. A file that is not in the format was not written
   by `rivet apply install` — which is the same reasoning the existing
   `ControlCharacter` refusal already states in as many words.
2. **Accepting it would make the field decorative.** A permanently available,
   silent way to install with no attribution is worse than no attribution field,
   because the field would then read as a guarantee it does not give.
3. **Refusing costs almost nothing here.** The only writer is a CLI shipped in
   the same image as the reader — `/system` is replaced wholesale by an update
   — so the version skew that would make this expensive does not arise on a
   machine, and the cost when it does is one install asked for again, said out
   loud in the journal with the command that writes a whole request.

This is `ADR-007` §3.6.5's posture applied one layer along: an input this build
cannot understand is **refused rather than partially applied**.

#### 4.5.4 `rivet get install` does not report it, and says where it is

The principal reaches `cmd/rivet-sysd`'s record of taking the request
up. It does **not** reach `cmd/rivet-deploy`: `definitions::deploy` hands the
installer one argument, the payload path, so every record *inside* an install
still carries `rivet_core::deploy::ACTOR` — the process, not a person. That is
honest, and it is why `rivet get install` cannot answer the question: its
chain is the installer's records grouped by `ADR-022` §5's correlation
identifier, the supervisor's record is neither, and **there is no key to join
them on**. A join on *"the last request note before this chain began"* would be a
guess dressed as an answer.

So both verbs point at the supervisor's own records instead of leaving
an operator to notice an absent field.

**This once said both verbs point at `rivet get log rivet-sysd`, and it was
corrected 2026-08-30 by the second cold-agent run.** `get log`'s declared inputs are the subject, the
boot selector, the limit and a cursor — **there is no service filter**, so
`rivet-sysd` bound to the boot selector and the machine answered
`"records":[],"returned":0`, which reads as an empty journal rather than as a
question that missed. The advice now says *read `rivet get log` and select the
records whose `svc` is `rivet-sysd`*, and `rivet_core::operation::log` refuses a
boot selector naming a boot the journal does not hold rather than answering it
with nothing — because a corrected sentence can go stale again and a refusal
is read at the moment of the mistake. `ADR-006` §8.13 carries the wider finding
that produced it.

**The honest fix is §8 item 6**, and it is the maintainer's: carry the principal
on the installer's argument vector so that every record of an install has it.
That changes `cmd/rivet-deploy`'s argument contract, which is a decision about
the shape of the software (`AGENTS.md` §1).

#### 4.5.5 What was run, and what that settles

Watched under QEMU on **Rivet's own kernel** (`ADR-026` §4.1),
`tools/qemu/run.sh --deploy-request` with the new `--deploy-from`,
`--deploy-key` and `--deploy-raw` flags, three boots:

| Request packed | What the journal showed |
| :-- | :-- |
| a request naming a caller | the supervisor consumed it and recorded the principal and the sentence; `rivet-deploy` started and refused for want of a trust store |
| a request naming no caller | the same, recording the no-caller token and the sentence denying the console in as many words |
| a bare path (the pre-2026-08-24 format) | `REFUSED`, removed, **nothing started**, and the refusal named `rivet apply install` as what writes a whole request |

The tokens those runs carried are not the ones §4.5.1 lists — they were renamed
on 2026-09-06 and git holds the transcripts. What the runs settled is the
supervisor's *behaviour*, which the rename did not touch.

**Re-run on 2026-09-12**, when the payload half became a digest, and the same
three boots under the same harness settled the same behaviour one step further:
a request naming a digest the store holds is **resolved** to the object's path
and adopted, with the record carrying both; a request naming a digest it does
not hold is `REFUSED`, removed, nothing started; a request naming a **path** —
which is now the old format — is refused by the parser rather than resolved.

**`ADR-026` §4 is what that does and does not settle.** QEMU settled the
supervisor's behaviour against the real target kernel. It settled **nothing**
about the half of this that only a board has: whether `cmd/rivet-mcpd` actually
puts `RIVET_CONTROL_PLANE_PEER` into the environment of the CLI it spawns, so
that a request written there names the key the caller was pinned by.

**CLOSED on hardware 2026-09-06 — `ADR-022` §11.8.1.** A mutating operation over
the control plane on `rivet-01` produced a journal record carrying
`"principal":"pinned-5108de…"` beside the kernel's own `ingest_uid`,
`ingest_gid` and `ingest_pid`. **The whole chain is observed**: a peer's key, the
daemon's environment, the CLI's read of it, the ingest socket, and the
supervisor's stamp — on one record, on a real board.

**CLOSED TOO, 2026-09-06 — `PLAN.md` §153.1.** The narrower half was that an
`apply install` *specifically* carries it, and an install over the control
plane recorded `"principal":"pinned-62bc792e…"` beside its payload. The whole
chain is observed for both operations now.

---

## 5. Ruled: the root of trust lives in the image, not in configuration

**Ruled 2026-08-23.** The maintainer delegated this one explicitly, so it is
written to be read and overturned rather than taken on faith.

### 5.1 The problem, in plain terms

`ADR-015` §4 made the set of trusted public keys **configuration**, for a good
reason: rotating a key should not mean rebuilding the operating system.

But configuration lives in `/state/config`, and `/state` is the one writable
filesystem (`ADR-018` §4). So **anything that can write configuration can add
its own key to the trust set — and from that moment every artifact it forges
verifies correctly.** The mechanism protecting updates was editable by the
machine the updates protect. That is not a layer; it is a loop.

### 5.2 The ruling

**The image carries a root key set that configuration cannot alter, and a
configuration-supplied key is trusted only if it is countersigned by a key
already trusted.**

* The root set ships in `/system`, which is read-only *by format* (`ADR-018` §3)
  and replaced only wholesale by an update that was itself verified.
* Configuration may still add and rotate keys — `ADR-015` §4's requirement is
  met — but it can only add a key the holder of an existing trusted key has
  vouched for. **Configuration can no longer introduce a trust anchor on its
  own.**
* Removing keys is permitted, **except the root set**, which configuration
  cannot remove. Removal otherwise reduces trust, which is the safe direction;
  removing the floor is how you would strand a machine on an attacker's key.

### 5.3 Why not the simpler options

| Option | Rejected because |
| :-- | :-- |
| Bake one key in, no configuration at all | `ADR-015` §4's objection stands: rotating it means rebuilding the OS, and a key that cannot be rotated will not be. |
| Leave the trust set as ordinary configuration | §5.1. The thing being protected can rewrite the thing protecting it. |
| A full PKI with certificates and revocation | `ADR-025` already rejected a CA for fleet identity, and for the same reason: Rivet has **one signer** for the foreseeable future. A countersignature is the whole of what a delegation needs at this scale. |

### 5.4 What this does not claim

It does not stop an attacker who can write `/system`, because at that point they
can replace the verifier. `/system` being read-only by format and replaced only
by a verified update is what makes the floor a floor — the guarantee is
`ADR-018` §3's, and this section rests on it rather than adding to it.

---
### 5.5 Ruled 2026-08-24: a key is trusted **for a set of kinds**, and that is the whole of the delegation Rivet needs now

§5.2 ruled *where* the root of trust lives. It did not say what a trust entry
**is**, and the gap became load-bearing the moment `cmd/cargo-rivet` began
producing signed application packages: `TrustSet` holds bare public keys, so a
key trusted at all is trusted for **everything**.

**The maintainer's framing, which is the reasoning of this section. He asked for
a *system image signing key*, used for the system image and for the extensions
and packages that are PART OF Rivet, kept separate from a personal signing key
used to sign an application package for one of his own projects. The entire
reason he separated the system identity from his own in the first place was that
he wanted the two certificates to have different roles.**
*(`.rulings/` R-045.)*

**Ruled: yes, and the separation is a capability rather than a label.**

#### 5.5.1 Why a label would be worthless

Two keys that are both simply "trusted" buy nothing but bookkeeping. The
property worth having is this:

> **The key that signs `Trends` must not be able to ship a base image.**

Without that, compromising the laptop that signs a hobby application yields a
signed operating system for every board in the fleet. With it, the blast radius
of that compromise is one artifact kind.

`ADR-029` §8 already puts `<kind>` **inside the signed header**, and
`read::open` already parses it. So the check costs one comparison against a
field that is already authenticated by the time it is read.

#### 5.5.2 What a trust entry carries

```
public key      32 bytes   Ed25519, raw (ADR-029 §7.1)
key id          32 bytes   BLAKE3 of the public key — what the header names
may sign        bitmask    over ADR-029's kinds
label           text       "rivet-system", "joel-apps" — for refusals, never for authority
```

**The mask names an arbitrary subset, and system/extension are NOT welded
together.** In practice the Rivet key gets `{system, extension}` and an
application key gets `{application}` — but nothing in the verifier *assumes*
that pairing, so handing a third party the right to ship one extension later is
a configuration change rather than a redesign. Ruled 2026-08-24 (option (a)).

**`card` joined `rivet-system`'s mask 2026-09-13** — `R-176` Q5, `R-177`: the
Rivet key gets `{system, extension, card}` now, and a key entitled only to
`application` is refused for a `card` exactly as it already is for `update`.
A card image is Rivet's own artifact in the same sense a system update is —
`cargo rivet image` never lets a third party's key sign one — so it joins the
Rivet key's mask rather than getting one of its own. Proven by
`rivet-core::package::verify_tests::a_card_signed_by_a_key_not_entitled_to_card_is_refused_by_name`.

**Every image `tools/card/build.sh` builds carries that mask in its trust
store** — `rivet-system` for `update`, `extension` and `card` — so a fleet
server's repository store admits the cards it would restore a host from
(`ADR-063` §6). The system key already signs system updates, which are strictly
more powerful than a card, so trusting it for `card` hands it nothing it could
not already do. **Trust is not the only question a machine's own store asks**:
`cmd/rivet-mcpd`'s install door (`PUT /payload/`) refuses a `card` by kind
whoever signed it, because a card is written to a card on a workstation and
never installed on a machine. Only the fleet server's repository store holds
one.

**`apk` joins the application key's mask** — `.rulings/` R-222, `ADR-061` §3.3:
the application key gets `{application, library, apk}`. An APK is the
maintainer's own application in the sense an `application` is, so it takes the
application key's mask rather than one of its own, and an `apk` signed by the
system key is refused exactly as an `application` signed by it is. The mask
lives in the image's trust store, so a machine learns it by system update.

**The label carries no authority.** It exists so a refusal can say *"signed by
`joel-apps`, which may not sign a system image"* instead of *"untrusted key"* —
which is `ADR-006`'s explainability argument applied to the one place in the
project where a bad diagnosis costs the most.

#### 5.5.3 The verifier already has the right shape, and that is worth recording

**Nothing about key *selection* changes**, because `ADR-029` §3 built it
correctly before this question was asked. `key_id` at offset 24 is a digest of
the public key, it sits **above** `UNAUTHENTICATED_BOUNDARY` so it can be read
before anything is trusted, and `read::open` does `trust.find(&key_id)` and
raises `UntrustedKey` when it misses.

Two properties fall out of that and are hereby **required rather than
incidental**:

* **The named key is the only key tried.** A verifier that fell back to trying
  every other trusted key after the named one failed would convert a signature
  failure into a search — and a search that succeeds is a downgrade path.
  Ruled 2026-08-24 (option (a) of question 4).
* **Naming a key you do not hold buys nothing.** The `key_id` is
  attacker-controlled, and that is fine: it selects which public key to verify
  *against*, and the signature still has to check out. Recording this because
  "the header names its own key" reads like a hole and is not one.

The delta this section actually asks for is therefore small: `TrustSet` entries
gain a mask and a label, and `open` gains one refusal — the signature verified,
the key trusted, **and the kind not permitted to it**.

#### 5.5.4 Expiry: none, and revocation is an image update

Ruled 2026-08-24 (option (a) of question 5).

> **AMENDED 2026-08-25. This section called itself "a complete answer" and it
> was complete for UPDATES only.**
>
> An update is verified **every time one is installed**, so a trust store that
> no longer holds the key genuinely stops the next one. An **application** was
> verified once, at install, and then run for ever — so revoking a key stopped
> future installs and did nothing whatever about what that key had already put
> on the machine. The application kept running, across reboots, indefinitely.
>
> **This section and `ADR-015` §4.2 are only consistent with each other once
> §4.2 is built**, and neither said so. §4.2 was the unbuilt one.
>
> The maintainer put it as the question that settles it: whether verifying at
> launch is not the only way to be able to revoke the key behind an installed
> application and so stop it running if it is found to be malicious or
> corrupted. He accepted the cost in the same breath, which is that
> revoking a key stops **every** application signed by it.
> *(`.rulings/` R-276.)* That cost is a
> **key-granularity choice rather than a structural one**: the store already
> carries several labelled keys with independent masks, and per-application or
> per-vendor keys narrow it whenever that is wanted. §5.5's rejection of a full
> PKI is unaffected — one signer is still one signer.
>
> **Built 2026-08-25** (`cmd/rivet-sysd/src/verify.rs`): the installer retains
> the signed header beside the install, and the supervisor re-verifies it
> against the trust store **as it is at that moment** before every spawn. The
> store is read on each launch and never cached, because a cached one answers
> with the store the machine booted under — and this section's whole mechanism
> is replacing that file.
>
> **What the revocation still requires is a reboot**, and that is this
> section's own doing rather than a limitation of §4.2: the trust store is in
> `/system` (§5.5.6), an A/B update installs into the *inactive* slot, and the
> new store is only in force once the machine boots it. So the sequence is
> ship → reboot → the application stops launching. A revocation list on
> `/state` would bite sooner and is **not** proposed: it would be
> attacker-writable state on the one filesystem an update cannot repair, which
> is the trade §5.5.6 already refused.

Short-lived delegations are the conventional answer and are **wrong for this
project**. They imply a re-signing cadence nobody maintains on a home fleet,
and an expired trust entry does not fail safe — it removes the ability to update
the machine that needs updating, on a headless board, which §6 already
identifies as indistinguishable from bricked.

**Revocation is `ADR-018`'s A/B update**: ship an image whose trust store no
longer holds the key. That is a mechanism this project already has, already
tests, and already trusts, and for a fleet the operator controls it is a
complete answer — unlike CRL or OCSP, which are infrastructure that would never
be run and would therefore fail open.

#### 5.5.5 An image with no trusted key must not be built by accident

Ruled 2026-08-24 (option (a) of question 2).

`cmd/rivet-image` **refuses** to produce an image with an empty trust store,
unless `--no-trust` is passed explicitly — in which case the image is stamped
development-only and the supervisor says so **on every boot**, at
`Severity::Warn`, naming the consequence: this machine cannot verify an update
and cannot be updated safely.

This is `AGENTS.md` §3 in its usual form — *stubs refuse, they never fake
success*. An image that silently cannot verify updates is not discovered at
build time; it is discovered on a board, later, by someone trying to fix
something else.

#### 5.5.6 Where the trust store lives

Ruled 2026-08-24 (option (a) of question 1): **a file in `/system`**, not
constants compiled into the binaries.

Security is not the discriminator — both sit on the same unverified SD card, and
§5.4 already concedes that an attacker who can write `/system` has already won.
What decides it is that **a machine should be able to say what it trusts.**
`rivet get trust` returning the labels, key ids and masks is a machine fact in
`ADR-006` §3's sense; a constant in a stripped binary is a fact nobody can ask
for. It also puts the trust store inside the thing an update replaces, which is
exactly what §5.5.4 makes the revocation mechanism.

##### `get trust` is BUILT — 2026-08-29

Pile B item 3, `PLAN.md` §102.3 item 2. **For five days the argument above was
the reason for a file that nothing could read back**: the store landed on
2026-08-24 and the only things that could open it were the two binaries about to
verify something with it, so *"will this board install what I signed"* was
answerable only by signing something and trying. `rivet_core::package::trust_store`'s
own module documentation has said *"`rivet get trust` is how a human asks"*
since the day it was written, which made this the rare gap that was **documented
as present before it existed** — `AGENTS.md` §7's defect pointing forwards.

What it answers, per key and in **file order**:

| Field | Why it is there |
| :-- | :-- |
| `id` | `BLAKE3` of the public key — **the field that joins an artifact to a machine**, since `ADR-029` §5 step 6 makes verification a table lookup on it and `cargo rivet inspect` prints it for an artifact. Reported in **full** hex, never `KeyId`'s eight-byte `Display` form: a truncated value that looks comparable is worse than none |
| `key` | the 32 raw bytes as hex, which is what an operator has in the `.pub` file beside their seed. The two answer different questions |
| `label` | §5.5.1's name for refusals. **Never an authority** — nothing is decided by comparing it |
| `may_sign` | the `<kind>` tokens. An **empty** list is a real state rather than a missing value: §5.5.4 makes it what a revoked entry becomes before the image carrying its removal is built |

**An empty store is an ANSWER and an unreadable one is a REFUSAL**, and keeping
those apart is the whole of the subject's honesty. §5.5.5's `--no-trust` image
really does trust nobody, and that is a reading. A store that is missing or will
not parse is refused in `cmd/rivet-deploy`'s own words for the same file: *"a
store that cannot be read is not a store that trusts nobody, and treating it as
one is how a corrupt file becomes an install nobody authorised."*

**It verifies nothing, and every answer says so.** A key being listed is the
first step of `ADR-029` §9.2 rather than the whole of it — a payload also needs a
signature that checks out, a kind this key is entitled to sign, and an arch and a
platform this machine accepts. A subject that answered *"yes, this will
install"* from a trust store alone would be answering a question it had not
checked, and it would be believed.

**Read at the point of use, never cached**, because §5.5.4 makes revocation an
image update that replaces the file — `cmd/rivet-sysd`'s launch check already
re-reads it on every `execve` for the same reason.

**Unproven:** `get trust` has answered against hand-composed stores and against
bytes `TrustStore::to_bytes` rendered. **It has not been asked of a board.** What
only a board settles is whether the `/system/trust` a real card carries parses —
and the honest note is that a `--no-trust` machine and a machine whose store is
corrupt are distinguishable *by this subject's design* and have never been
distinguished *in fact*.

### 5.6 The root key, designed and deliberately unbuilt

The maintainer asked whether one root could sign the two role keys, with the
root baked into images. **Yes — and §5.2 already ruled that shape**: a
configuration-supplied key is trusted only if countersigned by a key already
trusted. That *is* root-and-delegation, written before the word was used.

**It is not being built yet, and the reason is a property rather than an
effort estimate.** A root buys exactly two things:

1. **An offline anchor.** The root signs twice in its life and then lives on a
   hardware token or in a safe. Compromising the build machine then costs a
   *delegated* key — replaceable — rather than the anchor baked into every image
   ever shipped.
2. **Rotation without re-imaging**, if the delegation travels with the artifact.

**Both evaporate if the root sits on the same laptop as the keys it delegates
to.** The offline discipline *is* the feature, so the root should be generated
on the day it will actually be kept offline, not before.

#### 5.6.1 When it is built, the delegation is a **member**, not a header field

Recorded now because it is the design constraint that makes deferral safe.

The header has **24 reserved bytes** (`format.rs`: `OFF_HEADER_RESERVED` is
4008, the signature starts at 4032). A delegation record is ~130 bytes and
**does not fit**, so an inline design would force a format version bump.

It does not need to be inline. `ADR-029` gives a payload up to 40 members, and a
delegation is naturally one of them:

1. read `key_id`; look it up in the trust store,
2. **miss** → look for a `delegation` member; verify it against the *root*,
3. check the delegation's subject key matches `key_id`, and its mask permits the
   header's `kind`,
4. verify the header signature with that key, as now.

The apparent circularity — the delegation member's digest is in the header the
delegated key signs — is not one: the root's signature over the delegation is
checked on its own, and only then is the header's signature checked against the
key the root vouched for.

**So adding the root later changes `TrustSet` and adds a branch. It does not
change the container, the header, or any artifact already produced.** That is
what makes §5.5 safe to ship alone.

### 5.7 What this does not close

* **An image carries a trust store and the updater reads it — both halves done
  2026-08-24, §5.8 and §5.8.6**, and this list was written while neither was.
* **`ADR-015` §4.2 is reachable and absent.** `cmd/rivet-sysd` executes
  application binaries out of writable `/state` having verified nothing. The
  trust store is the precondition for fixing it, not the fix.
* **Nothing here defends `/system` itself.** §5.4 stands unchanged.

**§5.8 answers the first of these**, and re-states what is left of the other two
at its end.

---

### 5.8 The system key exists, 2026-08-24 — and what that changed

§5.5 ruled what a trust entry **is** and §5.7 listed what it did not close. The
first item on that list was *"no image carries a trust store yet"*, and by the
end of the day one did — but it trusted a key generated for a probe and thrown
away, because **Rivet had no signing key**. Every card `tools/card/build.sh`
produced was therefore a development card, honestly stamped and unable to verify
anything.

**There is now one system key, and its public half is in the repository** at
`keys/rivet-system.pub`, trusted for `update`, `extension` and `card` — the
mask §5.5.2 gives it, and deliberately not `application`.

#### 5.8.1 A public key is repository content, and that is not a §8.3 exception

`AGENTS.md` §8.3 forbids *"credentials, tokens, certificates or private keys"* in
any tracked file, and binds history rather than the working tree. A public key is
none of those things. More than that, **it is the thing an image is built to
carry**: §5.5.6 ruled the trust store into `/system` precisely so that a machine
could say what it trusts, and a trust anchor nobody can read is a contradiction
in terms.

The private seed is not here and is not going to be. It was produced by the
operating system's own CSPRNG, it lives on the build machine — `ADR-015` §4's
first bullet — and `.gitignore` refuses `*.seed` as a **backstop**. The backstop
is not the plan; the plan is that a seed is never inside the tree to begin with,
which is §8.3's own framing.

#### 5.8.2 A card now trusts something by default, and a missing key is refused

`tools/card/build.sh` reads `keys/rivet-system.pub` unless told otherwise, so a
card built from a clean checkout can verify an update signed by the system key.
Two overrides and **no silent fallback**:

| | |
| :-- | :-- |
| `RIVET_TRUSTED_KEY=<path>` | trust that key instead |
| `RIVET_NO_TRUST=1` | a development card, on purpose, stamped as one |
| a `RIVET_TRUSTED_KEY` that does not exist | **refused, exit 1** |

The last row is §5.5.5 one level up. `cmd/rivet-image` already refuses an empty
trust store unless `--no-trust` is passed; a *script* that fell back to
`--no-trust` when it could not find the key would defeat that refusal by
satisfying it. A card that cannot verify an update is not discovered at build
time — it is discovered on a board, later, by somebody trying to fix something
else.

#### 5.8.3 The tool that publishes a key, and the one this project will not build

`cargo rivet key SEED [--out PATH]` derives the public half of a signing key
and prints it with its **key id** — the BLAKE3 digest that an `.rvt` header names
(`ADR-029` §3) and that a refusal prints, so it is how a board says *which* key
it wanted. The seed is read and never printed, and a test asserts the report does
not contain it. An existing `--out` file is refused rather than overwritten: a
public key is written once and read forever, and silently replacing one is how an
image comes to trust a key nobody holds.

**`key new` exists — ruled 2026-08-28** (`cmd/cargo-rivet/src/key.rs`'s own
module doc has the ruling and the correction: `Cargo.lock` already carried
`getrandom` before this, and `getrandom` is not an RNG — it is a shim over
`getrandom(2)`/`BCryptGenRandom` that asks the kernel, which is what Ed25519
needing no randomness to sign or verify (`ADR-029` §7.1, one of the reasons it
was chosen over ECDSA) was always compatible with). **The operating system
makes the randomness; the tool makes the public key.**

**The same call now also mints a seed backup's salt and nonce — `cargo rivet
key backup`, `.rulings/` R-183 D3, 2026-09-14 — and that is a narrower thing
than it sounds, worth stating plainly rather than left for a reader to work
out.** A salt and a nonce are encryption *parameters*, not key material: they
are written alongside the ciphertext in plain sight (an AEAD's own associated
data, authenticated but never secret), and using a fixed or reused one is the
actual hazard — a reused nonce breaks the cipher outright. Minting a signing
seed and minting a nonce ask the identical kernel call for the identical
reason (`ADR-001` §7.3: entropy comes from the kernel, not from this
project), but they answer different
questions: a seed IS the secret an image is built to trust for years; a nonce
is scaffolding this project already discards the moment the file that used it
is decrypted once. `ADR-009` §3.10 is the dependency survey for the cipher
and the key-stretching function this introduced (`chacha20poly1305`,
`argon2`); this paragraph is the answer to the narrower question of why
minting that randomness meets the same bar the seed's own bytes are held to.

#### 5.8.4 One key file, one meaning — an `ADR-011` §3 defect, found and closed

`cmd/cargo-rivet` read a key as 32 raw bytes **or** 64 hex characters.
`cmd/rivet-image`, written later, read one as 32 raw bytes and nothing else.
Two answers to *what does a key file look like*, and the failure they produce is
nasty in a specific way: a hex key that one tool signs with, the other refuses to
trust, with a refusal that reads as a complaint about length rather than as the
two tools disagreeing.

`rivet_core::package::key_material` is now the single answer, and both call it.
Raw is tried first and deliberately — a file of exactly 32 bytes **is** a key
whatever those bytes look like, and sniffing for hex first would make a key of 32
ASCII hex characters parse as something else, which is a silent wrong answer
rather than a refusal. Nothing is padded and nothing is truncated, because
quietly adjusting key material to fit is how a key becomes a different key.

#### 5.8.5 What is still open

**The root is still unbuilt, and §5.6 is unchanged.** A root buys an offline
anchor and rotation without re-imaging; both evaporate if it sits on the same
machine as the keys it delegates to, so the offline discipline *is* the feature
and the root gets generated on the day it will be kept offline. §5.6.1 records
why deferring is safe rather than merely cheap: the delegation is a payload
**member**, so adding a root later changes no artifact already produced.

**There is no application key yet.** §5.5.2's example pairing — `rivet-system`
with `{system, extension}` and an application key with `{application}` — is half
built. `cargo rivet package` signs with whatever seed it is given, and nothing
yet says which key an application *should* carry.

**`ADR-015` §4.2 is still reachable and absent.** `cmd/rivet-sysd`
executes application binaries out of writable `/state` having verified nothing.
A trust store now exists on a card, which was the precondition; it is not the
fix.

**`cmd/rivet-deploy` reads `/system/trust` — closed the same day, §5.8.6.** It
took a key on the command line and nothing else until then.

#### 5.8.6 The updater reads the store, and the flag became an override

`cmd/rivet-deploy` required at least one `--trusted-key` because, when it was
written, nothing put a store in `/system`. Something does now, so **no key on the
command line is the ordinary case** and the store answers — each entry carrying
its own label and its own mask, not a narrowed one, so a key the card holds for
extensions is refused by the same rule that refuses it anywhere else rather than
by that binary's private opinion of one.

**Three refusals where there was one**, because *"no key"* was three situations
wearing one message:

| Situation | What it says |
| :-- | :-- |
| no store at all — a `--no-trust` card | names the path, `--trusted-key`, and `--no-trust` |
| a store with no entries | says it is empty, and that this is what a `--no-trust` card looks like from here |
| a store that does not **parse** | **refused, not skipped** |

The third is the one worth arguing. A store that cannot be read is **not** a
store that trusts nobody, and treating it as one is how a corrupt file becomes an
install nobody authorised. Here the two happen to coincide — an empty trust set
refuses everything — but the shrug is the defect, not its outcome in this one
case.

`--trusted-key` survives for the two cases that still need it: a card built
`--no-trust`, and verifying against a key that is not on this machine. It stays
narrowed to `update` alone, and it is now journalled at **`Warn`** rather than
`Debug` — an override means the install was *not* authorised by what the card
says it trusts, which is the sentence somebody will want in the journal
afterwards.

`trust_store::PATH` moved into `rivet-core` in the same pass. The image builder
writes that file and the updater reads it, and they are binaries that cannot see
each other; two spellings of one path is `ADR-011` §3, and the failure it
produces is a machine that says what it trusts in a file nothing consults.

---

## 6. Deferred with a named destination: version monotonicity

**The maintainer's ruling, 2026-08-23:** monotonicity may be a goal for the
future, but enforcing it today could quickly become a trap.
*(`.rulings/` R-277.)*

**He is right, and the reason is worth writing down rather than merely
accepting.** A device that refuses any image not newer than the one it is
running will, the first time a release is bad in a way `tryboot` does not catch
— boots fine, breaks something a week later — refuse the only thing that fixes
it. On a headless board reached over the network, *refusing to downgrade* and
*bricked* are the same state. The safety mechanism would have to be defeated
exactly when it is most needed, and a mechanism you routinely defeat is not one.

### 6.1 What is done instead, now

**Detect and report; do not refuse.**

1. **The manifest carries the version.** `ADR-027` §5 already requires it, so
   this costs nothing and is captured from the first artifact — enforcement
   later needs no flag day and no artifacts left behind.
2. **An update whose version is not greater than the running one is a named,
   attributed event** — in the journal (`ADR-022`) and in the `plan` the
   operator confirms (`ADR-006` §6), not a silent success.

**That is most of the defence, because replay's win condition is invisibility.**
An attacker who can serve a machine an old signed image, and have the operator
see *"this is older than what you are running"* before confirming, has not
achieved much. One who can do it silently has.

### 6.2 The shape enforcement should take: a fleet policy

**The maintainer, in the same conversation,** suggested that a fleet might carry
a policy requiring monotonicity, in the future. *(`.rulings/` R-277.)*

**That is a better answer than deferral, and it dissolves §6's own objection.**
§6 argues that a mechanism you must routinely defeat is not a mechanism. That is
only true of a rule applied to *every machine, always*. A **policy** is scoped
and set deliberately, so:

* **The risk profile is not uniform, and a policy can say so.** A server holding
  an application's sensitive data (`PLAN.md` §9.2) and the fleet server, which
  updates
  itself before it updates anyone (`PLAN.md` §9.4) — both want strict
  monotonicity. The board on the bench, which is reflashed several times a day,
  emphatically does not.
* **Relaxing it stops being a bypass and becomes an act.** Under §4 an
  authorised principal already instructs every update; a policy is that
  principal's *standing* instruction. Lowering it is then an ordinary
  configuration change — a `plan` that is confirmed, attributed (`ADR-012`) and
  in the journal (`ADR-022`) — rather than a safety mechanism being worked
  around. **The difference between "defeating a control" and "changing a policy"
  is entirely whether the system was designed to be told.**
* **It reaches the fleet, which is where replay actually pays.** Downgrading one
  bench board is a nuisance. Downgrading twenty machines to a version with a
  known hole is the attack worth defending against, and that is exactly the
  scope a fleet policy covers.

**What it does not cover, stated so no one assumes otherwise:** an attacker who
can already write a machine's configuration can relax its policy. The policy
defends against replay by someone *outside* the machine — which is the case it
is for — and it still forces that attacker to have compromised configuration
first. `ADR-028` §5's floor is the layer for the other case, and neither is
asked to do the other's job.

### 6.3 What enforcement would need, when it is wanted

Recorded so the later decision is smaller than this one:

* **A high-water mark, not a comparison against the running slot.** Comparing
  against the running slot is defeated by two updates: downgrade once, and the
  next downgrade is legitimately "newer" than what is now running. The mark
  belongs in `/state`, which an update never touches (`ADR-018` §5.2).
* **A first-class way to lower it**, deliberate and attributed — otherwise §6's
  straitjacket returns wearing a different coat. §6.2's policy is that way, and
  the requirement is met by the shape rather than by an escape hatch bolted on.
* **A ruling on what the policy applies to when a machine is not in a fleet.**
  A bench board has no fleet to hold a policy, so the default for an unenrolled
  machine is a separate question — and `ADR-019` makes *every* machine arrive
  enrolled, which may mean there is no such case. Worth checking rather than
  assuming.
* Both are decisions about the *administration surface*, which is why this waits
  for `ADR-006` §7's catalogue to carry real operations rather than being
  designed against a surface that does not exist yet.

---

## 7. What this changes

| Place | Change | State |
| :-- | :-- | :-- |
| `docs/ADR-028` | This file | Done 2026-08-23 |
| the build plan | The "note what this does not rest on" paragraph struck in place | Done 2026-08-23 |
| `PLAN.md` §3 | Decision-log row | Done 2026-08-23 |
| `ADR-015` §4 | Pointer to §5's floor | Done 2026-08-23 |
| `ADR-018` §5.2 | Pointer: the flow verifies a signature and checks no version | Done 2026-08-23 |
| `README.md` | The ADR range | Done 2026-08-23 |
| The install request's principal | §4.5 | **Built 2026-08-24 evening.** `rivet_core::deploy::Principal`, written by `cmd/rivet`, recorded by `cmd/rivet-sysd`, watched under QEMU on Rivet's own kernel (§4.5.5). **Attribution only** — §4.5.2. The control-plane half is unproven until it runs on a board |
| The root key set in the image | §5.2 | `cmd/rivet-image` ships no keys, and `/system` carries no root set — that is unbuilt, and this row said nothing verified anything until 2026-08-24. **Something verifies:** `ADR-029` §13 built the container and `rivet-crypto` implements the signature check, so §5.2's `TrustSet` exists as a type a caller fills. **Nothing fills it from `/system`**, which is the whole of what §5.2 rules. |
| The version report | §6.1 | **Not built, and the reason has changed** — it once could not be built until there was an update path at all. The version is now carried in the signed header (`ADR-029` §3), so it is *available*; what is missing is an ordering, which no ADR defines — `ADR-029` §12 item 7. Reporting "this is older" needs somebody to rule what older means. |

**This section said nothing in this ADR was implemented, and it was corrected
2026-08-24** — the file's own §4.1 and `AGENTS.md` §7 are about exactly that.
What is implemented is the *mechanism* a layer needs, not any
layer: `ADR-029` §13 built the verifier, and it is the row of §2's table that
says a signature *"does not — it **is** the thing being protected"*. **None of
§3's layers is standing on a machine.** No root of trust ships in an image (§5),
no principal's authority is checked anywhere (§4), and no version is compared
(§6). The transport row is the exception and the only one: `ADR-035`'s control
plane is built and carries payloads to a board today.

**That sentence survives §4.5 intact and is not struck**, because §4.5 built
*attribution* and this sentence is about *authority*. An install request now
names who asked; nothing checks whether they may. The two are different claims
and §4.5.2 exists so that this one is not quietly upgraded.

### 7.1 Amended later on 2026-08-24: something now *calls* the verifier

`ADR-029` §14 — `cmd/rivet-deploy`, ruled by the maintainer as its own
supervised binary. **Read what follows as narrowing the paragraph above, not as
softening it.**

| Layer | Was | Is |
| :-- | :-- | :-- |
| **Authenticity** (§4, second half) | a library nothing called | **checked before any byte is written**, by a binary that has been run against a real card image over a loop device. `ADR-029` §14.6 |
| **The root key set** (§5.2) | not built | **still not built.** The installer takes `--trusted-key PATH`, repeatable and required, and refuses when given none. Nothing puts a key under `/system`, and inventing a path there would be an installer claiming authority over a machine's trust anchors (`ADR-011`) |
| **Authority** (§4, first half) | not checked | **Still not checked**, and nothing anywhere verifies a principal's authority to install anything. **Amended 2026-08-24 evening — §4.5, and the amendment is narrower than it looks:** the **request now names who asked**, and the supervisor records it. **Nothing carries a principal into the installer** remains true — `definitions::deploy` hands `cmd/rivet-deploy` one argument, so every record *inside* an install still has the process as its actor. §4.5.2 and §4.5.4 |
| **Version monotonicity** (§6) | not built | **reported, not compared.** Both version strings go in the journal and neither is called newer, because `ADR-029` §12 item 7 has no ordering. §6.1's *"a named, attributed event"* is half met: named and attributed to the process, not to a principal — and **still** attributed to the process after §4.5, because §4.5's principal reaches the supervisor's record and not the installer's |
| **Transport** | none | **none.** The installer takes a path to a file already on the machine |

**So the sentence most likely to be misread has moved rather than gone.** It was
*"the verifier exists"*; it is now *"an installer exists"*. Neither means updates
are trustworthy on a Rivet machine, because **§5's floor is the layer that is
missing**, and it is the one that decides whose signature counts.

`ADR-015` §4's signing is ruled and unbuilt; this ADR constrains how it gets
built. Recorded plainly because a security ADR that reads as though it describes
a running system is worse than no ADR — **and that is more true now than it was,
because "the verifier exists" is the sentence most likely to be mistaken for
"updates are trustworthy".**

---

## 8. Open

1. **What key format and signature scheme.** `ADR-025` uses Ed25519 for fleet
   identity and the same choice is the obvious one here, but the two key
   *purposes* are separate — `ADR-025` §2.2 forbids coupling the device identity
   to any other key material the machine happens to carry — and this should be
   ruled rather than inherited by proximity.
2. **Whether the countersignature in §5.2 is a distinct artifact or a field in
   the configuration document.** The second is simpler and puts a signature
   inside a document the operator edits, which `ADR-007` may have opinions about.
3. **Whether extensions and applications share the root set with images**, or
   have their own. `ADR-015` §4 says *"one signing, verification and transport
   path serves all three"* — it does not say one trust set, and those are
   different claims.
4. **What happens when verification fails on the inactive slot — ANSWERED by
   `ADR-029`, and built 2026-08-24.** Most of it dissolved: verification
   precedes writing, so a failed verification leaves the slot untouched rather
   than partial. Where a slot *is* left partial — a power cut, or a card that
   refused mid-write — `ADR-029` F3 rules it **left inert, not wiped**, on the
   grounds that wiping is a second destructive operation on the one thing whose
   virtue is being unreachable. `cmd/rivet-deploy` deletes and zeroes nothing,
   and `ADR-029` §14.6 records both halves being watched on a card image.
5. **How a standing grant is bounded** — §4.2 item 4. Expiry, a version ceiling,
   or re-confirmation when the scope changes. `ADR-012` §5's capability scoping
   is the place it belongs, and §5 does not currently give a scope a lifetime.
6. **Whether the principal should reach the installer's own records** — §4.5.4.
   Today it stops at the supervisor's record of taking the request up, so
   `rivet get install` cannot answer *"who asked"* and points at
   `rivet get log` instead. The fix is to carry it on `cmd/rivet-deploy`'s
   argument vector, which changes that binary's argument contract and would make
   `rivet_core::deploy::ACTOR` a per-install value rather than a constant.
   **Recommended**, and put here rather than done, because an argument contract
   is the shape of the software (`AGENTS.md` §1). The cost of leaving it is one
   indirection for an operator; the cost of getting it wrong is every install
   record claiming a provenance the process performing it does not have.
7. **Whether a principal should also name the local user, and what that would
   mean when there is only one.** Everything reaching this machine arrives as
   `root`, so `USER` distinguishes nothing today and §4.5 does not record it. It
   would start to mean something the day a second local identity exists, and
   recording a constant until then would train a reader to ignore the field.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §5.8 — the image trusted the system key for updates and extensions only

**Changed 2026-09-30, for `ADR-063` §6.** The superseded sentence read:

~~`keys/rivet-system.pub`, trusted for `update` and `extension` — the mask
§5.5.2 predicted it would get, and deliberately not `application`.~~

§5.5.2 had put `card` in the system key's mask on 2026-09-13, and
`tools/card/build.sh` still built images trusting it for `update,extension`
alone, so every fleet server's repository store refused every card. A fleet
server that cannot hold a card cannot write one for a host it recovers. The
lead agent decided it, recorded for the maintainer to overturn: the system key
already signs system updates, which are strictly more powerful than a card, so
the image trusts it for `card` too; and at the same time a machine's own
install door refuses a card by kind, because it checked trust and not kind.

### In §4.4 — the advice named a service filter that does not exist

**Corrected 2026-08-30**, by the second cold-agent run. The superseded sentence
read:

~~So both verbs point at `rivet get log rivet-sysd`.~~

`get log` has no service filter, so `rivet-sysd` bound to the boot selector and
the machine answered `"records":[],"returned":0` — which reads as an empty
journal rather than as a question that missed.

### In §5.8.3 — "there is deliberately no `key new`"

**Corrected 2026-09-14, while closing R-183's seed-backup work.** The
superseded text — already stale before this pass touched it, since `key new`
was ruled in on 2026-08-28 and this section had not caught up — read:

~~There is deliberately no `key new`. `rivet-crypto`'s Cargo.toml records
that this project has no RNG in its dependency graph at all, because Ed25519
needs none to sign or to verify — one of the reasons `ADR-029` §7.1 chose it
over ECDSA. Adding a randomness source so that a tool can save a person one
shell command does not meet `ADR-009` §5's gates.~~

`key new` exists, uses `getrandom` (not an RNG — `ADR-001` §7 corrected the
same "no RNG in the graph" claim project-wide the same day this section
should have been updated and was not), and the current body above states
that plainly rather than repeating the stale refusal.

### In §5.7 — no image carried a trust store

**Both halves done 2026-08-24 — §5.8 and §5.8.6.** The superseded bullet read:

~~**No image carries a trust store yet.** Until `cmd/rivet-image` writes one
and `cmd/rivet-deploy` reads it, `.rvt`s are verifiable on a host and on no
Rivet.~~

### In §5.8 — the updater did not read the store

**Closed the same day, 2026-08-24 — §5.8.6.** The superseded sentence read:

~~**§5.7's second item stands.** `cmd/rivet-deploy` takes a key on the command
line and does not yet read `/system/trust`.~~

### In §7 — two rows of the implementation table

**Both went stale on 2026-08-24.** The superseded cells read:

~~**Not built.** `cmd/rivet-image` ships no keys and nothing verifies anything
yet.~~

~~**Not built**, and cannot be until there is an update path at all.~~

Half of the first was still true and is in the body: `cmd/rivet-image` ships no
keys and `/system` carries no root set. What changed is that something verifies.

### In §7 — nothing in the ADR was implemented

**Corrected 2026-08-24.** The superseded sentence read:

~~**Nothing in this ADR is implemented.**~~

### In §7 — the authority row

**Amended 2026-08-24 evening — §4.5, and the amendment is narrower than it looks.**
The superseded cell read:

~~**still not checked.** Nothing carries a principal into the installer, so every
record's actor is the process itself. The maintainer's 2026-08-24 ruling left *how an operator
asks* unruled, and that is the same gap seen from the other side~~

Both of its claims are still true. What changed is that the request now names
who asked.

### In §8 item 4 — what happens when verification fails

**Answered by `ADR-029`, and built 2026-08-24.** The superseded item read:

~~**What happens when verification fails on the inactive slot** — refuse and
report is obvious, but whether the slot is then wiped, left, or marked is
`ADR-018` §5.2's question and it does not currently ask it.~~
