# ADR-040: Serving in the SDK

* **Status:** **Accepted, 2026-09-04. Amended by its first adopter,
  2026-09-05.** The module is built, its suite runs, and **one application has
  adopted it and served real requests over TLS on an emulated aarch64 guest.**
  It has still **not run on a board.** §9 says what was verified and by what
  method; **§10 is the first adopter's report, written after contact, and it
  contradicts §8 about that adopter.**
* **Date:** 2026-09-04
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Amends:** `ADR-036` §5 (*"`rivet-sdk` depends on no Rivet crate"*) for the
  optional-feature case only — see §6.
* **Depends on:** `ADR-036` §5.4A (the precedent that a role-specific heavy
  dependency goes behind a feature), `ADR-035` §7.2 and `ADR-025` §2 (where TLS
  lives), `ADR-011` (one authority per fact).

---

## 1. Context

**Rivet exists to run network services, and every network service in this
repository wrote its own HTTP server.**

Three of them, independently:

| Program | Concurrency | Head parser | Server certificate | Connection cap | Head deadline | Body bound |
| :-- | :-- | :-- | :-- | :-- | :-- | :-- |
| `cmd/rivet-fleetd` | OS threads | its own | its own | 16 | 30 s | 1 MiB |
| `cmd/rivet-proxyd` | tokio | its own | its own | none | 15 s | streams |
| `cmd/rivet-mcpd` | OS threads | its own | `rivet-tls` (pinned) | 16 | 10 s | 1.2 GB |

Reliquary and TempMon sit beside them on `axum`, and Trends is next.

**The measurement is not a line count.** Three HTTP/1.1 head parsers; three
`rustls::ServerConfig` constructions; the same five things — a bounded head
read, a body bound, a connection cap, a per-connection deadline, and a server
certificate from the fleet authority — implemented three times with different
constants and two different concurrency models. One head deadline is 30 s, one
is 15 s and one is 10 s, for the same operation, and only one of the three
wrote down why.

### 1.1 The argument that settled it is the defect record

The audit of 2026-09-03 found **the same class of bug independently in two of
them**: `rivet-mcpd` had no connection cap and no socket deadlines;
`rivet-fleetd` had no cross-site check, a 1 KiB read limit that silently
truncated a request, and one thread for the whole server. Both were rated HIGH,
both were found separately, and both were fixed separately.

**Neither inherited the other's hardening, because there was nothing to inherit
from.** On 2026-09-04 the same shape arrived from a different angle: `tls12` was
declared in five manifests, and removing it from two of them changed nothing on
a board, because Cargo unified the feature back in from the three that still had
it.

That is the cost being paid. It is not duplication of typing; it is that a fix
lands in one copy and the other two keep the defect.

---

## 2. The decision

**`rivet-sdk` grows a `serve` module behind an optional `serve` feature, and it
is built before Trends so that Trends adopts it from day zero rather than being
retrofitted.** The other consumers are retrofitted *as needed*, which is a
judgement per consumer and not a sweep — §8.

Ruled by the maintainer 2026-09-04. In his framing: Rivet exists for writing
network services, so if this is not in the SDK, an optional component is what
makes sense. (`.rulings/` R-143.)

### 2.1 Why the SDK and not a companion crate

An earlier proposal put it in a crate beside the SDK, on the ground that
`rivet-sdk`'s dependency table is deliberately almost empty. **That reasoning
was upside down.** The SDK already carries an optional heavyweight dependency
for a role and already says why: `draw` is opt-in because *a network service has
no business linking `libdrm` or carrying a font rasteriser.*

**`serve` is the exact mirror of that sentence: a kiosk application has no
business linking `rustls`.** The precedent is in the same manifest, three
paragraphs up from where the new feature is declared.

### 2.2 The Reliquary trap, and why it does not apply

Reliquary could **not** use a Cargo feature to keep `ignition` out of its Rivet
composition root, because **feature unification defeats a feature used as an
enforcement boundary**: absence cannot be guaranteed by something any other
crate in the build can switch on.

