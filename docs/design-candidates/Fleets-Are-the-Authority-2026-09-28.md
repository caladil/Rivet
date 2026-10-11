# Fleets Are the Authority — the Armada as bookmarks

* **Status:** Ruled 2026-09-29 (`.rulings/` R-228) and promoted to `ADR-063`,
  which is the decision. Kept for its argument and evidence.
* **Date:** 2026-09-28
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`ADR-059` places an operator's whole estate on one PC: the certificate
authority, the artifact repository and the registry of fleets all live in that
PC's `~/.rivet`, and §4.3 rules one Armada per PC. The first time the maintainer
used a second workstation, that model produced two Armadas.

What the second workstation exposed, briefly:

1. **Copying `~/.rivet` makes two Armadas, not one in two places.** They agree
   only until either changes. They diverged the same evening: a system update
   built on the Mac went through a scratch repository so the two copies would
   stay identical, which left a board running a build neither repository held.
   The Windows workstation then built `0.1.121`, which only the Windows copy
   holds, and whose publish pruned an older card the Mac's copy still lists.
2. **The workstation repository is a second source of truth the moment it is
   copied.** Nothing notices when the estate and the repository disagree. What
   stops worse is incidental: `fleet sync --prune` refuses unless the server's
   manifest is this workstation's own `main`.
3. **The Armada's state is split across two directories**, one of them in the
   platform's own configuration directory (`rivet-fleet`'s register), and
   nothing tells the operator.
4. **A fresh PC's first `provision` silently mints a new certificate
   authority.** On a second workstation that is exactly wrong, and the only
   guard was an agent reading the ADR first.
5. **No backup verb covers an Armada.** The seeds have one, the authority has
   one; the registry, identities, repository and register have none.

These are not tooling defects. Each one follows from the premise that the
estate's record lives on a workstation.

---

## 2. The proposal

**Each fleet server is the authority for its fleet.** It holds the fleet's
repository, its member register, and — if it has no domain — its own
certificate authority. **A
workstation holds bookmarks to fleets, the operator's keys, and fetch-only
clones.** A second workstation is then a second client of the same fleets, with
nothing to reconcile.

| | Today (`ADR-059`) | Proposed |
| :-- | :-- | :-- |
| Artifact repository | the workstation's `~/.rivet/fleet`; each fleet server holds a copy pushed from it | each fleet server's `/state/git/fleet.git` and object store; a workstation holds a **clone** that fetches and never publishes |
| Member register | the fleet server (`ADR-034`), cached on the PC | unchanged |
| Registry of fleets | the Armada Registry on the PC | the same, renamed to what it is: **bookmarks** |
| Browser server certificate | issued by the one operator root, installed on every device | **domain mode**: a public certificate, nothing installed; **fleet-root mode**: that fleet's own root (§5.2) |
| Browser login | a client certificate from the operator root | **passkeys**, in both modes (§5.3) |
| Operator identity | minted per workstation, trusted through `peers` | unchanged in mechanism; a second workstation is **enrolled**, never copied (§4) |
| Signing seeds | on the PC | on the PC, reframed as **publisher** keys (§6) |
| "One Armada per PC" | ruled | **retired** — there is nothing to be one of |

---

## 3. Two modes, and the Armada as bookmarks

**A board is a fleet server or a fleet member.** `ADR-059` §2 already made a
lone box a server with no other members and removed `StandAlone`, so this is
the existing model stated from the board's side, with no third mode.

**A bookmark is how to reach a fleet and nothing it could be wrong about:**

    name        rivet-01
    server      rivet-01.local:7777
    pin         <the fleet server's Ed25519 public key>
    identity    which of this workstation's operator identities it knows

Everything else a tool wants to show — members, artifacts, what each machine
runs — is **read from the fleet server** at the point of use, and cached behind
`ADR-059` §5's 24-hour TTL where a cache is worth having. A bookmark cannot go
stale in any way that matters, because it asserts nothing about the fleet's
contents. Losing every bookmark costs retyping one address and one pin per
fleet.

This is `ADR-059` §5's own rule — *the PC is a pure client* — extended from the
member list to everything.

---

## 4. Operators are enrolled, not certified

**The control plane already works this way.** `ADR-025` §2 makes the pin the
whole of the trust model, and `ADR-019` §6.2 makes a board's `peers` file its
whole answer to who may reach it: a list of Ed25519 public keys, in the shape of
`authorized_keys`. No certificate authority is consulted on port 7777.

