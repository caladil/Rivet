# ADR-020: Secrets at Rest

* **Status:** Accepted — ruled 2026-08-22. **§11 (2026-08-29): the mechanism was still UNBUILT four weeks after acceptance and `ADR-025` §5 was resting on it. Ruled to build. §11.2 RULED 2026-08-30 (§11.2.3): plain text, as Raspberry Pi OS does; obfuscation is HEX, labelled `psk-hex`, and never to be called encryption. §11 is now unblocked work.**
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

Secrets have accumulated across the record without ever being given a home:

| Secret | From |
| :-- | :-- |
| Wireless PMK | `ADR-004` §5 |
| Device identity private key | `ADR-019`, `ADR-025` §2 |
| User password hashes | `ADR-008` §7.3 |
| Application secrets — API keys, database credentials | `ADR-015` |
| Trusted **public** keys for artifact verification | `ADR-015` §4 — not secret; see §9 |

Each was handled locally and correctly. None had a general answer.

---

## 2. The constraint that determines everything

**A Raspberry Pi 4 has no TPM and no secure element.**

Any key used to decrypt storage must therefore be reachable by the device
unattended, which on a headless machine means it is derivable from something on
or in the machine itself. **Encryption at rest on a stock Pi does not defend
against someone holding the hardware**, and claiming otherwise would be exactly
the dishonesty `ADR-011` exists to prevent — applied to a security claim, where
it matters more than usual.

This ADR is therefore as much about stating what is *not* protected as about
what is.

---

## 3. Threat model

| Threat | Defensible | By what |
| :-- | :-- | :-- |
| Someone takes the Pi | **No** | — accept and state it |
| Someone takes **only the SD card** | **Yes** | §7, board-bound key |
| A compromised application reads another's secrets | Yes | `ADR-015` §5.1 sandbox and per-app UID |
| A compromised application reads system secrets | Yes | root-owned, mode 0600 |
| **Secrets leak into `plan`, `export`, logs, errors** | **Yes** | §4, the `Secret` type |
| **`rivet export` committed to git** | **Yes** | §4 and §5 |

The bottom two are the failures that will actually occur, and they are entirely
preventable. **The effort belongs there**, not on encryption that defends
against a threat the hardware cannot defend against anyway.

---

## 4. A `Secret` type, because secrecy is a property of the type

The highest-value and cheapest decision here.

Configuration is already typed (`ADR-007` §3.1), so **secrecy is expressed in
the type system rather than as a convention people must remember.** A `Secret<T>`
whose `Debug` and `Display` implementations redact, rendering as `<secret>` in
`plan`, `--explain`, `export`, logs, and error messages.

This is `ADR-011` applied to secrets: **one place knows what is secret, and it is
the type.** Not a list of field names, not reviewer attention, not a lint —
none of which survive a hundred commits.

Existing crates do the work: `secrecy` for the wrapper, `zeroize` for clearing
memory on drop. Rung 1 of `ADR-009`'s ladder.

**The failure this prevents** is the realistic one:
`rivet export > machine.toml` followed by `git add`. Under §5 that document
contains references, not values, and under §4 nothing that renders a secret can
do so accidentally.

---

## 5. The store is separate, and referenced by name

**`/state/secrets/`, root-owned, mode 0700.** The document declares *which*
secrets exist and what they are for; the store holds the material.

This is not a second authority. The document remains authoritative for the
existence and identity of every secret; the store is where one field's value
lives, the way a database row references a blob.

**The honest cost, which is documented rather than hidden:** `rivet export`
produces a document that reconstitutes a machine **except its secrets**, which
must be re-provisioned. That is the price of an export being safe to commit, and
it is the right trade.

---

## 6. Envelope encryption behind a `KeyProvider` port

The store is **not** encrypted with the board-derived key. It is encrypted with
a random data key, and that key is wrapped by whatever a provider supplies:

```
/state/secrets/store         encrypted with a random 256-bit DEK
/state/secrets/dek.wrapped   the DEK, wrapped by the provider's key
/state/secrets/salt          for the board provider's KDF
```

`KeyProvider` is a **trait in `rivet-core`** and each provider is an adapter
crate, exactly as `ADR-014` §3 requires. Nothing above the port knows which
provider is in use.

