# ADR-047: The Name Service

* **Status:** Accepted
* **Date:** 2026-09-07
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Supersedes:** the `rivet-named` proposal, which owned addresses as well as names
* **Evidence:** `docs/design-candidates/DNS.md` — the measured study this rests on

---

## 1. Context

The household's machines need names that resolve. `rivet-mdnsd` announces
`<hostname>.local` and nothing else, and mDNS is not answered by every client on
every network — so a fleet server whose certificate carries
`arbor.<fleet>.home.arpa` has a name that nothing publishes.

**The router owns ADDRESSES and Rivet owns NAMES.** The maintainer's WiFi will
not let him take the DHCP role, so the original `rivet-named` proposal — which
owned both — is not available. The router's reservations become static
name→address records here.

**Rivet answers for the household, not only for itself.** A resolver a router
advertises must answer for names outside its own zones or the house has no
internet, so this service **forwards upstream**, to Quad9. That is the decision
this record turns on and it is taken in §3.

---

## 2. Decision

**Rivet gains a name service built on `hickory-server` with its `resolver`
feature, serving a static table of the household's names and forwarding
everything else upstream.**

It is a **rung-1 reuse decision** under `ADR-009` §3 — a protocol implementation
somebody else maintains — and it is taken deliberately rather than by default,
because `ADR-009` §3.4 places a server framework that owns its own process loop
in the column requiring a design decision rather than a dependency choice.

**The resolver a router advertises to the household must not be a development
board.** A machine that takes two A/B updates in a day is not the machine every
device in the house depends on for names: when it stops, the house has no
DNS. A development fleet's server may run the service for a device pointed at it
by hand — `ADR-063` §9 step 4 proves it that way, on dev-01, with only a phone's
own Wi-Fi setting naming it and the router untouched — because a device that
chose it by hand is one device, and its owner can point it elsewhere.

**It runs only where a document asks for it.** The service ships disabled on
every card (§6), so no update turns a resolver on for a machine that did not
request one.

Two static name servers share no state, so a second one costs almost nothing to
stand up — and that is a property of statelessness, **not failover**, which
would need shared state to be worth the word.

---

## 3. Why forwarding decides the whole question

**A static A-record responder is two hundred lines and this project should write
it.** Nothing about serving half a dozen names from a table justifies a
dependency. If that were the whole requirement, a responder over `simple-dns` —
two crates, already in the tree behind `cmd/rivet-mdnsd`, a 422 KB binary and no
async runtime — would be the correct answer and it would not be close.

**A forwarder is a different program**, and each of its parts fails
intermittently and silently when subtly wrong, which is `ADR-009` §3.3's named
case for reuse:

1. **EDNS0 must be relayed faithfully in both directions.** A forwarder that
   strips the client's OPT record forces every upstream answer into 512 bytes.
2. **Truncation implies TCP.** An answer too large arrives with `TC` set and the
   client retries the same query over TCP to the same resolver. A server bound
   only to UDP/53 turns that retry into a dead end, and the symptom is one
   particular name failing while everything else works.

   **The forwarder does the same upstream, and it asks one upstream at a
   time** so that it can. It asks with an EDNS0 buffer of 1232 bytes, so a
   larger answer comes back truncated from both of Quad9's addresses. The
   dependency's default asks two upstreams in parallel, and its pool then reads
   the second truncated UDP reply, arriving after the first had already switched
   the query to TCP, as a TCP reply that was still truncated: the query fails and
   the client is answered `SERVFAIL`, for every answer over 1232 bytes.
   `cmd/rivet-dnsd`'s `serve::UPSTREAMS_AT_ONCE` sets it to one, and a test
   replays a truncated reply and a whole TCP reply that Quad9 sent through two
   loopback stand-ins to hold it. **The cost:** the second upstream is asked
   after the first has failed rather than alongside it, so an upstream that never
   answers costs that question the resolver's timeout before the other is tried.
3. **Off-path spoofing is a standing risk.** Every forwarded query is
   outstanding state with a window in which a forged answer beats the real one.
   Random transaction IDs, source-address and port checking on the reply, and a
   bounded outstanding-query table are ordinary and non-negotiable.

