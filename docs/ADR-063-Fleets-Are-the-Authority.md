# ADR-063: Fleets Are the Authority — the workstation holds bookmarks

* **Status:** Accepted, landing step by step (§9); supersedes parts of `ADR-059`, `ADR-052`, `ADR-019` §6 and `ADR-046` §4 as each step below lands
* **Date:** 2026-09-29
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Ruled:** `.rulings/` R-228, R-229, and R-230 (the relying party, §4.3)
* **Candidate:** `docs/design-candidates/Fleets-Are-the-Authority-2026-09-28.md`, which keeps the full argument and the evidence

---

## 1. Context

`ADR-059` places an operator's whole estate on one workstation: the
certificate authority, the artifact repository and the registry of fleets live
in that workstation's `~/.rivet`, one Armada per workstation. A second
workstation exposed what follows from that premise. Copying `~/.rivet` makes
two estates that agree only until either changes; the workstation's repository
becomes a second source of truth nothing checks; the estate is split across two
directories; a fresh workstation's first `provision` silently mints a new
authority; and nothing backs the estate up as a whole.

None of these are tooling defects. Each follows from keeping the estate's
record on a workstation, so this ADR moves the record to the fleets.

**`ADR-059` remains the description of what runs today** until the steps of §9
land. Each step rewrites the part of `ADR-059` it replaces.

---

## 2. The decision

**Each fleet server is the authority for its fleet**: its repository, its
object store, its member register and, in fleet-root mode (§4), its own
certificate root.

**A workstation holds bookmarks, the operator's keys, and fetch-only clones.**
A second workstation is a second client of the same fleets, with nothing to
reconcile. The Armada as a thing a workstation owns is retired, and so is the
rule of one per workstation.

| | Before (`ADR-059`) | Now |
| :-- | :-- | :-- |
| Artifact repository | the workstation's `~/.rivet/fleet`, copied to each fleet server | the fleet server's; a workstation holds a clone that fetches and never publishes |
| Member register | the fleet server, cached on the workstation | unchanged |
| Registry of fleets | the Armada Registry | **bookmarks** |
| Browser server certificate | one operator root on every device | per fleet: a public certificate (domain mode) or the fleet's own root (fleet-root mode) |
| Browser login | a client certificate from the operator root | passkeys |
| Operator identity | minted per workstation | unchanged, and a second workstation is **enrolled**, never copied |
| Signing seeds | on the workstation | on the workstation, as **publisher** keys |

### 2.1 A bookmark

A bookmark is how to reach a fleet and nothing it could be wrong about: a name,
the fleet server's address, its pin, and which of this workstation's operator
identities it knows. Members, artifacts and what each machine runs are read
from the fleet server at the point of use, behind `ADR-059` §5's 24-hour cache
where a cache is worth having. Losing every bookmark costs retyping one address
and one pin per fleet.

**Where they are.** `~/.rivet/bookmarks.toml`, one `[fleet.<name>]` table per
fleet carrying `server`, `host`, `pin` and `identity`; a fleet's name is one
DNS label. `cargo rivet fleet bookmark add <fleet> <server>` reads the server's
address and pin from `rivet-fleet`'s register once, when the bookmark is made,
and writes nothing to the register; `--host`, `--pin` and `--identity` win over
it. A register record with no address — a DHCP server — is bookmarked as
`<server>.local:<port>`, which the transport resolves over mDNS at each
connection: the pin is the trust, so the address is kept as the name that finds
the machine rather than the address it had that day. A different bookmark for
a fleet already bookmarked is refused without `--replace`, because a pin is a
trust decision. `bookmark list` and `bookmark remove <fleet>` complete it.

The bookmarks file, the per-fleet clones beside it (§6) and the workstation
repository `~/.rivet/fleet` with its register are separate files. A fleet
reached through a bookmark touches neither of the last two, so a fleet still
run the `ADR-059` way (`home-fleet`) and a bookmarked one (`dev-fleet`) live on
one workstation side by side. Those two names, here and in §9, stand for an
operator's own production and development fleets.

---

## 3. Operators are enrolled

The control plane already trusts by key list, not by authority: a board's
`peers` is its whole answer to who may reach it (`ADR-019` §6.2, `ADR-025` §2).
Adding a workstation is adding its public key to a fleet.

1. The new workstation mints its own operator identity. Its private key never
   leaves it.
2. An enrolled workstation hands that public key to the fleet server, which
   records it as an operator of the fleet, adds it to its own `peers`, and
   carries it to every member's `peers`.
3. The new workstation adds a bookmark.

**Revoking a workstation removes one key** and touches no other workstation,
which a copied identity can never offer: one key in two places cannot be
revoked for one of them.

### 3.1 What exists and what is built

`apply restore-secret peers` replaces a running board's `peers`, and refuses a
malformed file and one that omits the caller's own key before writing anything.

**`rivet-mcpd` reloads `peers` live**, so enrollment and revocation need no
reboot:

* **When.** The file is read again each time a control-plane session's answer
  passes through `rivet-mcpd` on its way to the caller. `restore-secret` runs in
  the `rivet mcp` child of one of `rivet-mcpd`'s own connections and writes the
  file before it answers, so the new set governs the next handshake before the
  caller has read that the write happened. A change made some other way is
  picked up when any session next answers, or at the next start.
* **Why not a file watcher.** Checking as answers pass costs one read of a file
  of a few hundred bytes, needs no `inotify` and no thread, and — the deciding
  reason — knows **whose session** observed the change, which is the key the
  reload must not remove. A watcher sees a change with no caller attached.
