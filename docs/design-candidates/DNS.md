# DNS for the household's names: `hickory-server`, or a responder of our own

* **Status:** Design candidate — proposed, not accepted
* **Date:** 2026-09-07
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Why it is in Rivet's record:** `STATE.md` §3.2 carries the DNS service, and
  names `ADR-009`'s question — 46 crates against a responder written here — as
  the thing standing in front of it. This document is that question answered
  with numbers rather than adjectives, so it can be ruled from.
* **Method:** every figure in §5 was produced on 2026-09-07 in the build
  container, cache tag `-dnsagent`, from three probe crates under the
  git-ignored `target/dns/` and two throwaway crates in the container's own
  `/tmp`. The commands are printed beside the numbers. Nothing in the workspace
  was modified, no dependency was added to any tracked manifest, and no board
  was touched.

---

## 1. Context

### 1.1 What is settled, and is not reopened here

The framing in `STATE.md` §3.2 is taken as given and this document argues none
of it:

* **The router owns addresses; Rivet owns names.** The maintainer's WiFi
  environment will not yield the DHCP role, so the router's reservation table is
  what makes an address stable and Rivet's job is the name→address mapping over
  the top of it.
* **DHCP is out of scope**, and with it the lease store, the pool, and the
  shared-memory argument that justified one process doing both.
* **A plain UDP socket is all the transport this needs**, and the supervisor can
  hand one over. `rivet_core::package::application::Socket::Udp(u16)` binds it
  and `rivet_sdk::listen::Socket::into_udp_socket` claims it, which is
  `ADR-015` §5.2's privileged-port argument extended to datagrams.
* **It does not run on the development board.** A machine that takes two A/B
  updates in a day is not the machine the household's name resolution lives on.
* **A second server is nearly free and is not failover.** Two Rivets read their
  own documents and both answer authoritatively for the same records, so there
  is no transfer, no serial, and no replica to fall behind.

### 1.2 The question this document answers

`ADR-009` §3's ladder has three rungs and a step down each one requires a
recorded reason. The question is which rung this component sits on:

> Is `hickory-server`'s dependency closure the right trade for the household's
> names, or is this a responder written here over `Socket::Udp`?

**A third option is in the tree already and is the one the survey should have
started from.** `cmd/rivet-mdnsd` depends on `simple-dns` directly, and its
manifest says why in as many words: it is *"the named fallback"* if
`simple-mdns` stops fitting, *"and it is already here."* That makes the choice
three-way rather than two-way, and §5 measures all three.

---

## 2. What the household actually needs

`ADR-009` §3.3's most common failure mode is a crate that does eighty per cent
of what is wanted. The only defence is to write the eighty per cent down first,
from the tree and the record rather than from what a DNS server usually does.

### 2.1 The names that exist

Two suffixes, and the authority already constrains itself to exactly them.
`cmd/rivet-provision`'s `ca::permitted_suffixes` reads the root certificate's
own `nameConstraints` extension and refuses to mint a leaf outside it, and the
default list a fresh authority is offered is `local` and `<fleet>.home.arpa`.

| Suffix | Who answers for it | What this proposal changes |
| :-- | :-- | :-- |
| `<hostname>.local` | `cmd/rivet-mdnsd`, over multicast on 5353 | unchanged — mDNS keeps it |
| `<host>.<fleet>.home.arpa` | the certificates carry it and no server answers | **this is the gap** |

`home.arpa` was minted into the constraint in anticipation of this and serves
nothing else, and RFC 8375 reserves it for exactly this use — which is also why
a public resolver is *right* to answer NXDOMAIN for it, and why a fallback
resolver cannot cover for Rivet on fleet names.

**The client side needs no new key.** `rivet_config`'s `[network.resolver]`
already takes `nameservers` and `search`, so a fleet member pointed at the new
server is an existing setting with a new value in it.

### 2.2 The record types, one at a time

