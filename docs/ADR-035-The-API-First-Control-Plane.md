# ADR-035: The API-First Control Plane

* **Status:** **Accepted.** This ADR is the authority on how a Rivet is administered: MCP over TLS on port 7777, mutually authenticated with pinned Ed25519 keys. There is no other channel and no unauthenticated path. Ruled 2026-08-29.
* **Date:** 2026-08-28
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Supersedes:** `ADR-005` §5.4.2's Talos rejection, `ADR-006`'s, `ADR-007` §2's comparison row, and `ADR-032` §4.1's transport decision. Each is struck in place and cites this.
* **Was:** `docs/design-candidates/CANDIDATE-01-An-API-First-Machine.md`, proposed the same day.

---

## 0. How this was ruled, and the one caveat on it

**Accepted by the maintainer in conversation on 2026-08-28.** He gave the
approval explicitly, scoping it to *the pivot as discussed in that conversation*
and stating that the conversation itself was the approval — so this document was
unblocked to proceed. *(`.rulings/` R-001 for the wording; it does not ship —
`ADR-029` §16.6.)*

**He was remote and had not read the document.** That is recorded rather than
smoothed over, because it is the material fact about this ruling: the approval
is of *the pivot as discussed*, and this document is one agent's rendering of
that discussion. The load-bearing contents were summarised back to him before
proceeding — MCP over TLS, `ADR-025`'s pinned keys with no CA, git over smart
HTTP rather than a hand-written server, a bulk endpoint separate from MCP, images over git
with unlimited retention, packing placed by RAM rather than ISA, the interactive
path removed last, MCP-over-serial as break-glass, and the explicit note that this
does not make the image pure Rust.

**If any section here turns out not to be what was meant, this section is how
that happened**, and the section in question is wrong rather than the approval.

Two decisions inside this ADR were left open by it and are **not** covered by
that approval: §7.2's wire format and §7.4's push question. §7.2 is taken below
as a recommendation adopted-until-overturned, because it blocks phase 1 and is a
wire format rather than a trust model. §7.4 stays deferred.

---

## 1. Context

**The maintainer's framing, 2026-08-28: Rivet is not a desktop operating system.
It is a network- and cloud-oriented device, AI-first, and free of UNIX legacy.**
*(`.rulings/` R-002.)*

With one carve-out, stated the same day: **infotainment stays** — `aura-renderer`
and anything like it. That is not a contradiction. `ADR-015` already makes such
a thing an *application*, so keeping a rendering surface does not reopen the
desktop question; it means the display path stays a supported concern and
"free of UNIX legacy" does not mean "headless".

**And the carve-out strengthens the architecture rather than excepting itself
from it** — Gemini's observation in review, and it is the right one. A kiosk is
not a desktop; it is an appliance that happens to have a screen. So the display
application is a supervised service (`ADR-017`) that takes the framebuffer, and
when it needs to show Wi-Fi state, an address, or an update badge it **speaks
MCP to the local control plane** rather than parsing `/sys` or shelling out to
`ip`. The same catalogue a remote agent uses, over a local socket.

That gives the GUI no privilege it does not need, no shell, and no second answer
to any question — which is `ADR-006` §3.1's co-equality of adapters arriving
somewhere nobody planned for it.

**This is less a pivot than a commitment.** `ADR-012` is already *"The Agent as
a First-Class Operator"*; `ADR-032` is *"Asking a Running Machine"*; `ADR-006`
§3.1 already makes every operation reachable from every adapter a conformance
property. `rivet mcp` exists and works — it was used to drive the board on
2026-08-28. What is new is dropping the *human interactive* path that was
hedging all of that, and putting the machine-facing transport on TLS.

### 1.1 What "free of UNIX legacy" means here, precisely

Worth stating before someone reads it too widely. Rivet keeps the Linux kernel,
cgroups, namespaces, `ext4`, `/proc`, `/sys`, POSIX processes and `execve`. The
legacy being retired is the **interactive and administrative** one:

* a general-purpose shell on the machine,
* a remote login as the administrative transport,
* users-as-logins as the identity model.

Nothing here proposes leaving the kernel ABI, and an amendment that reads that
way has been misread.

---

## 3. `rivet-mcpd` — the proposal

An MCP server listening on TLS, with identity managed at the fleet level,
replacing `ADR-032` §4.1's *stdio over a login session*.

### 3.1 What this costs, itemised by the ADR it overturns

`ADR-032` §4.1's argument for stdio over a login session was one sentence:

> No listening socket, no TLS, no identity system, no new attack surface.

Every one of those four savings is a cost this candidate chooses to take on.
That is the honest framing and it should not be softened: the pivot buys
legacy-freedom and pays in exactly the four things the existing ADR listed as
reasons not to.

### 3.2 MCP is the wrong shape for bulk transfer

A system update is **683,631,164 bytes** and the board has **926,680 kB** of
RAM. JSON-RPC means base64 in a string — roughly a third larger, and buffered.

So the control plane needs two endpoints on one listener:

* **MCP** for operations — the catalogue projection `ADR-006` §3.1 already
  requires;
* **a bulk endpoint** — an HTTP `PUT`/`GET` with a content digest — for payloads.

This is not a concession. It composes with §4 below, which needs an HTTP
surface anyway.

### 3.3 An application's backup and restore ride the bulk endpoint's listener

**The control plane is the control plane** (`.rulings/` R-234): a workstation
backs up and restores a machine's applications (`ADR-051` §8, §12) over this
listener and no other. Two routes, beside the bulk endpoint and in its shape —
HTTP/1.1 on 7777, one request per connection, the caller a pinned key:

```text
GET  /backup/<service>    HTTP/1.1
POST /restore/<service>   HTTP/1.1   Content-Length: <n>
```

`<service>` is one application's name, as `/run/rivet/backups` carries it and
as `rivet_core::service::ServiceName` accepts it; the routes are spelled once,
in `rivet_core::backup`, for both ends.

**The machine reaches the application, never the caller.** `rivet-mcpd` looks
the name up in the backup table, takes the loopback port `rivet-proxyd`'s
routing table gives that application's route, and dials it over TLS presenting
the machine's fleet certificate from the proxy's data directory, verifying the
application against the proxy's `upstream-name` — the connection the proxy
itself makes (`ADR-046` §4.2). It sends `GET /<backup-path>`, or
`POST /<restore-path>?confirm=true` carrying the caller's body with the declared
format's media type, and **streams the application's answer back verbatim** in
64 KiB pieces. A restore's body is relayed the same way as it arrives. Nothing
holds a whole payload: a Reliquary archive is tens of megabytes on a board with
under a gigabyte of memory.