`serve` is the opposite kind of feature. It is an **additive capability** an
application asks for. The worst unification can do is link `rustls` into a
program that did not request it, which costs bytes and breaks nothing. Nobody
should read Reliquary's note and conclude a separate crate is required.

---

## 3. It ships the PIECES and never the loop

**This is the constraint that shapes the whole module.** `rivet-fleetd` is one
OS thread per connection; `rivet-proxyd` is tokio. **A module that shipped an
accept loop could be adopted by exactly one of them.**

So there is no loop, no `spawn`, and nothing here owns a listener:

* **[`Head::parse`] works on a buffer** the caller filled by whatever means it
  likes, and answers `Ok(None)` for *not yet*. A blocking loop, a tokio task and
  anything else can all drive it, because the reading is the caller's.
* **`Limit` is a counter that refuses**, never a semaphore that blocks. A caller
  told *no* retries; a caller queued behind fifteen others holds a descriptor.
  It is `Clone`, so it can be claimed in an accept loop and carried into a
  thread or a task.
* **Deadlines are `Duration`s, not behaviour.** `Bounds::apply_to` is offered
  for a blocking `TcpStream` because two of the three consumers write exactly
  those two calls; an asynchronous caller takes the number and wraps its read in
  `tokio::time::timeout`, as `rivet-proxyd` already does.
* **`read_body_into` takes a sink**, so one function serves a JSON call and
  `rivet-mcpd`'s 683 MB payload. On a 926 MB board, buffering that payload would
  mean the OOM killer choosing between a daemon and the supervisor.

A convenience blocking `read_head` **is** provided, and that is deliberate
rather than an inconsistency. **A bound means nothing unless the loop filling
the buffer enforces it, and the loop is where the defect actually was**:
`rivet-fleetd` read `[0u8; 1024]` once, so a request over a kilobyte was
silently truncated and a body in a second TCP segment was silently missing.
Shipping the parser and leaving each caller to write that loop again would share
the cheap half.

---

## 4. Every differing number is the caller's

`.rulings/` R-143 left this open explicitly: one of 30 s, 15 s and 10 s would
become the default, and the reason had to be written down rather than the
largest or the newest silently winning.

**The answer is that none of them wins.** The shared thing is the logic; the
numbers are a `Bounds` value the caller states. A consumer that keeps its own
writes one line:

```rust
let bounds = Bounds { head_patience: Duration::from_secs(15), ..Bounds::default() };
```

The defaults, and the argument for each:

| Field | Default | Why this one |
| :-- | :-- | :-- |
| `connections` | 16 | **Two of the three chose it independently and wrote the same reason.** One operator is one session; a fleet page holds one per device it displays; a clone is one more. Sixteen is several times any of that and small enough that the thread and descriptor cost is bounded at something a Pi can hold. |
| `head_bytes` | 16 KiB | `rivet-fleetd`'s. A request line plus a browser's header block is a small fraction of it, so it is not a working limit — it is the point past which the caller is no longer sending a head. `rivet-proxyd` chose 32 KiB; neither was measured, and the smaller is the default because the bytes come from whoever connected. |
| `body_bytes` | 1 MiB | `rivet-fleetd`'s. A JSON call is small, and an application that has not thought about it should not be handed a gigabyte. A `u64` rather than a `usize`: `rivet-mcpd` accepts 1,200,000,000 bytes, which does not fit a 32-bit `usize`, and the Pi Zero is a deferred compatibility target rather than a dead one. |
| `head_patience` | 10 s | **The one default that overrules a larger number already in the tree.** See below. |

### 4.1 Why the head deadline defaults to the smallest of the three

This deadline covers a peer that has proved nothing. Anyone who can reach the
port can open a socket, and without a deadline each one holds a slot until the
machine reboots. `connections` bounds how many; this bounds how long.

**`rivet-mcpd`'s 10 s is the only one of the three whose choice was written
down** — *before the handshake completes, the peer is nobody* — and the other
two are larger with no recorded reason. A default has to be defensible on its
own, and *the smallest reasoned value* is defensible in a way that *the largest
existing value* is not.