* **What it accepts.** Any valid file: a new set of keys is exactly a set of
  additions and removals. **What it refuses**, keeping the set already in force:
  a file that does not parse or cannot be read, one naming no keys, and one
  that omits the key of the session that observed the change. Each refusal is
  written to `rivet-mcpd`'s journal once per caller rather than once per answer.
  A refusal that depends on the caller is not a verdict on the file: the session
  whose key the file keeps loads it when it answers.
* **Sessions already open are not torn down.** A removed key is refused at its
  next handshake.
* **The listener resumes no TLS sessions.** A resumed TLS 1.3 session restores
  the client's certificate from the server's session store without consulting
  the verifier, so with resumption on a removed key would still be admitted by a
  client holding a ticket from before the removal. Every connection is a full
  handshake; `rivetos-tls`'s `live_server_config` is where that is set, and its
  tests measure the admission it prevents.
* **The identity is still read only at start.** Replacing `device.seed` or
  `device.crt` is live at the next boot.

Still to build for this ADR:

* **An operator record on the fleet server** — `enroll-member`'s twin, keyed by
  the operator's public key, with a matching removal.
* **A walk over the members** that adds or removes one key in each `peers`.

**Every key in `peers` is a full operator**, and anyone already in `peers` may
enroll another. A compromised fleet server could already rewrite its members'
`peers` before this ADR, so enrollment adds no new power to it. Member keys are
never added to a fleet server's `peers` by this mechanism, which is what keeps a
compromised member from reaching the rest of the fleet.

---

## 4. Browsers: three arrangements, one login

Only the browser path needs a certificate authority. The control plane, board to
board, and phone applications (`ADR-061`) all trust pinned keys and are
unchanged.

### 4.1 The server certificate — chosen per fleet at provisioning

| | **Fleet-root, no phone browser** | **Fleet-root plus name service** | **Domain mode** |
| :-- | :-- | :-- | :-- |
| Issued by | the fleet's own root | the fleet's own root | a public authority, by ACME DNS-01 |
| Names | `<host>.local` | `<host>.local` and `<host>.<fleet>.home.arpa` | `<host>.<fleet>.<domain>` |
| A phone's browser | cannot reach it: Android does not resolve `.local` | reaches it on the LAN, through `ADR-047`'s name service | reaches it |
| Installed on each device | the fleet's root | the fleet's root | nothing |
| Needs | nothing | the name service on the fleet server | a domain, a DNS API token on the fleet server, outbound internet to renew |

**Domain mode is the recommended arrangement**, because it installs nothing on
any device and leaves nothing irreplaceable to back up. Fleet-root mode stays
for a fleet with no domain or one that must never reach the internet. Choosing
fleet-root mode gives up `ADR-059` §4.1's single root: a second fleet is a
second root on each device.

**A fleet may switch arrangements after provisioning**, by an explicit verb
that states before it acts that passkeys must be re-enrolled when the domain
changes. Adding or removing the name service changes no domain and re-enrolls
nothing: every passkey keeps working at the name it was enrolled at, and a
device reaching the machine by the name service's name enrolls one there (§4.3).

**Domain mode works on the maintainer's network without the fleet server
answering its own names**: the LAN's resolver returns a private address for a
public name, so no DNS-rebinding guard intervenes.

### 4.2 A fleet's root is minted on the fleet server

In fleet-root mode the root is minted **on the fleet server at first boot**,
from `getrandom(2)`, so it cannot be left behind on a workstation. The Pi 4's
hardware generator feeds the kernel's pool, and `getrandom(2)` blocks until
the pool is ready rather than returning weak bytes. The root stays
name-constrained to the fleet's names, as `cmd/rivet-provision/src/ca.rs`
constrains today's, so a leaked root forges only that fleet's names. A
workstation fetches the root over the pinned control plane: the pin vouches for
it.

#### 4.2.1 The card carries a request, and PID 1 mints

A fleet-root mode server's card carries no root and no leaf. Its `ADR-049`
staging bundle carries one entry addressed to `rivet-initd` at `fleet-root`: a
request naming the fleet, the host and the suffixes, in the line format
`rivet_core::fleet_root` defines. The request holds no secret.

**The suffixes are the fleet's names and nothing else**: each is one of
`local`, `<fleet>.home.arpa` and `<fleet>.lan`, at least one is named, and a
request naming anything else is refused whole — on the workstation when the
bundle is composed and again on the board. That is what refuses a root that
would permit everything. `local` and `<fleet>.home.arpa` are offered;
`<fleet>.lan` is accepted when named and never offered, because `.lan` is
undelegated rather than reserved and a constraint lasts as long as the root.

On first boot PID 1 reads the bundle, takes the request out of it — no
application's share ever holds it — and:

1. **mints** the root and the server's certificate when `/state/secrets` holds
   no root: ECDSA P-256, the root self-signed with path length 0 and
   constrained to the request's suffixes, the server's leaf carrying
   `<host>.<suffix>` for every suffix with the first as its subject;
2. **writes** the request it minted for, the leaf's key, the leaf, the root's
   key and, last, the root's certificate to `/state/secrets`
   (`fleet-root.request`, `fleet-root-server.key`, `fleet-root-server.crt`,
   `fleet-root.key`, `fleet-root.crt`), root-owned, `0600`, and flushes;
3. **hands** the leaf, its key and the root's certificate to `rivet-proxyd`
   (`tls-cert`, `tls-key`, `tls-root`) and `rivet-fleetd` (`server.crt`,
   `server.key`, `ca.crt`) by adding them to the bundle, which the supervisor
   then delivers with each application's own uid exactly as it delivers a
   workstation-minted leaf.

The card's bundle is blanked and unlinked only once all of it is on `/state`,
so a second boot finds no request and mints nothing.