**Refused before the application is dialled**, each by name: a connection with
no pinned key (`403`), the wrong method (`405`), a name that is not one service
or an application the table does not carry (`404`, naming what it does offer),
a feed or a restore to an application with no `restore-path` (`409`), a restore
with no `Content-Length`, a chunked one, an empty one or one over the bulk
ceiling (`411`, `400`, `413`), and an application the proxy does not route or a
machine without the proxy's certificate (`503`). An application that cannot be
reached is a `502`. **Every one carries `Rivet-Answered-By: rivet-mcpd`**, which
nothing relayed carries, so a caller tells the machine's refusal from the
application's answer.

**No digest travels on these routes**, unlike `PUT /payload/`. A payload there
lands in the machine's store under the name its digest gives it, so the digest
is the store's key and the transfer's check at once. Here nothing lands: a
backup is checked on the workstation by `ADR-051` §7's ladder against the
application's own `Content-Length`, and a restore is validated whole by the
application before it changes anything (`ADR-051` §12.7). The channel is the
control plane's authenticated TLS in both directions.

---

## 4. Git over the control plane: smart HTTP, not a second protocol

**Recommendation: do not implement a second remote-shell server in Rust.** It keeps the
protocol the pivot exists to leave, and adds a *second* crypto stack and a
*second* identity system to a design whose point is having one of each.

Git already specifies smart HTTP, and the server side is **the same two
programs the SSH-era `rivet-gitd` already exec'd**:

```
GET  /<repo>/info/refs?service=git-upload-pack
        → git upload-pack --stateless-rpc --advertise-refs

POST /<repo>/git-upload-pack
POST /<repo>/git-receive-pack
        → git {upload,receive}-pack --stateless-rpc
```

So git becomes an HTTP handler on the TLS listener — DONE 2026-08-30, and the
`rivet-gitd` crate was purged in the same commit — execing the
vendored git it already execs, authenticated by the same client certificate as
everything else. **Strictly less code than today**, one crypto stack, one
identity system, and the same shape as §3.2's bulk endpoint.

### 4.1 What is built, and how it is proven

**Built and proven on a booted guest, 2026-09-14.** `cmd/rivet-mcpd/src/git.rs`
fronts `git-http-backend` as a CGI on 7777, routed ahead of the bulk endpoint:
the caller's pinned key becomes `REMOTE_USER`, which is what lets
`receive-pack` take a push, and the three routes above are the only ones it
recognises. `rivet-sysd` creates each repository a document declares under
`[repos.<name>]`, on every boot. A fleet server's document declares
`[repos.fleet]` (`ADR-052` §9 step 4), and `cargo rivet fleet push <host>`
pushes the operator's fleet repository into it.

`tools/qemu/adr-035-git-proof.sh` is the proof. On one QEMU guest carrying the
vendored `git` and `git-http-backend` where a card puts them, it shows the
supervisor creating the declared repository; `cargo rivet fleet push` planning,
then pushing and reading `main` back; the container's stock `git` cloning the
repository over the same port, verifying the guest's certificate and pinned key
itself, with refs, tree and files matching; a second push fast-forwarding and a
fetch bringing it back; an identity not in `peers` refused by both clients; and
uncommitted work and a non-fast-forward push refused.

**A gzipped request body must be named to the CGI.** `git` compresses a fetch's
negotiation once it passes 1 KiB, and `git-http-backend` inflates only when
told through `HTTP_CONTENT_ENCODING`. `rivet-mcpd` passes the request's
`Content-Encoding` through for that reason; the proof's first run found a
fetch failing with `bad line length character` without it.

**A chunked request body is refused, not decoded** (`411`). `git` chunks a
body larger than `http.postBuffer`. `cargo rivet fleet push` never sends one:
it hands `git` a loopback relay that makes the pinned handshake itself and
forwards every body with a `Content-Length`. The relay exists because a stock
`git` can be given this connection's trust only by writing the operator's
private key as a PEM file, dialling the machine by the name in its self-signed
certificate, and adding `http.pinnedPubkey` on top of chain verification; the
proof does exactly that for its second client, with throwaway keys, and the tool
does not.

**What it leaves unproven**: a board, rather than a guest; `rivet-netd`'s
network path, rather than QEMU's forwarding; a card's ext4 `/state`; and a
repository of realistic size — `rivet-mcpd` holds a git answer whole in memory
before sending it.

### 4.2 The HTTP routes on the same listener, and fleet-as-code's git operations

`rivet-mcpd` decides an HTTP request's handler by its target, and serves five
families on 7777:

| routes | handler | for |
| :-- | :-- | :-- |
| `GET /backup/<service>`, `POST /restore/<service>` | `cmd/rivet-mcpd/src/backups.rs` | §3.3: an application's backup and restore, relayed over loopback. Decided first, because git's routes are suffixes a service name could end in |
| `/<repo>.git/info/refs`, `/git-upload-pack`, `/git-receive-pack` | `cmd/rivet-mcpd/src/git.rs` | §4's smart HTTP |
| `POST /fleet-as-code/head`, `/machines`, `/file`, `/commit` | `cmd/rivet-mcpd/src/fleet_as_code.rs` | `ADR-044` §5.5: a fleet server's desired-state repository, asked by `rivet-fleetd` |
| `PUT /repository-objects/<name>`, `GET /repository-objects/`, `POST /repository-objects/prune` | `cmd/rivet-mcpd/src/repository.rs` | `ADR-052` §7.4: a fleet server's repository store |
| `PUT /payload/<name>` | `cmd/rivet-mcpd/src/bulk.rs` | §3.2's bulk transfer into the machine's artifact store |