So adding a workstation is adding one key to one list:

1. The new workstation mints its own operator identity (`cargo rivet identity
   new`). Its private key never leaves it.
2. An **already enrolled** workstation sends that public key to the fleet
   server over the control plane, and it reaches the members' `peers` files.
   Members already carry one key that is not the provisioning workstation's —
   `rivet-fleetd`'s, staged into each member's `peers` by the provisioning
   interview (`ADR-049` §12) — so a list longer than one is not new. **Carrying
   a key to members that are already running is.**
3. The new workstation adds a bookmark. Done.

**Revoking a lost laptop removes one line** and touches no other workstation,
which is the property copying `~/.rivet` can never have: a copied identity is
one key in two places, and revoking it locks out both.

**The first operator of a new fleet is enrolled at birth**, by the provisioning
that writes the fleet server's card, as today.

**What this needs:** a verb that appends a key to a fleet's `peers` and
carries it to the members. `apply enroll-member` (`ADR-059` §6.1) is the
nearest shape — a control-plane subject writing a record the fleet server then
serves — and the operator list is a second record of the same kind.

---

## 5. Browsers: two modes for the server certificate, one login

### 5.1 Every channel, and which one needs a certificate authority

| Channel | Secured by | Needs a CA |
| :-- | :-- | :-- |
| Workstation tools ↔ board (`ADR-035`) | TLS 1.3, pinned Ed25519 keys on both ends (`ADR-025` §2) | **no** |
| Board ↔ board (fleet server and members) | pinned keys in `peers` | **no** |
| Phone ↔ board (Gantry) | short-lived tokens signed by the phone's hardware key (`ADR-061`) | **no** |
| Browser ↔ board | a certificate the browser trusts, and a login | **yes, today, for two separate jobs** |

The first three rows are the SSH and WireGuard trust model, and nothing in this
proposal changes them. `ADR-019` §6.1 already calls the fleet authority the one
optional credential of the five, because a board is completely reachable
without it.

**The authority exists for browsers, and does two unrelated jobs there:**

1. **The server certificate.** A browser must trust `rivet-01.local` before it
   will talk to it. No public certificate authority issues for `.local` or
   `home.arpa` names, so only a private root can, and that root has to be
   installed on every device that visits.
2. **The login.** `rivet-proxyd` requires a client certificate chaining to the
   same authority (`ADR-046` §4, `.rulings/` R-129), and the operator is the
   certificate that names no host (R-187).

**The two jobs have different answers.** The first depends on whether the fleet
has a domain; the second does not depend on anything, and is replaced in both
modes (§5.3).

**A workstation never needs to know a fleet's root in advance.** It already
knows the fleet server's pin, from provisioning or from a bookmark, so it
fetches the root over the pinned control plane: the pin vouches for the root.

### 5.2 Two modes for the server certificate

A fleet declares one of two modes. The proxy's source of certificates is the
only thing that differs; the login, the control plane and every non-browser
channel are identical in both.

| | **Fleet-root mode** | **Domain mode** |
| :-- | :-- | :-- |
| Who issues the server certificate | the fleet's own root, name-constrained to the fleet's names | Let's Encrypt, through ACME's DNS-01 challenge |
| Names browsers use | `rivet-01.local`, `<fleet>.home.arpa` | `rivet-01.<fleet>.<domain>` |
| Reaches an Android phone | **not by a `.local` name** — Android's browsers do not resolve mDNS names; a `home.arpa` name works only if a LAN DNS server answers it | yes: public DNS answers the name |
| Installed on each device | **the fleet's root, once** | nothing |
| Needs | nothing beyond the fleet | a domain, a DNS provider with an API, and a token for it on the fleet server |
| Internet | never | outbound, to renew, every month or two |
| Survives losing the fleet server | only if the root is in the backup clone (§7) | always: certificates are disposable and reissued in a minute |
| Suits | a fleet with no domain, or one that must never reach the internet | any fleet that can have a domain |

**Domain mode is the recommended mode and fleet-root mode is the fallback.**
Domain mode removes the certificate authority from Rivet's browser path
entirely: nothing is installed on any device, nothing irreplaceable exists to be
backed up, and a visitor's own phone simply works.

**What domain mode costs, stated rather than minimised:**

* **A domain**, about $10–15 a year. Free dynamic-DNS services with an API
  (deSEC, DuckDNS) can answer DNS-01 at no cost, at the price of depending on a
  third party's free service staying up — a stepping stone rather than the
  settled answer.