**A cache is NOT separable, and it is not a decision this record gets to take.**
It arrives with the dependency and it is on. Measured on the board: a name
absent from the cache costs 36–74 ms, the same name a moment later costs
4.5–8.5 ms, and the TTL counts down locally — 60, 58, 56, 54 across six seconds.
There is no cache setting in the composition.

So adopting `hickory-server` *is* adopting a resolver cache, and the honest
statement of the trade is that the cache came free rather than that it was
deferred. What remains genuinely ours to set is how long **our own** records
live in other people's caches, which is §8.

---

## 4. What was rejected, and the cost of this choice

### 4.1 A hand-rolled responder over `simple-dns`

**Rejected because it does not forward**, and would have to grow §3's three
mechanisms to. Correct and preferred **if forwarding is ever taken back out of
scope** — and in that event this decision should be revisited rather than
extended, because the forwarder is the whole justification.

### 4.2 `hickory-proto` for the wire format with a server written here

The middle option: 50 crates rather than 92, wire parsing reused and the server
loop owned. **Rejected because the parts worth reusing are in the server, not
the protocol** — §3's list is resolver behaviour, and taking `hickory-proto`
alone buys the easy half.

### 4.3 The cost, stated rather than minimised

**92 crates for a table of half a dozen names is a great deal**, and this record
should not pretend otherwise. 37 are new against the lock and **27 of those 37
are the ICU, `idna` and `url` stack** — Unicode machinery for a deployment whose
names are ASCII labels under two suffixes. `hickory-server` 0.26 has no default
features, so none of it can be turned off.

The binary is **1.13 MB static for `aarch64-unknown-linux-musl`**, against a
422 KB hand-rolled responder and 640 MiB slots.

**Nobody here has read `hickory-proto`.** `ADR-009` §3.3's last bullet is that
reading a dependency closely enough to trust it can cost as much as writing the
thing; that cost is accepted here and has not been paid.

---

## 5. What the deployment needs, and does not

**A/AAAA for static names, and whatever the forwarder relays.** The household's
own names are ASCII labels under the suffixes the fleet authority permits, which
`ca::permitted_suffixes` reads from the root certificate rather than from a note
beside it (`ADR-011` §3).

**Deliberately not wanted:** DNSSEC signing of the local zone, dynamic update,
zone transfer, and any form of DHCP. The first three are authority-server
features for a table edited by hand; the fourth is the router's job by the
constraint in §1.

---

## 6. Consequences

* **An async runtime is on the board already**, so this brings none that is new:
  `tokio` is in the lock, `cmd/rivet-proxyd` and `cmd/rivet-smbd` both declare
  it, and 42 of the 92 crates are present.
* **It is an image service, `rivet-dnsd`, shipped disabled.** §8 puts the table
  in the desired-state document, and the document is read by image services;
  an application is handed only its own `[applications.<name>]` settings, as
  flat strings declared in its manifest, which cannot carry a table of records
  whose labels the operator chooses. So the program is on every card at
  `/system/bin/rivet-dnsd`, and `[services.rivet-dnsd] enabled = true` turns it
  on — `rivet-mdnsd`'s arrangement. It changes by system update.
* **It runs under its own identity and a Landlock ruleset**, as `rivet-proxyd`
  does (`ADR-060` §2.1): the uid and gid `app_identity("rivet-dnsd")` derives,
  `confinement::image_service_policy`, and a home at `/state/app/rivet-dnsd`. It
  is the one service that parses datagrams from any device on the network and
  answers from the internet, so a defect in its query path reaches only what
  that ruleset grants — its own `data/` and `tmp/` to write, read-only views,
  and the sockets it was handed — and never root's reach.