**Fleet-as-code's operations are git run as root on behalf of a confined
application.** `rivet-fleetd` is refused `/system/bin` and `/state/git` by its
Landlock policy (`ADR-042`) and asks this daemon instead (`.rulings/` R-187 D2).
Each operation runs git against `rivet_core::git::repository_dir(<name>)` for a
repository name the request's parser accepted — never `fleet`, `ADR-052`'s — and
reads or writes only `fleet/machines/<label>.toml`. A caller is a key in
`peers`; a connection with no key is refused. A commit is git's plumbing on the
bare repository and moves the branch with `update-ref` only if its tip is still
the head the caller read, refusing a stale head `409 stale-head`. The formats,
path rule and refusals are `rivet_core::fleet_as_code`, read by both ends.
`tools/qemu/adr-044-fleet-code-proof.sh` runs them on a guest, asked by a
confined `rivet-fleetd`.

---

## 4A. Following a log — the question the record said not to discover late

**Raised by Gemini, and it is already in the record twice.** `ADR-032` §4.3,
third bullet:

> **A decision about whether the machine may push.** MCP has notifications, and
> `--follow` proves the machine has something to say. Out of scope here; **noted
> so it is not discovered late.**

And `ADR-022` §9.5.1, which *reversed* an earlier ruling on 2026-08-27 to make
`--follow` an **input of the operation** rather than a property of the
transport — for the express reason that `ADR-006` §3.1's co-equality of adapters
had been broken:

> `--follow` exists in exactly one adapter and no other can see it. It is not in
> `CATALOGUE`, so `rivet mcp` cannot offer it and `rivet-fleet`'s terminal
> interface polls instead, with a comment saying why.

Status: **recorded, not built.** `PLAN.md` names the blocker plainly — *"MCP
cannot use a follow until `ADR-032` §4.3's notifications are ruled, and the
ruling's stated purpose was that MCP would gain it."*

### 4A.1 The pivot changes its severity, not its existence

A live tail was once one remote command away, so the gap was
an inconvenience with a workaround. **There is no such command now**, and a
control plane that cannot tail a log is one that cannot be debugged while it
misbehaves — which is exactly when a tail is wanted.

So this moves from *deferred* to *dated*: it must be answered before phase 5,
and its shape should be settled before phase 1 so the listener is designed for
it rather than retrofitted.

### 4A.2 Three answers, and they are not alternatives

| | What | Needs a ruling? |
| :-- | :-- | :-- |
| **a** | MCP **notifications** — the machine pushes | **Yes** — `ADR-032` §4.3's deferred question |
| **b** | A **cursor** on `get log` — the caller keeps asking | No |
| **c** | **SSE** on the HTTP listener §3.2 and §4 already require | No |

**Proposed: (b) as the floor, (c) for the live tail, (a) only if ruled.**

**(b) is the honest unblock.** `get log` already answers with `limit`,
`returned` and `held` — it knows how much it cut and out of what. An opaque
`since` cursor makes repeated calls a tail, in pure request/response, inside
MCP, today, with no protocol decision at all.

**And it must not be called `follow`.** `ADR-022` §9.5.1's amendment is that an
operation may declare an input meaning *keep answering* — the **machine** keeps
answering. A client that keeps asking is doing something else, and naming it
`follow` would be an adapter claiming a property it does not have. `PLAN.md`
already describes the correct shape and calls it the cheap half:

> the catalogue telling the truth, adapters refusing by name what they cannot
> stream — is ready to build on a word.

So: `follow` stays an operation input; adapters that cannot stream **refuse it
by name**; `since` is a separate, honestly-described input that any adapter can
satisfy. That restores `ADR-006` §3.1's co-equality — every adapter can answer
the question — without pretending polling is streaming.

**(c) is nearly free.** §3.2 needs an HTTP surface for bulk payload transfer and
§4 needs one for git. `GET /log?follow=1` as Server-Sent Events on the same
listener is a small amount of code on infrastructure that exists for two other
reasons, and it gives a real low-latency tail with `ADR-022` §3's JSONL going
over the wire byte-for-byte — which is what `cmd/rivet/src/follow.rs` already
guarantees on stdout.

**(a) stays deferred, with a deadline.** *May the machine push* is a genuine
design question — it implies subscription state, back-pressure, and what happens
to a notification nobody is listening for. It should not be answered as a side
effect of a transport change. But it may no longer be answered *never*: before
phase 5.

### 4A.3 What this does not settle

`ADR-022` §9.5.1's bounds still apply and none of the above changes them: a
follow is bounded in duration, record count and poll interval by
`rivet_core::journal`'s constants, *"because every adapter that grows a follow
must bound it the same way"*. An SSE stream is an adapter and inherits that.

---

## 5. Images over git — measured, 2026-08-28

The maintainer's observation was that git might be useful for moving images,
with the obvious hazard that history is append-only. **Every number below was
measured rather than argued.** The initial objection — that git would disable
delta above `core.bigFileThreshold` and therefore be a worse `POST` — was
**wrong**, and the measurements are why.

### 5.1 The payloads

Every `.rvt` is exactly **683,631,164 bytes**, because the system member is a
fixed-size 640 MiB partition dump. Two builds therefore differ in a handful of
binaries inside an otherwise identical image — the best case for delta, and the
worst case for storing copies. **Only ~55 MB of that 640 MiB is used**; the rest
is zeros, which turns out to dominate everything.

### 5.2 Two payloads, on the development host (git 2.55.0)

| | default threshold | `core.bigFileThreshold=1g` |
| :-- | --: | --: |
| after payload A | 46,738,449 | 46,737,515 |
| after payload B | 93,459,220 | 93,523,801 |
| **after `gc`** | **70,081,919** | **35,356,451** |

**One 683 MB payload is 46 MB in git** — 14.6×, from zlib on the zeros alone.
Raising the threshold past the payload size makes *two* payloads cost less than
*one* does at the default.

### 5.3 Six payloads, and the retention question

Threshold raised, each release an **orphan commit tagged `image/<version>`** so
that a tag is the only reference to its payload:

```
after 1 images (05bde7c):     43,440,870
after 2 images (3be953c):     86,888,095
after 3 images (38c70dc):    130,334,983
after 4 images (2c270c8):    174,285,229
after 5 images (33c89f9):    221,036,859
after 6 images (fc59f06):    267,806,121

server gc (13s):              37,240,901      ← six images
raw on disk would be:      4,101,786,984      ← 110×
```

**Then the finding that reframes retention.** Deleting the oldest tag and
pruning:

```
six images                    37,240,901
five images                   36,888,797
reclaimed                        352,104      ← 0.35 MB, not 683 MB
```

After `gc` the payloads are delta-compressed **against each other**, so removing
one frees only its unique delta. **Keeping releases is nearly free.**