| Provider | Wrapping key |
| :-- | :-- |
| `board` | Argon2id(board serial, salt) — §7 |
| `otp` | a random 256-bit value in the SoC's one-time memory — §7.2 |
| `tpm` | DEK sealed to the TPM |
| `fleet` (speculative) | unwrapped by the dashboard after the device proves identity |

### 6.1 Why envelope encryption, specifically

Because it answers the question the maintainer asked — whether adding a TPM
later is a plug-in or a refactor.

**Migration re-wraps one small blob. The store is never re-encrypted.** Install
the module, run `rivet secrets rekey --provider tpm`, and the data is
untouched. Without the envelope, changing providers means decrypting and
re-encrypting the entire store, which is slower, riskier, and needs a
half-finished-migration story.

**`rekey` must be a first-class operation from the start.** It is cheap to
design in and genuinely annoying to add later, and it is the thing that makes
the provider a swappable part rather than a decision baked into the data.

### 6.2 Cipher choice

**ChaCha20-Poly1305**, not AES-GCM. The Pi 4's Cortex-A72 **does not implement
the optional ARMv8 cryptography extensions**, so AES has no hardware
acceleration on this board while ChaCha20 is fast in software regardless.

**Verified 2026-08-22 on a Pi 4 Model B Rev 1.1.** `/proc/cpuinfo` reads
`Features : fp asimd evtstrm crc32 cpuid` — `crc32` is present, and `aes`,
`pmull`, `sha1` and `sha2` are all absent. `CPU implementer 0x41` / `CPU part
0xd08` confirms the part is an ARM Cortex-A72. The expectation this section was
written on was correct, and the cipher choice stands on measurement rather than
on anticipation.

**This finding reaches further than this ADR.** No SHA extensions means
`ADR-015` §4.2's launch-time signature verification hashes every binary with a
software SHA-256 — which is exactly why `PLAN.md` §4.25 parks it for measurement
and why BLAKE3 is the first thing to try.

---

## 7. The board provider, and what it honestly is

**Ruled 2026-08-22: the board serial is the first provider.**

The wrapping key is `Argon2id(board serial, salt)`, with the salt stored beside
the encrypted store.

### 7.1 It is binding, not obscurity

Worth stating precisely, because the two get conflated and the distinction is
the whole justification.

**Security through obscurity** means the security depends on the *method* being
secret. Here the method is published in this document. What an attacker lacks is
an **input**: the Pi's serial lives in the SoC and is not on the card.

So an attacker holding only the card is missing a value they cannot read from
it. That is a real barrier.

**What it buys:** the lost, discarded, repurposed, or sent-for-repair card —
which is a genuine and ordinary way for storage to leave one's control.

**What it does not buy:** anything against someone holding the Pi, who can read
the serial in seconds. This is by design and by arithmetic, not a weakness to be
fixed later.

The serial is **low entropy**, which is why the KDF must be memory-hard —
Argon2id, with parameters tuned so that they are painful on an attacker's
machine without being painful on a 4 GB Pi at boot. A plain hash here would
make a serial-guessing attack instant.

### 7.2 OTP is the stronger variant of the same design

Recorded now so the path is known, and **not adopted**.

The Pi's one-time-programmable memory includes customer-programmable rows
totalling **256 bits — exactly a wrapping key.** That is materially better than
the serial, and for a reason worth naming: it is a *random key*, not a
low-entropy identifier being stretched by a KDF. There is nothing to guess.

It is not adopted now because **OTP is one-way and permanent.** A mistake makes
the board scrap. It inherits precisely the concern that deferred Pi 4 secure
boot in `ADR-018`'s neighbourhood, and both should be revisited together, once
the signing and update paths are real, tested, and there is a spare board to get
it wrong on.

Because of §6, adopting it later is another provider adapter and a `rekey`.

---

## 8. Delivery to applications

An application **never opens a secrets file and never hardcodes a path.** It
receives its secrets the way it receives its configuration — through the SDK
(`ADR-036`, `ADR-013` §6, `ADR-015` §9).

That keeps `/state/secrets` reachable only by root, keeps the per-application
subset explicit and declared in the manifest, and means an application's
secrets can be rotated without it knowing where they were stored.

---

## 9. What is not protected

Stated here so the limits live in the record rather than in a conversation.

* **A running system exposes everything.** Secrets are decrypted in memory and
  readable by root. Encryption at rest protects the card at rest, and nothing
  else.