**It is not a session timeout and must not be read as one.** A caller that has
been admitted may then be given as long as the application likes; `rivet-mcpd`
raises its own to 900 s once a pinned key has been presented, and that number is
a different fact from this one.

### 4.2 Three refusals the shared parser makes that no hand-written one did

Sharing the code is what made these worth writing once:

1. **A repeated `Content-Length` is refused, whether or not the values
   agree.** Two that disagree are the smuggling shape. A server that takes one
   and a proxy in front that takes the other disagree about where this request
   ends and the next begins, and `rivet-proxyd` is a proxy in front of exactly
   these servers. Two that agree are refused as well, so no parser in the path
   ever has to decide which of two fields with one name is the real one. Every
   HTTP parser in this project applies the same rule, the ones that read a
   response as well as the ones that read a request (`.rulings/` R-210): a
   client that took the first of two lengths would be judging a body its
   server framed differently. RFC 9112 §6.3.
2. **`Content-Length` beside `Transfer-Encoding` is refused.** The classic
   smuggling shape. No caller in this project sends both, so refusing costs
   nothing.
3. **The head bound is checked on the head, not only while looking for the blank
   line.** Both hand-written parsers check the bound only in the
   not-yet-terminated case, which leaves the limit to be enforced by the size of
   whatever read buffer the loop happened to use — so a complete 64 KiB head
   delivered in one read passes a 16 KiB limit.

---

## 5. TLS: the files the platform handed the application

`Tls` names a certificate, a key and optionally an authority **by file name in
`RIVET_APP_DATA`**, and builds a `rustls::ServerConfig`.

**The files are per-machine and are deliberately not in the payload.** A
certificate names a machine and a payload is copied to every machine; putting
them in the artifact would put one machine's identity in every copy. They are
placed by provisioning or by `rivet apply install-secret`.

**A caller certificate is required or it is not asked for — there is no
*optional*.** `rustls` will happily build a server that requests a certificate
and then serves whoever declines, and an unauthenticated administrative surface
on a LAN is the failure this whole arrangement exists to avoid. An application
that authenticates some other way simply does not call
`requiring_a_caller_certificate_from`.

**Every refusal happens at startup**, before a socket is served on. A server
that accepted a connection and only then discovered its key was unreadable would
report the failure to whoever connected, which is the one party that must not be
told about it. That is why an authority file with no certificate in it is a
startup error rather than a server that refuses every caller.

---

## 6. The dependency on `rivet-tls`, which amends `ADR-036` §5

`ADR-036` §5 rules that **`rivet-sdk` depends on no Rivet crate**, and the reason
it gives is that an application is a foreign program: linking `rivet-core` would
drag an operating system into a program that wanted a temperature reading.

**The `serve` feature depends on `rivetos-tls`, and that rule is amended here
rather than quietly broken.**

The amendment is not new in kind. That manifest's own note about `libc` already
states the invariant precisely: **"no operating system in an application", not
"no crates at all"**. `rivet-tls` is `rustls` plus pinned-Ed25519 verification.
It depends on no other crate in this workspace, it contains nothing of the
operating system, and it is reached only by an application that asked to serve.
With no feature named, `rivet-sdk` still links `libc` and nothing of Rivet's.

**The alternative is worse than the edge, and it is not hypothetical.**
`rivet-tls` names the project's one `CryptoProvider`; `rivet-proxyd` wrote its
own `rivet_provider()` returning `ring::default_provider()`, with a comment
saying it matched the others. That is a second answer with a note attached
rather than one answer, and `ADR-011` is about exactly that. A fourth
independent construction inside the SDK would be the same mistake with wider
reach.

The version and feature list of `rustls` in `rivet-sdk` is the same as
`rivet-tls`'s, `rivet-fleetd`'s and `rivet-proxyd`'s — **no `tls12`**
(`.rulings/` R-140) — because Cargo unifies features across a build and a list
that disagrees puts the feature back with no single manifest looking wrong.

---

## 7. How a feature that nothing enables gets tested