*Honesty about the best case:* those six are builds days apart sharing almost
everything. Releases six months apart will share much less, so marginal cost
grows with divergence. 352 KB is the favourable end of the range, not the
middle.

### 5.4 Pruning works, and needs no reflog games

A bare repo does not keep reflogs (`core.logAllRefUpdates` unset). Delete the
tag, then:

```
git -c gc.reflogExpire=now -c gc.pruneExpire=now gc --prune=now
```

Measured on the development host (70,082,107 → 35,059,541, **0s**) and on the
board (the whole test repo returned to **40,116 bytes** with zero tags).

### 5.5 On the board — and the one hard limit found

Raspberry Pi 4, 926,680 kB total, ~874,604 kB available:

| Operation | Result |
| :-- | :-- |
| receive a 652 MiB blob (push) | **22s and 23s. Memory untouched** |
| serve a fetch | cheap |
| `gc`, default threshold | **41s, works.** Two payloads → 62,733,339 bytes (**21.8×**) |
| `gc`, `core.bigFileThreshold=1g` | **`pack-objects died of signal 9` — OOM-killed in 6 seconds** |

**A board can receive and serve. A board cannot pack at a raised threshold.**

---

## 6. Where packing happens — proposed policy

The maintainer's first formulation was *pack on a fleet server only if it is
x86, otherwise pack on the development device*. **The conclusion is right and
the discriminator is wrong: the constraint is RAM, not ISA.** Keyed on
architecture it fails both ways — it would refuse a 16 GB Pi 5 that would cope
easily, and permit a 1 GB x86 box that would die exactly as the Pi 4 did.

**Proposed instead:** the machine reads its own `MemAvailable` and the size of
the largest object, and **degrades rather than refuses**:

```
enough memory   →  raise the threshold   →  ~110×
not enough      →  leave it default      →  ~20×, and it works
```

This is `AGENTS.md` §5's shape — a reading, not an assumption — and it keeps a
Pi-class repo host *viable* rather than disqualified.

### 6.1 The trap to design around now

**Git auto-`gc`s.** `gc.auto` fires once enough loose objects accumulate, so a
repo host will pack **on its own, unprompted**. If `core.bigFileThreshold` has
been raised in that repo's config, that is an OOM at 3am with nobody watching —
and the 2026-08-28 measurement is exactly what it would look like.

**Proposed rule: the threshold belongs to the repo on the host, and the host's
value wins.** Never inherited from whoever pushes. A development machine may
raise it locally for its own packing; a Pi's copy stays default.

### 6.2 Retention

Space is not a reason to limit images: `/state` is 118 GiB on the board's
128 GB card, marginal cost is sub-megabyte, and the fleet server is expected to
have 1 TB. **Proposed default: unlimited.**

The knob should still exist, for **policy** rather than capacity — how far back
a fleet may roll, and the ability to revoke a release later found to be bad.
Fleet-level default with a per-device override, in the shape
`Device::service_memory` already established on 2026-08-28.

---

## 7. What must be ruled before code

### 7.1 The clock — ANSWERED IN 2026 by `ADR-025` §2, and it is not a hazard

**This was written as an open hazard and it is not one.** The concern: a board
boots at 1970, `rivet-timed`'s own journal says *"certificate expiry and
signature freshness cannot be trusted"* until it synchronises, so a
certificate-expiry check on the only way in makes an unreachable, headless,
unrecoverable machine.

**`ADR-025` §2 anticipated exactly this on 2026-08-22 and used it as an argument
against X.509 in the first place:**

> * **Expiry needs a clock**, and `ADR-021` establishes that this hardware has
>   no RTC and boots with a wrong one. Certificate validity windows are a poor
>   thing to depend on when time is itself unreliable at exactly the moment a
>   device is trying to connect.
> * **A CA is a thing to run**, back up and protect.
>
> This is a pinned-key trust model, and it is the right size.

Identity is an **Ed25519 keypair, pinned** — the fleet holds device-to-public-key,
the device holds the fleet's public key. **Trust is established by the pin, not
by a validity window**, so a wrong clock cannot lock anyone out. There is no CA,
and §— of that ADR notes the payoff directly: *"no CRL, no OCSP, no
distribution"*.

**What survives of this item is a rule rather than a ruling:** whatever wire
format is chosen in §7.2, it must not reintroduce expiry as a *trust condition*.
A self-signed certificate carrying a validity window that TLS libraries check is
precisely how this hazard would come back in through the door `ADR-025` closed.

### 7.2 The one identity question that IS open — `ADR-025` §2.1

`ADR-025` §2.1 deferred exactly one thing, and gave a reason that the pivot
retires:

> **The transport wrapper is deliberately deferred.** The dashboard does not
> exist. How a key is *presented* on the wire — raw, inside a self-signed
> certificate pinned by key so TLS libraries are satisfied, or as an […]

**The dashboard still does not exist, but `rivet-mcpd` does now**, so the
condition that justified deferring has fired. The choice is small and concrete:

* **raw Ed25519** over a hand-rolled handshake — no TLS library, no expiry, most
  control, most code;
* **a self-signed certificate pinned by public key** — TLS libraries are
  satisfied, the pin is what grants trust, and **expiry must be ignored or set
  absurdly far out** per §7.1;
* something else.

This is a *wire format* decision, not a PKI.

**TAKEN 2026-08-28: a self-signed certificate, pinned by public key.** Adopted
until overturned, and taken rather than deferred because it blocks phase 1 and
because it is a wire format rather than a trust model — `ADR-025` §2 already
ruled the part that matters.

The reasoning, and the constraint that comes with it:

* **TLS libraries want a certificate.** Presenting a raw Ed25519 key means a
  hand-rolled handshake, which is a second crypto surface to write, review and
  get wrong — precisely what §4 refuses a hand-written server for.
* **The pin is what grants trust, and nothing else may.** The certificate is a
  container. `ADR-025` §2's map of device-to-public-key is the authority, and a
  peer whose key is not in it is refused whatever its certificate says.
* **Expiry must not be a trust condition** (§7.1). Either the validity window is
  ignored on both sides, or it is set far enough out that a 1970 clock cannot
  fall outside it — and whichever is chosen, **the machine must say which**,
  because a check that looks real and is not is the failure this project keeps
  finding.
