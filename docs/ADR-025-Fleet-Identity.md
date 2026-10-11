# ADR-025: Fleet Identity

* **Status:** Accepted — ruled 2026-08-22. Answers `ADR-019` §5 and closes the
  last item from the subsystem audit.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`ADR-019` ruled the *shape* of enrollment — a machine arrives already enrolled,
from a generic image plus a per-device provisioning file — and left four
questions about what identity actually consists of. It also flagged them as the
gap most likely to reach backwards into the configuration schema, because how a
machine identifies itself is exactly the kind of thing that wants a field nobody
added.

---

## 2. Identity is an Ed25519 keypair, pinned

**Ruled 2026-08-22: raw keypairs, and no certificate authority.**

The fleet holds a map of device to public key; the device holds the fleet's
public key. Mutual, symmetric, and about as much machinery as it sounds like.

An X.509 infrastructure buys delegation, intermediate authorities, expiry-based
revocation and standard tooling. **None of those is needed for three machines
and one operator**, and each is complexity that can fail. Two specifics make it
a worse fit here than usual:

* **Expiry needs a clock**, and `ADR-021` establishes that this hardware has no
  RTC and boots with a wrong one. Certificate validity windows are a poor thing
  to depend on when time is itself unreliable at exactly the moment a device is
  trying to connect.
* **A CA is a thing to run**, back up and protect. Removing it removes an entire
  category of operational obligation.

This is trust-on-first-use with the pin written down in advance, and it is the
right size.

### 2.1 The transport wrapper — ANSWERED 2026-08-28 by `ADR-035` §7.2

**A self-signed certificate, pinned by public key.** `ADR-035` §7.2 rules it,
quoting this section's deferral verbatim. It is **built**: `rivet-tls` is a
workspace member, `PinnedPeer` walks no chain and ignores expiry deliberately,
and `cargo rivet identity new` mints the certificate. A stock `git` has cloned
from and pushed to a board over it (`PLAN.md` §88).

**This section deferred the question until 2026-08-29, and the delay is the
finding.** The deferral fired and the answer was written into a different ADR;
nothing brought it back to the section that posed the question, so an agent
reading `ADR-025` end to end concluded the wire format was undecided **after
`rivet-tls` had shipped**. Note the deferral's own condition did not hold
either: it said the wrapper would arrive *"with the dashboard"* and it arrived
with `rivet-mcpd`, which `ADR-035` §7.2 says is precisely why the question could
not wait.

### 2.2 It is not coupled to any other key the machine carries

The device identity key is **its own**, with its own lifetime and its own
purpose, and it is not derived from or shared with any other key material on the
machine. Coupling would mean rotating one forces rotating the other, and it
would collapse two independent things into one — which §5.1 depends on staying
independent.

### 2.3 Where it lives

The private key is a `Secret` under `ADR-020` §4, held in `/state/secrets` and
wrapped by the key provider. The fleet's public key is ordinary configuration —
public by definition, and what matters for it is integrity, not confidentiality.

**The wrapping is not built, and as of 2026-08-28 the key is a plain `0600`
file.** `ADR-020` §6's envelope and §7's board provider are ruled and unbuilt,
so this sentence describes the destination rather than the state. `ADR-020`
§9.2 rules the interim and §9.2.1 carries the trigger that ends it — and the
path is `/state/secrets` either way, which is why `cmd/rivet-mcpd` is given a
*directory* and not a format.

---

## 3. The fleet tool generates it, and keeps only the public half

**Ruled 2026-08-22**, and it follows from `ADR-019` §2's offline requirement.

A device generating its own keypair must get the public half to the dashboard
somehow, which needs the dashboard reachable at first boot — the exact
circularity that enrolling-before-boot exists to avoid.

So the fleet tool generates the pair, writes it into the provisioning file, and
**retains only the public key.** The private half exists transiently on the build
machine and on the card until first boot, where `ADR-019` §3.1's existing rule
blanks it.

Device-generated keys with a one-time registration token remain available as an
alternative mode for the case where the fleet *is* reachable and better key
hygiene is wanted. Not built, and not needed for the ruled default.

---

## 4. The logical device is separate from its credential

This is the modelling decision the reflash question actually required.

> **The fleet knows a logical device. The logical device currently trusts a
> key.**

`kitchen-pi` is a stable identity: a role, a location, a set of fleet-layer
configuration (`ADR-007` §3.4), and a history. The keypair is a **credential
that device currently holds**, not the device itself.

Replace a dead board, provision a new key, and the dashboard updates which key
`kitchen-pi` presents. **Configuration and history survive the hardware.**

Without the separation, replacing a board loses everything the fleet knew about
that device — which is obviously wrong, and would have been discovered the first
time a Pi died.

It also collapses two problems into one mechanism: **reflash and revocation are
the same operation seen from opposite sides.** Both are "this device now trusts
a different key".

### 4.1 The identifier is opaque; the name is a label

A stable opaque identifier is authoritative for a logical device. The
human-readable name is **configuration** and may change.