* **It is handed a projection of the document, never the document itself.** A
  confined program is not granted `/state/config`, and an application's flat
  settings cannot carry the table, so the supervisor writes the service its own
  file:
  * **Path:** `/state/app/rivet-dnsd/dns.toml`
    (`rivet_core::name_service::path`), beside its `settings` file, root-owned,
    mode `0644`, in the root-owned home directory so the service can neither
    plant a link there nor rewrite the zone it serves. The ruleset grants it
    read and nothing else.
  * **Contents:** a valid configuration document holding `version`, the
    `[dns]` table, and `hostname` set to the name the machine carries — derived
    or stated, as `cmd/rivet-sysd`'s naming settled it — and nothing else.
    `Document::dns_projection` builds it and `Document::to_toml` renders it, so
    one serialiser writes it and the service reads it with `Document::parse`,
    exactly as it would read the whole document.
  * **Writer and when:** `cmd/rivet-sysd`'s `deliver_name_service_document`,
    wherever settings are delivered — at boot before the first convergence,
    after a finished install, and at a reload, after the service's home is made
    so a reload that turns the service on writes the file before it starts. Only
    while the service is enabled. A changed table is served after the service
    restarts, as §8.3 says.
  * **The hostname travels in the file** because the service may not read
    `/proc/sys/kernel/hostname` under its ruleset, and the SOA and NS records
    must name the machine (§8.4).
* **The supervisor binds UDP 53 and TCP 53 on every address** and hands both
  over; neither is `proxied`, because a resolver is asked directly.
* **The socket arrives blocking.** `cmd/rivet-sysd`'s `open_listeners` binds
  with `std::net::UdpSocket::bind` and sets no options, so the handoff is
  `into_udp_socket` → `set_nonblocking(true)` → `tokio::net::UdpSocket::from_std`.
  Omitting the middle call fails at runtime rather than at compile time.
* **`Socket::Udp(u16)` is the declaration**, with `into_udp_socket` beside it —
  `rivet-sdk/src/listen.rs`.

---

## 7. What the experiment showed, and what is still unproven

**This decision has survived contact.** Both candidates were built, deployed to
the test board as applications on non-standard ports, and driven with real
queries. `hickory-server` runs on the Cortex-A72, answered every question put to
it, and did not panic, hang or restart.

### 7.1 The four queries

| | result |
| :-- | :-- |
| A name in the static table | `NOERROR`, authoritative, the table's address |
| A gap **inside** our own zone | `NXDOMAIN` from the zone with SOA — not forwarded |
| A name outside it | forwarded to Quad9 and answered |
| A large answer without EDNS0 | 512 bytes with `TC` set; the **TCP retry returned it whole** |
| The same with EDNS0 at 4096 | 853 bytes untruncated over UDP |

**§3's three mechanisms are all exercised.** The client's OPT record reaches
Quad9 and the large answer comes back, so EDNS0 is relayed in both directions;
truncation and the TCP retry work end to end; and the longest-suffix dispatch is
correct, which is the property that keeps a hole in our own zone from leaking
out to an upstream resolver.

**Malformed datagrams are handled and none of them is fatal.** Empty, short,
lying header counts, a compression pointer aimed at itself, a label length with
nothing behind it, and 512 bytes of noise each produced a drop or a `FORMERR`.
The probe answered a normal query immediately afterwards, with the same pid and
zero restarts.

### 7.2 Two findings that change what §4 argued

**A truncated UDP answer carries the partial bytes of the record it dropped.**
The 512-byte datagram declares one answer; the counted records occupy 292 bytes
and the remaining 220 are a half-serialised second record. Tolerant parsers
ignore the tail and retry over TCP, which is the behaviour that matters — but a
strict one rejects the datagram outright. Not a blocker, and an interoperability
edge nobody had seen because nothing had run.

**The two hundred lines this project "should write" shipped a reflection path.**
`simple-dns` answered a message with `QR` set — a *response*, not a query — with
`REFUSED`, which means a spoofed source address turns it into a small traffic
source aimed at a victim. `hickory-server` refuses to process one at all, in a
single line, with the reason in its source. §4.1 rejected the hand-rolled
responder on the strength of the **forwarder**; this says the reuse argument
also holds for the part nobody was worried about, which is the more uncomfortable
result and the reason it is recorded here rather than in a journal entry.

### 7.3 What is still unproven