**`draw`/`screen` already learned this lesson and it was only half learned.**
The `screen` feature was split out of `draw` on 2026-09-01 so that drawing could
be tested on a host with no cross-compiled `libdrm`, and the manifest note says
the split buys `cargo test -p rivetos-sdk --features draw` *on any machine* — a
command nothing in this repository ran. A feature nothing enables is code
compiled by nothing, which is the failure `tools/test-all.sh` exists to catch.

So `serve` is tested two ways, and both of them run:

1. **The module is `#[cfg(any(feature = "serve", test))]`.** Everything but the
   TLS half is pure Rust, so it compiles into this crate's own test binary with
   the feature off — which means `cargo test --workspace`, which leaves every
   optional feature off, runs every bound and every refusal. The TLS half is
   `#[cfg(feature = "serve")]` alone, because it needs `rustls`. This is the
   same device as `flip.rs`.
2. **`tools/test-all.sh` runs `cargo test -p rivetos-sdk --features draw,serve`**
   as a gate of its own, so the TLS half runs on every full pass — and `draw`'s
   tests, which had not been run by any command, now do too. `screen` is
   deliberately absent from that list: it links `libdrm`, which lives only in
   the aarch64 sysroot, so a test binary enabling it does not link on a host.

---

## 8. Which consumers retrofit

`.rulings/` R-143 says *as needed*, which is a judgement per consumer. This ADR
records the judgement and **changes none of them**; retrofitting is separate
work with its own risk.

| Consumer | Recommendation |
| :-- | :-- |
| **Trends** | **`Tls` and nothing else — and this row is a correction, 2026-09-05.** Trends is on `axum` over `axum-server`, exactly as Reliquary and TempMon are, so the same judgement applies to it: the pieces here do not replace a router. It adopted `Tls`, it served, and it reached none of the rest. §10 is the whole account and the History section has what this row said before. |
| `cmd/rivet-fleetd` | **Retrofit, and it is the cleanest of the three.** Its head parser, connection cap, deadlines and `browser_tls()` map onto `Head`, `Limit`, `Bounds` and `Tls::requiring_a_caller_certificate_from` almost line for line. Its 30 s becomes one `Bounds` field. What does **not** move is `drivable` — the cross-site check is a policy about an administrative endpoint, not a framing rule — though its cross-site half is `rivet_core::cross_site`, shared with `rivet-proxyd` (`ADR-043` §5.2); neither does `who`, the SHA-256 certificate fingerprint, which no other consumer wants. |
| `cmd/rivet-proxyd` | **Retrofit the certificate and the cap; leave the parsing.** Its `server_config` is `Tls` with no client auth and its missing connection cap is `Limit`, which is the gap worth closing. Its head reading is byte-at-a-time into a `String` it then rewrites and forwards verbatim, and `Head` does not model *the original bytes, minus hop-by-hop headers, with the request line rewritten*. Forcing it through this type would cost more than it saves. |
| `cmd/rivet-mcpd` | **Only the bulk endpoint fits.** It is a transport, not an HTTP server: its main path hands a caller the stdin and stdout of `rivet mcp`. `bulk::parse_head` and `bulk::accept` map onto `Head::parse` plus `read_body_into`, and its 1.2 GB is one `Bounds` field. Its `Slot`/`LIVE` pair becomes `Limit`, which also fixes a smaller thing: both existing caps are process-wide statics, so a second listener in one program would silently share the first one's budget. |
| **Reliquary**, **TempMon** | **Probably not.** Both are already on `axum`, which owns the accept loop, the parsing and the routing. For them this is a migration rather than an adoption, and the pieces here do not replace a router. What they could take without migrating is `Tls`, if the duplicated `ServerConfig` construction ever bites. |

---

## 9. What is built, and what is not

**Built and verified:**

* The module, its bounds, its refusals, its connection limit and its TLS
  builder. **37 tests of its own**, of which **30 run with the feature off** and
  the seven `Tls` ones need it. `cargo test -p rivetos-sdk` answers 85 with no
  feature named and 121 with `draw,serve`; `tools/test-all.sh` runs both and
  exits 0, with the root lint count level with `tools/lint-baseline`.