* **A DNS API token on the fleet server**, scoped to the one zone. Whoever holds
  it can obtain certificates for the fleet's names; Certificate Transparency
  makes any such issuance publicly visible.
* **Renewal needs outbound internet.** A fleet server cut off for longer than a
  certificate's lifetime shows browser warnings until it reconnects. The
  control plane never uses these certificates and is unaffected. rivet-01
  already reaches the internet for time, weather and an addon's relay.
* **Certificate Transparency publishes every certificate's names.** A wildcard
  certificate (`*.<fleet>.<domain>`) publishes only the wildcard.
* **Public names that resolve to private addresses are blocked by some home
  routers** (DNS-rebinding protection). The fleet server answering its own names
  on the LAN avoids it, as does allowing the domain in the router.
* **One new dependency, an ACME client** — a `§1.1` third-row decision.

**Fleet-root mode is today's design with the root moved into the fleet**, and
it is still better than today: one root per device replaces a root plus a
client certificate per browser (§5.3).

**A phone is where fleet-root mode is weakest, and not because of the
certificate.** The maintainer's Android phone could not open
`https://dev-01.local/passkey/` at all: the name did not resolve. `.local` is
multicast DNS, which Android's browsers do not use, so no certificate or login
choice rescues it. Fleet-root mode reaches a phone only through an ordinary DNS
name — the fleet root already permits `<fleet>.home.arpa` and `<fleet>.lan` —
and something on the LAN has to answer those names.

**`ADR-047` is that something.** It rules a name service on the fleet server
that answers the household's names under the fleet's suffixes and forwards
everything else upstream, with the router keeping DHCP and advertising the fleet
server as the resolver. A probe proved it on a board (`ADR-047` §7); the service
itself, and its table, are not built — the workstation's resolvers today are the
two routers, and neither answers `dev-01.home-fleet.home.arpa`. Once it runs,
fleet-root mode reaches a phone that trusts the root, provided each host's
certificate also carries its `home.arpa` name.

**What that costs is `ADR-047`'s own trade, restated because it now carries
browser access too:** the household's name resolution runs through the fleet
server. A public resolver configured as the router's second choice keeps the
internet working through an outage, but phones query either resolver, so the
fleet's own names then resolve intermittently rather than always. Domain mode
skips this step, because public DNS already answers its names.

**Phones reach the fleet today without any of this** — Gantry and the phone
applications of `ADR-061` connect by address, with their own pinned trust and
signed tokens, not through a browser. What `.local` denies a phone is only a
**browser** on it: the web pages of TempMon, the other adopters and the portal.

### 5.3 Passkeys replace the client certificate, in both modes

The browser login becomes **passkeys (WebAuthn)**: Touch ID, Windows Hello or a
phone, synced across devices by the platform or a password manager, and bound
to the site's domain so they cannot be phished. `rivet-proxyd` is the relying
party: it checks the passkey and issues a `SameSite=Strict` session cookie.

**Enrolling the first passkey** uses a one-time code issued over the control
plane (`cargo rivet run <fleet> -- …`), the shape Gantry's pairing already has.
An enrolled operator can then enroll further devices from the browser.

**What it removes:**

* **The client certificate on every browser**, its picker dialog, and the
  near-impossibility of installing one on a phone.
* **R-187's defect class.** A member's front-door certificate once passed as the
  operator because both came from one authority and nothing distinguished them.
  An operator who is a passkey is not a certificate at all, so no certificate
  can be mistaken for one.
* **Most of `ADR-046` §4.5's reason to exist.** A client certificate is an
  ambient credential presented to any request, whichever page caused it. A
  `SameSite=Strict` cookie is not sent on a cross-site request. The `Origin` and
  `Sec-Fetch-Site` check stays as defence in depth.

**What it costs:** a WebAuthn implementation, which is the second new dependency,
and the build. R-129 kept client certificates because their weight was already
paid when `rivet-fleetd` was built; passkeys pay a new weight, and that ruling's
own argument counts against this section. The case for paying it is the phone
and every browser beyond the first, where client certificates do not work well
at all.