**Behaviour under load and concurrency.** Nothing was measured beyond single
queries, and no `simple-dns` forwarder exists, so none of §3's mechanisms has a
comparison on that side.

**No client has been observed honouring a TTL this project chose.** The `[dns]`
table is read by `rivet-dnsd` and its answers — the table's address and TTL, a
gap answered `NXDOMAIN` with the negative TTL, nothing sent back for a packet
with `QR` set — are asserted by its tests against the real catalog on
loopback. What a phone's or a laptop's resolver then does with those TTLs is
unread.

---

## 8. The static table, its TTLs, and what a reload means

**Ruled by the maintainer 2026-09-07.** The table and both TTLs live in the
desired-state document, with defaults that work untouched.

    [dns]
    zone = "my-fleet.home.arpa"
    ttl = 300
    negative-ttl = 60

    [dns.records.arbor]
    address = "192.168.50.10"

    [dns.records.printer]
    address = "192.168.50.40"
    ttl = 60

**The shape is precedent rather than invention.** `[applications.<name>]` is
already modelled as a name mapping to a flat key/value table, and `[dns.records]`
is that one level over — so per-record override costs no new idea.

### 8.1 Why 300, and why the asymmetry decides it

**Too long is expensive and too short is nearly free.** A record that outlives a
change leaves the whole LAN wrong with no way to force it; a record re-fetched
often costs a query the board answers from a table in **2.7–9.7 ms** at about a
megabyte resident. When one direction of error is cheap and the other is not,
lean into the cheap one.

Five minutes self-heals a mistake while the operator is still at the keyboard
and is long enough that nothing chatters.

**Per-record override earns its place through one operational move**: drop a
record's TTL the day before renumbering something, move it, put it back. That is
what per-record TTL is for in real DNS, and it is the whole reason the field is
not global-only.

### 8.2 The negative TTL is the one that decides whether ADDING a name works

**60 seconds, and it matters more than the positive TTL for a hand-edited
table.** A client that asks for a name before it exists caches the *absence* for
this long. So the failure is: add `printer` to the table, reload, and the one
device that happened to ask two minutes ago still cannot resolve it — and
lowering the record TTL does not touch that, because the record was never the
thing cached.

Adding names is the common operation on a table a person edits. The number that
governs the common operation should be the short one.

### 8.3 A restart rebuilds the zone, and cannot reach into other caches

**The service reads the table when it starts.** Editing the document and then
`apply service-restart rivet-dnsd confirm` rebuilds the zone, and the new
answers are served immediately; the next boot does the same. That is the whole
of what this machine controls. The restart also empties the service's own
resolver cache, which a reload that rebuilt only the zone would keep — a cost
of one cold lookup per name, and the reason a zone-only reload is still worth
building.

**A client holding the old record keeps it for the OLD TTL.** Lowering a TTL
only helps if it was lowered *before* the change that needed it — which is the
oldest operational fact in DNS and the one that makes the difference between a
setting that works and a setting that works the way people expect.

The resolver cache of §3 is a separate thing and a zone-only reload would not
touch it: it holds foreign answers, and our zone is not foreign to us. The same
split BIND drew between `reload` and an explicit cache flush.

### 8.4 What the table's shape needed that §8's ruling did not say

Decided when the service was built, each recorded here so it is easy to
overturn:

| | Decision | Why |
| :-- | :-- | :-- |
| The zone | `zone` is required once `[dns]` appears, and is `home.arpa` or a name beneath it | A record is a label and names nothing until it is placed under a zone. RFC 8375 reserves `home.arpa` for exactly this, so no public resolver answers for it and a zone here cannot shadow a public name — a zone outside it would make every client of this resolver get `NXDOMAIN` for the real names beneath it. Whether the fleet's root permits the zone is the certificate authority's check (`ca::permitted_suffixes`), not the document's. |
| A record | `[dns.records.<label>]`, a single lowercase host label, with `address` required and `ttl` optional | §8's own shape. A-records only, as §5 says. |
| TTL bounds | 1 to 86400 seconds | §8.1's asymmetry: too long is the expensive error, and a hand-edited table has no use for a record cached longer than a day. |
| SOA and NS | Both name `<hostname>.<zone>`, the hostname folded to lowercase. A projection with no hostname, or one that is not a single label, is refused at start with the reason. The SOA's TTL and its `minimum` are both `negative-ttl` | A resolver caches an absence for the lower of the two (RFC 2308 §5), so both must say what §8.2 set. A substitute server name would rename both records with nothing saying so; DNS compares names case-insensitively (RFC 4343), so folding the case changes no name. |
| The upstream | Quad9, `9.9.9.9` and `149.112.112.112`, by address and not configurable | §1 took Quad9. By address because a resolver that had to resolve its own upstream would depend on itself. |
| No `[dns]` | An enabled service with no table refuses to start, naming the key | Forwarding with no zone is a resolver nobody asked for; the refusal is one line in the journal. |
| `/etc/hosts` | The table is not projected into `/etc` | Its reader is the service. A copy in one machine's `/etc/hosts` would be a second answer to the same name. |

---

## History

**2026-10-03 — §6 said the service ran as root, and §8.4 let a nameless machine
serve `ns.<zone>`.** §6 read:

> ~~**It runs as uid 0 with no confinement**, like `rivet-mdnsd`: an image
> service has no manifest to compute a ruleset from, and it reads the document
> under `/state/config`.~~

The machinery `ADR-060` built for the proxy — a derived uid, the image-service
ruleset and a home — fits this service unchanged except in what it reads. The
confinement policy grants no program the machine document, and flat settings
cannot carry the table, so the maintainer chose a supervisor-written
projection of the document over granting the service a read of
`/state/config`. §8.4's row read:

> ~~Both name `<hostname>.<zone>`, or `ns.<zone>` when the machine's name is
> not one label.~~

Confined, the service cannot read the kernel's hostname file, and the
`.ok()` fallback would have quietly renamed both records. The maintainer ruled
that the name travels with the projection and that its absence is a refusal.

**2026-09-29 — §6 said the service was an APPLICATION, and §2 kept it off the
development board.** §6 read:

> ~~**It is an APPLICATION and not part of the image.** `tools/card/build.sh`'s
> `OURS` list is what puts a program on a card, and the executor reaching the
> board through `rivet-proxyd` reached it the same way.~~

Building it showed §6 and §8 could not both hold: §8's table lives in the
desired-state document, and an application is never handed the document, only
its own flat settings. `rivet-proxyd`, the precedent §6 cited, had meanwhile
become an image service itself (`ADR-060`). §2 read:

> ~~**It must not run on the development board.**~~

`ADR-063` §9 step 4 makes dev-01 — the development fleet's server — the place
the service is first proven, reached by one phone pointed at it by hand. The
argument §2 rested on is about the household's advertised resolver and still
stands in the body.

**2026-09-07 — §3 said a cache was "the honest omission for a first version".**
It was not an omission and was never available to omit: `hickory-server` brings
a resolver cache switched on, measured on the board as a 36–74 ms cold query
against 4.5–8.5 ms warm, with the TTL counting down locally. The sentence
implied a decision this record could take later; the decision was taken by
adopting. §3 now says so, and §8 covers the part that genuinely is ours.

The lesson is the one §4.3 was already trying to make: **a dependency's defaults
are decisions, and calling one "deferred" does not defer it.**

**2026-09-07 — the crate figure this decision was nearly taken on was wrong.**
`STATE.md` §3.2 and the journal entry behind it recorded `hickory-server` as
costing **46 crates**. That figure was minted for `hickory-server` **and
`dhcproto` together**, when DHCP was still in scope and the note correctly said
the two were one ecosystem rather than two choices. DHCP was then dropped —
§1's constraint — and the number was never re-measured, so it survived as
hickory's bill while being two crates'.

Measured 2026-09-07 for `hickory-server` 0.26.2 alone: **79 transitive crates,
92 with `resolver`, 37 net new against the lock.**

The lesson is `AGENTS.md` §2 rule 5's, and it is why this paragraph names a
command rather than a number: **a figure that outlives the question it was
measured for is worse than no figure**, because it reads as evidence.