* **Someone holding the Pi defeats the board provider**, by design (§7.1).
* **`rivet export` cannot reconstitute secrets** (§5).
* **Trusted public keys are not secrets** — they are public by definition. What
  matters for them is *integrity*, not confidentiality, and nothing in this ADR
  protects it. That is what Pi 4 secure boot would address, and it is deferred.
* **Rivet backs up the configuration document and the secret store**
  (`ADR-030`, 2026-08-24); whole-`/state` backup remains the operator's, which
  is what this bullet said of all backups until then. **The reason `ADR-030` is
  scoped the way it is:** a
  backup contains the encrypted store but its wrapping key is bound to the board,
  so a restore onto *different* hardware **loses every secret**. That is §7
  working as intended, and `ADR-030` §5 states it in the product rather than only
  here — because *"it must be documented before anyone relies on a backup they
  have not tested"* is a requirement on the thing that makes the backup, not a
  note in an ADR.

### 9.1 A plain secret on `/state` is outside §6, and `/state` is forced

**Added 2026-08-23** for the first secret that had to live on a card before §6's
store existed, and kept because **the reasoning is what recurs**, not that
particular key.

**`/state` is forced by elimination.** An image is byte-reproducible and signed
(`ADR-015` §4), so a key in it is the same key on every board; `/system` is
replaced wholesale by an update; `/run` and `/etc` are tmpfs, so the key would
change at every boot and every peer would report that the machine's identity had
changed. Root-owned, mode `0600`, in a `0700` directory.

**What that buys, in this ADR's own terms.** It satisfies §3's row *"a
compromised application reads another's / system secrets — yes, root-owned, mode
0600"*. It does **not** satisfy §3's row *"someone takes only the SD card"*,
which is the row §6's envelope encryption exists for. So the second row of that
table is, for a secret held this way, a **no** — stated rather than left to be
inferred.

**Why such a secret is not simply put in the store.** A third-party mechanism
opens a *file path*; it cannot be handed bytes, and Rivet does not patch its
mechanisms (`ADR-010` §3.2 notes what patching would oblige).
**A secret whose consumer insists on a filesystem path cannot live only inside an
encrypted blob.**

**The migration, so it is a known step rather than a discovery.** When §6's
store exists, such a secret becomes a record in it, is decrypted into `/run` at
boot by whatever starts the consumer, and the consumer is pointed at the `/run`
path. Nothing about the consumer changes, because what it takes is a path either
way. That is the same shape as §8's delivery to applications and it costs one
`rekey`-era decision rather than a redesign.

One property of that arrangement is this ADR's rather than `ADR-017`'s:

* **Nothing reads the key.** Whatever ensures the key exists `stat`s the path
  and never opens it, so no journal record, no error and no console line can
  carry key material (`AGENTS.md` §8.3). It is asserted rather than observed —
  as a property over the whole journal, not over one record.

**Generalise before the next one arrives.** The wireless PMK (`ADR-004` §5) will
want exactly this treatment and `wpa_supplicant` has the same path-not-bytes
constraint. The pattern is worth naming: *a mechanism that takes a path gets a
`/run` projection of a record in the store, never a second copy on `/state`.*

### 9.2 One plain secret exists today, and it is allowed — ruled 2026-08-28

**The device TLS private key** (`ADR-025` §2, `ADR-035` §7.2) is the plain secret
on `/state`, taken as an exception to §9.1's own preference rather than by
quietly stretching it.

`ADR-025` §2.3 already named the destination — *"a `Secret` under `ADR-020` §4,
held in `/state/secrets` and wrapped by the key provider"* — and **§6's
envelope and §7's board provider are ruled and unbuilt**, so that sentence
cannot be satisfied by any build that exists. The choice was between blocking
`ADR-035` phase 1 on a subsystem nobody has started and taking the exception a
second time. `PLAN.md` §76 tabled four options; the maintainer ruled on
2026-08-28.

**Ruled: a plain file, `/state/secrets/device.seed`, mode `0600`, root-owned.**

The argument, which is §9.1's own applied one notch further:

* **The threat row it gives up is already given up.** §9.1 records in as many
  words that a plain secret does not satisfy §3's *"someone takes only the SD
  card"* row, and this card already carries everything an attacker holding it
  would need. The set of secrets such an attacker gets is unchanged in kind.
