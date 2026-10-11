# ADR-019: Fleet Enrollment

* **Status:** Accepted — ruled 2026-08-22. The shape is here; the identity
  mechanics §5 left open are answered in `ADR-025`.
* **Date:** 2026-08-22
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`ADR-006` §5.1 committed Rivet to being a fleet system: a dashboard on a
Windows desktop reading the state of two or three Raspberry Pis, pushing updates
and changing configuration. `ADR-007` §3.4 gained a *fleet* layer in the
configuration stack for exactly that.

**How a machine joins a fleet was never designed**, and it was flagged as the
one remaining gap most likely to reach backwards and change the configuration
schema — because how a machine identifies itself is exactly the kind of thing
that wants a field nobody added.

**The maintainer's model, 2026-08-22: in "fleet" mode, a machine's image is
created from the fleet admin tools, flashed, and the card put into the device —
which then wakes up already carrying its Wi-Fi, its LAN, its fleet connectivity
and its certificates.** *(`.rulings/` R-041.)*

---

## 2. Decision: enrollment happens before first boot, not after

**Ruled: a machine arrives enrolled.** There is no join protocol, no discovery,
no trust-on-first-use handshake, and no enrollment step performed against a
running device.

This is a better answer than the conventional one, and the reasons are worth
stating because "design a join protocol" is what most systems reach for:

* **Nothing to design.** No discovery, no token exchange, no rendezvous service.
* **No bootstrap race.** A join protocol needs the network up before the device
  has the credentials that configure the network — a circularity that
  provisioning-before-boot simply does not have.
* **It works offline.** A device on a bench with no fleet server reachable still
  comes up correctly configured.
* **Credentials never traverse a network to enroll.** They are written to a card
  by the operator's own tooling.
* **The device is useful on first boot**, with no manual step afterwards.

---

## 3. Generic image, per-device provisioning file

The maintainer's starting picture was an image made for each machine by the
fleet admin tools, which suggests one image per device. *(`.rulings/` R-041.)*
**Ruled: the image is generic and the per-device content is the provisioning
file.**

The mechanism already exists. `ADR-004` §6 puts `rivet.toml` on the FAT boot
partition, readable and writable from any host with a card reader, and
`ADR-018` §4 keeps it there. Enrollment extends that file rather than adding
anything:

```
rivet.toml   (FAT boot partition, per device)
  wireless credentials      NOT BUILT — refused at parse time; see §3.1
  fleet endpoint
  device identity           three files BESIDE rivet.toml, not keys in it
```

*(**Annotated 2026-08-29.** Two of the three rows are not what a reader would
take from this block. The identity is `device.seed`, `device.crt` and `peers`,
carried as separate files on the boot partition rather than as keys in this
document — `ADR-020` §4's `Secret<T>` is not built, so a seed in the schema
would render in `export`, in `--explain` and in `get schema`. §3.1 carries the
wireless row.)*

**Why generic beats per-device:** you build one image and flash N cards with N
small text files, rather than building, storing and tracking N full images each
containing credentials. Image builds are slow, images are large, and an image
per device means every device's credentials exist as a multi-gigabyte artifact
on the build machine indefinitely.

### 3.1 The secret's lifetime — HALF of it is handled, stated 2026-08-29

`ADR-004` §6 established the sequence and it extends unchanged: PID 1 reads the
provisioning file, derives what it needs, writes the result into **two
destinations**, and **blanks the credentials from the provisioning file.**

**The two destinations, because one name for both was a conflation that cost a
reader.** The provisioning file carries a document and an identity, and they do
not go to the same place:

| What | Where | Authority in the source |
| :-- | :-- | :-- |
| the desired-state **document** | `/state/config/machine.toml` | `cmd/rivet-initd/src/seed.rs` |
| the machine's **identity** — device key, certificate, seed | `/state/secrets` | `cmd/rivet-initd/src/identity.rs`'s `SECRETS_DIR`, and `cmd/rivet-sysd`'s `MCPD_IDENTITY_DIR` points `rivet-mcpd` at it |