| Type | Needed | Why |
| :-- | :-- | :-- |
| **A** | **yes** | the whole point. `Document::nameservers` returns `&[core::net::Ipv4Addr]`, and every address in the schema is v4 |
| **SOA** | **yes** | a zone without one has no negative-caching TTL, so an NXDOMAIN from Rivet is cached for however long each client feels like. It is one record per zone, written once |
| **NS** | **yes**, one | a query for the zone apex that returns no NS makes some clients treat the zone as broken. One record, pointing at the server's own name |
| **PTR** | **defer** | reverse lookup is the router's address space, not Rivet's, and nothing in the record asks for it. Adding it later means a second zone under `in-addr.arpa` and no new mechanism |
| **AAAA** | **defer** | there is nothing to serve one from. `rivet-mdnsd` records the same gap about its own single A record; the schema carries no IPv6 addressing to read an address out of |
| **CNAME** | **probably** | an alias for a service — `tempmon` pointing at whichever machine runs it — is the first thing wanted after the machines themselves, and it is cheap either way |
| **SRV** | **no** | service discovery on this network is mDNS's, and `ADR-005` §3 puts it there |
| **TXT** | **no** | nothing asks |
| **DNSSEC** | **no** | signing a zone whose clients pin a certificate authority by name constraint buys nothing. The forwarder half is a different question — see §2.4 |

### 2.3 The forwarder is the requirement that decides this

**A resolver a router advertises must answer for names outside its own zones,
or the house has no internet.** That is not a nicety layered on top of the
static table; it is the difference between a fleet convenience and the
household's name resolution, and the record is explicit that the upstream
forwarder is required rather than optional. The maintainer has taken Quad9 as
the upstream.

Three things follow, and each of them costs code:

1. **EDNS0.** A forwarder that strips the client's OPT record forces every
   upstream answer into 512 bytes. It does not have to *originate* EDNS0, but it
   must relay it faithfully in both directions.
2. **Truncation, and therefore TCP.** When an upstream answer exceeds what the
   client's buffer allows, the answer arrives with `TC` set and the client
   retries the same query **over TCP to the same resolver**. A server that binds
   only UDP/53 turns that retry into a dead end, and the symptom is one
   particular name failing while everything else works. TCP/53 means length-
   prefixed framing, per-connection timeouts and a connection limit.
3. **Off-path spoofing.** Every forwarded query is an outstanding piece of state
   with a window in which a forged answer beats the real one. The defences are
   ordinary and non-negotiable: a random transaction ID per upstream query,
   source-address and port checking on the reply, and a bounded outstanding-query
   table so a flood of queries cannot exhaust memory.

**A cache is not separable from all three, and this study said it was.**
`hickory-server` brings a resolver cache switched on, and the composition offers
no setting that turns it off — so adopting the dependency *is* adopting the
cache, and the honest statement of the trade is that it came free rather than
that it was deferred. Measured on the board: a name absent from the cache costs
36–74 ms, the same name a moment later costs 4.5–8.5 ms, and the TTL counts down
locally. `ADR-047` §3 carries the corrected reasoning and §8 there covers the
part that genuinely is the operator's — how long **Rivet's own** records live in
other people's caches.

The general lesson is worth more than the correction: **a dependency's defaults
are decisions, and calling one "deferred" does not defer it.**

### 2.4 What is deliberately not wanted

* **Recursion.** Rivet forwards to one upstream; it does not walk from the root.
  A recursive resolver is an order of magnitude more surface and buys nothing
  that Quad9 does not already provide.
* **Dynamic update, AXFR, zone transfer.** The zone is static records in a
  machine document. §1.1's second-server argument depends on there being no
  transfer at all.
* **DNSSEC validation.** Quad9 validates. Validating a second time on the Pi
  costs crypto in the query path for no additional guarantee the household can
  act on. Worth revisiting the day the upstream is untrusted; not before.

---

## 3. `ADR-009`'s ladder, walked

### 3.1 The ladder, as it is written

`ADR-009` §3, in descending order of preference, with a step down requiring a
reason recorded in the commit or the ADR that takes it:

| | Option | When it is right |
| :-- | :-- | :-- |
| 1 | An existing Rust crate or project | Default |
| 2 | An existing C component, wrapped | When no Rust equivalent is credible, or the C one carries a currency benefit |
| 3 | Write it | Only when the component *is* Rivet |

**Rung 2 is dismissed in one line and is not revisited below.** `dnsmasq` and
`unbound` are the C answers, both would be new Tier C members under `ADR-001`
§4, and `ADR-009` §3.1's whole argument is that a protocol implementation is
what rung 1 exists for. There is a credible Rust equivalent, so the condition
rung 2 requires is not met.

### 3.2 Rung 3's boundary says this is not Rivet

`ADR-009` §3.1 draws the line precisely — *"Rivet writes the things that make
Rivet a distinct system, and reuses everything else"* — and then names the
reusable side with an example that lands on this document's desk:

> Reused wherever possible: everything that is a general-purpose utility, a
> protocol implementation, or a wire format. A DHCP client, an mDNS responder, a
> JSON Schema generator and a netlink binding are all *solved problems that
> someone else maintains*, and Rivet's version of any of them would be worse.

An mDNS responder and a unicast DNS server are the same wire format with
different transport rules. **On §3.1's own list, the default answer is rung 1**,
and a decision to write one has to earn its way down two rungs against that
sentence.

**The counter-argument, stated fairly, is that this project has walked to rung 3
before and been right.** `ADR-009` §3.6 rejected all three netlink crates and
wrote four requests by hand, on the reasoning that *"netlink is bytes — the wire
format, the decisions and every failure path are platform-independent if they
are written to be."* DNS is bytes by the same argument. What §3.6 also records
is the scope of that verdict: **it is scoped to the four messages Stage 1
sends**, and §3.6.2 names the thing that would overturn it as **decoding
messages Rivet did not construct**. A datagram from a LAN client is precisely
that.

### 3.3 §3.4's wall test splits the candidates in half

> **Can I put a wall around it?**

`ADR-009` §3.4's two columns answer this cleanly, and the answer is different
for the parser and for the server:

| | Column | Why |
| :-- | :-- | :-- |
| `simple-dns` | **left** — "protocol implementations, parsers, formats" | it parses bytes and returns a struct. It owns no socket, no loop and no lifecycle |
| `hickory-proto` | **left**, same row | same shape, larger |
| `hickory-server` | **right** — "anything wanting to own the process lifecycle", "async runtimes" | `Server::block_until_done` is the program's main loop, and it is a tokio one |

**The right column is a design decision rather than a dependency choice**, which
is §3.4's own instruction, and it is why this document is a design candidate
rather than a commit.

**But the strongest objection to the right column is spent, and this is
measured.** `ADR-021` §4.1 and `rivet-adapter-net` have both rejected crates for
bringing an executor, and `cmd/rivet-mdnsd`'s manifest states the workspace
position as *"this workspace has no executor."* That sentence describes a tree
that has changed: `cmd/rivet-proxyd` and `cmd/rivet-smbd` both declare `tokio`,
`rivet-proxyd` is what answers 443 on the test board today, and `tokio 1.53.1`
is in `Cargo.lock`. **42 of `hickory-server`'s 79 transitive crates are already
in this workspace's lock file**, tokio among them (§5.2). The runtime is not new
to the project, and it is not new to the machine.

What remains true is narrower and is worth keeping: **`tools/card/build.sh`
selects the card's binaries by name and neither `rivet-proxyd` nor
`rivet-smbd` is among them**, so the executor is on the board as an
*application* and not in the image. Whether the DNS service is one or the other
is §9's second decision, and it is where this cost actually lands.

### 3.4 §5's gates, measured rather than asserted

`ADR-009` §5 names five, and §3.7.3 added a sixth.