* **The migration is §9.1's, unchanged.** When §6's store exists the seed
  becomes a record in it, is projected into `/run` at boot, and `rivet-mcpd` is
  pointed at the `/run` path. **`cmd/rivet-mcpd` takes `--identity <directory>`
  precisely so that path can move**, and being Rivet's own code it needs no
  path-not-bytes exception at all.

### 9.2.1 The count is the rule, and this is the trigger

A rule that has been bent once and not rewritten is a rule nobody will believe
the next time, so it is stated as a count rather than as a preference:

> **The SECOND plain secret on `/state` does not get an exception. It gets
> `ADR-020` §6.**
>
> One is enumerated and closed: the device TLS seed (§9.2). A proposal to add
> another is a proposal to build the envelope store, and *"§6 is not built yet"*
> stops being a reason on the day this is read a second time.

The wireless PMK (`ADR-004` §5) is the candidate most likely to arrive next, and
it is **not** grandfathered by this: §9.1 already routes it through the store,
and nothing here changes that.

---

## 10. Open

1. **Argon2id parameters**, tuned against real boot-time measurements on a Pi 4
   rather than guessed.
2. **Restore onto replacement hardware** (§9). If a board dies, the correct
   procedure is re-provisioning rather than restoring — but that should be a
   documented procedure rather than a discovery.
3. **Revisit OTP and secure boot together**, once the update path is tested.

---

## 11. RULED 2026-08-29: build it — the status was accurate and the code was missing

**This ADR has read `Accepted` since 2026-08-22 and none of it exists.**
`ADR-025` §5 relies on it. `PLAN.md` §102.4.1 item 3 called that the worst shape
a decision can be in: a reader hits the status block, believes the mechanism is
there, and builds on it.

**Two options were put to the maintainer — build it now, or downgrade the status
to deferred and make the dependency visible from both ends. The recommendation
was the second. He ruled the first.** *(`.rulings/` R-106.)*

### 11.1 Why the recommendation was wrong

It argued that nothing on the board holds a secret this would protect, so the
urgent problem was the false impression of settledness and a status-line edit
would remove it. **That is true and it is not the whole question.** A downgrade
makes the record honest about a gap and leaves the gap; the first thing that
needs a secret — `ADR-034` §7.1's wireless PMK is already waiting, `PLAN.md`
§102.4.1 item 9 — would then be blocked on days of cryptographic design decided
under time pressure. **Deferring does not remove the work, it moves it to the
worst moment to do it.**

### 11.2 The sub-question this makes urgent, and it is genuinely open

**What "bound to this board" means on a Pi 4.** §7 already calls the board
provider what it honestly is; there is **no TPM and no secure element**, and the
serial number is readable by anything on the machine, so it is an identifier and
not a secret.

So an implementation has to answer, before it writes any code: **what does the
key derive from, and what attack does that actually stop?** A key derived from
readable board facts stops an attacker who has the card and not the board. It
does **not** stop an attacker who has the board — and `ADR-025` §5.2 already
rules that **a stolen Pi is a compromised identity**, so the honest claim is
narrow and must be stated as narrow. **An envelope that implied more than that
would be worse than none**, because the operator would store something in it
that deserved better.

**That question is the first deliverable, not an implementation detail**, and it
is the maintainer's to rule once the options are written up.

#### 11.2.1 What the closest relative does, measured 2026-08-30

Asked rather than assumed, of a Raspberry Pi running **Raspberry Pi OS on
Debian 13 (trixie)**, with **NetworkManager** active and `wpa_supplicant` as
its backend — the same stack a current Pi deployment gets.

Its one wireless credential lives at
`/etc/NetworkManager/system-connections/<ssid>.nmconnection`:

| Fact | Measured |
| :-- | :-- |
| Mode and owner | `-rw-------  root root` |
| Secret form | the **raw passphrase, stored as typed** — not a derived 64-hex PMK |
| `psk-flags` | **absent everywhere**, so the default `0` applies: the secret is stored in the file and owned by the system |
| Encryption at rest | **none** |
| TPM or secure element | **none present** |

**So Raspberry Pi OS does not answer §11.2 — it declines to.** The entire
boundary is file permissions plus physical possession of the card. Anyone who
removes that card reads the wireless passphrase.