**A passkey on a `.local` name works in Chromium under an installed root.**
`examples/probes/passkey-probe`, served from dev-01 through its proxy as
`https://dev-01.local/passkey/` under the operator's installed root, created
and then used a discoverable credential with `rp.id = "dev-01.local"` twice:
once stored by Windows Hello, from the Chromium in the Claude desktop app, and
once stored by Google Password Manager, from Chrome on Windows. All four
ceremonies succeeded, recorded in dev-01's journal. A passkey held by Google
Password Manager syncs to the operator's other devices, so a phone that trusts
the root needs no enrollment of its own.

**Still unread:** Firefox, which has its own certificate store and its own
WebAuthn implementation, and Safari. **Fleet-root mode's login depends on the
browsers a fleet's users have**, so the probe stays in the repository to be
pointed at the next one. If one refuses, that fleet keeps client certificates
and domain mode alone gets passkeys.

**Switching a fleet from one mode to the other re-enrolls its passkeys**, because
a passkey belongs to a domain and the domain changes. That is the one thing the
two modes do not carry across.

### 5.4 Tools that go through the proxy

Some workstation tools use the operator's client certificate on HTTPS routes
today — Rivet Monitor's backups and restores (`ADR-051`) are the main case.
Passkeys are for people, so those tools need another credential: the pinned
operator key they already hold for the control plane, presented to the proxy,
or a short-lived token signed with it, in the shape `ADR-061` gives phones.
**This is a design item, not a blocker**, and it is the same answer in both
modes.

### 5.5 Where a fleet's root is minted, in fleet-root mode

Two choices, both allowed:

* **On the workstation, at provision time**, written onto the fleet server's
  card and then kept only in the fleet's backup clone (§7). This is today's
  arrangement with the root scoped to one fleet.
* **On the fleet server, at first boot.** `ADR-001` §7.3 permits it: the
  randomness Rivet depends on comes from the operating system, and a board
  asking the kernel through `getrandom(2)` is exactly that. No card binary links
  a randomness crate today, but that is an observation about the image, not a
  prohibition.

**The recommendation is the board**, because a root minted on the fleet server
cannot be left behind on a workstation. First-boot entropy is not a concern:
rivet-01's process table shows the kernel's `hwrng` thread parked in
`add_hwgenerator_randomness`, so the Pi 4's hardware generator feeds the pool,
and `getrandom(2)` blocks until the pool is ready rather than returning weak
bytes.

**The root stays name-constrained**, as today's already is
(`cmd/rivet-provision/src/ca.rs` sets X.509 Name Constraints and refuses a root
that permits everything). A leaked fleet root can forge that fleet's names and
nothing else.

### 5.6 What the two modes retire, and what they reverse

* **`ADR-059` §4.1** — one operator root, no new browser roots per fleet.
  Reversed: in fleet-root mode, a second fleet is a second root on each device.
  The cost is real and is the reason domain mode is the recommendation.
* **`.rulings/` R-129** — operator surfaces behind a fleet-authority client
  certificate. Replaced by passkeys, if §5.3's check passes.
* **`.rulings/` R-187 D1** — the operator as a certificate naming no host.
  Retired with the client certificate.

**Rejected:**

* **An overlay network (Tailscale, or Headscale self-hosted)** would supply
  certificates and remote access, but hands identity to a coordination server.
  Worth revisiting only if remote access becomes a goal.
* **A DNS zone run by the Rivet project** for every fleet, in the manner of
  `plex.direct`, would make the project operate infrastructure for ever.

---

## 6. The signing seeds are publisher keys

The two seeds (`ADR-028` §5.5) are the one thing this proposal leaves on the
workstation, and they are **not operator credentials at all**. They say who
built an artifact, not who administers a fleet. Today the maintainer is both
publisher and operator, so the two look alike; in a public Rivet they separate
cleanly:

* the **Rivet project** signs system updates;
* **each developer** signs their own applications;
* **each fleet** decides which publishers' keys it trusts.

This proposal does not change seed handling. It names the seeds as a separate
concern so the Armada's retirement is not read as retiring them, and so the
backup verb they need (wart 5) is designed for a *publisher* rather than bundled
into an operator's workstation backup.

A second workstation that publishes needs the seeds; one that only operates
does not. That is a smaller carry than today's whole `~/.rivet`, and it is the
only one left.

### 6.1 What identifies the operator, and how each part travels

The split between signing an operating system and signing an application
already exists and is enforced: `rivet-system` signs `update`, `extension` and
`card`; `rivet-app` signs `application` and `library`; and `cmd/rivet-image`
asserts the system key is not entitled to sign an application (`ADR-028` §5.5).
**The maintainer's Android signing key is a third publisher key** of the same
kind — it says who built an APK — and belongs with the other two.