* **No chain, no intermediate, no CA.** A self-signed leaf is the whole
  certificate. If an implementation ever wants a chain, that is a change to
  `ADR-025` §2 and not a detail of this section.

**The implementation was surveyed 2026-08-28 and is `rustls` with the `ring`
provider** — `ADR-009` §3.7 carries the measurements. This section ruled a wire
format and named no library on purpose; that survey is where the library was
chosen, and it turned on something neither document anticipated: the best
candidate on every other axis, `graviola`, **requires ARMv8 crypto extensions
the Raspberry Pi 4 does not have**, and compiles for the target perfectly
regardless. `ADR-009` §5 gained a gate as a result — a dependency shipping
CPU-specific code must be *run* on the target CPU before adoption.

The design in this section was proved end to end on the board in that survey: a
TLS 1.3 handshake between a server holding an Ed25519 self-signed leaf and a
client trusting it by pin alone, refusing any chain.

### 7.3 Where fleet TLS identity lives — smaller than this document first claimed

**Withdrawn as written.** This section called it *"the substantial new subsystem
and the real cost of the pivot"*. It is not, because two ADRs already did the
hard parts:

* **`ADR-025` §2** rules the trust model — pinned Ed25519, no CA, no CRL, no
  OCSP. Revocation is *removing a row from the fleet's map*, not a distribution
  problem.
* **`ADR-019` §2** rules enrollment — **a machine arrives enrolled.** No join
  protocol, no discovery, no trust-on-first-use, no bootstrap race, and
  *"credentials never traverse a network to enroll — they are written to a card
  by the operator's own tooling"*.

So a device keypair is written to the card at build time exactly as the
provisioning document already is, and the register gains a public
key per device — the same shape as the `service_memory` map added on 2026-08-28.

**What remains is real but ordinary:** key generation at card-build time,
carrying the fleet's public key in the image's trust surface, rotation, and the
`ADR-020` question of how the device's private key is held at rest. That is a
work item, not a subsystem.
### 7.4 May the machine push? — `ADR-032` §4.3, now dated

§4A. Deferred since it was written, and the pivot gave it a deadline: **before
the interactive path went**, because nothing else falls back to for a live
tail. The shape should be settled before phase 1 so the listener is designed for
it. §4A.2 proposes an unblock that needs no ruling at all, which is why this is
third rather than first.


---


## 8A. Break-glass — how you get in when the control plane is the thing that broke

**Contributed by Gemini in review, and it is the best addition to this
document.** The migration kept an interactive login as the safety line until
the end — and said nothing about afterwards. "Rewrite the card" is not
a recovery story for a headless machine in another room.

Three tiers, and two of them already exist:

### 8A.1 Tier 1 — MCP over the serial console

`ADR-002` §2.2 and every `cmdline.txt` this project writes end
`console=serial0,115200 console=tty1`, and PID 1 already speaks on
`/dev/console`. **The same MCP the network exposes can be spoken over that
serial line**, with a physical cable and no network stack, no DHCP, no TLS and
no identity — because physical presence is the credential.

That is a strictly better break-glass than a remote login ever was: it survives a broken
`rivet-netd`, a wrong address, a firewall, and a `rivet-mcpd` that cannot bind.
It is also the same protocol and the same catalogue, so nothing new has to be
learned or maintained.

**It should be built in phase 1, not phase 5.** It is the thing that makes
removing the interactive path safe rather than brave.

#### 8A.1.1 RULED 2026-08-29: serial break-glass does not gate phase 5

**Moved here from `PLAN.md` §93.1 on 2026-08-29, unaltered.** Only the heading
number changed. **Bare `§N` references in the body are `PLAN.md` sections**;
`§92.4` is the hand-off entry this withdraws, and unprefixed `§8A.x` are this
ADR's own.

**It corrects §8A.1 above rather than replacing it.** *"It should be built in
phase 1, not phase 5"* stands. What is withdrawn is the reading that its
**verification** — which needs a cable — gates anything.


§92.4 listed the serial cable as *"should exist before phase 5 lands"*, which
framed a piece of **hardware** as a gate on a piece of **software**. The
maintainer overturned it: **the blast radius on the development board is writing
a new SD card, so the serial port can be set up and left unverified until the
hardware arrives — and nothing is blocked in the meantime.** *(`.rulings/`
R-003.)*

He is right, and `ADR-035` §8A already said so plainly enough that §92 should
not have needed telling:

* **Tier 2 — the FAT boot partition — is the real break-glass, and it already
  works.** §8A.2 was *named* on 2026-08-28 and §78 demonstrated the mechanism
  rather than assuming it. It needs no working software on the machine and no
  Linux on the rescuing host. A headless board is already recoverable today.
* **Tier 3 is rewriting the card**, always available, always needs hands — and
  that is the whole blast radius on a *development* board, which is what makes
  this cheap to be wrong about.