That is worth knowing for two reasons. It sets the bar Rivet is actually
measured against, which is lower than it feels. And it means **the storage form
is a separate question from the derivation form**: Raspberry Pi OS stores the
passphrase and lets the supplicant derive, which is the shape `ADR-034` §7.1.1
already ruled for Rivet when it chose to add PBKDF2 rather than make the
operator paste a PMK.

#### 11.2.2 The three honest options, for the maintainer

**A — No envelope, said plainly.** Raspberry Pi OS's answer. Secrets sit in a
root-only file; the card is the boundary; the documentation says so in one
sentence and never implies more. Cheapest, and no claim can rot.

**B — Derive from readable board facts, and claim only what that buys.** An
envelope keyed on the board serial and similar. It protects **a card separated
from its board** and nothing else — which is not a hypothetical case but the
most likely leak in practice: a card pulled and imaged, a failed card discarded,
a card cloned for a spare. It does **not** protect a board, and it must never be
described as though it does.

Its consistency argument is already in the record: `ADR-025` §5.2 rules that **a
stolen Pi is a compromised identity**. B claims exactly the complement of that
and nothing more, so the narrow sentence B requires is one this project has
already committed to writing.

**C — An operator-supplied secret at boot.** The only option that protects a
stolen board. It requires someone present at every boot, which is incompatible
with a headless fleet that must come back after a power cut — the posture the
whole system is built for.

**Recommendation: B, conditional on the claim being stated as narrowly as
`ADR-025` §5.2 already states its complement.** A is defensible and is what the
neighbours do; C is disqualified by the deployment model. B's cost is small and
it covers the card-disposal case, which is the one that actually happens. **If
the narrow sentence cannot be kept narrow, take A instead** — §5.4 of `ADR-005`
governs, and an envelope that implies more than it delivers is worse than an
honest absence, which §11.2 already says in its own words.

#### 11.2.3 RULED 2026-08-30: plain text, and obfuscation is optional

After the measurement above was put to him, the maintainer ruled that a Rivet
stores its secrets as plain text, and that obfuscating them is left to whoever
implements it. *(`.rulings/` R-270.)*

**So §11.2 is closed and the answer is A.** A Rivet's secrets are stored in
plain text in a root-only file, exactly as its closest relative does, and the
card is the security boundary. **Obfuscation is permitted and is not required** —
an implementer may add one, and nothing depends on it.

Two consequences follow and neither is optional:

1. **It must be documented as plain text**, in the operator-facing words, not
   only here. A secret whose protection is *"the card"* is fine; a secret whose
   protection the operator has to guess at is not. `ADR-011`'s rule is that the
   system says which one, at the moment the answer is needed.
2. **An obfuscation, if one is written, may never be described as encryption or
   as binding.** It buys the card-separated-from-its-board case (§11.2.2 B) and
   nothing more, and `ADR-025` §5.2 already commits this project to saying so.
   `ADR-005` §5.4 governs: the partial promise is the failure mode, not the
   missing feature.

**This unblocks §11 itself**, which was ruled BUILD IT on 2026-08-29 and was
waiting on this sub-question. It is now work rather than a question, and
`ADR-034` §7.1.1's wireless PBKDF2 — which needs somewhere to put a passphrase —
is unblocked with it.

##### 11.2.3.1 The obfuscation, chosen the same day: hex, and it must be labelled

The maintainer chose obfuscation over plain text, in the simplest form: store
the secret hex-encoded. *(`.rulings/` R-322.)*

**Accepted, with one constraint that is not stylistic.**

Hex is an *encoding* and not a protection, and nothing here pretends otherwise.
What it buys is narrow and real: a card grepped for a known password does not
hit, and a document pasted into a bug report or a chat does not leak a
credential in the clear. For a personal fleet that is the disclosure that
actually happens, and the cost is nearly zero.

**The constraint: the field name must say it is hex.** `ADR-034` §7.1.1 adds
PBKDF2, so this system is about to have genuine 64-hex-character PMKs in the
same documents. An unlabelled hex blob beside them is ambiguous in both
directions — a hex-encoded passphrase read as a derived key, or a derived key
read as recoverable plaintext — and both misreadings are dangerous. So the key
is `psk-hex` and not `psk`, and the encoding is never the thing that carries the
meaning. **The label is the safeguard; the hex is the convenience.**

`ADR-011` is the rule being applied: the system says which one, at the moment
the answer is needed.