| What | Secret | Changes | How it travels |
| :-- | :-- | :-- | :-- |
| **Bookmarks** (§3): name, address, pin | no — but a swapped pin is an impostor accepted, so its integrity matters | when a fleet is added | copied freely, to any private storage |
| **Operator identity** | yes | never | **never copied**: each workstation mints its own and a fleet enrolls it (§4) |
| **Publisher keys**: the two seeds and the Android signing key | the most sensitive things the operator holds | essentially never | a **one-time encrypted transfer**, not a sync |
| **Fleet backups** (§7) | partly | constantly | each workstation fetches its own clone |

**Nothing secret needs continuous synchronisation.** The publisher keys are
minted once and never change, so moving them is a single transfer; what does
change — the bookmarks — is not secret. Everything else is already the fleet's.

### 6.2 The recommendation: a workstation kit, and a workstation's own operator key

1. **`cargo rivet workstation export` / `import`** — one passphrase-encrypted
   file carrying the publisher keys and the bookmarks, and **deliberately not
   the operator identity**. `cargo rivet key backup` already produces the
   encrypted-seed half of it. Stored in the operator's password manager it is
   also the recovery copy. Importing it on a new workstation is followed by
   minting that workstation's own operator identity and enrolling it (§4). This
   closes wart 5 — no backup verb covered the Armada — and replaces carrying
   `~/.rivet` by hand.
2. **The Mac mints its own operator identity and is enrolled.** Today it uses
   the Windows workstation's, copied with `~/.rivet`: one key in two places, so
   revoking it for one locks out both. Of everything the second workstation
   exposed, this is the one arrangement that is wrong rather than awkward.

### 6.3 Stronger options, for when the threat model grows

* **A publisher key per workstation**, so no seed is ever copied. A board's
  trust store is a list of keys, each entitled for kinds
  (`rivet-core/src/package/trust_store.rs`), so trusting a second key beside
  the first fits the format. The card build takes one key per kind today, and
  adding a key to boards is a system update to every fleet; `ADR-028` §5.6's
  countersigned delegation would remove that update, and is deferred.
* **A hardware key for `rivet-system`**, so the key that signs an operating
  system never exists on a disk and is carried rather than copied. It needs
  signing code that talks to the device and a second key as its backup. Worth
  it before a public Rivet; not before.

---

## 7. The workstation's repository becomes a clone

### 7.1 The rule

**A clone fetches and never publishes.** Every publish goes to a fleet server,
where `ADR-052` §7.4's door verifies the signature and the digest before
anything is admitted. Every workstation's clone only pulls.

Two workstations then hold two clones that can be **stale but never
contradictory**, and the question the second workstation raised — *does the PC
disagree with the Mac?* — becomes a question of lag with an obvious answer:
fetch.

The mechanics exist on both halves. The text is git, and `/state/git/fleet.git`
already serves it over `ADR-035` §4's smart HTTP. The objects are addressed by
BLAKE3 digest, so a mirror is incremental and checks itself, and the listing
route (`ADR-052` §7.4.2) already says what the server holds.

### 7.2 What the clone is, and what it is not

* **It is the fleet's backup:** the repository's text, every object it holds,
  the member register, and — for a fleet in fleet-root mode — the fleet's root
  key. A fleet in domain mode has no root to keep: its certificates are reissued.
* **It is not the applications' data.** Trends' database, TempMon's readings
  and the rest are Rivet Monitor's backups (`ADR-051`). The two are kept apart
  on purpose: one restores a fleet, the other restores what users did with it.
* **If it carries the fleet's root key, it is sensitive**, and is encrypted at
  rest or keeps the key in a separate, named backup directory (`ADR-052` §4.1's
  rule: a tool that holds a secret takes a directory and invents none).

### 7.3 Restoring from it

**Provisioning a fleet server from a clone** is the restore verb: the same
interview, handed a clone instead of an empty repository, producing a server
that holds the fleet's text, objects, register and, in fleet-root mode, its
root. Its members keep
their identities and their pins; the new server's own pin is the one thing that
changes unless its seed is in the clone too, which is §9's question.

### 7.4 A system backup, and recovering a host from one

The clone is half of a **system backup**. The whole of one is everything
needed to put a host back as it was, and it has four parts:

| Part | Lives | Carried by |
| :-- | :-- | :-- |
| Desired-state documents, the register, the manifest of what each host runs | the fleet's git repository | the clone — **its whole history**, not only `HEAD`, because the history is the record of what changed |
| The artifacts those documents name | the fleet's object store | the clone — every object the fleet holds, which retention already bounds to three per product |
| Each host's identity, and each application's per-machine secrets | the workstation's identity directories and each board's `/state/secrets` — deliberately outside every repository (`ADR-052` §4) | an **encrypted** part of the backup, passphrase-sealed as `cargo rivet key backup` already is. Without it a recovered host has a new pin |
| Application data | each application, behind `ADR-051`'s backup contract | Rivet Monitor's application backups |

**One repository, two halves.** The documents and the artifacts are not two
repositories: `ADR-052` §2 keeps text in git and artifacts in a digest-keyed
store beside it, referenced by digest.

**The schedule runs on the machine that is always on.** Rivet Monitor measures
cadence from the last success, persisted in its own state, so it survives
restarts and reboots — but it runs only while the workstation is awake, which is
why it deliberately has no wall-clock schedule (the FleetMonitor repository's
scheduling specification says why: a sleeping desktop misses an appointed hour).
**The fleet server** is always on and its clock is set over NTP, so it can take
a system backup of every host at a time the operator chooses — *daily at 02:00*
— into its own store, and **Rivet Monitor mirrors those backups** onto the
workstation's backup folder on its own cadence. The two copies guard against
different losses: the fleet server's against a member failing, the
workstation's against the fleet server failing.