The same pattern as everywhere else in this record: the machine-readable thing
is the authority, and the human-facing rendering is a view of it. A name that
was the identity could not be corrected for a typo without losing the device.

---

## 5. Revocation is removing a key from a list

Not having a CA pays off here. There is no CRL, no OCSP, no distribution
problem, and nothing that expires while nobody is looking. Revoking a device is
deleting its public key from the fleet's map — a configuration change like any
other, previewable, attributed and confirmed.

### 5.1 The other direction, and why an out-of-band path must stay independent

If the *dashboard's* key is compromised, a device cannot be told to stop
trusting it via the dashboard. Something with independent credentials has to be
able to reach the machine, or the compromise is unrecoverable in place.

**Today that path is `ADR-035` §8A's break-glass over the serial console**,
which works precisely because it is an independent channel whose credential is
**physical presence** rather than a key at all (§2.2). It is also the narrowest
possible one: it needs somebody to touch the machine.

**Say the cost out loud.** A fleet whose members are not physically reachable
has no remote recovery from this failure. That is a consequence of there being
one network channel, it is accepted rather than overlooked, and it is a concrete
reason never to collapse the break-glass path into the fleet channel however
tempting the simplification looks later.

### 5.2 A stolen Pi is a compromised identity

Stated plainly rather than left to be discovered.

**`ADR-020` §7's board-bound encryption protects nothing yet**, and this
section's own §2.3 says so eighty-three lines above it. It claimed to protect a
stolen **card** until 2026-08-29.
`ADR-020` §6's envelope and §7's board provider are **ruled and unbuilt** —
`PLAN.md` §102.4 has carried that row since 2026-08-22, and §94.4 calls this the
worst shape a decision can be in: *"made, depended upon, and absent … strictly
worse than an open question, because an open question is visible and this reads
as settled."* There is no `KeyProvider` implementation in the tree;
`cmd/rivet-initd/src/identity.rs` writes `device.seed` into `/state/secrets` as
thirty-two raw bytes at mode `0600`.

**So a stolen CARD is a compromised identity too, today.** §2.3 was annotated on
2026-08-28 and this paragraph was not — one sibling corrected, the other left,
and the one left is the one making a security claim. It **also does nothing**
when the thief has the **board**, because the wrapping key would be derived from
it.

**The section's title and conclusion survive unchanged and are now stronger, not
weaker.** The response is revocation at the dashboard (§5). That is an adequate answer,
and it is only adequate because §4 separated the logical device from its
credential: revoking the stolen device's key does not destroy the record of what
that device was for.

---

## 6. Open

1. **Rotation cadence, if any.** Rotation is a normal configuration change on
   both ends (`ADR-019` §4), so the mechanism exists. Whether keys rotate on a
   schedule or only on suspicion is unasked, and scheduled rotation is only
   worth having if it is reliable enough not to lock a device out.
2. **What the device does when the fleet is unreachable.** It must keep running
   on its last-applied configuration — which the desired-state model already
   gives for free — but how long it waits, whether it warns, and what
   `rivet status` reports, is undesigned.
3. **The transport (§2.1) — CLOSED
   2026-08-28 — `ADR-035` §7.2; see §2.1. It did not arrive with the dashboard,
   which this item expected and which `ADR-034` §4.4 has since ruled *"a
   different product"*.**

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §2.1 — the transport wrapper was deferred

**Answered 2026-08-28 by `ADR-035` §7.2**, which quotes the deferral verbatim
and rules it. The superseded heading read ~~The transport wrapper is
deliberately deferred~~, and the paragraphs under it:

~~The dashboard does not exist. How a key is *presented* on the wire — raw, or
inside a self-signed certificate pinned by key so TLS libraries are satisfied —
is a decision for whoever builds the transport (`ADR-006` §5.1).~~

~~**Decide the key now, decide the packaging when there is something to pack it
into.** The key is the durable artifact; the wrapper is not.~~

**Marked here only on 2026-08-29, and the delay is the finding.** The deferral
fired and the answer was written into a different ADR; nothing brought it back
to the section that posed the question, so an agent reading `ADR-025` end to end
concluded the wire format was undecided **after `rivet-tls` had shipped**.

### In §5.2 — board-bound encryption was said to protect a stolen card

**Corrected 2026-08-29**, and this section's own §2.3 had said so eighty-three
lines above it. The superseded sentence read:

~~`ADR-020` §7's board-bound encryption protects a stolen **card** — the key
material is unreadable without the board.~~

Also superseded, two paragraphs later: ~~does nothing~~, replaced by *also does
nothing*, because the card case had joined the board case.

§2.3 was annotated on 2026-08-28 and this paragraph was not — one sibling
corrected, the other left, and the one left is the one making a security claim.

### In §6 item 3 — the transport was open and expected with the dashboard

**Closed 2026-08-28 — `ADR-035` §7.2.** The superseded item read:

~~**The transport** (§2.1), which arrives with the dashboard.~~

It did not arrive with the dashboard, which `ADR-034` §4.4 has since ruled *"a
different product"*.