**Confirmed 2026-08-30, against a proposal to do more:** obfuscation is enough,
because it stops a casual attacker and nothing can stop a determined one who
holds the board, so the design does not go further. *(`.rulings/` R-363.)*

**That is the right test and it settles the level.** There is no TPM, so an
attacker holding the board is unwinnable by construction and `ADR-025` §5.2
already concedes it. Design budget spent on that case buys nothing. What
obfuscation buys is the casual and accidental case, and that is all it is asked
to buy. A proposal to keep secrets out of the document entirely was put and not
taken; it is recorded in §11.2.4 rather than lost, and nothing waits on it.

**The form the hex should take: the derived PMK.** `ADR-034` §7.1.1 already
rules PBKDF2 in, so the primitive is paid for. A PMK is 64 hex characters, so it
is exactly as unreadable to a casual reader as hex-encoded plaintext and stops
precisely the same person — and it is salted with the SSID, so it is useless on
any other network and never puts a passphrase that may be reused elsewhere onto
the card. Same effort, same protection, one fewer thing leaked. It also disposes
of §11.2.3.1's labelling constraint, because a PMK **is** a derived key and can
no longer be mistaken for one.

#### 11.2.4 Not taken: keeping secrets out of the document

Proposed and declined the same day. `/state/config` holds the document and
`/state/secrets` holds credentials (`ADR-019` §3.1, `ADR-025` §2.3), and the
document is what `get desired-state` prints and what gets pasted into reports —
so a reference in the document and the secret in `/state/secrets` would remove
the disclosure surface rather than encode it.

It was not taken because it needs new surface in `apply document` and the
compose path to carry a reference without a value, and the ruled level of
protection does not justify it. **Recorded because the same question returns the
moment a second kind of secret arrives.**

#### 11.2.5 BUILT 2026-08-30

`rivet-core/src/secret.rs`, and it is **much smaller than this ADR describes**
because §11.2.3 removed two of its four mechanisms.

**What was built:**

* **`Secret`** — §4's redacting type, on `secrecy` and `zeroize` as §4 named
  (`ADR-009` rung 1). `Debug` renders `<secret>` and **there is no `Display` at
  all**: redacting `Debug` alone is not enough, because `format!("{value}")`
  would still print it and that is the shape the accident takes. `expose()` is
  the only route to the bytes, named so that reaching for it is visible to a
  reviewer and greppable.
* **`state::SECRETS`** — §5's directory as one authority. It had been a string
  literal in `cmd/rivet-initd`, `cmd/rivet-image` and `cmd/rivet-sysd`; those
  now read the constant, and where a `const` cannot be built from another by
  concatenation the agreement is a tripwire test instead.
* **The stored form is hex** — §11.2.3.1. `to_stored` and `from_stored`, with
  `from_stored` **refusing** an odd length, a non-hex character or bytes that
  are not text, because a secret reconstructed from a damaged file is a wrong
  credential presented as a right one.

**What was NOT built, and will not be:** §6's envelope encryption and §7's board
provider. §11.2.3 ruled plain text, so there is no `KeyProvider` port and no
cipher. Those sections describe a design that was considered and declined; they
are left readable because §11.2.2's options are what the decision was made
against.

**Thirteen tests**, and two of them exist to stop a later reader mistaking the
encoding for protection: one asserts the stored form does not contain the plain
value, and one asserts it is **reversible by anybody**, which is the whole
claim.

**Not proven on a machine.** Nothing writes or reads a secret on a board yet —
`ADR-034` §7.1.1's wireless PBKDF2 is the first consumer and is the next thing
to build.

### 11.3 What does not change

* **The register still holds no secrets** — `ADR-034` §3, and this does not
  reopen it.
* **The signing seed is still out of scope.** It never leaves the build machine
  (`ADR-015` §4); this is about secrets a *machine* holds.
* `ADR-025` §5's dependency is real and is now being satisfied rather than
  documented as unsatisfied.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §9 — backups were entirely the operator's problem

**Superseded in part 2026-08-24 by `ADR-030`.** The superseded sentence read:

~~**Backups are the operator's problem.**~~

Rivet now backs up the configuration document and the secret store;
whole-`/state` backup remains the operator's. The rest of the bullet is
unchanged and is the reason `ADR-030` is scoped the way it is.