* **Tier 1 — MCP over the serial console — is buildable now.** §8A.1 says it
  *"should be built in phase 1, not phase 5"*, and the console half is already
  on: the live board's `/proc/cmdline` carries `console=ttyS0,115200
  console=tty1`, read off it directly on 2026-08-29.

**The correction is therefore: build it, defer its verification.** Serial-MCP is
ordinary software work whose *proof* waits on a cable due about 2026-09-01. It
is not a precondition of anything, and §92.4's entry is withdrawn.

The general error is worth keeping, because it is the kind an agent repeats:
**an unverifiable thing is not a blocked thing.** Verification pending hardware
is a stated gap in the report, not a stop.



### 8A.2 Tier 2 — the FAT boot partition

`ADR-002` §4.1 already makes the boot partition the file-transfer and
provisioning channel, and `AGENTS.md` §10.5 already records that the development
host is Windows with no ext4 access, *which is why* that channel is FAT. So a
card can be pulled, edited from any machine on earth, and returned — without
rewriting it.

*(**Cited as `ADR-010` section 10.5 until 2026-08-29**, here and once below.
`ADR-010` is licensing and distribution and has no §10 at all; the Windows host
and its lack of `ext4` are `AGENTS.md` §10.5, *Known environment friction*. The
same wrong citation was in `PLAN.md` §78 and §79 and is corrected there too.
Found by `tools/docs/claims.py`.)*

This already works and needs nothing built. It should be *named* in the record
as recovery rather than left as an implementation detail of provisioning.

**Named, 2026-08-28.** `PLAN.md` §78 built the provisioning path and it uses
exactly this channel, so the mechanism is now demonstrated rather than assumed:

> **Tier 2 is the boot partition, and it is a RECOVERY channel.** A card whose
> machine will not come up can be pulled, put in any reader on any operating
> system, and edited — `rivet.toml` rewritten, a trust file corrected,
> a `device.seed` and `device.crt` replaced — then returned. The machine picks
> the change up on its next boot because PID 1 reads that partition before
> anything else runs (`ADR-004` §6, `ADR-019` §3.1).

Two properties make it a *break-glass* and not merely a provisioning step, and
both were measured while building §78:

* **It needs no working software on the machine.** `rivet-mcpd`,
  `rivet-netd` and the supervisor can all be broken or absent; the firmware
  reads the FAT partition and PID 1 reads it again, and neither consults
  anything on `/state` or `/system` to do so.
* **It needs no Linux.** The partition is FAT precisely because `AGENTS.md` §10.5
  makes the development host Windows, so the editing machine needs nothing
  installed. `/state` — where a machine's identity, journal and repositories
  live — is `ext4` and is **not** reachable this way, which is the honest
  limit: this tier recovers a machine, it does not recover its data.

### 8A.3 Tier 3 — rewrite the card

Always available, always last, always needs hands.


---

## 9. Dropping `nushell` — cheap, with one precondition

`nu` is **44,522,208 bytes** — by a wide margin the largest thing on the card;
`rivet-sysd` is 1,086,824. Dropping it is a ~44 MB win on a 640 MiB slot and
removes a large audit surface. The ruling that makes `nu` optional **already requires the
no-nu path to work**, so it is supported and tested today.

**The precondition is diagnosis.** Every finding taken from the board on
2026-08-28 came from `open --raw` against `/proc/cmdline`, `/proc/self/mounts`,
`/sys/class/block/*/uevent`, `/sys/fs/cgroup/...` and the raw journal file.
Without a shell a machine answers only what the CLI has a verb for — which is
`ADR-006`'s principle taken to its conclusion, and correct — but **the read
surface must grow before `nu` goes**, or the board becomes undebuggable.

This is the one item in this document that is independent of everything else:
it needs no TLS, no `mcpd`, and no new subsystem.

---

## 9B. Half the `/etc` projection dies with the login path — and the MOTD relocates

**Raised by the maintainer, 2026-08-28:** removing the login path and `nushell`
makes the MOTD work obsolete (`.rulings/` R-283). It does, and it is broader than
the MOTD.
`ADR-007` §3.5's generated `/etc` exists in large part **for a program that logs
somebody in**, and each file's own `why` string — read off a running machine —
sorts them without any judgement being needed:

| File | Its own reason | Now |
| :-- | :-- | :-- |
| `/etc/motd` | *"a machine says what it is when somebody logs in, and the login program prints this above whichever shell the account uses"* | **dead** |
| `/etc/shells` | *"absent, musl substitutes `/bin/sh` and `/bin/csh`"* — the rule being that a login program rejects an account whose shell is not listed | **dead** |
| `/etc/shadow` | *"says positively that no account on this machine has a password"* | **dead** — nothing authenticates a password |
| `/etc/passwd` | *"uid 0 owns every file in the image and `getpwnam("root")` must answer; and an application's own uid must have a name, or `ls` prints a number and `SO_PEERCRED` gives a listening service an unnameable caller"* | **stays** |
| `/etc/group` | *"gid 5 is already load-bearing: `stage0_plan` mounts devpts with `gid=5`, and every installed application runs under a group of its own"* | **stays** |
| `/etc/hosts` | *"absent, localhost is resolved over DNS — musl has no builtin"* | **stays, and matters MORE** |

**Three of six go.** That is a real simplification and it should be *taken* in
phase 4 rather than left as output nothing reads — a projection that renders a
file no program opens is exactly the kind of thing that survives for years
because removing it feels risky and keeping it feels free.

**`/etc/hosts` is the one that gains importance**, and `PLAN.md` §71 is why:
musl has no built-in `localhost`, the loopback interface was found **down** on
2026-08-28, and `ADR-035` §1's display application resolving `localhost` to
reach the local control plane depends on both being right.

### 9B.1 The MOTD is gone, and nothing replaced it

The original rule was *"a machine says what it is when somebody logs in"*.
Nobody logs in any more — SSH and the login path are gone — and `[greeting]`
went with the file it configured, removed outright rather than migrated: the
maintainer judged the feature dead rather than worth relocating, and ruled
that no accommodation is owed to a document that does not exist
(`.rulings/` R-168.1, 2026-09-13).

**No compatibility shim, no schema version bump.** A document carrying a
`[greeting]` table is refused by the ordinary unknown-key refusal, exactly
like any other table this schema does not declare — `rivet-config::parse`'s
tests prove it. Nothing in `rivet-fleet`'s register, its compose step or its
diff carries a `greeting` field or path any more either.

---

## 9A. One claim to resist: this does not make the image pure Rust

Removing the login path removes a C component and shrinks the attack surface
considerably — that much is true and is a real prize. **It does not leave an
all-Rust image**, and the record should say so before the claim takes hold:

* **`git` stays and is C.** §4 keeps it deliberately — smart HTTP execs the same
  `upload-pack` and `receive-pack` the retired `rivet-gitd` exec'd, and
  re-implementing git in Rust is not on any roadmap here.
* `wpa_supplicant` is Tier C by `ADR-001`'s own vocabulary table, which uses it
  as the example of a Tier C choice being *right*.

So the honest claim is **"no interactive shell and no remote-login daemon"**, not "no C".
`ADR-001` §4's tiers are unaffected by this candidate and none of them changes.

---

## 10. What this does not settle

* **The wire never deltas.** Every push transferred a full ~43 MB pack; all the
  compression appeared at `gc`. Per-update bandwidth is ~43 MB, not ~350 KB —
  still 16× better than 683 MB, but not what §5.3 implies. Possibly a
  pack-objects setting; unchased.
* **One OOM and one success.** §5.5 has a single failure point (652 MiB blob,
  874 MB available) and a single success. That is not enough to write a memory
  multiplier into an ADR. Two or three more points first.
* **`gc` timing on ARM** was measured once, at the default threshold, on two
  payloads (41s). Six payloads on a Pi is unmeasured.
* **Git over smart HTTP (§4) on a board.** §4.1 records it proven on a QEMU
  guest; a board, its network path and a repository of realistic size are not
  measured.

---

## 11. Why this is worth doing

Beyond the maintainer's framing, the measurements found an upside neither side
of the conversation started with: **a fleet server can hold versioned, signed,
content-addressed releases and serve them to members at roughly the cost of
their differences.** 110× on six images, with pruning that genuinely reclaims.
That is a distribution mechanism, not just a transport — and it arrives mostly
free with a decision already being taken for other reasons.

---

### 8A.1.2 RULED 2026-08-29: pid 1 hosts the serial break-glass

§8A.1 left open whether Tier 1 is hosted by pid 1 or by `rivet-mcpd`. **Ruled:
pid 1.** *(`.rulings/` R-112.)*

**`rivet-mcpd` is supervised, and break-glass exists for the case where the
supervisor is dead.** An MCP server that the supervisor starts cannot answer
when the supervisor is what failed — which is the single scenario Tier 1 is
for, so hosting it there answers a different question than the one asked. A
third small binary started by pid 1 was considered and loses to the same test
one step later: started by pid 1 is not the same as *surviving anything pid 1
survives*, and it adds a component to the boot path to avoid adding code to one.

**The cost is real and is code in the process that must never die.** §8A.1's own
framing applies: pid 1's failure mode is the machine, so what goes in it must be
small, must not allocate unboundedly, and must not be able to block. That
constrains the implementation rather than the decision — a line-oriented reader
over the console with a fixed buffer, refusing anything it cannot serve.

**This is the one with a deadline.** The interactive path was removed on
2026-08-29, so Tier 1 is no longer *the third safety net*; Tiers 2 and 3 — the
FAT boot partition and rewriting the card — are what remains, and both need
hands on hardware. §13.2 records that the maintainer accepted exactly that for a
board on his desk, **and that the acceptance does not transfer to a machine in
another building.** This section is what closes that gap.

#### 8A.1.2.1 BUILT 2026-08-30

`cmd/rivet-initd/src/breakglass.rs` frames the lines,
`breakglass_board.rs` is the `Board` pid 1 presents, and `crate::pid1`'s tick
serves at most one line per pass — after the reaping, because a dead supervisor
must be reaped whatever anybody is typing, and before the confirmation, so a
console asking for a restart cannot race a trial boot being committed.

**The three constraints are answered by construction rather than by care.**
Cannot block: `Kernel::read_console` asks `poll(2)` with a zero timeout, and
does **not** set `O_NONBLOCK` on descriptor 0 — that would be a process-wide
change to a descriptor `say` also writes through and everything pid 1 spawns
inherits. Cannot allocate unboundedly: the buffer is `[u8; 2048]` in the struct,
and a longer line is discarded and refused rather than truncated, because a
shortened command is a different command. Small: the board implements
thirty-seven methods and **answers six**.

**What it serves** is the 2am repair and nothing wider: read the document, edit
it, say which slot is running, arm a trial boot, restart, power off. Every one
is something pid 1 already does; nothing was added to pid 1 to make break-glass
possible.

**`get status` is refused, and that is the design.** The supervisor answers it,
over a socket, and Tier 1 exists for the case where the supervisor is what
failed — so an answer from pid 1 would either lie or hang.

##### The departure: words, not JSON

§8A.1 says *"the same MCP the network exposes can be spoken over that serial
line"*. **This serves the word form instead** — the same words that follow `--`
in `cargo rivet run`, which is what `rivet_core::operation::perform` takes
natively; MCP's arguments object is a wrapper the control plane adds and then
unwraps.

It is the same catalogue and the same verbs, so §8A.1's *"nothing new has to be
learned"* holds. What decided it is who uses this: hand-writing
`{"jsonrpc":"2.0","id":1,"method":"tools/call",…}` on a serial console at 2am,
with no shell history and no editor, is a worse interface than the failure it
recovers from. A JSON line is **refused with a sentence naming the word form**
rather than half-parsed — `ADR-005` §5.4.1's partial promise.

##### `Origin::Console`, added for this

`rivet_core::deploy::Origin` had no way to say *somebody was physically at the
machine*, and `Origin::NoSession`'s own text says it does **not** mean that —
*"a script, or any parent that cleared its environment, looks identical from
here"*. So a variant was added rather than one reused: a cleared environment and
a person with a cable are different facts about who changed a machine.

##### Corrected 2026-08-30: it was listening to the wrong console

The first implementation read **descriptor 0**, which is `/dev/console`. The
board was then asked what that is:

```text
tty1                 -WU (EC  p  )    4:1
ttyS0                -W- (E   p a)    4:64
```

`tty1` carries `C`, so `/dev/console` is **HDMI**. Break-glass was listening to
a USB keyboard, and somebody arriving with a serial cable — which is what §8A.1
describes — would have found nothing.

**The security premise survived and the description did not.** A keyboard at the
machine is physical presence exactly as a cable is, so nothing was less safe
than it claimed. But *"MCP over the serial console"* was not what had been
built.

It now **resolves the line from `/proc/consoles`** and opens it by name, taking
the first console that is not a virtual terminal. Resolved rather than
hard-coded because `crate::boot::cmdline_txt` writes `console=serial0`, a
device-tree alias the kernel resolves — which is what keeps that line correct
whether the port is the PL011 or the mini-UART, and hard-coding `/dev/ttyS0`
would have thrown that away.

**A machine with no serial console says so once and carries on.** Break-glass is
then unavailable and §8A's other two tiers are what remain, which is a worse
position and not a fault.

**This also frees the console layout.** Had break-glass gone on reading fd 0, a
boot that dropped `console=tty1` — which a splash screen wants — would have
silently MOVED the recovery channel from HDMI to serial depending on boot mode.
A recovery channel that is somewhere different on a trial boot than an ordinary
one is worse than one that is merely in the wrong place.

##### PARKED 2026-08-30 — officially, and it blocks nothing

The maintainer was close to dropping the serial work altogether: a machine in
that much trouble could have its card pulled and its data copied off by hand, so
the serial path looked like a red herring and a poor use of the time. He then
made the parking formal in the same conversation. The serial cable work is
parked; it blocks nothing; the cost of its absence is known and has workarounds;
it may well be returned to, because it has a genuine use, but not now.
*(`.rulings/` R-284.)*

**Accepted, and §13.2 already said it.** That section records the maintainer
accepting Tiers 2 and 3 — hands on hardware — **for a board on his desk**, and
records that the acceptance *"does not transfer to a machine in another
building."* There is one board and it is on the desk. Every failure of
2026-08-30 was diagnosed over the control plane or from an HDMI capture; none
needed this.

**What is deferred is the PROOF, not the code.** The reader, the dispatcher and
the board are built and carry 29 tests between them, so leaving them costs
nothing per boot. What is not being spent is the hardware time: a cable, a
client, and a boot to watch.

**It blocks nothing, and that is the operative half.** No other work waits on
it: the control plane is the administration surface, the journal carries pid 1
since `PLAN.md` §102.17, and Tiers 2 and 3 are the accepted fallback. Anything
that begins to depend on Tier 1 should unpark it rather than assume it.

**What would end the parking** is the thing §8A.1.2 named in the first place —
the first Rivet somewhere its maintainer cannot walk to. Until then this section
is a design that exists and has not been exercised, which is a fair state for it
to be in and is written here so nobody later reads the silence as an oversight.

##### Still NOT proven

**None of this has run on a board or in the emulator**, and the correction
above narrows what would count: a USB keyboard at the HDMI console is no longer
the interface, so proving it needs **a serial cable and a client program**. It
was never a thing to hand-type — `cargo rivet run`'s word form exists so a
machine can drive the line, and the maintainer declined to type commands by hand
on 2026-08-30, correctly. Nineteen tests drive the
reader and the dispatcher through a fake kernel — a line typed one byte at a
time, `
`, two lines in one read, an overlong paste discarded and the reader
recovering, an unreadable console, invalid UTF-8, and every refusal — and a fake
console is not a UART. What hardware would settle: whether `poll(2)` on
descriptor 0 behaves as assumed when that descriptor is a real serial console
rather than a pipe, and whether the answer is legible at 115200 baud beside the
supervisor's own output on the same line.

---

## 12. Who may hold a connection, and for how long

A Rivet is reached through the control plane and has no second way in, so the
control plane is also what a hostile host on the LAN reaches first. Three rules keep one host from locking everybody
else out.

| Rule | Value | Why this value |
| :-- | :-- | :-- |
| Connections a listener holds | 16 | Several times one operator session, a fleet page and a clone, and a thread and descriptor cost a Pi can carry. |
| Of those, one source address | 12 | The rescue guarantee needs only a small reservation: 4 slots are always free for another address, so an operator can reach a machine one host is flooding. On a home LAN the busy client is usually one workstation doing several things at once (an install, git, the fleet page, a session), and it must not throttle itself. No address is privileged; an address vouches for nothing (`ADR-025` §2). |
| Time to finish the handshake | 3 s per read | Before the handshake completes the peer is nobody. Three seconds is several times a mutual-TLS handshake across a LAN, and a peer that opens sockets and says nothing gives each slot back quickly. After the handshake the idle allowance is 900 s, because a caller holding a pinned key thinking between two calls is not an attack. |

A flood of new connections is a rate limit's job, not the slot cap's:
`rivet-mcpd` also holds each source to a token bucket and collapses its refusals
to one journal line per source per minute, so a flood can neither take the slots
nor roll the journal. **The slot rule is shared code**
(`rivet_core::connection_slots`): `rivet-fleetd`'s listener applies the same 16
and 12, and `rivet-proxyd` the whole governor with a pool of 64, because it
fronts every application rather than serving operators (`ADR-046` §10).
`rivet-smbd`'s caps are numbers of their own, decided separately.

What this does not stop: a flood from many addresses on the same segment gets a
share per address, up to the listener's 16. That raises the bar from one host to
control of many addresses on the LAN, which is all `ADR-025` §2's trust model
claims.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §10 — "nothing here has been implemented" outlived the implementation

**Narrowed 2026-09-14** (`.rulings/` R-184). §10 closed with a bullet written
on the day this ADR was proposed:

~~**Nothing here has been implemented.** No `rivet-mcpd` exists. The git
numbers are real; the control plane is a proposal.~~

`rivet-mcpd` was built within days, §4's git handler on 2026-08-28/29, and §4
itself recorded it done on 2026-08-30 — but §10 was never revisited, and
`ADR-052` §8.1 and `.rulings/` R-182 repeated the claim that git was not
served. The phase 3 design found the handler in the tree, and
`tools/qemu/adr-035-git-proof.sh` proved it on a booted guest (§4.1), finding
and fixing one gap on the way: a gzipped fetch body. The bullet is replaced by
the one part of §4 still unmeasured.

### In §9B.1 — the greeting was going to move into `initialize`, then was removed instead

**Superseded 2026-09-13, `.rulings/` R-168.1.** The section originally argued
that nobody logs in any more, so the greeting belonged in the MCP
`initialize` response's `instructions` field instead of `/etc/motd`:

~~**Nobody logs in any more; things connect.** So the greeting belongs in the
MCP `initialize` response, which already carries `serverInfo` and has an
`instructions` field for exactly this — a human- and agent-readable statement
of what this server is. The fleet still sets it, `ADR-034` §6 still owns it,
and the first thing any client sees is still the machine saying what it is.
The rule is therefore amended rather than retired: it stands, the word "logs
in" becomes "connects", and `/etc/motd` stops being where it lands.~~

That migration was never built. When the maintainer reviewed the deployed
portal he judged the feature dead outright, long overtaken by SSH's own
removal, rather than worth relocating, and `[greeting]` was removed from the
schema, not moved (`.rulings/` R-168, R-168.1).

### In §7.1 — the clock was an open hazard

**It is not one, and `ADR-025` §2 had anticipated it on 2026-08-22.** The
superseded heading read:

~~The clock, and it is the dangerous one~~

### In §7.3 — fleet TLS identity was the real cost of the pivot

**Withdrawn as written.** The superseded heading read:

~~Where fleet TLS identity lives~~

Two ADRs had already done the hard parts, so the section is smaller than this
document first claimed.

### In §9.2 — the Windows-host fact was cited to the wrong document

**Corrected 2026-08-29**, here and once below. The superseded citation was
~~`ADR-010` §10.5~~; `ADR-010` is licensing and distribution and has no §10 at
all. Found by `tools/docs/claims.py`.