`PLAN.md` §102.4.1 item 10 recorded this section and `ADR-025` §2.3 as
**disagreeing**; they were not. They were describing different contents, and
this one named the wrong home for the credentials. Worth naming, because two
ADRs that genuinely disagree need a ruling and this needed a reader; filing it
as the former is what `PLAN.md` §102.4.3 is about.

**The device identity: TRUE, built, and tested.** `cmd/rivet-initd`'s
`identity` module copies all three files to `/state/secrets`, `sync`s,
**then** overwrites `device.seed` on the boot partition with zeros, `sync`s,
unlinks and `sync`s again. Blanking and unlinking are two steps rather than
one because *unlinking a file on FAT does not erase its bytes* — a board that
had provisioned itself was found still carrying its own seed in the boot
partition's free space, which is a measurement rather than a worry. Three
tests hold it, including one that asserts the zeros are written **before** the
unlink. `PLAN.md` §78 is the board reading.

**The wireless passphrase: NOT built.** `cmd/rivet-initd`'s `seed` module says
so in its own words — *"`ADR-004` §6 step 3 — removing the credential from the
provisioning file — is outstanding. **It is not merely unimplemented: it has
nothing to remove.**"* `ADR-007` §7.3 keeps wireless credentials out of the
configuration document entirely and puts the PMK in `ADR-020`'s encrypted store,
so **`rivet-config` declares no key a passphrase could be written under and
refuses one at parse time as an unknown key.** A `rivet.toml` carrying
`psk = "…"` is `Provisioning::Refused` today, by test.

**So the honest statement of §3's promise:** a card cannot carry a wireless
passphrase at all, which means none is left on one — and also that **§2's *"let
it wake up already with WiFi"* is not deliverable today.** Wired provisioning
works and is proven on a board; wireless provisioning is blocked on `ADR-020`
§6's envelope and §7's board provider, which `PLAN.md` §102.4 has carried as
**ruled and unbuilt since 2026-08-22**. Until 2026-08-29 this section reported
that unbuilt half as done, in the highest-stakes place the record has such a
claim — a present-tense statement about what an attacker gets from a stolen
card, in the section a fleet operator reads before writing N of them.
`AGENTS.md` §7 binds a claim of presence exactly as hard as a claim of absence.

`tools/card/provisioning.example.toml` is the operator-facing truth and
already agrees: it documents `[network]` and offers no wireless
table.

#### 3.1.1 A card seed that differs from `/state`'s is left in place

`cmd/rivet-image` writes the identity onto both boot partitions, so PID 1
looks at the one it booted and at the other. On either, a seed is blanked and
unlinked **only when `/state` holds the same key** — byte for byte, with its
certificate beside it, which is the copy proven to be durably on `/state`.

**When `/state` holds a complete identity that differs from the card's, PID 1
keeps `/state`'s, says so, and leaves the card's seed in place**, on both
partitions alike. (`.rulings/` R-210 D15.) A seed on the card of a provisioned
machine is never an instruction to replace that machine's identity — §4 makes
rotation a configuration change — so nothing from the card is installed. But
nothing on the machine can tell a leftover from the flash from the only copy
of a key an operator still needs, and destroying a key nobody here holds on the
strength of a comparison that failed is the one outcome that cannot be undone.
The cost is a private key on a FAT partition until somebody removes it by hand
or rewrites the card, which is what §3.1 would rather not have; the mismatch is
said on the console and in the kernel's log at every boot it persists, so it is
not a cost anyone pays without being told. A seed `/state` does not hold at all
is left for the same reason: it may be the last copy.

**"Complete" is judged by length**, because the identity set carries no
checksum. `/state`'s seed is the same length as the card's and differs from it:
that is a different key, not a fragment of this one. One of a different length
is the shape an interrupted copy leaves, and is overwritten from the card.

**A checksum in the identity set was considered and is not added.**

| | For | Against |
| :-- | :-- | :-- |
| A digest file beside `device.seed`, written by the fleet tool and by PID 1 | proves completeness directly rather than inferring it from a length; would let PID 1 tell a torn `/state` seed from a different key of the same length | a seed is 32 bytes and `write_secret` already publishes it atomically, so a torn seed of the right length is not a state the copy can produce; a digest beside a secret is a second file that can itself be torn or stale, and its absence on every card already written has to be read as something |
| A digest inside the certificate | the certificate already travels with the seed | the certificate is self-signed DER that peers pin, and the seed's public half is already in it — which is a stronger check than a digest, and one that needs no new field |