**Recovering a host is one interview: `cargo rivet recover`** (`restore` is
already `ADR-051`'s application verb):

1. **Choose a backup set and a date.**
2. **Choose the host.** Its desired-state document decides the rest: its role,
   its applications, and therefore which artifacts are needed.
3. **The artifacts are selected by digest** from that backup's objects —
   provisioning selects and does not build (`ADR-054`), and the card carries
   the applications (`ADR-050`).
4. **The host's identity comes from the backup**, so its pin is unchanged and
   every bookmark and member still recognises it.
5. **The card is written and verified** — `cargo rivet provision --existing`,
   which already loads a recorded document and reuses the host's identity.
6. **After first boot, each application's data is restored from the same
   backup set**, so the system and its data are from one moment.

**A recovery that writes over real data asks first, every time**, whatever
autonomy is granted for anything else.

**What exists:** `provision --existing` (steps 2, 4 and 5 for a workstation that
still holds the identity), `ADR-051`'s application backup and restore, and
`fleet objects backup`. **What does not:** the encrypted identity part, the
fleet server's scheduled system backup, Rivet Monitor's mirror of it, and the
interview that joins them.

### 7.5 Where builds go

A card or update is built in `out/` as today and **published to a fleet**, not
to the workstation. Scratch builds — probes, dirty builds, test APKs — stay in
`out/` and never reach a repository unless somebody publishes them.

This puts development traffic on whichever fleet it is published to. §9 asks
whether that should be rivet-01, which is production.

---

## 8. What changes in the record

| Document | Change |
| :-- | :-- |
| `ADR-059` | §2's Armada loses its CA and repository; §4.1 is reversed (a public certificate, or one root per fleet); §4.2 inverts (the fleet's repository is authoritative, the workstation's is a clone); §4.3 is retired; §5's registry becomes bookmarks |
| `ADR-052` | §2's repository lives on the fleet server; §6 already says *the signature carries the authority, not the repository*, and that argument carries over unchanged; §7.3's object backup becomes the clone |
| `ADR-019` §6 | the fleet authority leaves the workstation's first-run set; that leaves the operator identity, and the two seeds for a workstation that publishes |
| `ADR-046` | §4's gate checks a passkey session instead of a client certificate; §4.5's cross-site check stays as defence in depth; the proxy's certificate comes from ACME or the fleet's root, by mode |
| `ADR-043`, `ADR-051` | the portal serves the same certificate as the proxy; tools reach backup routes with the pinned operator key or a token signed by it (§5.4) |
| `.rulings/` R-129, R-187 D1 | replaced by passkeys, if §5.3's check passes — a ruling of their own, recorded as new rulings rather than by editing these |
| `SPEC-05` | a new workstation's first run asks *new fleet, or join one?* rather than creating an Armada |

---

## 9. Open questions

**Where the maintainer stood when this was written — leanings, not rulings.**
The direction itself (fleets authoritative, the Armada as bookmarks) is his. He
favoured a fleet minting its own root over an operator root; favoured the two
certificate modes with passkeys in both; favoured the clone as a fleet backup,
a system backup on a schedule he sets that survives restarts, and a `recover`
interview driven by the chosen host's desired-state document. He was undecided
whether a phone's browser needs the fleet's pages, calling that a large
limitation to weigh (question 8). **The next step is a grilling of this
document**, question by question, before anything is promoted to an ADR.

**The maintainer's to rule:**

1. **The two modes (§5.2).** Domain mode recommended, fleet-root mode the
   fallback — which reverses `ADR-059` §4.1 for a fleet in fleet-root mode. And
   for this estate: a domain of its own, a free dynamic-DNS service, or
   fleet-root mode for now?
2. **Passkeys for the browser login (§5.3)**, replacing R-129's client
   certificate and adding a WebAuthn dependency; domain mode adds an ACME one.
3. **Where development artifacts go.** Publishing to rivet-01 puts probes and
   dirty builds on the production server's store. The alternative is **dev-01 as
   a fleet server of its own**, a development fleet that carries the junk and
   can be wiped, with only release builds published to rivet-01.
4. **Where a fleet-root mode fleet's root is minted:** the board at first boot
   (recommended) or the workstation at provision time (§5.5).
5. **Does the clone carry the fleet server's own seed**, so a restored server
   keeps its pin and no member or bookmark needs changing? The convenience is
   large; the cost is that the clone then holds the fleet's identity outright.
6. **Scheduled system backups on the fleet server, mirrored by Rivet Monitor**
   (§7.4), and the `recover` interview — against keeping every schedule on the
   workstation.
7. **The workstation kit and the Mac's own operator identity** (§6.2).
8. **Whether a phone's browser must reach the fleet's web pages** (§5.2). The
   answer decides between fleet-root mode as it stands, fleet-root mode plus
   `ADR-047`'s name service, and domain mode.

**Checks before any of it is built:**

* **C1. Passkeys on a `.local` name under an installed root** — **passed in
   Chromium on Windows** (Windows Hello and Google Password Manager, §5.3).
   That covers every browser the maintainer uses on a workstation; Firefox and
   Safari matter only for another fleet's users, and
   `examples/probes/passkey-probe` reads them when one is at hand. **A phone cannot be read in fleet-root mode
   on this estate**: Android did not resolve `dev-01.local`, and no LAN DNS
   answers the fleet's `home.arpa` or `.lan` names (§5.2). It is readable the day
   a name resolves on the phone — in domain mode, or once `ADR-047`'s name
   service runs.
* **C2. Whether the maintainer's router blocks public names resolving to private
   addresses**, which decides whether domain mode needs the fleet server to
   answer its own names (§5.2).
* **C3. What `apply enroll-member`'s shape needs to carry an operator key** to a
   fleet server and on to its members (§4).
* **C4. Whether rivet-01's `/state/git/fleet.git` and object store already hold
   everything a restore needs**, or only what the workstation chose to push.
   `ADR-052` §6.1 has a member hold only what its documents name; a fleet
   server has to hold everything.

---

## 10. Migration, if ruled

The order that keeps the estate reachable throughout:

1. **Make the fleet server's repository complete** (§9 check C4) and make the
   workstation's `~/.rivet/fleet` a clone of it. Nothing about publishing
   changes yet; the workstation simply stops being the only full copy.
2. **Move publishing to the fleet server.** `install --rvt` and `cargo rivet
   image` publish there; the workstation's copy fetches.
3. **Add operator enrollment** (§4) and enroll the Mac's own identity, replacing
   the copied one.
4. **Give each fleet its browser certificate by its mode** — ACME in domain
   mode, a root of its own in fleet-root mode — and serve the portal and proxy
   from it. A browser notices this step, once per fleet.
5. **Add passkeys beside the client certificate**, enroll them, move the tools
   off the client certificate (§5.4), and only then stop accepting it. The two
   logins overlap so no step locks the operator out.
6. **Retire the Armada's CA, its repository and "one Armada per PC"**, and
   rewrite `ADR-059` to the bookmark model.

Wart 4 — a fresh PC silently minting an authority — **can be closed before any
of this**, by making a first `provision` on a workstation with no bookmarks ask
whether it is starting a new fleet or joining an existing one. That is small,
independent, and worth doing first whatever is ruled here.