* **Each bound was broken and its test watched to fail** before the change was
  committed — `AGENTS.md` §4. Fourteen breaks, fourteen failures, and the tree
  green again after each.

**Not done, and stated plainly rather than implied:**

* **One consumer has adopted it, and it took one type of five.** Trends, on
  2026-09-05 — §10. Everything else in §8 is still a recommendation and not a
  change.
* **It has not run on a board.** It has served real requests on an emulated
  aarch64 guest under `rivet-initd` (§10.1), which `ADR-026` §4 is the table of
  the limits of. *"It works in QEMU"* is not *"it works"*.
* **The TLS half now reaches a real handshake, and only without client
  authentication.** A client that verified the chain completed one against a
  server built by `Tls::handed(...).server_config()`, and a boot with the
  certificate absent refused at startup and answered nothing (§10.5). What is
  still asserted about the configuration and not about a refused connection is
  the client-certificate half: **no test and no adopter has ever built
  `requiring_a_caller_certificate_from`** — the first application to want client
  authentication wanted a shape this module does not offer (§10.2).
* **Everything but `Tls` remains unexercised outside the suite.** `Head`,
  `Bounds`, `Limit`, `read_head` and `read_body_into` have no consumer at all,
  and the first adopter could not reach them.

---

## 10. The first adopter's report — Trends, 2026-09-05

**Written after contact, and marked as such so a later reader can tell which
half of this document was predicted and which was observed.** `PLAN.md` §130.1
put four questions to the first adopter and asked for them verbatim in its
brief; §10.2 to §10.5 are those four, in order. `PLAN.md` §130 is the argument
for why they were asked in advance: a new abstraction's design section is
written by its author against imagined use, and the feedback from the first real
consumer evaporates within a day unless somebody was asked beforehand to write
it down.

The port itself lives in the Trends repository and is not described here. What
is here is what it did to this module.

### 10.1 What was adopted, and what it was measured against

**One type of five.** The whole of the `serve` surface the Trends port names is:

```rust
rivet_sdk::serve::Tls::handed(TLS_CERT, TLS_KEY)?.server_config()?
```

`Bounds`, `Head`, `Limit`, `Slot`, `Refusal`, `read_head`, `read_body` and
`read_body_into` are not reached, and no `rustls` type is named anywhere in the
port — `rustls` appears in it only inside comments.

**The reason is structural and this document already knew it.** §1 records that
Reliquary and TempMon *"sit beside them on `axum`, and Trends is next"*. Trends
serves with `axum` over `axum-server`, which owns the accept loop, the head
parsing and the routing; there is nothing left for a head parser to do. §8's own
judgement for the two `axum` consumers — *"Probably not… What they could take
without migrating is `Tls`"* — is exactly what happened to the one consumer §8
told to adopt from day zero. That row is corrected above and the History section
keeps what it said.

**What it was measured against.** The Trends repository's `tools/qemu-proof.sh`
boots an aarch64 guest under `rivet-initd`, installs the daemon as a signed
application the supervisor starts, and then talks to it from outside the guest.
Two boots, because one is worthless — `AGENTS.md` §4:

| | |
| :-- | :-- |
| `GET /api/v1/health` over HTTPS | **200**, eight seconds after the guest was started |
| the same port in plaintext | refused — so the 200 is evidence about TLS |
| the certificate served | byte-identical by SHA-256 to the one placed in `RIVET_APP_DATA` |
| `GET /api/v1/streams`, no credential | **401** — the application's own middleware is in the path |
| `POST /api/v1/pair`, unknown code | **401**, which is a **read of the SQLite database** the daemon created and migrated on `/state` |
| **boot 2**, certificate absent | refuses **at startup**, names the file and the `install-secret` line, and answers nothing |

Boot 2 is what closes §9's third bullet. It also settles something a reader of
this ADR would get wrong: **the port stays open on a boot where the application
is dead**, because the supervisor owns the listening descriptor. A health check
that only opened a socket would call that service up.