The second row's observation is the more useful one: whether a certificate's
public key is the seed's public half can be checked from the two files that are
already there. It is not done here, because it would put certificate parsing in
PID 1 and the length rule already covers every state the atomic copy can
leave. Adding a digest file would
change the card's on-disk format and every tool that writes one, which is the
maintainer's to rule.

### 3.2 It composes with the configuration layers

`ADR-007` §3.4's stack is schema defaults, image defaults, **fleet**, machine.
Enrollment supplies two different things into two different layers:

* the **fleet endpoint and identity** — which is what lets the device receive
  the fleet layer at all
* **machine-layer values** the operator set at flash time, such as a hostname

That distinction matters: the fleet layer is *received*, not provisioned. The
provisioning file carries only what is needed to start receiving it.

---

## 4. Rotation is a configuration change, not a reflash

Once a device has enrolled, its identity lives in `/state/config` and is subject
to everything else in the record: it is typed, it has provenance, changing it is
a `plan` the operator confirms, and it can be pushed from the dashboard.

**Reflashing is never required to rotate a credential.** That is the property
that makes provisioning-at-flash-time acceptable rather than a trap — the
enrollment path is used once, and the ordinary configuration path handles
everything afterwards.

---

## 5. Answered in `ADR-025`

The *shape* is ruled here. What identity actually consists of was settled the
same day in `ADR-025`: an Ed25519 keypair pinned by the fleet with no
certificate authority, generated by the fleet tool which retains only the public
half, and — the decision the reflash question actually needed — **a logical
device that is separate from the credential it currently holds**, so
configuration and history survive a board replacement and revocation and reflash
become one mechanism.

The original questions are kept below for their reasoning.

1. **What is the device identity?** A long-lived key pair with a certificate is
   the obvious answer, and the fleet endpoint would pin the trust set. A
   one-time enrollment token exchanged on first boot was considered and rejected
   for §2's offline reason, but remains available as an *alternative* mode where
   the fleet service is reachable and a leaked card should be useless.
2. **Who issues it?** The fleet admin tool holds a signing key, which makes it a
   small certificate authority. That is not difficult and it is a responsibility
   worth naming before it is implicitly acquired.
3. **What happens on re-flash?** Whether a device keeps its identity across a
   reflash — implying identity is derived from something stable like the board
   serial — or receives a new one, which is simpler and means the fleet sees a
   new member.
4. **Revocation.** How the dashboard stops trusting a device whose card was
   lost, given that the device may not be reachable to be told.

None of these blocks Stage 0 through 2. All of them want answering before the
fleet dashboard is built, because they shape the identity field in the schema
and that field is written into every device's configuration.

---

## 6. The first run — what an operator must hold before a card is worth building

Decided 2026-09-05. §2 rules that **a machine arrives enrolled**, and §3 rules
that its credentials reach the card from the operator's own tooling. Neither
says what that operator has to have on their workstation first, or in what
order — and the absence was expensive, because every piece existed and nothing
sequenced them.

### 6.1 The set, and why each member is separate from the others

| | Where | Signs or proves | Made by |
| :-- | :-- | :-- | :-- |
| **system signing seed** | `~/.rivet/keys/rivet-system.seed` | `update` and `extension` payloads | `cargo rivet key new` |
| **application signing seed** | `~/.rivet/keys/rivet-app.seed` | `application` and `library` payloads | `cargo rivet key new` |
| **operator identity** | `~/.rivet/identity/operator/` | the workstation end of `ADR-035`'s control plane | `cargo rivet identity new` |
| **board identity** | `~/.rivet/identity/board/` | the machine end, plus the `peers` file naming the operator | `cargo rivet identity new` |
| **fleet authority** | `~/.rivet/ca/` | the web portal's TLS and the browser's client certificate | `cmd/rivet-provision` |

**The two signing seeds are two seeds and must stay so** — `ADR-028` §5.5. A
machine able to sign an application must not be able to ship an operating
system, and `cmd/rivet-image` has a test asserting the system key is not
entitled to sign an application.