**A root is never minted over one that rests.** The root's certificate is the
marker: without it nothing counts as minted and any partial file is
overwritten. With it, the rest of the set must be there and the resting
request must equal the carried one; a retried first boot then hands over the
same bytes, and anything else is refused with nothing minted. The leaf rests
beside the root for that reason — a retry hands over identical material
without PID 1 ever parsing a certificate back, which keeps an X.509 parser out
of PID 1.

A bundle carrying the request **and** a workstation-minted leaf for the proxy
or `rivet-fleetd` has two answers for one file and is refused whole.

#### 4.2.2 Why PID 1 mints, and not `rivet-fleetd`

* **The root's key must not live in a process that serves browsers.**
  `rivet-proxyd` and `rivet-fleetd` both do; a compromise of either would hand
  over a key that forges every name in the fleet. `/state/secrets` is
  root-owned, beside the machine's own identity, and is where §7's encrypted
  backup takes each host's secrets from.
* **The proxy needs its certificate before its first start.** It is an image
  service started on the first pass; `rivet-fleetd` is a carried application
  installed after PID 1 has exited, whose install may fail without stopping the
  machine (`ADR-050` §10).
* **The hand-over already exists at PID 1's split**, so minting there puts the
  board's leaf on the same path as a workstation's with nothing new downstream.

The cost is minting code in PID 1. The minting itself is
`rivet-fleet-root` (package `rivetos-fleet-root`), a library with no I/O that
PID 1 calls. It brings `rcgen` and its date library (`time` and its
companions), `yasna`, `pem` and `base64` onto a card for the first time —
every one of them already in `Cargo.lock` through the workstation tools — and
not `x509-parser`: PID 1 never parses a certificate back, and the supervisor's
judging in §4.5 reads only the handful of fields it needs with a DER walk of
the crate's own. `ring` was already on every card through `rustls`. A failure returns a value: the
staging is refused, the card keeps its bundle for the next boot, and the
machine still boots and answers its control plane.

#### 4.2.3 Randomness and the clock

**A random source that refuses mints nothing.** PID 1's kernel port draws from
`getrandom(2)` with no flags, which blocks until the pool is ready; the port's
default refuses, and a source answering one byte repeated is refused as no
generator would answer. The drawn bytes are the two serial numbers. The keys
themselves are generated through `ring`'s system generator, which is the same
`getrandom(2)` — `ring` accepts no caller-supplied bytes, so the port is the
gate and the kernel is the keys.

**A certificate lives its full term from the day it is issued**: ten years for
the root, five for every leaf, as `ca.rs` sets them. The window starts at the
issuing day's midnight UTC and ends on the same day the term later — a window
starting on 29 February ends on 1 March of a year without one, so no
certificate is shorter than its term. A leaf issued in December therefore lives
five years, not four and a few days.

**A clock that reads earlier than 2026 dates from the first of January 2026
instead.** A Raspberry Pi has no real-time clock and first boot runs at the
epoch (`ADR-021`), so a window computed from that clock would have expired
decades ago; dating from the floor gives a board with no real time a
certificate that is valid once its clock is right. A clock so far ahead that
the end cannot be written as a certificate date is refused, not a panic: PID 1
links the minting.

#### 4.2.4 How a workstation reads the root, and how a card asks for one

`rivet get fleet-root` answers the root's certificate as PEM and the SHA-256 of
its DER. It composes the one path it reads and takes no argument, so the key
beside it cannot be named; a machine holding no root answers `present = false`.
The digest is an `Editor` port method that refuses by default. `cargo rivet
fleet root <host> [--out DIR]` asks it over the pinned control plane,
recomputes the fingerprint and refuses a mismatch before writing
`<fleet>-root.crt` (PEM) and `<fleet>-root.cer` (DER), and prints the Windows
and Android install steps. `<host>` is looked up in the register; given all
three of `--host`, `--identity` and `--pin`, the register is not read, as for
`cargo rivet run --host …`, so a machine no register holds can be asked.

A card asks for fleet-root mode in either of two ways:

* **the first-provision guard** (§8) offers a third answer, *a new fleet whose
  root is minted on its fleet server*: it asks the fleet's name and suffixes,
  records them in the authority directory as `fleet-root.txt`, mints nothing,
  and counts as an authority so it is not asked again; each server document
  the interview composes afterwards carries the request;
* **`--fleet-root FLEET [--suffixes LIST]`**, on `rivet-provision stage` and on
  `cargo rivet provision`, which passes it to the staging build. It needs no
  workstation authority and is refused for a member's document.

A member of a fleet-root mode fleet is issued its certificate by its fleet
server (§4.4); until that issuance exists, staging refuses fleet-root mode for
a member's document by name.

#### 4.2.5 Re-roling a recorded machine, and carrying its passkeys

`cargo rivet provision --existing HOSTNAME --document PATH` writes a new
document onto a machine that is already recorded: the document must name
HOSTNAME, the machine keeps its identity and pin, its staging bundle is built
when absent — a new fleet's server has no members trusting an older
`rivet-fleetd` key — and nothing is recorded. `--existing` alone still rewrites
the recorded document.

`--passkeys FILE` carries a `passkeys` file read off the machine's proxy to the
proxy on the new card, verbatim. A passkey keeps working at the name it was
enrolled at wherever the new certificate still carries that name.

### 4.3 Passkeys replace the client certificate

`rivet-proxyd` is a WebAuthn relying party and issues a `SameSite=Strict`
session cookie. The first passkey is enrolled with a one-time code issued over
the control plane; an enrolled operator enrolls further devices from the browser.
The `Origin` and `Sec-Fetch-Site` check of `ADR-046` §4.5 stays as defence in
depth. `ADR-046` §4.6 is the gate's side of it.

Passkeys remove the client certificate on every browser, its picker, and the
difficulty of installing one on a phone; and because an operator is no longer a
certificate, no certificate can be mistaken for one.