### 10.2 Question 1 — what did it have to reach around?

**Zero direct `rustls` imports**, which is the answer this question was mostly
asking for and is a real result: the one thing an adopter most obviously has to
reach around is the crypto provider, and `Tls` removed the need.

Three things it went around the module for, in decreasing order of how much they
matter.

**(a) A caller certificate that is REQUESTED rather than REQUIRED. Not worked
around — recorded, and the work deferred with it.**

§5 rules that *"a caller certificate is required or it is not asked for — there
is no optional"*, and gives the argument: `rustls` will happily build a server
that requests one and serves whoever declines. That is right for
`cmd/rivet-fleetd`, whose only caller is an operator.

**The first application to want client authentication has two kinds of caller on
one port**, and cannot have that rule. Trends' operator surface is to sit behind
a certificate issued by the fleet authority while `/api/v1/` keeps its device
token, because an Android client cannot present one — the maintainer ruled it in
that project on 2026-09-02. A single TLS configuration cannot both require a
client certificate and let the phone through; the shape that works is
`WebPkiClientVerifier`'s allow-unauthenticated form plus middleware that refuses
the operator routes to a connection which presented none.

**Nothing in the port works around it.** The operator surface is the dashboard,
and the dashboard is out of scope for this port (`PLAN.md` §130.3), so what
shipped serves the phone's surface over TLS with no client authentication —
which is what that project's specification asks for today and is not a downgrade
of anything. Building the optional verifier by hand would have put a `rustls`
import in the adapter and made the gap invisible.

**So this is the module's shape being wrong for its first adopter, reported
rather than patched around.** §5's rule should not be relaxed into *optional*;
what it is missing is a second, differently-named way to say *requested, and my
routes decide* — a name that cannot be reached by accident.

**(b) ALPN, which the module builds a configuration without and offers no way to
set.**

`server_config()` hands back a finished `rustls::ServerConfig` with an empty
`alpn_protocols`. Trends' `axum` is built for `http1` alone and must say so, so
the port added a method to its own API crate — `TlsMaterial::adopt(ServerConfig)`
— whose entire body is *put this crate's ALPN list on a configuration somebody
else built*. That is a hand-rolled substitute for one line on `Tls`.

**And the finding is bigger than the adopter.** `grep -rn alpn` over `cmd/`,
`rivet-tls/`, `rivet-sdk/` and `rivet-remote/` finds **nothing**: none of the
three servers this module was extracted from advertises a protocol either. Every
HTTPS server in this project relies on clients that offer `h2` first falling back
by good manners. That is not something the Trends port introduced and it is
something its adoption found.

**(c) Which certificate did you load?**

`Tls` reads the certificate file, parses it, and gives back only a
`ServerConfig`. Trends must quote its leaf's `SubjectPublicKeyInfo` digest to a
pairing client, so its adapter opens `tls-cert` a **second** time with
`std::fs::read` and parses it a **second** time with a different parser.

Two reads of one file with a window between them is a small, real defect: rotate
the certificate inside that window and the daemon quotes a pin for a certificate
it is not serving. An accessor on `Tls` for what it loaded would remove the
second read, the second parser and the window.

### 10.3 Question 2 — which parameters did it override, and why each?

**None. `Bounds` was never constructed.**

`PLAN.md` §130.1 offered two readings of the answer — *every default taken*
means the defaults are right and the type may carry knobs nobody turns; *most
overridden* means the abstraction is in the wrong place. **The observed answer is
neither**, and it is more useful than either: the type is on the far side of a
boundary this adopter does not cross. §4's careful argument for each of the four
defaults, and §4.1's argument for the head deadline in particular, were not
tested by the first adopter in either direction.

**The consequence is concrete and it is bad.** `Bounds::connections` and `Limit`
are the piece of this module an `axum` consumer would most want, and **the Trends
daemon has no connection cap** — which is precisely the HIGH-rated defect the
audit of 2026-09-03 found in `cmd/rivet-mcpd` and which §1.1 gives as the
argument for building this module at all. The first adopter of the module built
because two servers independently grew the same bug has that bug.

