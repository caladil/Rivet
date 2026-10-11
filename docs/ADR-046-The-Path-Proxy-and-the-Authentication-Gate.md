# ADR-046: The Path Proxy and the Single Authentication Gate

* **Status:** Accepted, and **running on a board** — §9.3. The gate of §4, the
  column of §4.3, the index of §6 and the port of §7 are all code with tests
  behind them; §9 says what is proved, what remains, and what no machine has
  done.
* **Superseded in part:** §4.3's unit of declaration is a path prefix rather
  than an application — `ADR-048`. The gate itself, the table, the index and the
  ports are unchanged by it.
* **Extended by:** `ADR-061` §7 — a `plain:` route opens port 80 on the machine
  that declares one (§7), and an `app:` route is one the application
  authenticates itself (§4.4).
* **Date:** 2026-09-05
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Ruled:** `.rulings/` **R-133** (a path proxy, and one name and one
  certificate) and **R-152** (the proxy is the gate, answering R-133's open
  second question) and **R-186** D1 (the proxy refuses a write another site
  caused, §4.5) and **R-187** D1 (the operator is a certificate naming no host,
  §4.1) and **R-188** (a proxied application's port listens on loopback only,
  §7.1) and **R-228**/**R-230** (passkeys beside client certificates, §4.6,
  decided in `ADR-063` §4.3)
* **Code:** `cmd/rivet-proxyd/src/main.rs` (the gate, the index),
  `cmd/rivet-proxyd/src/passkey.rs` and `rivet-webauthn` (§4.6),
  `rivet-tls/src/caller.rs` (§4.1's operator),
  `rivet-core/src/proxy.rs` (the table), `rivet-core/src/cross_site.rs` (§4.5's
  rule), `rivet-core/src/package/application.rs`
  (`proxy-auth`, and §7.1's `:direct` and `Socket::bind_address`),
  `cmd/rivet-sysd/src/runtime.rs` (`publish_routes`, which
  derives it), `cmd/rivet-sysd/src/kernel.rs` (`open_listeners`, which binds
  §7.1's address), `rivet-sdk/src/serve/tls.rs` (what a backend uses to verify a
  caller, and the proxy's own `asking_for` constructor)
* **Answers:** *"how does one machine serve several applications, over one name,
  to one operator, without each of them inventing its own front door?"*

---

## 1. Context

A Rivet runs several applications that speak HTTP — today `aura-controller`,
`tempmon-rivet`, `reliquary-rivet` and `rivet-fleetd` among them, with Trends
waiting. Each has a port. Without something in front, reaching them means
knowing every port number, and securing them means one implementation of the
same certificate check per application.

**R-133 settled the addressing:** the maintainer asked whether a path-based
proxy eliminates the DNS headaches, and decided that the proxy is a path proxy
only, a position held for now. *(`.rulings/` R-133.)*

So the machine has **one name and one certificate**, and an application that
asked for one is reached at `https://<host>/<path>/`. It deliberately left the
authentication question open, and named it as the expensive one.

**R-152 closes it**, because the maintainer stated what he wants of the surface
— *his own applications, Trends, Aura and Fleet all on SSL and authenticating
against his operator certificate; TempMon on SSL and unauthenticated* — and that want turned
out to be structurally exclusive with path routing. §3 is why.

---

## 2. The shape

    https://rivet-01.local/tempmon/…   →  tempmon-rivet   127.0.0.1:8081
    https://rivet-01.local/aura/…      →  aura-controller 127.0.0.1:8080
    https://rivet-01.local/reliquary/… →  reliquary-rivet 127.0.0.1:8444
    https://rivet-01.local/            →  the index, §6

One origin, one certificate in the browser, one certificate prompt.

---

## 3. Why path routing and per-application mTLS cannot both hold

**Path routing requires reading the HTTP request line. Reading the request line
requires terminating TLS. Whoever terminates TLS is who the client certificate
was presented to.**

That is not an implementation limit to be engineered around; it is what TLS is.
An intermediary that passes the handshake through cannot see the path, and one
that can see the path has already ended the client's TLS session.

So a machine may have **`/reliquary` as an address**, or **Reliquary checking
the operator's certificate itself**, and not both. R-152 chose the address and moved
the check to the proxy.

**The rejected alternative is recorded because it was close.** Per-application
mTLS needs no new code at all — `rivet-sdk`'s `serve::tls` already takes an
authority file and requires a client certificate minted by it. It costs a port
per application in every URL, and a browser prompting for a certificate once per
origin. It remains the right answer for any application that must know *which*
operator called it, and §4.4 is where that reappears.

---

## 4. The gate

### 4.1 What the proxy verifies

`rivet-proxyd` **requires the operator on every request to 443** — except on a
route declared otherwise (§4.3) — and admits the operator by **either** of two
credentials: a client certificate chaining to the fleet authority (`R-129`),
checked as this section describes, **or** a passkey session (§4.6,
`ADR-063` §4.3). The certificate is asked first and decides exactly what it
decided before passkeys existed; a session is asked only when the certificate
did not already make the caller the operator. This is the only place on the
machine an operator is authenticated for HTTP.

**A machine's document can retire the certificate** — the proxy's
`operator-certificate` setting, `accept` or `refuse` (`.rulings/` R-234,
`ADR-063` §4.3):

```toml
[applications.rivet-proxyd]
operator-certificate = "refuse"
```

**Absent is `accept`**, so a machine whose document says nothing serves exactly
as this section describes. `refuse` builds the handshake with **no client
certificate requested at all** — a browser holding one is never asked to choose
it — and only a passkey session then admits the operator; a caller with
neither is answered `403` saying the certificate is retired and where to sign
in, and a browser asking for a page is sent to `/sign-in` as before. Public
routes and open prefixes are untouched. **Any other value is read as `refuse`**
and journalled at `error`: the key exists to retire a credential, and a
misspelling must not leave it accepted while the operator believes it gone. The
proxy reads the setting at start, and journals at `warn` on every start that the
certificate is not accepted. The value is delivered as every proxy setting is,
from `[applications.rivet-proxyd]` through the settings file
(`rivet_core::service::IMAGE_SERVICE_HOMES`).

**The requirement is enforced per route rather than in the handshake, and that
is not a weakening.** The route is not known until the request line has been
read, which is after the handshake by definition, so a handshake that refused
every certificate-less caller could not serve §4.3's public route at all.
`rustls` is therefore asked for a certificate, **verifies one that is presented
against the fleet authority**, and reports whether the peer authenticated;
`ServerConnection::peer_certificates` answers `Some` only for a chain it
checked, so what the proxy reads is a verdict rather than the fact that bytes
arrived. `rivet_sdk::serve::Tls::asking_for_a_caller_certificate_from` is that
arrangement, and its documentation says why almost nothing else may take it: a
server that calls it and never checks has an unauthenticated administrative
surface and a configuration that reads as though it does not.

**A caller presenting a certificate from another authority never reaches the
proxy's code at all.** That refusal is `WebPkiClientVerifier`'s, in the
handshake, and the distinction is worth stating because the two failures look
different from the outside: no certificate is a `403` on an established
connection; a wrong one is a connection that does not establish.

**The refusal says what to present.** Its reader is an operator at a browser
that declined a certificate prompt, and *Forbidden* alone sends them looking for
a password.

**The operator is a certificate that names no host** (`.rulings/` R-187 D1).
`peer_certificates` answers for any chain to the fleet authority, and that
authority also signs each fleet machine's own `tls-cert`
(`cmd/rivet-provision/src/stage.rs`). A machine's leaf carries the names it
answers to in `subjectAltName`; the operator's browser certificate carries none.
So the proxy asks `rivetos_tls::classify_caller` of the verified leaf — the
predicate `rivet-fleetd` asks too, and `ADR-043` §5.3 states the rule in full:
the presence of `subjectAltName` decides, the subject's `commonName` is not
consulted, and a certificate that cannot be read that far is not the operator's.

| the caller presents | the index, a gated route or prefix | a public route or open prefix |
| :-- | :-- | :-- |
| nothing | `403`, saying what to present — or, to a `GET` asking for HTML, `303` to `/sign-in` | served |
| a fleet certificate naming no host | served | served |
| a live passkey session and no operator certificate | served (§4.6) | served |
| a fleet certificate naming a host, or one that cannot be read, and no session | **`403`, naming the host** | served |
| a certificate from another authority | no TLS session | no TLS session |

**A machine's certificate passes only where no certificate would**, because a
public route or an open prefix asks nobody who called. Each refusal of one is a
journal record at `warn` with typed fields `path` and `names`. The gate's answer
is computed once for the index and the routes, so the two cannot disagree about
who the operator is.

**The proxy's own leaf, presented to a backend (§4.2), is not this gate** and
is unaffected by it: a backend admits any fleet certificate, the machine's
included. That same rule is why a proxied application's port listens on
loopback only (§7.1): a backend that admits any fleet certificate must not be
reachable except through this gate.

### 4.2 What a backend verifies, and why this is not "trusting localhost"

The proxy reaches a backend over TLS **presenting its own fleet-issued
certificate**, and **the backend verifies it**. A backend accepts the proxy; it
does not accept whatever connects to its loopback port.

This is what contains R-133's stated cost. `ADR-024` §8.1 is honest that the
sandbox is not enforced, so *"the connection came from 127.0.0.1"* is not an
authentication claim — but *"the caller holds a key chaining to the fleet
authority"* is, and it is the same claim the proxy's own callers make.

**Both ends were already TLS.** `rivet-proxyd` has connected to backends with
`TlsConnector` since it was written, because Reliquary is HTTPS-only and a proxy
assuming plaintext backends could not have reached it. What changes here is that
backends **verify** rather than merely accept.

**The proxy's half is built.** Its client configuration presents the machine's
own certificate and key — the same pair it serves to browsers, because the
machine has one name and one certificate — and refuses at startup if the two do
not go together, rather than at the first request.

**What a backend must check, exactly.** This half is each application's, and it
is three things:

1. **Require a client certificate chaining to the fleet authority** —
   `rivet_sdk::serve::Tls::handed(…).requiring_a_caller_certificate_from("tls-root")`,
   which is one line already in the SDK. `requiring`, never `asking_for`; the
   latter exists for the proxy alone.
2. **Serve a certificate for the name the proxy verifies it against** — the
   machine's one fleet name, the proxy's `upstream-name` setting.
3. **Do NOT check which certificate.** There is one operator and no forwarded
   identity (§4.4); a backend that pinned the proxy's leaf would break the day a
   certificate was reissued and would be asserting something this design does
   not promise.

A backend that does none of this is not broken — it accepts the proxy — it is
merely accepting anything else on its loopback port too.

### 4.3 What the gate demands is a property of the PATH

TempMon is to be on SSL and unauthenticated (`.rulings/` R-152). So whether an operator
certificate is required is not a property of the machine, and — like every other
column in the table — it is **derived from the application's signed manifest**,
never configured. An application that wants to be reached without one says so in
the artifact that was signed, and changing it means shipping a new one.

**The unit of that declaration is a path prefix, not an application** —
`ADR-048`, which supersedes this section's original per-application reading. One
application may hold a gated page and an open door at once, on one listening
socket, because the proxy decides per prefix. §4.3.1 is the whole-route form of
the same declaration and is what a manifest naming no prefix means.

**The default is to require.** A manifest key that must be present to *tighten*
security is a key an author forgets and a machine that is quietly open; the
absence of a declaration fails closed. That holds at both granularities, and
`ADR-048` §4 is why the per-prefix form may nonetheless point in either
direction.

### 4.3.1 The key is `proxy-auth`, and it answers for the whole route

    proxy-path = "tempmon"
    proxy-auth = "none"

* **Values: `operator` and `none`.** `operator` is the default and is **never
  emitted** — the canonical manifest carries the key only when it is `none`,
  which is what lets a `.rvt` built before the key existed render and parse
  byte-identically and become **gated** rather than open the moment a proxy
  starts checking. `operator` is nonetheless *accepted* on parse, because
  refusing the explicit spelling of the safe choice would be a refusal its
  author could not act on.
* **It is the answer wherever no prefix says otherwise** (`ADR-048` §4), which
  is what keeps every manifest written before per-path rules meaning what it
  meant.
* **`proxy-auth` without `proxy-path` is refused.** It says nothing on its own
  and would say *serve me to anybody* the moment somebody added a path — a
  security-relevant key changing meaning by an edit elsewhere in the file — and
  it is also what a manifest looks like when an author deleted the wrong line.
* **The word reaches the routing table**, whose fourth column is `operator` or
  `public` rather than a bare flag, because people read that file too. **A row
  with only three fields — one written by a supervisor predating the column — is
  read as gated.** `rivet-proxyd` is an application, so that version skew is
  reachable on one board, and the reading that fails closed costs a refused
  request while the other costs the machine. `ADR-048` §6 applies the same rule
  to the fields after the fourth.
* **A public route is journalled by name**, at every derivation and at every
  proxy start, and is labelled on the index. It is a signed declaration that
  cannot be turned off on the machine, so saying so is what stands between it
  and a public route nobody remembers agreeing to. An open *prefix* earns the
  same treatment and needs it more, because the route it sits under reads as
  authenticated everywhere it is summarised.

### 4.4 What the proxy tells a backend about the caller

The proxy knows which operator authenticated; the backend does not. Forwarding
that identity is **not built** and is not required by anything today, because
there is one operator.

**When there is more than one, this ADR needs revisiting** — that is the moment
a forwarded identity stops being a formality and becomes an authorisation
decision made by a header. R-152.2 records the same boundary. An application
that must make per-operator decisions before then should terminate its own TLS
and sit outside the proxy, as §3's rejected alternative allows.

**That last sentence is a hatch `ADR-048` §7 closes in practice while leaving it
open mechanically**, and it says why: per-path authentication removes every
reason anybody had to take it, so the question becomes the proxy's rather than
an application's. Its recommendation — build nothing until a second operator
exists, then forward an identity the proxy verified over a header the proxy
strips inbound — is where this section's revisiting starts.

**A phone's caller identity does not come from the proxy at all.** `ADR-061`
§7.2 makes an application that authenticates its own callers — a phone signing
a short-lived token with a hardware key — the pattern for phone traffic. It
declares its paths `app:`, the proxy forwards them as it forwards `none:`, and
the `Authorization` header reaches the application unchanged. That pattern, not
an application's own TLS port, is how an application behind this gate learns
which phone is calling.

### 4.5 A write another site caused is refused, for every application

`.rulings/` R-186 D1. **The gate authenticates a browser, and a browser presents
the operator's certificate to any request to this machine, whichever page caused
it.** A page on another site can make that browser `POST` a body of a
CORS-safelisted type — `text/plain`, `application/x-www-form-urlencoded`,
`multipart/form-data` — with no preflight, and the request arrives as the
operator. Behind the proxy that is an application's restore route replacing its
data (`ADR-051` §12), or any other write any application accepts.

**So the proxy refuses it, before any application is reached.** The rule is
`rivet_core::cross_site::refuses`, the one `rivet-fleetd` asks for its own
endpoints (`ADR-043` §5.2):

| method | `Sec-Fetch-Site` | `Origin` | verdict |
| :-- | :-- | :-- | :-- |
| `GET`, `HEAD`, `OPTIONS` | anything | anything | forwarded |
| any other | `same-origin` or `none` | anything | forwarded |
| any other | `same-site`, `cross-site`, or any other value | anything | **refused** |
| any other | absent | equal to this request's own origin | forwarded |
| any other | absent | another origin, `null`, or no single `Host` to compare with | **refused** |
| any other | absent | absent | forwarded |

* **This request's own origin** is `https://` and the `Host` header, compared
  without regard to case and with `:443` treated as absent on either side. The
  rule applies on 443. On port 80 (§7) no write is served at all — a plain
  route answers `GET` and `HEAD` and refuses every other method `405` — so the
  question of a write's origin does not arise there.
* **Every value counts.** A head carrying a passing `Sec-Fetch-Site` and a
  failing one, or two `Origin`s one of which fails, is refused.
* **Methods are case-sensitive** (RFC 9110 §9.1): `get` is not `GET`, and is
  refused like any method the table does not name.
* **A request carrying neither header passes**, because `curl`,
  `cargo rivet backup` and `restore`, a phone's client and one application
  reaching another send neither, and requiring a browser header would make an
  operator's own tools impersonate a browser. **The assumption this rests on:
  every browser engine that presents a client certificate on a cross-site request
  sends at least one of the two headers on a write.** Chromium, Gecko and WebKit
  send `Origin` on every cross-origin `POST`, and `Sec-Fetch-Site` on every
  request in current releases. Page script can set neither. A browser that sent
  neither would not be stopped by this rule.

**`same-site` is refused.** Every legitimate browser write through the proxy
comes from an application's own page served through the proxy — relative URLs,
so `same-origin`. `same-site` would admit a page on the same host at another
port or scheme: `rivet-fleetd` on 4443, or any application that opened a socket
of its own (§9.4). No application's flow was found to need it — TempMon's
restore, import and edits, Reliquary's pairing form and revocation, Aura's
forms, and the fleet portal's calls to its own origin.

**`OPTIONS` is forwarded, as before.** A CORS preflight reaches the application,
and none of the applications behind this proxy answers one with permission, so a
browser sends nothing after it. An application that answered one too
generously still could not be written to cross-site: the write that follows
carries `Sec-Fetch-Site: cross-site` and is refused here.

**Where it sits in the gate's order.** After the traversal refusal (`ADR-048`
§5), and before the index, the route lookup, the operator gate and the
trailing-slash redirect — a property of the request, binding gated and public
routes alike, and answered before a `308` could invite the browser to send the
body again. It refuses whoever presented what.

**The refusal.** `403 Forbidden`, `text/plain`, naming the two headers and
saying no application was reached. Its reader is the operator looking at a
failed request; the page that caused it cannot read it. The body is never
forwarded: the proxy reads and discards up to 1 MiB of it for up to two seconds
after answering, so the connection does not close with bytes unread and reset
under its own answer. Each refusal is one journal record at `warn`, with typed
fields `method`, `path`, `sec_fetch_site` and `origin` (`(absent)` for a header
not sent).

**Applications rely on this and need no guard of their own.** `ADR-051` §12.7
says so for the restore route, and it holds for every write: no application is
asked to check `Content-Type`, `Origin` or a CSRF token to be safe from a page
on another site. An application may keep one it has (Reliquary's restore answers
`415` to a body not typed `application/x-tar`); the platform's contract does not
depend on it. **It holds because the proxy is the only way in from off the
machine**: a proxied application's ports listen on `127.0.0.1` (§7.1). A port
the application declares `:direct` is outside this guarantee, and what it
serves there is the application's to guard.

### 4.6 A passkey session is the gate's second credential

`ADR-063` §4.3 is the decision and names the relying party; this is what the
gate does with it (`.rulings/` R-228, R-230).

* **Either credential admits the operator.** A request with no operator
  certificate is asked for a `rivet-session` cookie the proxy issued and still
  holds; a live one is served wherever the operator certificate is served, the
  index included. Nothing about the certificate path changed: the handshake,
  `classify_caller` and every refusal in §4.1 answer as before, and the
  session is not consulted when the certificate already made the caller the
  operator.
* **The pages a person visits are `/sign-in` and `/sign-in/enroll`, on every
  machine.** One address, learned once, is one pattern a person can trust on
  each machine they reach (`.rulings/` R-249). **`sign-in` is a reserved
  application path**: `rivet_core::package::application::RESERVED_PROXY_PATHS`
  is the one list, and a `proxy-path` naming it is refused by the manifest
  builder `cargo rivet package` uses, by the parser an installing machine and
  its supervisor read with, and by the proxy reading its routing table.
* **The reserved prefix `/.rivet/` is the proxy's own, for what the pages
  call.** It holds the four ceremony endpoints under `/.rivet/passkey/` and
  `/.rivet/logout`. No application can be mounted there: a `proxy-path` begins
  with a lowercase letter (`rivet_core::package::application::is_proxy_path`).
  The pages' first addresses, `/.rivet/login` and `/.rivet/enrol`, answer
  `308 Permanent Redirect` to `/sign-in` and `/sign-in/enroll` with the query
  string kept, so a bookmark or a note naming them still arrives.
* **Both are answered before the gate.** `/sign-in` and `/.rivet/` are
  answered after the traversal refusal (`ADR-048` §5) and the cross-site guard
  (§4.5) and before the index, the route lookup and the gate, because they are
  how a caller with no credential obtains one.
* **A browser is sent to sign in; everything else is told.** A `GET` whose
  `Accept` names HTML, presenting neither credential, is answered `303` to
  `/sign-in?next=<where it was going>`. Every other refused caller — a tool, a
  script, a write — gets the `403` of §4.1, whose words name the sign-in page
  beside the certificate.
* **A session admits the operator at one name.** The proxy answers a ceremony
  at each DNS name its own certificate carries, and nowhere else: `/.rivet/`
  under any other `Host` is answered `421`. A session is bound to the name it
  was granted at, so a cookie presented under another of the machine's names is
  no session there, as though none had been sent — the cookie carries no
  `Domain`, so a browser does not send it there anyway. `ADR-063` §4.3 is why a
  passkey belongs to one name. Backends are still verified as the one
  `upstream-name` (§4.2); the names a browser may use are a property of the
  gate, not of what stands behind it.
* **The session cookie never reaches an application.** `rivet-session` is
  `Path=/`, so a browser sends it to every route, public ones included; the
  proxy removes it from every head it forwards, leaving the rest of the
  `Cookie` header as it was. An application that saw it could replay it here.
* **The page that the cookie protects cannot be framed**: ceremony answers
  carry `X-Frame-Options: DENY` and `frame-ancestors 'none'`, and a ceremony run
  inside a foreign frame is refused by its own `crossOrigin` flag.
* **`SameSite=Strict` and §4.5 are two locks on one door.** A cross-site
  request carries no session cookie in any current browser, and a cross-site
  write is refused by §4.5 whatever it carries. Neither is relied on alone.

---

## 5. The routing table is derived, and that is the whole design

`rivet-core/src/proxy.rs` holds the table's shape, in `rivet-core` rather than
in the daemon because three programs need it — the supervisor that writes it,
the proxy that reads it, and any tool reporting what is routed (`ADR-011`).

**Nothing in it is configured.** Every row comes from an installed
application's signed manifest — its `proxy-path`, its first TCP `listen` port,
and (§4.3) whether it requires an operator. An operator cannot add a route,
cannot move one and cannot make one public: each of those means shipping a new
artifact that somebody signed.

**The writer is `cmd/rivet-sysd`'s `runtime::publish_routes`**, and it runs
once before the first convergence, again at each finished install, and again
when an uninstall removes an application, so the table on disk follows what is
installed without a reboot. It takes no document, because no column comes from
one. **The proxy reads the table at its start**, so a running proxy serves the
rows it started with until it is restarted with `apply service-restart
rivet-proxyd confirm`; the supervisor never restarts it by itself (§7). An
uninstall that left the table alone would have that restart, or the next boot,
read a route to an application that is gone.

**It is written into the proxy's home beside `data/`, not into `data/`** — the
same move `ADR-038` §8.1 made for the settings file. `data/` is the one directory
the proxy owns, at mode `0700` under its own uid, and writing into it as root by
path is the shape that produced the 2026-09-03 audit's symlink truncation.
`<volume>/app/rivet-proxyd` is root-owned at `0755` — the supervisor makes it
that way for the image's proxy exactly as an installer makes an application's
(`ADR-060` §2.1) — so there is nowhere to plant a link and **a proxy cannot
rewrite its own routes.**

**Nothing about it is fatal.** A machine that does not run the proxy, a manifest that
will not parse, an application that asked for a path and declared no TCP socket,
two applications asking for one path: each is reported and none stops a
convergence. That is `ADR-038` §4.4's containment pointed at routing, and it
matters more here than for a setting — the machine is reached *through* this.

**A machine with no `proxy-path` anywhere is the ordinary machine**, and both the
proxy and the index say so rather than failing.

---

## 6. The index at `/`

A request for `/` renders **the routes that exist right now**, from the table of
§5, as a list a person can click. The proxy already holds the only source of
truth about what is routable, so an index built from anything else would be a
second answer (`ADR-011`).

**No framework, no CDN, and nothing fetched.** The style is one embedded
constant and there is no script. The board is on a house LAN with no guaranteed
route to the internet, and a stylesheet fetched from one would make the only
page that says what a machine runs the one page that needs working DNS. It
follows `prefers-color-scheme` in the one way that needs no JavaScript: two
colour blocks, nothing else varying.

**It is behind the gate like everything else** — the list of what a machine runs
is not public.

**A public route is labelled on the page**, beside its path, for §4.3.1's
reason: the list an operator came to read is where they are most likely to
notice one.

**A machine with no routes gets a sentence, not an empty box.** That is the
ordinary machine (§5), and a page that looked broken would send its reader
hunting a fault that is not there.

**A path nothing serves is `404` to everybody, gate or no gate**, and the order
is deliberate. It leaks nothing either way — the index would name the route to
anyone with a certificate — while answering `403` for a typo would tell an
authenticated operator that their mistake was a permission problem.

---

## 7. Ports

* **`rivet-proxyd` listens on 443**, the machine's front door.
* **`rivet-proxyd` also listens on 80 — only on a machine where an installed
  application declares a `plain:` route** (`ADR-061` §7.1, `.rulings/` R-221).
  Port 80 serves exactly those routes, over plain HTTP, to `GET` and `HEAD`. It
  answers everything else `404`, the index included, and never redirects to
  HTTPS, so it can never read as a second front door. A plain route exists for
  a phone that has not paired and so cannot check the fleet's private
  certificate: HTTPS would show it a warning and protect nothing, because the
  application verifies its own authenticity after pairing. The supervisor binds
  80 when the routing table carries a plain row and hands the descriptor over
  as it does 443, as `tcp-80` after `tcp-443`; a machine with no plain row has
  no listener on 80 at all.
* **Whether 80 is wanted is decided where the routes are published**, which on
  every boot is before the proxy's first start, so a machine that boots with a
  plain route installed opens 80 at once. **An install on a running machine
  that adds or removes the only plain route does not restart the proxy.** The
  supervisor records the new listen set and journals a warning naming the
  change and the command that applies it, `apply service-restart rivet-proxyd
  confirm`; the restart binds or closes 80. **The deploy offers that restart as
  one of its steps, defaulting to No** (`ADR-055` §6.2, `.rulings/` R-224), and
  names port 80 when it is what changes. The supervisor never restarts the
  proxy by itself.
* **`rivet-fleetd` listens on 4443.** `R-152.3` left the number unruled and the
  maintainer named it on 2026-09-05. Fleet keeps its own mTLS and stays
  **directly reachable**, because the administration surface must not depend on
  the thing it administers: a proxy that will not start is a machine to be
  fixed, and the tool that fixes it must not be behind the proxy. `/fleetd` is
  therefore deliberately not a route.
* **The proxy reaches a backend on `127.0.0.1` and its declared port**, and a
  proxied application's ports listen there and nowhere else (§7.1).

**Neither program names a port** — the supervisor binds it as root and hands
over the descriptor (`ADR-015` §5.2): `rivet-fleetd`'s from its manifest's
`listen`, the proxy's from its image definition (`ADR-060`). The supervisor is
also what catches the collision: two things cannot bind one port, and it
refuses the second start by name before either program runs. Neither is behind
the proxy, so both are bound on every address (§7.1).

**The cost of fleet's move, stated where it will be met:** the web portal moves
from `https://rivet-01.local/` to `https://rivet-01.local:4443/`, and a bookmark
to the old address reaches the proxy's index instead.

### 7.1 A proxied application's port listens on loopback only

`.rulings/` R-188. **An application whose signed manifest declares a
`proxy-path` has every `tcp:` and `udp:` port bound on `127.0.0.1`**, unless the
manifest declares that port `:direct`. `rivet-proxyd` dials loopback, so every
route still reaches its backend; nothing off the machine does, so the operator
gate (§4.1) and the cross-site guard (§4.5) are in the path of every request
that arrives from the network. This matters because a backend built to §4.2
admits any fleet certificate and deliberately does not check which, and
`rivet-netd` applies no firewall (`ADR-023` §9): a routed port reachable from
the LAN would let another fleet machine's front-door key read and write the
application, and let a page on another site make the operator's browser write
to it.

| the application | its port | bound on |
| :-- | :-- | :-- |
| declares a `proxy-path` | `tcp:<port>` or `udp:<port>` | `127.0.0.1` |
| declares a `proxy-path` | `tcp:<port>:direct` or `udp:<port>:direct` | every address |
| declares no `proxy-path` | `tcp:<port>` or `udp:<port>` | every address |

* **"Proxied" is a property of the signed manifest**, not of the machine: a
  `proxy-path` binds loopback whether or not a proxy is installed and whether or
  not the route won a path collision (§5). What a signature means must not
  depend on what else is installed.
* **An application with no `proxy-path` is bound on every address.** It has no
  gate to bypass — its own port is its only door. `rivet-proxyd` (443) and
  `rivet-fleetd` (4443) are two such applications, so they need no flag, and
  binding loopback there would shut the operator out of both on any machine
  running a `.rvt` built before the flag existed.
* **`:direct` is written after the port**, inside `listen`:
  `listen = ["tcp:8444", "tcp:8443:direct"]`. It is part of the manifest member
  the header signs, so an operator cannot add it on the machine. The descriptor
  still arrives named `tcp-8443`; the program does not change.
* **`:direct` is refused on the route's backend** — the first TCP port, which
  `publish_routes` routes (§5). A routed port reachable directly is the bypass
  this section closes. `Manifest::route_port` is the one computation both the
  routing table and the refusal use.
* **`:direct` is refused without a `proxy-path`**, on §4.3.1's footing for
  `proxy-auth`: it is a declaration attached to nothing, and a second spelling
  of a port already bound on every address.
* **Both refusals are made twice**: by `cargo rivet package`, in words, before
  anything is signed, and by `Manifest::parse`, which is what a manifest read
  off a card on a machine goes through.
* **UDP follows the same rule.** Nothing proxies UDP, so no UDP port is ever a
  route's backend, and no proxied application declares one today. One rule for
  every declared port of a proxied application is the reading that fails
  closed; a proxied application that serves datagrams to the network says
  `udp:<port>:direct`. A unix socket has a path and no address and is unchanged.
* **One port on two reaches is one socket.** `tcp:8080` on loopback and
  `tcp:8080` on every address cannot both be bound, so the supervisor refuses
  the second application by name as it refuses two identical declarations.
* **Every bind is journalled** at `info` with a `bound` field —
  `127.0.0.1:8444,0.0.0.0:8443` — at each start, and in the report an installed
  application's definition gets.

**What stays reachable, and how.** The proxy dials `127.0.0.1`.
`aura-controller` reaches TempMon's backend on `127.0.0.1:8081`, presenting the
machine's own leaf (§9.2 item 3). Reliquary's phone listener is
`tcp:8443:direct`, reached from the LAN; its administrative 8444 is routed and
loopback.

**Why the supervisor, and not the backends or the network.** A backend that
admitted only a certificate naming this machine would close the same two holes
for each application that adopted it, at the cost of a change in every
application, several in other repositories. A `rivet-netd` ruleset would close
them for every application, and is unbuilt. Binding loopback is the platform
refusing by default — the posture §4.3 takes for the gate itself — so no
application reached only through the proxy changes, and an application that
wants a second door says so in what was signed.

**What a manifest signed before the flag does.** It parses unchanged, and a
proxied application's ports now bind loopback: an existing Reliquary `.rvt`
keeps 8444 behind the proxy and loses 8443 from the LAN until it is rebuilt with
the flag. A supervisor older than the flag refuses a manifest carrying it.

`tools/qemu/adr-046-backend-port-proof.sh` proves this on a booted guest, §9.1.

---

## 8. What this does NOT do

* **It does not rewrite bodies.** An application mounted at `/reliquary` that
  emits an absolute URL of its own emits a wrong one. R-133 recorded the answer
  — the declaration is two-sided and the application has to be *told* its base —
  and **that half is still not built**.
* **It does not keep connections alive.** Every hop is `Connection: close` in
  both directions, which is slower and is what makes forwarding correct without
  a response parser. A response's chunked encoding, trailers and content length
  pass through because nothing interprets them.
* **It does not forward a request whose framing is ambiguous.**
  * A chunked request body is refused with `411`, because nothing behind the
    proxy decodes one.
  * `Transfer-Encoding` beside `Content-Length`, any coding other than exactly
    `chunked`, and a repeated `Content-Length` are each refused with `400`.
* **It does not pass WebSockets.** `Upgrade` is stripped as a hop-by-hop
  header, and it stays stripped until WebSockets through the authentication gate
  are designed. The gate judges each request, and a WebSocket is one long
  connection (`.rulings/` R-210).
* **It does not authorise.** It establishes that a caller holds an operator
  certificate or a passkey session. What that operator may do is the
  application's question.
* **It is not a load balancer, and there is one machine.**

---

## 9. What is built, what is proved, and what is unproven

### 9.1 Built, tested, and RUNNING ON A BOARD since 2026-09-05

The gate (§4.1), the proxy's half of §4.2, the `proxy-auth` key and its column
(§4.3), the index (§6) and the ports (§7). Also, from before: the path proxy
itself, the derived table's shape, TLS on both sides, the 32 KiB request-head
bound and its 15-second timeout.

**How the gate was proved, and by what method**, because "it compiles" is not
the claim. `cmd/rivet-proxyd`'s suite drives real TLS in-process against a real
backend, and **three callers are three tests**:

| the caller | what happens | what it proves |
| --- | --- | --- |
| presents no certificate | the connection establishes, the request is `403` | the refusal is this program's, per route, and not the handshake's |
| presents one from another authority | no TLS session at all | `WebPkiClientVerifier` is verifying against the fleet authority |
| presents a fleet certificate naming no host | `200` from a backend built with `requiring_a_caller_certificate_from` | the gate admits the operator, **and §4.2 in the same request** |

Plus: a public route reached with no certificate at all (§4.3), the index
refused and then served (§6), and a backend refusing a caller that is not the
proxy. **Both halves were broken and watched to fail** — removing the gate's
refusal fails four tests, and presenting no client identity fails exactly the two
that reach a backend — then restored and re-run.

**§4.5, and by what method.** *Unit tests on the host* (`rivet-core/src/cross_site.rs`):
the whole table, nine methods × eight `Sec-Fetch-Site` values × four `Origin`
cases, against the rule written out separately; each safelisted content type by
either header; `cargo rivet restore`'s head, TempMon's page's head and an
`Origin`-only engine on a port passing; case folding, `:443`, repeated headers,
neighbouring origins. *Unit tests in the container* (`cmd/rivet-proxyd`): a
cross-site write refused on every route shape, authenticated or not, and after
the traversal refusal. *Over real TLS in-process*: an operator certificate, a
gated route and a backend counting its connections — four cross-site `POST`s,
each head and 4 KiB body sent as separate writes, all `403` with the backend
accepting **no connection**; three legitimate `POST`s and a cross-site `GET`
forwarded, and the backend accepting four. **Broken three ways and watched to
fail**: the proxy's call to the rule disabled fails the over-TLS refusal; the
`Sec-Fetch-Site` half disabled fails four core tests, two `rivet-fleetd` tests
and the over-TLS refusal; the `Origin` half disabled fails four core tests,
`rivet-fleetd`'s `Origin` test and the over-TLS refusal. *Under QEMU*,
`tools/qemu/adr-051-restore-proof.sh` sends the attack's headers with `curl`
through the proxy on a booted guest (`ADR-051` §12.9). **No browser has been
driven against this rule, and it has not run on a board.**

**§4.1's operator, and by what method.** *Unit tests on the host*
(`rivet-tls/src/caller_tests.rs`): rcgen leaves with no `subjectAltName` under
five subjects, dotted ones included, are the operator; DNS, IPv4, IPv6, URI,
email and `otherName` entries are not; every prefix of a certificate, a padded
one, and hand-built certificates with no version, two `subjectAltName`s, an empty
one, an undefined entry tag or a malformed extension are unreadable.
*Provisioning pins* (`cmd/rivet-provision`): every leaf `issue_from` mints names
its host by both `classify_caller` and `x509-parser`, and it refuses an empty
list; `Authority::operator` names none; a server's two staged front doors, a
member's and `ca server`'s and the menu's leaves name their hosts; a scan of the
shipped source fails when a minting call is added. *Over real TLS in-process, in
the container* (`cmd/rivet-proxyd`): a provisioned member leaf is refused the
index `403` naming `rivet-02.local`, refused a gated `POST` with the backend
accepting no connection, and served a public route whose backend accepts one;
the provisioned operator is served the index. **Broken and watched to fail**:
the gate treating a host-naming certificate as the operator fails exactly those
three refusal tests. *Under QEMU*, `tools/qemu/adr-046-backend-port-proof.sh`
— the operator served the index, another machine's certificate refused it and a
gated route, the refusal journalled — passes on a booted guest, and
`tools/qemu/adr-051-restore-proof.sh` passes with the provisioned operator
certificate through the new gate. **No real browser certificate has been
inspected, and nothing has run on a board.**

**§7.1's loopback bind, and by what method.** *Unit tests on the host*
(`rivet-core`): the `:direct` grammar and its misspellings, the round trip
through the canonical text, `Socket::bind_address` for every row of §7.1's
table, and both refusals in `Manifest::parse`; `cargo rivet package`'s reader
carries the flag into a manifest that parses back and refuses both cases in
words. *Unit tests in the container* (`cmd/rivet-sysd`): a proxied definition's
sockets opened on `127.0.0.1:8444`, `0.0.0.0:8443` and `127.0.0.1:5354` and an
unproxied one on `0.0.0.0:443`, each journalled; one port on two reaches refused
as a collision. **Broken and watched to fail**: `bind_address` never choosing
loopback and `Manifest::parse` skipping the refusal fail four `rivet-core`
tests, two each; the packaging check skipped fails its refusal test; the
supervisor passing `proxied = false` fails its bind test; comparing sockets by
spelling fails the collision test. *Under QEMU*,
`tools/qemu/adr-046-backend-port-proof.sh` on a booted guest: the supervisor
journals the probe bound on `127.0.0.1:8080,0.0.0.0:8081` and `rivet-proxyd` on
`0.0.0.0:443`; through the proxy the operator mutates and restores and another
machine's certificate and a cross-site `text/plain` restore are refused `403`
with the data unchanged; on the routed port, through QEMU's forward to the
guest's non-loopback address, another machine's read and write and the
operator's cross-site restore complete nothing and leave the data unchanged;
the same read is answered `200` on the probe's `tcp:8081:direct` port. With
`bind_address` never choosing loopback the same proof reports the bind record
and all three routed-port lines as failures. **No LAN peer and no board has
tried a port.**

### 9.2 Not built, and this ADR is still the specification for it

1. **TLS in three applications — one deployed, two written.** One of the
   maintainer's own applications is **on the board and serving** (§9.3).
   `tempmon-rivet` has a committed TLS
   build held back by item 3, and `examples/aura/aura-controller` is a Rust
   port that serves HTTPS through `rivet_sdk::serve::Tls` with no plaintext
   branch and is not deployed.
2. **Backends verifying the proxy — the same one deployed.** §4.2's second half
   is implemented in all three by the same one-line
   `requiring_a_caller_certificate_from("tls-root")`, and the deployed one has
   now verified a real proxy on real hardware.
3. **A backend that is also a CLIENT — BUILT, not deployed.**
   `aura-controller` reaches TempMon, so it needs a certificate to *present*
   and not merely one to serve; `examples/aura/aura-controller/src/tempmon.rs`
   presents the machine's.

   **It surfaced a gap in §4.2 that this ADR had not stated.** §4.2 says a
   backend must *"serve a certificate for the name the proxy verifies it
   against"* and says nothing about how one backend addresses another. The
   machine's certificate carries exactly one name — `DNS:rivet-01.local` — and
   a Rivet's `/etc/hosts` holds `rivet-01` without the suffix while musl has no
   mDNS, so **the name in the certificate does not resolve on the machine that
   serves it.** `rivet-proxyd` never hit this because it connects to a loopback
   address and hands rustls `upstream-name` separately; any other caller has to
   do the same. The controller does, through a fixed resolver and a
   `tempmon_name` setting whose absence means *verify the URL's host*.
4. **Telling an application its base path.** §8's first bullet, `R-133`'s
   two-sided declaration, untouched by this work. `aura-controller` works
   around it with relative links throughout, which resolve correctly under any
   prefix **so long as the route is reached with a trailing slash**.

**Retiring Aura's password is DONE.** `portal_password` was a shared secret in
cleartext in `/state/app/aura-controller/data/config.json`; the Rust port has no
such field, and a config file still carrying one is reported and rewritten on
load — which is what removes it from the disk rather than merely ignoring it.
R-152.1.

### 9.3 On the board — what a machine and a person have actually done

`0.1.73` (`7b740f4`), 2026-09-05. `PLAN.md` §140 has the sequence.

**The maintainer's browser presented an operator certificate to
`https://rivet-01.local/` and got the index** — two cards, `/tempmon/` and the
other application's, in the dark theme his browser asked for. A certificate-less `curl`
to the same address gets a `403` on an established connection, naming what to
present. That is §4.1 and §6 proved by a person rather than by a process.

**Both rows read `operator`** — the default doing its job. Every `.rvt` already
installed became gated the moment the proxy started checking, without being
rebuilt.

**One defect, and only a board could have found it.** `rivet-proxyd` failed five
times: *"the routing table … could not be read: Permission denied"*. The file was
`0644 root:root` and `proxy::path` calls itself application-readable, so every
check that inspected the FILE said yes — what was missing was the confinement
grant, and a policy is invisible to anything that looks at a mode. Fixed in
`0.1.73`; `PLAN.md` §140.2.

**`aura-controller` is absent from the table and from the index**, because the
`aura-controller` on the board is the Go one: it declares `network = "none"` and
`listen = []` while serving HTTP on 8080 — it opens its own socket rather than
being handed one — so there is no port to route `/aura` to. The page shows two
applications because two declare a port the platform opened.

The Rust port written on 2026-09-05 declares the port properly and is **not
deployed**, so this paragraph describes the board and not the tree. See §9.4.

**§4.2 END TO END, 2026-09-05.** That application's TLS build was deployed and
restarted, and it journalled *"TLS is on and a caller certificate from the fleet
authority is REQUIRED"*. An operator certificate then fetched its route through
the proxy and got **`200` and the dashboard, 10,604 bytes**; the same client
with no certificate got **`403`** from the gate, naming
what to present. That is both halves of this ADR in one request — the gate
verifying an operator, and a backend verifying the proxy — on hardware rather
than in a test.

**The step that unblocked it was `cargo rivet secrets`**, which installs
`tls-cert`, `tls-key` and `tls-root` from `~/.rivet/ca`. There was nothing to
mint: §4.2 gives every backend the machine's one certificate, and
`rivet-proxyd` had been running on exactly those files since 2026-09-05.

**Still unproven here:** no **public** route has been served, because
`proxy-auth = "none"` exists only on TempMon's package and that is held back by
§9.2 item 3.

### 9.4 What this surfaced about the platform, which is larger than the proxy

An application that opens its own listening port is **the platform failing to be
a platform**. `aura-controller` was the live case: nothing stopped it, the
manifest said it listens on nothing, and the consequence was a route that could
not exist and a socket the gate could not protect.

**The case has been fixed in the application and NOT in the platform, and the
distinction is the whole point.** `examples/aura/aura-controller` is a Rust port
that declares `listen = ["tcp:8080"]`, `ready = "listening"` and
`proxy-path = "aura"`, and binds nothing — so `/aura/` becomes a route the gate
protects the moment it is deployed (`PLAN.md` §141). Nothing about the platform
changed: **a third application could still open its own port tomorrow**, and
nothing would stop it or say so.

This belongs beside `ADR-024` §8.1's admission that the sandbox is not enforced,
and the maintainer named it as the concrete argument for returning to that work.
**It was never an Aura bug**; Aura is where it happened to show, and a
well-behaved Aura does not make the next one behave.

**This is deployed.** `0.1.73` carries it — `rivet-proxyd` on 443 and
`rivet-fleetd` on 4443, §9.3. The supervisor's table writer (§5) needed a
**system image** to reach the board, because `cmd/rivet-sysd` ships inside the
image rather than as an application; that is why the move went out as an image
and not as two deployments.

**The two ports had to move together, and still must.** A proxy claiming 443
deployed without the fleet that vacates it leaves two applications declaring
443, and the supervisor refuses whichever starts second.

**Trends** — one of the four applications `R-152` names — has never run on a
board at all.

---

## 10. How many connections, and how long each may take to ask

`rivet-proxyd` is what every device on the LAN reaches for every application,
so it is also what one misbehaving or hostile host reaches first. It weighs
every accepted connection, on 443 and on 80 alike, before it costs a task, with
the same governor as the control plane (`rivet_core::connection_slots`,
`ADR-035` §12):

| Rule | Value | Why |
| :-- | :-- | :-- |
| Connections held at once, across both listeners | 64 | The control plane's 16 is sized for operators. The proxy fronts every application for every device, each page load opening several connections, so it holds four times as many. |
| Of those, one source address | 12 | The same share as every listener: one workstation doing several things at once is not throttled, and the rest are always free for another address. |
| New connections from one source | a burst of 32, then 8 a second | The control plane's rate. A flood is cut to a trickle and its refusals are journalled once per source per minute with a count, so it cannot roll the journal. |

**Three deadlines before the request is in hand, because one step waits on a
person:**

| Step | Deadline | Why |
| :-- | :-- | :-- |
| Accept to the first byte | 3 s | A browser sends its `ClientHello` the moment it connects. A client that has sent nothing is silent, not slow, and is dropped with no answer. |
| Accept to a finished TLS handshake | 15 s | The handshake can pause while the browser asks its user which client certificate to present (§4). The control plane's 3 s would cut that choice off mid-dialog, so this step keeps the longer allowance. A caller that never finishes is dropped with no answer: there is no TLS session to answer on. |
| Handshake (or, on port 80, accept) to a complete request head | 3 s | Once the handshake is done the head follows at once; one that does not is answered `408`. |

A passkey ceremony's small request body is read under the later of the
handshake's and the head's deadlines.

**The handshake deadline is the one a machine's document can change**, with
the proxy's setting `handshake-seconds` under `[applications.rivet-proxyd]`: a
whole number of seconds from 3 to 120, written as a string like every
application setting, read at the proxy's start. Absent, it is 15. A person waits
inside this step and no other, which is the reason it may differ
between machines. Below 3 the handshake would be stricter than the steps around
it. Above 120, each waiting connection would hold one of the 64 slots long
enough for a few silent clients to fill the front door. `apply edit` refuses a
value outside that range before it is written. A value that reaches the proxy
anyway is set aside with a journal line, and the proxy starts on 15 s, because
the front door must not fail to start over a tuning value. The other two
deadlines and the connection caps are fixed.

---

## History

### 2026-10-04 — §4.6's pages were under the hidden `/.rivet/` prefix

§4.6 said:

> ~~It holds the login page (`/.rivet/login`), the enrolment page
> (`/.rivet/enrol`), the four ceremony endpoints under `/.rivet/passkey/` and
> `/.rivet/logout`.~~

and the gate's `303` went to `/.rivet/login?next=…`.

**Why it changed.** The prefix was chosen only because no application path may
begin with a dot, and the pages a person reads in the address bar looked like a
development path there. The maintainer ruled that they are `/sign-in` and
`/sign-in/enroll` on every machine, that `sign-in` is a reserved application
name, that the calls behind the pages may stay under the hidden prefix, and that
the old addresses redirect (`.rulings/` R-249).

### 2026-10-03 — the proxy held any number of connections, with one 15 s deadline

Until §10, `rivet-proxyd` had no connection cap of any kind, and one deadline of
15 s from the accept covered the handshake and the request head together, so a
socket that sent nothing held a task and a descriptor for 15 s. **Why it
changed:** the maintainer ruled that the proxy gets the control plane's
admission now (`.rulings/` R-242), with a pool sized for applications rather
than operators and a short deadline for a silent client. The 15 s survives only
for the handshake step, for the certificate dialog's reason.

### 2026-09-29 — §4.1's gate admitted a client certificate and nothing else

§4.1 opened:

> ~~`rivet-proxyd` **requires a client certificate chaining to the fleet
> authority** (`R-129`) on every connection to 443, and refuses one that
> presents none — except on a route declared otherwise (§4.3).~~

and its table's first row was *"nothing — `403`, saying what to present"*, with
no second credential beside the certificate.

**Why it changed.** The maintainer ruled that passkeys replace the
client-certificate login in the browser, beside client certificates until every
tool has moved off them (`.rulings/` R-228, `ADR-063` §4.3), and chose a relying
party of Rivet's own (`.rulings/` R-230). The certificate path is unchanged and
stays the first thing asked; §4.6 is the session beside it.

### 2026-09-25 — §8 said upgrades and chunked request bodies pass through

§8 said:

> ~~chunked encoding, trailers, content lengths and 101 upgrades all pass
> through because nothing interprets them.~~

The proxy strips `Upgrade` as a hop-by-hop header, so no upgrade ever reached a
backend. Since 2026-09-24 it also refuses a chunked request body, because
nothing behind it decodes one. The maintainer ruled to keep stripping `Upgrade`
until WebSockets through the gate are designed (`.rulings/` R-210).

### 2026-09-23 — §5 and §7 described the proxy as an installed application

`ADR-060` moved `rivet-proxyd` into the system image, still run under its own
uid from its own home. §5 and §7 said:

> ~~**It is written beside `current`, not into `data/`** … `data/` is the one
> directory the application owns~~
>
> ~~**Nothing about it is fatal.** A machine with no proxy installed, …~~
>
> ~~**Both are one manifest line**, because neither program names a port~~

The image's proxy has no `current` and no manifest, so the table goes into the
home the supervisor makes for it and the port comes from its image definition.
The argument for writing beside `data/` rather than into it is unchanged and
stays in §5.

### 2026-09-14 — §7.1 was a known defect with three remedies; R-188 chose one

§7.1 recorded the defect and put the choice to the maintainer:

> ~~**The supervisor binds every `tcp:` port a manifest declares on every
> address** (`cmd/rivet-sysd/src/kernel.rs`, `open_listeners`, `0.0.0.0`), the
> routed port included, and no firewall stands in front of it … So anyone on the
> network holding a fleet certificate reaches an application on its own port,
> and neither the operator gate (§4.1) nor the cross-site guard (§4.5) is in the
> path.~~
>
> ~~`tools/qemu/adr-046-backend-port-proof.sh` proves both on a booted guest, and
> **fails today by design** … the other machine's certificate read the probe's
> data (`200`, 2048 bytes) and replaced it (3072 bytes after), and the
> operator's certificate was served a restore carrying `Sec-Fetch-Site:
> cross-site` and `Content-Type: text/plain` (`200`).~~
>
> | option | closes | costs |
> | :-- | :-- | :-- |
> | ~~**A. The supervisor binds a routed application's port on `127.0.0.1`**, with a signed manifest declaration for a port that must also be reached directly~~ | ~~both, for every application that does not declare otherwise, with no application changed~~ | ~~a manifest key and its default decided like `proxy-auth`'s (§4.3.1); Reliquary's manifest declares its direct port~~ |
> | ~~**B. A backend admits only a certificate naming this machine's own name**~~ | ~~both, for each application that adopts it~~ | ~~an SDK constructor and a change in every application, several in other repositories; §4.2 item 3 is rewritten~~ |
> | ~~**C. `rivet-netd` applies a ruleset closing application ports to the LAN**~~ | ~~both, for every application~~ | ~~`ADR-023` §9's ruleset generator, unbuilt, and a statement of which ports are open~~ |
>
> ~~**Recommendation: A, with B as defence in depth where an application already
> terminates its own TLS.**~~

§4.1 ended *"That same rule is why a backend's own port is the defect in §7.1"*
and §4.5 said the guarantee held only for requests through the proxy, *"and an
application's own port is reachable without it"*.

**Why it changed.** The maintainer chose A (`.rulings/` R-188). Building it
settled three things the option left open, each the conservative reading:
*proxied* means the manifest declares a `proxy-path`, so an application with no
route — `rivet-proxyd` and `rivet-fleetd` among them — keeps every address and
no operator is shut out by a `.rvt` signed before the flag; UDP follows the same
rule; and the declaration is `:direct` inside `listen`, refused on the routed
port and without a route. The reasoning for A over B and C is in §7.1's body.

### 2026-09-14 — §4.1 admitted every fleet machine, and §7 said backends were unreachable

§4.1 carried a known defect, proved by a test that was `#[ignore]`d and failed
when run:

> ~~**Known defect: the gate admits every fleet machine's front door as an
> operator.** `peer_certificates` answers for any chain to the fleet authority,
> and `cmd/rivet-provision/src/stage.rs` issues each fleet machine's own
> `tls-cert` from that authority with nothing in it a verifier tells apart from
> the operator's browser certificate.~~

**Why it changed.** The maintainer chose the recommended remedy (`.rulings/`
R-187 D1): the operator is the certificate that names no host, enforced in this
proxy and in `rivet-fleetd` through one predicate, with no certificate reissued.

§7 said, of the applications behind the proxy:

> ~~Backends keep their loopback ports and are not otherwise reachable.~~

**Why it changed.** Building R-187 checked it and it was false: the supervisor
binds a declared port on every address and nothing firewalls it. A QEMU run
proved another machine's certificate and a cross-site write reaching an
application on its own port, so the sentence became §7.1's known defect.

### 2026-09-07 — §4.3's gate was a property of the APPLICATION, not of the path

**Changed by `ADR-048`**, and the reason is worth keeping because the sentence
that was superseded reads perfectly well and was the source of two workarounds.

The body said this, and it was the whole of the decision:

> ~~**whether a route requires an operator certificate is a property of the
> route**~~

*Route* meant *application* throughout: one `proxy-auth` value, one manifest,
one verdict for everything the application serves. What that made impossible was
an application holding both postures, and both health services met it. A phone
could not reach Trends at all — the proxy gate and the application's own caller
check both demanded a certificate a phone cannot obtain, so the refusal happened
in the handshake and nothing was logged. Reliquary reached both postures by
declaring **two listening sockets** and relying on the supervisor routing the
first, which put a security decision in the order of two lines in a manifest.

**The argument the superseded sentence carried is still live and is in the
body**: the verdict is derived from the signed manifest and never configured.
What changed is its granularity.

### 2026-09-05 — §9's list of five became a list of four different things

The ADR was written in the morning naming five unbuilt items, and this is what
it said before four of them were built. Kept because the *shape* of the list is
the useful record: it is what an agent was handed as a specification, and what
came back differed from it in one instructive way — item 3 said *"the per-route
column"* as though only a column were missing, and the supervisor that derives
the table did not exist at all. §9 had said the derived table was built, and
what was built was its **shape** in `rivet-core`.

> **Not built, and this ADR is the specification for it:**
>
> 1. **The gate** — the proxy requires no client certificate today. §4.1.
> 2. **Backends verifying the proxy.** §4.2.
> 3. **The per-route "no operator needed" column.** §4.3.
> 4. **The index at `/`.** §6.
> 5. **Moving 443 from `rivet-fleetd` to the proxy.** §7.

The status block then read *"**Partly built** — the path proxy exists and
carries bytes; the gate of §4 does not"*, and §7 recorded fleet's port as *"not
ruled (R-152.3)"* before the maintainer named 4443.