**The relying party is Rivet's own** (`.rulings/` R-230), and it is small on
purpose:

| | |
| :-- | :-- |
| Where the checks live | `rivet-webauthn` (package `rivetos-webauthn`), a supporting crate beside `rivet-crypto`: pure checks over what a browser sent, no I/O, so every refusal is tested without a socket. `rivet-core` cannot hold it because it takes no dependencies (`ADR-014` §3). |
| Where the state lives | `cmd/rivet-proxyd/src/passkey.rs`: challenges, sessions, the credential store, the enrollment code, randomness and the clock. |
| Signatures | ES256 (COSE `-7`, ECDSA P-256 with SHA-256, DER) and Ed25519 (COSE `-8`), verified by `ring`, which the proxy already links through `rustls`. Every other algorithm is refused by number. |
| CBOR | `ciborium`, the one new crate, for the attestation object and the COSE key. It brings `ciborium-io`, `ciborium-ll`, `half`, `crunchy`, `zerocopy` and `zerocopy-derive`. |
| Attestation | `none` only, with an empty statement. The relying party trusts the operator who held the enrollment code, not an authenticator's maker. |
| RP ID and origin | **each name the proxy's own certificate carries**: for a request, the RP ID is its `Host` without the port, and the origin exactly `https://<that name>`, only when that name is one of the certificate's DNS names. Any other `Host` — an address, a name not on the certificate, none, or two — is answered `421` before a challenge is issued. The names are read off the certificate the proxy serves, the one source `upstream-name`'s default also comes from (`ADR-046` §4.2), so there is no second list to fall out of step. A passkey is bound to the name it was enrolled at, so a fleet that changes its domain re-enrolls (§4.1). |
| What is checked | `clientDataJSON`'s type, challenge, origin and `crossOrigin`; the RP ID hash; user presence **and user verification**, both required; the attested credential; the COSE key's parameters; the signature over authenticator data and the hash of `clientDataJSON`; and the signature counter, which must increase once either the stored or the presented value is non-zero. A counter that does not increase is refused and journalled as a possible cloned authenticator. |

**The enrollment code.** `rivet apply passkey-code confirm`, over the control
plane (`cargo rivet run <machine> -- apply passkey-code confirm`), draws ten
characters of Crockford base 32 from `getrandom(2)` on the board and writes them
to `passkey-enrolment` in the proxy's data directory, owned by the proxy's uid,
mode `0600` — `install-secret`'s act. The code is in the confirmed answer and
nowhere else: the plan is shown before anything is drawn, and the journal
records only the length. The proxy honours it for ten minutes from the file's
modification time, deletes it when a passkey is enrolled with it, and deletes it
after five wrong codes. A second code replaces the first; `apply remove-file`
on the path withdraws it. A modification time in the future counts as expired,
so a clock that moved backwards cannot stretch a code's life.
`rivet_core::passkey` holds the file name, the lifetime, the attempt limit and
the spelling, which both the CLI and the proxy read.

**A passkey belongs to one name.** A browser's authenticator binds a
credential to the RP ID it was created under and will not offer it at another,
and the proxy holds the same line itself: sign-in offers and accepts only the
passkeys enrolled at the name being signed in to, and refuses one enrolled
elsewhere before its signature is checked. So a device that reaches the machine
by a second name — a phone at `<host>.<fleet>.home.arpa`, which cannot resolve
`.local` — enrolls a passkey at that name, by the same two routes: the one-time
code, or a browser already signed in **at that same name**.

**Where the credentials live.** `passkeys` in the proxy's own data directory,
one line per passkey — ID, algorithm, public key, counter, enrollment time, and
the RP ID it was enrolled under — written by rename. A line without an RP ID
was written when the machine had one relying party, and belongs to its
`upstream-name`; it is read that way and written back without one, so no line
is rewritten into a claim nobody made. It holds no secret: a public key
verifies and cannot sign, so it is not `ADR-020`'s material and is not in
`/state/secrets`. It is read at every ceremony, so no copy in memory can go
stale.

**Sessions die with the proxy, and each belongs to one name.** Challenges (two
minutes, single use) and sessions (twelve hours) are held in memory only, each
bound to the name it was issued at: a challenge is answered there or not at
all, and a session admits the operator there and nowhere else. A session table
on disk would be a bearer credential at rest, and signing in again costs one
touch. The cookie is `rivet-session`, `Secure; HttpOnly; SameSite=Strict;
Path=/`, with no `Domain`, so a browser sends it only to the host that set it;
the proxy strips it from every request it forwards, so no application behind
it — a public route's included — ever sees it.

**The pages** are the proxy's own: `/sign-in` and `/sign-in/enroll`, plain HTML
and script calling `navigator.credentials`, with nothing fetched from anywhere
else. The same address on every machine is one pattern a person learns to trust,
so `sign-in` is a reserved application path that no `proxy-path` may name
(`ADR-046` §4.6, `.rulings/` R-249). The ceremony calls the pages make are under
the hidden `/.rivet/` prefix, which no application can claim either, because a
`proxy-path` must begin with a lowercase letter. The pages' first addresses,
`/.rivet/login` and `/.rivet/enrol`, answer a permanent redirect to the new ones.

**Test vectors are labelled by where they came from.** `rivet-webauthn` is
tested against the WebAuthn specification's own vectors (labelled *W3C spec
vectors*) and against a software authenticator in its tests (labelled
*self-generated*). `examples/probes/passkey-probe` journals a real browser's
`clientDataJSON`, attestation object, authenticator data and signature with the
challenge that produced them, which is where *captured from a real browser*
vectors come from.