**There is a seam and it is not free.** `axum_server::accept::Accept` is a
post-accept hook, so a `Limit` could be claimed in a custom acceptor wrapping
`RustlsAcceptor` — at the cost of a newtype that holds the `Slot` and delegates
`AsyncRead`/`AsyncWrite` to the real stream, and with a refusal that can only be
an `io::Error` the framework logs and drops rather than a 503 anybody sees.
Whether that belongs in `serve` is §10.6's first open question, and §3's *ships
the PIECES and never the loop* argues against it.

### 10.4 Question 3 — what did it have to know that the module did not tell it?

**The predicted source of that knowledge was `cmd/rivet-fleetd`'s source, and it
was not opened once.** For everything about `serve` itself, the module's own
documentation answered the question — including the two that would otherwise
have cost time: that `Bounds::apply_to` is for a blocking socket and an
asynchronous caller takes the `Duration`, and that the certificate files are
per-machine and deliberately not in the payload. **That is a good result and it
is recorded as one**, because the rest of this section is not.

Four things the port had to work out, and where each was found:

1. **The cross LINKER, which `tools/toolchain/env.sh` does not set.** An
   adopting workspace is outside this repository and inherits none of its
   `.cargo/config.toml`. `SPEC-01` §2.1 and the README both say so — and
   `examples/templates/template-network-service`, which is the artifact this
   repository tells an author to start from, **did not carry the setting.**
   Measured rather than inferred: that template copied to a directory outside
   the repository, with nothing changed but the SDK's path, compiles every crate
   for the target and then fails with `crt1.o: Relocations in generic ELF (EM:
   183)` — a message that names no linker and reads as a broken toolchain.
   **Fixed in the same change** as this section; the template now carries a
   `.cargo/config.toml`, and copied out again it links and produces an
   `ELF64 / AArch64` binary. The template built correctly inside the repository
   for the same reason `AGENTS.md` §10.5's last bullet gives: a green run in a
   tree you have worked in is not evidence about a clean checkout.

2. **`Socket::into_tcp_listener` hands back a BLOCKING listener**, and
   `tokio::net::TcpListener::from_std` neither accepts nor refuses one — its own
   documentation puts the duty on the caller. **Both SDK adopters are
   asynchronous and one of them is missing the line.** Reliquary's Rivet root
   writes `TcpListener::from_std(handed::listener(…)?)?` and never calls
   `set_nonblocking`; it has served on a board, because the symptom is a worker
   parked inside `accept` between the readiness notification and the connection
   — invisible at one connection at a time and a hang under load. Trends' port
   sets it, in its adapter, with a comment saying why. **Fixed in the same
   change**, in `rivet-sdk/src/listen.rs`'s documentation; whether it should be a
   named constructor instead is §10.6.

3. **The canonical rendered `manifest.toml`.** `tools/qemu/run.sh --app` and
   `tools/qemu/fixture-signer` both take a rendered manifest, and the only
   specification of its form — key order, which lists are always present, that
   `settings` comes after `devices` — is
   `rivet_core::package::application::Manifest::render`. `cargo rivet --help`
   lists the keys and the values each accepts and says nothing about order. This
   one genuinely was answered by reading source; it is simply not `rivet-fleetd`'s
   source. The port's proof guards against the hand-written copy drifting by
   asserting every line of it appears in the `.rvt` that `cargo rivet package`
   emitted, which a member being stored uncompressed makes possible.

4. **A setting's value must be a QUOTED string.** `ADR-038` §10.2, accepted
   unwillingly, and **both of Trends' settings are integers** — so every one of
   them has to be quoted. §4.4 is right that a line which cannot be carried is
   ignored rather than refusing the document, and the consequence for an adopter
   is that the failure mode is a daemon which starts, serves correctly, and runs
   on built-in defaults, with one journal record saying why. This proof was
   written with bare numbers first and caught exactly that.

### 10.5 Question 4 — what broke that the module's tests did not catch?