**The authority is the only one of the five that is optional.** A board is
completely reachable from a command line without it; only the administration
portal of `ADR-043` needs one. Everything above it is required, and its absence
is a refusal rather than a warning.

### 6.2 The order is the ruling, not the list

Each depends on the one above it, and the last is the one that matters: **the
board's `peers` file has to name the operator's public key**, and that key does
not exist until the operator identity does.

`tools/card/build.sh` copies that file onto the boot partition, `cmd/rivet-initd`
copies it to `/state/secrets`, and it is thereafter the machine's whole answer
to who may reach it. A card built without it produces a board that boots, takes
its address, brings up its control plane and refuses the person who built it —
on hardware with no shell, no login and no serial console. **There is no repair
short of writing the card again**, which is the property that makes this a
refusal rather than advice.

### 6.3 The sequencer is `cargo rivet setup`, and the argument for it

The alternative considered was a mode of `cmd/rivet-provision`, which describes
itself as the one command-line tool meant for a person and is the right model
for tone and interaction. It was rejected on two specific grounds:

* **Everything the sequencer calls already lives in `cmd/cargo-rivet`** — `key
  new`, `identity new`, and the crypto and TLS crates behind them. Putting it in
  the other binary means duplicating those or shelling out, and `ADR-011` is
  about the first.
* **`cmd/rivet-provision` argues against it itself.** That binary mints and
  holds the fleet's root key, and it hand-rolls a console-mode call rather than
  take a crate precisely to keep the number of things that could read one small.
  Handing it a signing seed and two crypto crates spends the budget that
  reasoning protects.

So the split is: **`cargo rivet setup` mints the keys; `rivet-provision` mints
the authority and composes the document.** Each stays the tool that already
owned its half, and each ends by naming the other.

**It is not a full-screen interface**, and that is deliberate rather than
unfinished. Every path is conventional and every name is cosmetic, so a prompt
would have one sensible answer — which teaches a person to press Return without
reading, the habit that makes a real confirmation worthless. It also means
`ADR-012`'s first-class agent operator can run it, where a screen blocking on
stdin could not.

**It never overwrites a key**, and that is the one property of it worth
asserting anywhere: overwriting a seed orphans every board that trusts its
public half and every payload signed with it, with no undo.

### 6.4 Two refusals, and the one thing that can only be a note

`cargo rivet provision` refuses before it writes anything:

1. the set above is not on this host — naming each missing piece and the command
   that makes it; and
2. **the image's own `peers` file does not name the operator's key.** This is
   read out of the image rather than inferred, because the card writer already
   opens that partition to inject the document. `--not-mine` is the escape, for
   a board somebody else operates.

What cannot be a refusal is whether the card's **trust store** carries the
operator's signing keys. That store lives on the `/system` slot, which is ext4;
only the FAT boot partitions are reachable from a host tool. So the tool
compares what it can — the checkout's committed `keys/*.pub` against the seeds
on this host — and says what it compared. Refusing on a guess about a build that
has already happened would be `AGENTS.md` §5 pointed backwards.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §3.1 — the secret's lifetime was said to be already handled

**Corrected 2026-08-29.** The heading read ~~The secret's lifetime is already
handled~~, and one of the two credentials it names is not handled and cannot be
yet. This is the `ADR-015` §3.2 defect in the highest-stakes place the record
has one: a present-tense claim about what an attacker gets from a stolen card,
in the section a fleet operator reads before writing N of them. The superseded
paragraph read:

~~So an enrolled card carries secrets only between the operator writing it and the
device's first boot. A card recovered from a running device does not carry the
wireless passphrase or the enrollment identity.~~

The device-identity half is true, built and tested. The wireless half was true
only vacuously: a card cannot carry a passphrase at all, because the schema
declares no key one could be written under.

### In §3.1 — one destination was named for two different things

**Corrected 2026-08-29.** The superseded path was ~~`/state/config`~~, which
named one destination for the desired-state document and the machine identity
both. `PLAN.md` §102.4.1 item 10 recorded this section and `ADR-025` §2.3 as
disagreeing; they were describing different contents, and this one named the
wrong home for the credentials. The two-row table in the body is the correction.