**Tools leave the client certificate by leaving the proxy** (`.rulings/`
R-234). `cargo rivet backup` and `restore` — and Rivet Monitor, which calls them
— go over the control plane with the operator's pinned key, and the machine
reaches the application on loopback (`ADR-051` §8.1, `ADR-035` §3.3). No token
and no new credential was needed. With that, nothing but a browser presents the
client certificate.

**A machine retires the certificate by its own document**, one machine at a
time:

```toml
[applications.rivet-proxyd]
operator-certificate = "refuse"     # "accept" when absent
```

Refused, the proxy asks for no client certificate at all and only a passkey
session admits the operator at the gate (`ADR-046` §4.1). **Absent is accepted**, so every
machine whose document says nothing behaves exactly as before the key existed.
The order is the maintainer's, so no step locks the operator out: the
development fleet's server first, once the tools have moved; each other machine
only after a passkey has been enrolled there and used to sign in.

A passkey on a `.local` name under an installed root works in Chromium on
Windows, with credentials held by Windows Hello and by Google Password Manager.
Firefox and Safari are unread; `examples/probes/passkey-probe` reads them.

### 4.4 A member's certificate is issued by its fleet server

In fleet-root mode the root lives on the fleet server, so the workstation
cannot issue a member's HTTPS certificate. **The fleet server issues it over the
control plane when the member is enrolled**, beside the register record
enrollment already writes, and reissues it on renewal the same way. The member's
private key is generated on the member and only its signing request travels. In
domain mode each machine obtains its own certificate by ACME and this does not
apply.

### 4.5 A fleet-root machine issues its own applications' certificates

PID 1 mints the root and the server's leaf once (§4.2) and never again. Every
other certificate a fleet-root machine needs — each application's, and a
replacement for the server's — **is issued by the machine itself, from the
root resting in `/state/secrets`** (`.rulings/` R-240, R-250). A workstation
cannot issue them: the root's key never leaves the board.