**Nothing in `serve` broke.** `Tls::handed(…).server_config()` was correct the
first time, cross-compiled to `aarch64-unknown-linux-musl` the first time, and
completed a real TLS handshake the first time it was asked to.

**That is a weak result and not a good one, and saying so is the point of asking
the question.** About a tenth of the module was exercised. Thirty-seven tests
were written for a parser, a bound and a counter that the first adopter never
called, and the seven that cover `Tls` are the only ones whose subject an
adopter touched. A module can pass its own suite and be untested by its first
consumer at the same time, and that is what happened here.

**What the adoption did close** is §9's third bullet, in the direction it can
close: a certificate built by this module now terminates a real connection from
a client that verified the chain, and its absence is a startup refusal that names
the file rather than a handshake error reported to whoever connected — which is
§5's central claim and was previously an assertion about a `ServerConfig`.

**What broke was around it, three times, and none of the three is this
module's:** the linker (§10.4 item 1); the settings quoting (§10.4 item 4); and a
harness defect worth naming because it reads exactly like a product one — the
guest console written straight onto a Docker bind mount lagged the guest by
several seconds, so the stage that reads the journal saw a log that stopped
during kernel memory setup and reported the daemon silent on a boot where every
request had already been answered.

**One thing that could have broken and did not:** `library = []`. `sqlx`'s
bundled SQLite and `ring` both compile their C into the binary, and `readelf`
reports the 11.25 MB result as `ELF64 / AArch64 / EXEC` with no `INTERP` segment
and no `NEEDED` entry — so the manifest's claim about the closure holds for an
application considerably heavier than anything this repository builds.

### 10.6 What follows, and which parts are the maintainer's

Two changes are made by this amendment, because both are documentation of a fact
the tree already had:

* `examples/templates/template-network-service` gains the `.cargo/config.toml`
  it needed, verified by building the template outside the repository;
* `rivet-sdk/src/listen.rs` says that the listener it hands over is blocking, and
  what an asynchronous caller owes.

Four are **not** made, because each is a judgement about this module's surface:

1. **Should `serve` reach an `axum` consumer at all?** §3's *ships the PIECES and
   never the loop* was written about two concurrency models and answered them
   both. It did not answer a third shape — a consumer whose framework owns the
   accept loop — and §8 now judges three of the five consumers to be that shape.
   An `axum_server::accept::Accept` implementation carrying a `Slot` would give
   them the connection cap and nothing else; it would also put a named framework
   in the SDK.
2. **A `requested, and my routes decide` client-certificate mode**, under a name
   that cannot be reached by accident — §10.2(a). Trends needs it for its
   operator surface and the dashboard work is where it will be needed.
3. **A way to state ALPN** on a `Tls`, or a sentence in §5 ruling that ALPN is
   the application's and this module's job stops at the certificate. Either is
   defensible; what is not is three servers in this repository advertising
   nothing with no decision behind it.
4. **An accessor for what `Tls` loaded**, so that an application needing the
   leaf's digest does not open and parse the file a second time.

---

## History

*Pruned: an entry earns its place by explaining why a decision changed
(`.rulings/` R-150).*

**2026-09-25 — §4.2 item 1 refused only a repeat that disagreed.** It read:

> ~~**Two `Content-Length` fields that disagree are refused.**~~

The SDK accepted two identical fields, `rivet-mcpd` refused any repeat, and
`rivet-proxyd` checked for none. So three parsers of one request applied three
rules. The ruling chose one, the strictest (`.rulings/` R-210).

**2026-09-05 — §8's row for Trends, replaced by its first adopter.** It read:

> ~~**Adopt from day zero.** It is the reason this was built before it.~~

It was written the day before Trends was ported and it was wrong about the one
consumer this module was sequenced ahead of. The reason it was wrong is worth
keeping: **§1 of this document already recorded that Trends is an `axum`
service**, in the same sentence that grouped Reliquary and TempMon — for whom §8
correctly judged *probably not*. The recommendation and the evidence for its
opposite were four pages apart in one file, and nothing reconciled them until an
adopter tried it. §10.1 is what actually happened.