| Gate | `simple-dns` | `hickory-server` |
| :-- | :-- | :-- |
| **Cross-compiles to `aarch64-unknown-linux-musl`** (hard veto) | **passes** — §5.1 built a binary | **passes** — §5.1 built a binary |
| **Builds on the development host** (§3.6.1, a cost not a veto) | **passes** — built on `x86_64-pc-windows-msvc` | **passes** — built on `x86_64-pc-windows-msvc` |
| **`cargo deny` against this repository's own `deny.toml`** | already in the tree | **`advisories ok, bans ok, licenses ok, sources ok`**, unchanged (§5.3) |
| **Dependency weight** (`ADR-002` §5.1) | 2 crates | 79 crates, 37 of them new to the lock (§5.2) |
| **Abandonment** (§4's mitigation) | a small crate with one dependency; `cmd/rivet-mdnsd` already carries the exposure | the flagship Rust DNS project, and the one every other Rust DNS crate depends on |
| **Runs on the Cortex-A72** (§3.7.3) | **unproven** | **unproven.** Neither closure ships CPU-specific code — no assembly crypto in either — so the expectation is that both run, and only the board settles it |

**§3.6.1's development-host gate is the one that decided the netlink survey and
it decides nothing here**, because both candidates pass it. That is worth saying
plainly: the gate that made writing netlink by hand correct does not transfer.

---

## 4. The three options, described

### 4.1 Option A — a responder written here, over `simple-dns`

**What comes for free:** the wire format. `Packet::parse` decodes a datagram
into questions and records and returns a `Result`; `build_bytes_vec_compressed`
encodes one with name compression; `Name`, `CLASS`, `QTYPE`, `RData::A` and
`ResourceRecord` are the vocabulary `cmd/rivet-mdnsd` already uses. `Packet::opt`
and `opt_mut` reach the EDNS0 OPT record, and `rcode_mut` sets the response code.

**What has to be written:** the socket loop; matching a question against the
document's records; SOA and NS synthesis; NXDOMAIN versus NODATA; the forwarder
in §2.3 with its transaction-ID rewriting, outstanding-query table, timeout and
source checking; TCP/53 with its two-byte length framing; and the tests for all
of it. The static half is a couple of hundred lines. **The forwarder half is
where the real work is**, and it is the half that handles bytes from an
arbitrary upstream as well as from an arbitrary client.

**Where it wins:** `ADR-009` §3.4's wall is unnecessary because there is nothing
to wall — the only types crossing into Rivet's own code are Rivet's. Two crates,
both already in the lock. The smallest binary by a factor of two. And the
comprehension cost §3.3 names as a hidden charge is paid in understanding of
Rivet's own system rather than somebody else's.

### 4.2 Option B — `hickory-proto` for the wire format, a server written here

This is the shape that looks like the interesting middle and **it is dominated
by option A**, measured: `hickory-proto` alone, with no default features, costs
**50 crates** against `simple-dns`'s **2**, for the same job in the same
position in the design. It buys a more complete record-type vocabulary and
DNSSEC types §2.2 says are not wanted.

**It is recorded here so the option is visibly considered rather than missed.**
The one thing that would revive it is a decision to serve record types
`simple-dns` handles poorly, and nothing in §2.2 is one.

### 4.3 Option C — `hickory-server`

**What comes for free:** the whole server. `Catalog` dispatches a query to a
zone handler by longest-suffix match; `InMemoryZoneHandler` holds the static
records and answers A, NS, SOA, CNAME and the rest with correct NXDOMAIN and
NODATA semantics; `ForwardZoneHandler` registered at the root is the forwarder
in §2.3, with EDNS0, retry and a cache behind it; `Server::register_listener`
is TCP/53 with framing and timeouts; `Server::with_access` takes allow and deny
networks. Malformed input is `hickory-proto`'s problem and it is a `Result`.

**What has to be written:** the composition — read the machine document, build
the zone, register the sockets — plus the wall `ADR-009` §4 requires, so that no
`hickory` type appears in the configuration schema, a CLI output or a
`rivet-core` signature.

**One integration detail that is not guessable.** The supervisor binds UDP with
`std::net::UdpSocket::bind` and **sets no options at all** — deliberately, so
that it never learns what the socket is for. `Server::register_socket` takes a
`tokio::net::UdpSocket`, which requires the descriptor to be non-blocking. So
the sequence is `into_udp_socket`, then `set_nonblocking(true)`, then
`tokio::net::UdpSocket::from_std`. Omitting the middle call produces a runtime
error rather than a compile one.

**Where it wins:** §2.3 in its entirety, correct on the first day, written by
people who have read the RFCs. `ADR-009` §3.3's *"anywhere being subtly wrong is
expensive"* row is exactly the forwarder's error path.

---

## 5. The measurements

Everything in this section was produced on 2026-09-07 in the build container.
Where a number is an estimate it says so; nothing here is one.

### 5.1 Binary size, static `aarch64-unknown-linux-musl`

Three crates were written under the git-ignored `target/dns/`, each with the same
release profile — `opt-level = "z"`, `lto = true`, `codegen-units = 1`,
`panic = "abort"`, `strip = true` — and each built with:

    RIVET_CACHE_TAG=-dnsagent tools/container/exec.sh sh -c \
      'cd /rivet/target/dns/<crate> && export CARGO_TARGET_DIR=/tmp/t && \
       cargo build --release --target aarch64-unknown-linux-musl'

| Probe | What it does | Bytes |
| :-- | :-- | --: |
| `simple-dns` | binds UDP, parses a query, answers A from a static table, refuses a malformed datagram, sets NXDOMAIN | **422 336** |
| `hickory-server`, no features | `Catalog` + `InMemoryZoneHandler` with an SOA and an A record, on a tokio UDP socket | **871 224** |
| `hickory-server`, `resolver` | the same **plus** a `ForwardZoneHandler` at the root pointed at `9.9.9.9` | **1 131 824** |

**The deltas: +448 888 B for the server framework, +709 488 B for the framework
with the forwarder** — a factor of 2.06 and 2.68 respectively.

**Read that comparison honestly.** The `simple-dns` probe is a stub with no
forwarder, no TCP and no SOA synthesis, so it will grow; the `hickory` probes
are the real thing. The measurement bounds the framework's cost from below, and
the gap narrows as option A is finished rather than widening.

**None of these figures is decisive against `ADR-018` §4's 640 MiB `/system`
slots.** Three quarters of a megabyte is not an image-size argument, and this
document declines to make one.

### 5.2 Crate counts

    RIVET_CACHE_TAG=-dnsagent tools/container/exec.sh sh -c \
      'cargo new --bin /tmp/hs && cd /tmp/hs && cargo add hickory-server && \
       cargo tree --edges normal --target aarch64-unknown-linux-musl --prefix none \
         | sed "s/ (\*)$//" | sort -u | wc -l'

| Closure | Unique crates | Net new to `Cargo.lock` |
| :-- | --: | --: |
| `simple-dns` | 2 (`simple-dns`, `bitflags`) | **0** — both are in the lock today |
| `hickory-proto`, no default features | 50 | not computed |
| `hickory-server` 0.26.2 | **79** | **37** |
| `hickory-server` 0.26.2, `resolver` | **92** | not computed |

Three things the counting turned up that the numbers alone do not say:

* **`hickory-server` 0.26 has no default features**, so `--no-default-features`
  resolves the identical tree. Trimming it is not available as a lever.
* **Ten of the 79 are proc-macro crates** — build-time only, and absent from the
  binary.
* **42 of the 79 are already in this workspace's lock**, including `tokio`,
  `serde`, `time`, `thiserror`, `tracing`, `mio`, `socket2` and `syn`. The
  marginal cost of adopting it is the 37 in the right-hand column.
* **27 of those 37 are the ICU, `idna` and `url` stack** — `icu_*`, `zerovec`,
  `yoke`, `zerofrom`, `tinystr`, `writeable` and their siblings. That is
  Unicode normalisation for internationalised names and URL parsing, and the
  household's names are ASCII labels under two suffixes. **The largest single
  block of what this dependency costs is machinery this deployment has no use
  for**, and it is not removable by a feature flag.

**This does not reproduce `STATE.md` §3.2's figure of 46 and the difference
should be reconciled before it is quoted again.** The 46 was measured as the new
crates for `hickory-server` **and `dhcproto` together**, against a lock file that
has since gained `tokio` through `cmd/rivet-proxyd`. Both halves of that have
moved: DHCP is out of scope, and the baseline grew. **Neither number is wrong
about its own day**, which is the hazard `AGENTS.md` §2 rule 5 describes — the
count belongs in a command, and the command is above.

### 5.3 `cargo deny`, against this repository's own configuration

    cargo deny --config <repo>/deny.toml check licenses bans sources advisories

Run from the `hickory-server` probe crate on the development host:

```
advisories ok, bans ok, licenses ok, sources ok
```

**Unchanged, with no new licence allowance.** The `allow` list already carries
`Unicode-3.0` — added for the ICU family in a different survey — which is what
the `icu_*` and `idna_adapter` crates need, and every `hickory-*` crate is
`MIT OR Apache-2.0`. The one finding on the first run was the probe crate itself
being unlicensed, which is `[licenses.private] ignore = false` doing its job.

`bans` is `multiple-versions = "warn"`, and the tree does carry two `syn`
versions. It is a warning by configuration and it did not fail the check.

### 5.4 The development-host gate

The option A and option C probes were built a second time with a bare
`cargo build --release` on `x86_64-pc-windows-msvc`, and both finished — the
`simple-dns` one in 5.7 s, the `hickory-server` one in 18.7 s from cold.
`ADR-009` §3.6.1's gate therefore
separates nothing here, which is the useful negative: the argument that decided
the netlink survey is unavailable to this one.

### 5.5 What was NOT measured, named rather than implied

* **Query latency, memory, and behaviour under load. NOT ONE OF THE THREE
  PROBES WAS EVER RUN.** Each compiles, links and would bind a socket; none has
  answered a query. `AGENTS.md` §6 is explicit that this is *configured* rather
  than *working*, and every row in §6's table that describes runtime behaviour
  is read from an API rather than observed.
* **Anything on the Cortex-A72.** `ADR-009` §3.7.3's gate — a dependency must be
  **run** on the target CPU — is open for both candidates. Neither closure
  contains assembly or CPU-feature-gated crypto, so the expectation is that both
  run; **that expectation is a judgement and not a measurement**, and it is
  exactly the shape of the `graviola` failure §3.7.3 was written about.
* **A hand-rolled forwarder's real size.** Option A's binary is a static
  responder only, so the 422 336 B figure is a floor and not a like-for-like.
* **`hickory-proto`'s net-new count**, and the `resolver` feature's. Only the
  total was taken.

---

## 6. Attack surface, and what a malformed packet does

**This parses untrusted input from every device on the LAN, and after §2.3 it
also parses answers from the internet.** That is a different exposure from
anything else this project has built: `rivet-mcpd` demands a pinned certificate
before it reads a byte, and `rivet-proxyd` is `ADR-046`'s authentication gate. A
resolver the router advertises answers whoever asks.

| | Option A | Option C |
| :-- | :-- | :-- |
| **Malformed query** | `Packet::parse` returns `Err`. The loop names the sender, drops the datagram and continues — there is no state to corrupt because there is no state | `hickory-proto` returns an error and the framework answers `FORMERR` or drops. Read from the API shape; **not exercised** |
| **Code reading the bytes** | ours, and every line of it is ours to review | 79 crates, of which `hickory-proto` and `hickory-net` are the ones on the query path |
| **Cache poisoning** | ours to get right, and §2.3 lists the three defences that must be present | the forwarder's, and it is what a DNS library exists to have got right |
| **Resource exhaustion** | the outstanding-query table is the attackable structure and must be bounded | bounded by the library, and `Server::with_access` can refuse a source network outright |
| **Panic in the query path** | `cmd/rivet` and two others already deny `unwrap`, `expect` and `panic` by lint; this crate should join them | a panic inside a dependency is a supervisor restart, which `ADR-017` §3.5 handles and which is still a name failing for the household |

**The honest summary is that the two options trade different risks rather than
more against less.** Option A's surface is small and wholly owned, and its
exposure to hostile packets begins on the day it is deployed. Option C's is
large and has been under attack for a decade.
`ADR-009` §3.3's row — *"worse in ways that surface as intermittent failures
years later"* — is an argument about which of those two histories is worth more,
and it says the second one is.

---

## 7. Maintenance

| | Option A | Option C |
| :-- | :-- | :-- |
| **Who fixes a protocol bug** | us, after somebody notices a name resolving wrongly | upstream, usually before it is noticed here |
| **Upgrade cadence** | none. `simple-dns` is pinned and the responder is ours | a crate that moves. `hickory` renamed `ServerFuture` to `Server` and `Authority` to `ZoneHandler` between 0.25 and 0.26 |
| **What a new record type costs** | a match arm and a test | a document key and a test |
| **What `cargo deny` costs** | nothing new | 37 more crates in every advisory sweep |
| **What `AGENTS.md` §2 rule 5 costs** | prose describing a responder we own | prose describing a dependency, which rots when the dependency moves |
| **The wall (`ADR-009` §4)** | nothing to wall | a real, ongoing discipline: no `hickory` type in the schema, in a CLI structure, or in a `rivet-core` signature |

**The upgrade row is the one that is underweighted by default.** `hickory` is
pre-1.0 and renames public types across minor versions; the composition code in
§4.3 is where every one of those lands.

---

## 8. Recommendation

**Adopt `hickory-server` with the `resolver` feature, as a rung-1 decision under
`ADR-009` §3 — conditional on the forwarder being in scope.**

The reasoning, in the order it actually carries weight:

1. **§2.3 is the whole argument.** A static A-record responder is two hundred
   lines and this project should write it. A **forwarder** is EDNS0 relaying,
   truncation and TCP fallback, transaction-ID randomisation, source validation
   and a bounded outstanding-query table — every one of them a thing that fails
   intermittently and silently when it is subtly wrong, which is `ADR-009`
   §3.3's named case for reuse.
2. **The ladder's default is rung 1 and this component is on §3.1's reuse side
   by name.** A protocol implementation someone else maintains. Walking down two
   rungs needs a reason, and the reasons available are weaker than they look:
   §3.6.1's development-host gate passes for both (§5.4), and §3.6's netlink
   verdict scopes itself to messages Rivet constructs, which a query from a
   phone is not.
3. **Every gate in `ADR-009` §5 passes, measured.** Cross-compilation, host
   build, `cargo deny` unchanged with no new licence allowance, and a size delta
   of three quarters of a megabyte against 640 MiB slots.
4. **The strongest objection is spent.** "It brings an async runtime" was true
   of this workspace and is not: `tokio` is in the lock and on the board, and 42
   of the 79 crates are already there.

### 8.1 The condition, stated so it can be ruled on separately

**If the maintainer rules that Rivet answers only for its own suffixes and
forwards nothing, the recommendation flips to option A and it is not close.**
Two crates already in the tree, a 422 KB binary, no async runtime, no wall to
maintain, and a component small enough to read in an afternoon.

**The record recommends forwarding and does not rule it.** It is the first
decision in §9 for that reason. The costs of the alternative are real and the
journal already measured them: a client that gets `REFUSED` or `NXDOMAIN` from
Rivet for an internet name then depends on resolver-fallback behaviour that
differs across Windows, Android, macOS and glibc — one queries in order, one
queries both at once, one takes whichever answer arrives first. That is the
fragility the forwarder exists to remove.

### 8.2 The case against, put as strongly as it deserves

* **Seventy-nine crates for a table of half a dozen names is absurd on its
  face**, and the fact that 42 are already present is an accident of
  `rivet-proxyd`'s existence rather than a design.
* **27 of the 37 new ones are Unicode and URL machinery** (§5.2). The household's
  names are ASCII labels under two suffixes, so the largest single block of what
  is being adopted is for a problem this deployment will never have — and it
  cannot be trimmed away, because 0.26 has no features to turn off.
* **`ADR-009` §3.4 puts `hickory-server` in the right-hand column** — a
  framework that owns the process loop — and §3.4's instruction for that column
  is to treat it as a design decision rather than a dependency choice. This
  document is that treatment; it is not a substitute for the maintainer taking
  the decision.
* **The comprehension cost is real.** §3.3's last bullet: reading a dependency
  closely enough to trust it can cost as much as writing the thing, and leaves
  less understanding of our own system. Nobody here has read `hickory-proto`.
* **`ADR-009` §3.2's own record is that twice a cost verdict reached without a
  survey was wrong, in opposite directions.** This survey exists; that is what
  makes it a decision rather than a guess. But it measured builds and trees, and
  it did not run a single query.

### 8.3 What would settle the residual doubt

One experiment, and it is cheap: **build both, point them at the same static
table, and drive them with real queries** — a well-formed A lookup, a name the
zone does not hold, a truncating answer through the forwarder, and a
deliberately malformed datagram, on the board rather than the workstation.
Roughly a day. It closes §5.5's first two bullets at once — behaviour under
real input, and `ADR-009` §3.7.3's run-on-the-target-CPU gate — and a result
either way would move this recommendation where nothing in §5 can.

---

## 9. The decisions owed

Each of these is separable and can be ruled on its own.

1. **Does Rivet forward, or answer only for its own suffixes?** §8.1 — this is
   the one the recommendation hangs on, and everything else is smaller.
2. **Is this a card binary or an application?** `tools/card/build.sh` names the
   card's binaries explicitly, and `rivet-proxyd` is an application rather than
   an image component. An application is deployed and rolled back on its own
   schedule, which suits a service that must not follow the development board's
   images. A card binary is in the trusted set from boot. §1.1 says this runs on the
   fleet server, whose images are the ones that stop being experimental.
3. **`hickory-server`, or a responder over `simple-dns`?** §8 recommends the
   first, conditional on 1.
4. ~~**A cache, in the first version?**~~ **NOT OWED — it was never available to
   decide.** §2.3, corrected: the cache arrives switched on with
   `hickory-server` and there is no setting in the composition that removes it,
   so this item asked for a ruling on something the choice of dependency had
   already settled. `ADR-047` §3 states it. What is genuinely the operator's is
   how long Rivet's own records live in other caches, which `ADR-047` §8 rules
   as `[dns] ttl` and `negative-ttl`.
5. **Does the zone serve CNAME?** §2.2 — the first thing wanted after the
   machines, and cheap under either option.
6. **Is TCP/53 declared from the start?** It is a second `Socket::` entry in the
   manifest and costs nothing under option C; under option A it is a second loop
   and a framing implementation. It becomes mandatory the moment 1 is answered
   "forward".
7. **What the component is called**, since it is no longer the DHCP-and-DNS
   daemon the original proposal named. Naming it for what it does was the
   record's own recommendation.

---

## 10. Corrections this study owes other documents

Listed rather than made, because a design candidate's changes belong with the
ruling.

1. **`STATE.md` §3.2's crate figure needs restating.** §5.2 measures 79
   transitive crates and 37 net new for `hickory-server` alone; the pinned 46
   was `hickory-server` and `dhcproto` together against an older lock. The fix is
   a command rather than a number, per `AGENTS.md` §2 rule 5.
2. **`cmd/rivet-mdnsd/Cargo.toml` says *"this workspace has no executor"*** as
   the reason for taking `simple-mdns` with `sync` rather than `async-tokio`.
   `cmd/rivet-proxyd` and `cmd/rivet-smbd` both declare `tokio`. The *choice* is
   still right — that responder wants no runtime — but the reason as written is
   about the workspace and should be about the crate.
3. **`STATE.md` §3.2 does not mention the upstream forwarder**, which §8 finds
   to be the requirement that decides the whole question. Whichever way
   decision 1 goes, the summary should carry it.