**Each application that declares `fleet-tls` gets a leaf of its own**, with a
key generated on the board, so no application holds another program's private
key. The leaf carries the machine's own names — `<host>.<suffix>` for every
suffix the root was minted for, read from `fleet-root.request` — exactly as the
server's leaf does, and is built by the same builder (`rivet_fleet_root::
issue_leaf`), so it carries the authority and subject key identifiers, a
critical key usage of `digitalSignature`, and an extended key usage of
`serverAuth` and `clientAuth`. It lives five years.

| | |
| :-- | :-- |
| **When** | At the install of an application, before the supervisor adopts it; and at every boot, before the first service starts. |
| **What is judged** | Every installed `fleet-tls` application's `tls-cert`, at both. The server's leaf, as the proxy and `rivet-fleetd` hold it, at boot only: the proxy reads its certificate at its start, and at boot nothing has started. |
| **Re-issued when** | the certificate or its key is missing; it does not chain to this machine's root (a workstation-issued one, journalled with `old_issuer` naming its issuer); it ends within ninety days, or has ended; it lacks any of the fields above; or, for an application, its `tls-root` is not this root. |
| **Written** | `tls-root`, `tls-key`, then `tls-cert` into the application's data directory, each owned by the application's uid, mode `0600`, through a fresh file renamed over the old — so a reader sees the whole old file or the whole new one, and finds a new certificate only beside its key. A link planted at any name is replaced, never followed. |
| **The server's leaf** | One leaf, as PID 1 hands it over: written to `fleet-root-server.crt` and `.key` in `/state/secrets`, root-owned, and to the proxy's and `rivet-fleetd`'s homes under each program's own names and uid. The names do not change, so every passkey enrolled at one of them keeps working (§4.3). |
| **Restarted** | Nothing. At boot every program reads its new leaf at its first start. After an install, an application already running keeps its old leaf until its next start, which is journalled; whether an install may restart a running program is `ADR-055` §6's question, not this one's. |
| **A machine with no root** | Issues nothing and reads nothing in `/state/secrets`. A workstation-root fleet's material is staged by `cargo rivet secrets`, unchanged (`ADR-055` §3). |

**A short-lived helper signs, and nothing long-running holds the root's key.**
The supervisor judges with the root's certificate alone, which is public, and
reading a leaf back needs no key. For each issuance it forks a helper that reads
the root's key, signs, writes the files and exits; the supervisor waits for it,
which costs milliseconds. **A fork of the supervisor is enough, and no separate
program is executed for it**: §4.2.2's rule is that the root's key must not live
in a process that serves a network, and the helper serves none and ends after
one signature, while the key is read only after the fork, so the supervisor's
own memory never holds a byte of it. A separate program would add a binary to
every card to buy an isolation the fork already gives.

**The judging reads a certificate back with no X.509 library.** It needs the
issuer, the end date, the signature and which extensions are present;
`rivet_fleet_root::judge` reads those with a DER walk written for them and
checks the signature with `ring`. A leaf it cannot read is re-issued. A clock
that has not been set reads as the first of January 2026, as §4.2.3 dates a
mint, so the first boot's epoch clock never makes a leaf look expired.

**Installing to a fleet-root machine stages nothing.** `cargo rivet install`
and the deploy interview ask the machine `rivet get fleet-root`; one that holds
its root is planned with no material step, and `--enable` is not refused for
material the machine is about to issue (`ADR-055` §3.6).

---

## 5. The signing seeds are publisher keys

The two seeds (`ADR-028` §5.5) and the maintainer's Android signing key say who
built an artifact, not who administers a fleet, and stay on the workstation that
publishes. A workstation that only operates needs none of them.

**`cargo rivet workstation export` and `import`** carry the publisher keys and
the bookmarks in one passphrase-encrypted file, **never the operator identity**.
Importing it is followed by minting that workstation's own operator identity and
enrolling it (§3). The file is also the recovery copy of the publisher keys.

---

## 6. The fleet's repository, and the workstation's clone

**Every publish goes to a fleet server**, where `ADR-052` §7.4's door verifies
the signature and the digest. A workstation's clone only fetches, so two
workstations' clones can be stale but never contradictory.

**A fleet server's store holds every kind of artifact, cards and system updates
included**, kept three per kind as the workstation's repository keeps them. A
fleet server that cannot hold a card cannot be restored from, and cannot write a
card for a host it recovers. The rule is `rivet_core::repository_objects::holds_kind`,
and `ADR-052` §7.4.3 describes the door that applies it. A card is still
admitted only where a machine's trust store permits its signer to sign cards.

Scratch builds stay in `out/` and reach a repository only when published.

### 6.1 The system key signs cards, and a machine's own store refuses them

Every image trusts the system key for `update`, `extension` and `card`
(`ADR-028` §5.5.2), so a fleet server's store admits the cards the fleet is
provisioned and recovered from. The system key already signs system updates,
which are strictly more powerful than a card, so trusting it for `card` gives
it nothing it could not already do.

**A machine's own artifact store refuses a card by kind** (`PUT /payload/`),
whoever signed it, because a card is written to a card on a workstation and
never installed on a machine. Without that refusal the widened trust would let
a card into `/state/artifacts`, whose door checked trust and not kind.

### 6.2 Publishing to a fleet — `fleet publish FILE --fleet NAME`

    cargo rivet fleet publish FILE.rvt --trusted-key PATH --fleet NAME [confirm]

The publish verb gains a destination rather than a new verb beside it: the
verification, the manifest entry, keep-three retention and the commit are
`ADR-052` §7's, unchanged; only where the result goes differs. The bookmark
names the server, so `--fleet` takes no `--at`, `--sync`, `--host`,
`--identity` or `--pin`.

1. **Compose on the server's text.** A scratch repository beside the clone,
   `~/.rivet/fleets/.<name>.publishing`, takes the server's `main` (or is
   initialised when the server's repository has none) and the artifact is
   published into it. The clone is never the working copy, so a publish that
   fails leaves no commit anywhere but the server.
2. **The object first.** With `confirm`, the object goes to the server's
   repository store (`PUT /repository-objects/<name>`), verified at the
   server's door against the server's own trust store. A refused object ends
   the publish and `main` is not pushed, so the server's manifest never names
   what its store lacks.
3. **Then the text**, pushed as a fast-forward and never forced. A publish that
   lost a race to another workstation's is refused as not a fast-forward with
   its object already held, and running it again composes on the new `main`.
4. **Then the clone fetches** (§6.3), and the scratch repository is removed.

Without `confirm` nothing reaches the server; the plan says whether the store
already holds the object. `.rvt` artifacts publish this way; an APK still
reaches a fleet through a workstation repository's `fleet publish FILE.apk
--sync`.

### 6.3 The clone — `fleet fetch NAME`

    cargo rivet fleet fetch NAME [--into DIR]

A fleet's clone is `~/.rivet/fleets/<name>/`, laid out as any fleet repository
is — `manifest.toml` and the rest of the text in git, `objects/<digest>` beside
it — so `fleet list --at` and `install --artifact ID --at` read it unchanged.
A fetch:

1. fetches the server's `main` through the loopback relay `fleet push` uses and
   fast-forwards the clone's `main` to it. **A clone whose `main` is not an
   ancestor of the server's is refused and left untouched**: a clone never
   commits, so one holding history the server lacks was written by something
   else;
2. reads back every object the server holds that the clone lacks — `GET
   /repository-objects/<digest>`, which answers the object with its digest and
   its `ADR-027` name (`Rivet-Name`) — streamed to disk and kept only when its
   bytes hash to its digest, and hashes every copy already there again,
   replacing one that no longer matches;
3. removes every object the server no longer holds.

So two clones, on one workstation or two, are each the server's `main` at some
moment with every object hashing to its name: **stale or current, never
contradictory.** The report ends with the `main` and the object count, which is
what two clones are compared by. `--into DIR` fetches into another directory,
for a second clone or a check.

`GET /repository-objects/<digest>` answers the operator only, as every store
route does, and reads the object's directory and file with `lstat(2)`: the store
sits in `rivet-fleetd`'s own data directory, which that application can write,
and `rivet-mcpd` reads as root, so a planted link is refused rather than
followed.

---

## 7. System backups, and recovering a host

A system backup is everything needed to put a host back as it was:

| Part | Carried by |
| :-- | :-- |
| Desired-state documents, the register, what each host runs — with their whole history | the fleet repository |
| The artifacts those documents name | the fleet's object store |
| Each host's identity, each application's per-machine secrets, **the fleet server's own key**, and in fleet-root mode the fleet's root | an **encrypted** part, passphrase-sealed as `cargo rivet key backup` is, and never the plain clone |
| Application data | Rivet Monitor's application backups (`ADR-051`) |

**Carrying the fleet server's key means a restored server keeps its pin**, so
no member and no bookmark changes. Keeping it only in the encrypted part is what
stops the clone itself from holding the fleet's identity.

**The fleet server takes the backups on a schedule the operator sets**, because
it is always on and its clock is set over NTP, which a sleeping workstation is
not. **Rivet Monitor mirrors them** onto the workstation's backup folder. The
server's copy guards against a member failing; the workstation's against the
server failing.

**`cargo rivet recover` rebuilds a host from a backup.** It is built first as
control-plane operations behind a scriptable verb, a plan without `confirm`
and the act with it; the interview a person uses is designed afterwards, from
what the operations show a recovery needs (`.rulings/` R-235). Scheduling,
server-side bundles and the encryption of the secrets part wait for the same
reason.

    cargo rivet recover HOSTNAME --from DIR [--secrets DIR] [--at DIR]
                        [--image PATH-OR-DIGEST] [--device PATH]
                        [--host HOST:PORT] [--identity DIR] [--pin HEX]
                        [--after-boot] [--replace-secrets] [--replace APP]... [confirm]

It draws on three sources and takes nothing new: the host's recorded document,
and each application the document carries resolved to one artifact by digest,
from the fleet repository; the host's secrets from a `cargo rivet
backup-secrets` directory (`ADR-030`); each application's data from a folder
of `cargo rivet backup` output, `DIR/<application>/<file>`, or Rivet Monitor's
backup folder, `DIR/<host>/<application>/<files>`, whose newest file is used.

The steps, in the order a recovery needs them:

| Step | What | By |
| :-- | :-- | :-- |
| 1 | the card: the recorded document, the recorded identity, the same pin | `provision --existing`'s own machinery |
| 2 | after the first boot, the secrets, the operator typing the passphrase | `restore-secrets` |
| 3 | each application installed by its digest, then one converge | `install --artifact`, then `apply reload` |
| 4 | each application's data, sent to the restore route it declares | `restore` |

**The card step and the rest are separate invocations, and the card is a
person's.** A confirmed run without `--after-boot` writes the card and stops,
naming the command for after the first boot; it refuses without a terminal.
With `--after-boot` it runs steps 2 to 4 in order and stops at the first
refusal, so an agent proves those steps against a running machine. Every step
is safe to repeat: a secret already on the machine, an artifact it already
runs and data it already holds are each skipped.

**The plan reads everything and connects to nothing.** It refuses, before
anything is written: a host with no recorded document, or whose document names
another host; a host with no pin, in the fleet repository's register or given,
and a `--pin` the register disagrees with, because a recovery keeps the pin; an
application the document installs that the repository cannot resolve to one
artifact, or whose object is absent or does not hash to its digest; a secrets
directory holding no sealed backup; and data for an application the document
does not install. Which board a secrets backup belongs to is sealed inside it,
so it is checked once the passphrase is typed, before the card is written, and
again by `restore-secrets` before any secret is sent.

**A recovery that writes over real data asks first, every time.** Before any
application's data is sent, every application is asked what it holds through
its own backup route. The backup's own bytes are skipped and no data at all is
restored; anything else, a different file, an error or no answer, is refused
unless `--replace <application>` names it, and nothing is sent to any
application until all of them are settled. Nothing can tell an empty database
from a real one, or a stale backup from a current one, so the operator says
which it is, by name.

**Per-machine material is in no backup.** The TLS pair an application is handed
from the fleet's authority (`ADR-055` §3) is staged by `cargo rivet secrets`,
which reads the authority's private key and is the operator's to run. An
application declaring material is refused at step 3 by `install`'s own rule
until it is staged; one needing it undeclared cannot answer, and step 4 refuses
it. A bundle that carries what a recovery needs is designed from this.

---

## 8. The first-provision guard

A first `provision` on a workstation with no bookmarks asks whether it is
**starting a new fleet or joining an existing one**, and mints nothing until
answered. This closes the silent-authority trap and is built first, before and
independently of everything else here.

A new fleet is started in one of two ways: its authority minted on the
workstation, or in fleet-root mode, its root minted on its fleet server's first
boot (§4.2.4). The second mints nothing on the workstation at all.

---

## 9. Migration — proven on a development fleet first

**dev-01 becomes the server of its own development fleet, `dev-fleet`**, in
fleet-root mode plus the name service. Development builds are published there,
and only release builds reach the production fleet. **Nothing reaches rivet-01
until every step has been proven on `dev-fleet`**, and rivet-01 is currently
frozen for stability testing, keeping dev-01's entry in its register until the
freeze lifts.

**Everything that works the same on any Rivet is proven first, on dev-01 as it
stands, by remote system update** — so the card that makes dev-01 a fleet
server carries code already proven on a board, and passkeys are in place before
the board-minted root stops recognising the old client certificate.

| Step | On dev-01 | Proof |
| :-- | :-- | :-- |
| 1 | The first-provision guard (§8) | a workstation with no bookmarks is asked, and mints nothing unanswered |
| 2 | `rivet-mcpd`'s live `peers` reload (§3.1), by system update | a key added with `apply restore-secret peers` is accepted without a reboot, and a change dropping the caller's key is refused |
| 3 | Passkeys in `rivet-proxyd` beside client certificates (§4.3), by system update | a passkey enrolled by one-time code logs in to a gated dev-01 page |
| 4 | `ADR-047`'s name service, with only the phone pointed at it through its own Wi-Fi DNS setting (router and household untouched) | the phone's browser opens a dev-01 page by its `home.arpa` name and logs in with a passkey |
| 5 | **The card write:** dev-01 re-provisioned as the `dev-fleet` fleet server, its root minted on the board (§4.2) | the root is fetched over the pinned control plane, a browser trusts `dev-01.local` under it, and a passkey logs in |
| 6 | The fleet's repository is the authority, cards and updates included; the workstation's copy becomes a clone; publishing goes to dev-01 (§6) | a card and an update published from the workstation land on dev-01 and the clone fetches them |
| 7 | The Mac mints its own operator identity and is enrolled (§3) | the Mac reaches dev-01 with its own key, without a reboot, and may then publish to `dev-fleet` |
| 8 | Tools move off the client certificate to the operator key (§4.3) | Rivet Monitor backs up dev-01 without a client certificate |
| 9 | Scheduled system backups and `cargo rivet recover` (§7) — the second card write | dev-01 is rebuilt from its own backup with its pin unchanged |

**Before step 5 touches dev-01**, the code is read for whether rivet-01's
`rivet-fleetd` would journal repeated failures to reach a member that no longer
trusts it; if it would, that is put to the maintainer, because it would disturb
rivet-01's stability monitoring.

**The answer, read from the code:** it would not, in the arrangement rivet-01
runs. `rivet-fleetd` has one thread that reaches machines on its own, the
fleet-as-code poller, and it starts only when `[applications.rivet-fleetd]
fleet-repository` is set; provisioning leaves it unset, and rivet-01's recorded
document sets none. Every other contact with a member is made on demand — the
portal or the phone asking — and a failed one is one journal line per request.
With the poller configured, a member that refuses would be one `was not
reachable this cycle` line per poll, every sixty seconds by default. And the
re-roled dev-01 keeps its identity and pin and is written with the peers file
its identity directory holds, which still names rivet-01's `rivet-fleetd` key:
until that line is removed, rivet-01 reaches dev-01 exactly as before.

**dev-01's names in `dev-fleet`** are `dev-01.local`, `dev-01.dev-fleet.home.arpa`
and `dev-01.dev-fleet.lan`, and `rivet-dnsd`'s zone becomes
`dev-fleet.home.arpa`. A passkey enrolled at `dev-01.local` keeps working when
its line is carried; one enrolled at `dev-01.home-fleet.home.arpa` is carried
but no longer offered, because that name leaves the certificate.

**Every new dependency** — WebAuthn at step 3, a DNS library if step 4 needs one,
ACME when domain mode is built — is named to the maintainer before it is added.

**Domain mode is outside this migration.** It remains the recommended
arrangement for a fleet that can have a domain, and is taken up when the
maintainer asks for it.

---

## 10. What this changes in the record, as each step lands

| Document | Change |
| :-- | :-- |
| `ADR-059` | §2's Armada loses its authority and its repository; §4.1 holds only for a fleet-root mode fleet's own root; §4.2 inverts; §4.3 is retired; §5's registry becomes bookmarks |
| `ADR-052` | the repository lives on the fleet server and admits cards and updates; §7.3's object backup becomes the clone |
| `ADR-019` §6 | the fleet authority leaves the workstation's first-run set |
| `ADR-046` | §4's gate checks a passkey session; the proxy's certificate comes from the fleet's root or from ACME |
| `ADR-043`, `ADR-051` | the portal serves the proxy's certificate; tools reach backup routes with the pinned operator key |
| `SPEC-05` | a first run asks *new fleet, or join one?* |

---

## History

### 2026-10-04 — §4.2.3 dated every window from the first of January

§4.2.3 said:

> ~~So a validity window starts on the first of January of the later of the
> clock's year and 2026: ten years for the root, five for the leaf, as `ca.rs`
> sets them.~~

**Why it changed.** Counting from the first of the current year shortens the
term by however much of the year has passed: a leaf issued in December lived
about four years, and the root nine. The floor for an unset clock was kept,
because it is what lets a board at the epoch issue a valid certificate; only a
clock already past the floor now dates from its own day.

### 2026-10-04 — §4.3's pages were under the hidden `/.rivet/` prefix

§4.3 said:

> ~~**The pages** are the proxy's own, under the reserved `/.rivet/`: `/.rivet/login`
> and `/.rivet/enrol`, plain HTML and script calling `navigator.credentials`, with
> nothing fetched from anywhere else.~~

**Why it changed.** The prefix was chosen only because no application path may
begin with a dot, and to a person reading the address bar it looked like a
development path. The maintainer ruled that the pages a person visits are
`/sign-in` and `/sign-in/enroll`, one address learned once and trusted on every
machine, with `sign-in` reserved and the old addresses redirecting
(`.rulings/` R-249).

### 2026-10-03 — §7's recover was one interview, and is built as operations first

§7 described recover as a single interview:

> ~~**`cargo rivet recover` rebuilds a host from a backup** in one interview:
> choose the backup and the host; the host's desired-state document decides the
> artifacts, selected by digest; the host's identity comes from the backup; the
> card is written and verified as `provision --existing` does; after first boot
> each application's data is restored from the same backup.~~

`.rulings/` R-235 phased it: the operations and a scriptable verb first, proven
in QEMU, and the interview designed once there is something to test against.
Building them showed the card write must be its own invocation, a person's,
and that per-machine material is in no backup.

### 2026-10-03 — §4.3 left open how tools leave the client certificate

**§4.3 read:** ~~*"Client certificates remain accepted beside passkeys until
every tool that uses one (Rivet Monitor's backups and restores, `ADR-051`)
presents its pinned operator key, or a short-lived token signed by it, instead."*~~
Offered those two and a third — carrying backups over the control plane — the
maintainer chose the third (`.rulings/` R-234), and the retirement became a
per-machine document key rather than a release-wide switch, so it could be
proven on the development fleet before anything else.

### 2026-09-29 — §4.3's relying party was the machine's one name

§4.3's table said:

> ~~RP ID and origin: the machine's one name — the name its certificate
> carries and its backends are verified as (`ADR-046` §2) — and exactly
> `https://<that name>`.~~

and the store's lines carried no RP ID.

**Why it changed.** Step 4 (§9) put the name service on dev-01, and a phone —
which cannot resolve `.local` — reached the login page at the `home.arpa` name
the same certificate carries, where a passkey could not work because the RP ID
was the `.local` name. The lead agent decided, as a reversible choice inside
the settled design and recorded for the maintainer to overturn, that the proxy
answers at each name its certificate carries, with each passkey, challenge and
session bound to its own name. The alternative — one RP ID for every name —
is not available to WebAuthn: an RP ID must be the origin's host or a
registrable suffix of it, and `.local` and `home.arpa` share none.
