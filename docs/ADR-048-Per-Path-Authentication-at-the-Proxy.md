# ADR-048: Per-Path Authentication at the Proxy

* **Status:** Accepted. **Built, with tests, and not run on a board.** §8 says
  what is proved and by what method, and what no machine has done.
* **Date:** 2026-09-07
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Ruled:** by the maintainer, 2026-09-07 — per-path `proxy-auth` is the
  convergence point for Trends and Reliquary. `.rulings/` carries no id for it,
  and `AGENTS.md` §2 makes that file his own words, so this ADR states the
  decision in its own.
* **Supersedes:** `ADR-046` §4.3's reading of the gate as a property of a whole
  application. Everything else in `ADR-046` stands, and this ADR widens one
  declaration rather than replacing the gate.
* **Extended by:** `ADR-061` §7 — two more verdicts, `plain:` and `app:`
  (§4.5 here), `.rulings/` R-221.
* **Code:** `rivet-core/src/package/application.rs` (`proxy-auth-paths`,
  `ProxyRule`, the prefix grammar), `rivet-core/src/proxy.rs` (the matcher, the
  table, the traversal guard), `cmd/rivet-proxyd/src/main.rs` (the gate),
  `cmd/rivet-sysd/src/runtime.rs` (`publish_routes`, which derives the rows),
  `cmd/cargo-rivet/src/crate_meta.rs` (what an author may declare)
* **Answers:** *"how does one application show a gated page to an operator and
  an open door to a phone, over one name, on one socket?"*

---

## 1. Context

`ADR-046` put one gate in front of every application on the machine and made
what it demands a property of **the application**: `proxy-auth` is one value in
one manifest, and its absence means *require an operator*. So an application is
entirely open at the proxy or entirely shut, and two things follow from that one
fact.

**A phone cannot reach a health service at all.** Two locks want the same key:
the proxy's gate, and the application's own
`requiring_a_caller_certificate_from` behind it. A phone holds no operator
certificate and cannot obtain one, because the endpoint that would issue it a
credential is the endpoint being refused. The refusal happens in the handshake,
so the application never sees the request and has nothing to log.

**Holding both postures at once costs a second listening socket.** An
application can declare two TCP ports, put the gated surface on the first and
the open one on the second, and rely on `cmd/rivet-sysd` routing only the first.
That works. It also means the order of two lines in a manifest decides which
surface is published at the machine's front door, and getting it backwards
publishes somebody's personal records there. A declaration whose meaning depends on which
of two lines comes first is a declaration nobody can review.

**Both are the same defect**, and it is the granularity of the declaration
rather than the gate. Per-path authentication retires both: one listener per
application, everything on 443, and the proxy deciding per prefix.

**It is also `ADR-046` §2's *one name and one certificate* finally reaching a
phone.** A device pairs with the machine, not with an application on it, so the
pin it keeps is per machine — which is what makes the ceremony worth extracting
once rather than growing one per adopter.

---

## 2. The shape

    https://rivet-01.local/reliquary/        →  operator certificate required
    https://rivet-01.local/reliquary/pair    →  served to a caller with none
    https://rivet-01.local/reliquary/pairings→  operator certificate required

One route, one backend, one socket. The gate's answer is a function of the
request path rather than of which application it belongs to.

---

## 3. Two sockets were the shape that was available, and are not the pattern

**Reliquary's two-socket arrangement is transitional and must not be copied.**
It is recorded here rather than in a note somewhere because it has something no
design has: it is **proven on hardware** — a phone application paired to it over
a mint-once, redeem-once enrollment code, pinned the certificate in the same
handshake, and wrote an encrypted blob the machine's own disk was then read to
confirm. That is why it is worth citing. It is not why it should be repeated.

What it costs is written into its own manifest in capitals: `cmd/rivet-sysd`
routes an application's **first** TCP port, so swapping two lines publishes the
record instead of the door. A warning in capitals is what a design looks like
when the mechanism cannot express the intent.

**Trends waits rather than growing a second socket.** Copying the workaround
into a second health service would make the transitional shape the pattern by
weight of examples, and would double the surface a later convergence has to
unpick.

---

## 4. The declaration

### 4.1 The key is `proxy-auth-paths`, and `proxy-auth` keeps its meaning

    proxy-path = "reliquary"
    proxy-auth-paths = ["none:pair", "operator:pair/admin"]

* **`proxy-auth` is untouched.** It answers for the whole route, exactly as
  `ADR-046` §4.3 has it, and it is the answer wherever no prefix says otherwise.
  Every `.rvt` already signed therefore means what it meant, and the widening
  adds a state rather than reinterpreting one.
* **A rule is `<auth>:<prefix>`.** The verdict leads, matching `tcp:8080` and
  `photo-index:api`: a reader scanning a column sees what a rule *does* before
  they see what it applies to.
* **The prefix is relative to the application's own `proxy-path`**, so `pair`
  means `/reliquary/pair`. An absolute spelling is refused rather than trimmed —
  two spellings of one rule is two texts for one fact (`ADR-011`), and the
  canonical manifest may hold one.
* **The prefix grammar is `proxy-path`'s, joined by single slashes.** Lowercase
  letters, digits and hyphens, each segment beginning with a letter, no leading
  or trailing slash, bounded in depth. `..` and `.` cannot be spelled, which is
  what lets the matcher compare segments without normalising anything.
* **It is last in `KEYS`**, appended for the reason every optional key here is:
  the canonical order is that array, and a key at the end cannot make an older
  manifest out of order.
* **`proxy-auth-paths` without `proxy-path` is refused**, for the reason
  `proxy-auth` alone is: it names a prefix under a route the manifest never
  declared, and would begin meaning something the moment somebody added a path
  beside it.
* **Two rules may not name one prefix**, whatever their verdicts. Resolving the
  pair would make the answer depend on which line an author typed first.
* **The emitter sorts by prefix and the parser demands that order.** Two
  orderings of one intent would otherwise be two artifacts, because the
  manifest's digest is in a signed header.

### 4.2 A rule may tighten as well as relax, and only one direction is dangerous

`ADR-046` §4.3's key exists **only to relax**, because a key an author must
remember in order to *tighten* security is a key an author forgets. That
property is kept: the absent key still means *require an operator*, so a
misconfiguration fails closed.

A per-path rule may nonetheless point either way, and the asymmetry is the
reason it can. **Tightening is free to get wrong** — the worst outcome is a
refused request an operator can see and fix. **Relaxing is what costs the
machine**, so every default leans away from it and §5 is entirely about the ways
a relaxation can reach further than its author meant.

The tightening direction is what makes a gated island inside an open door
expressible: a public application with one administrative prefix behind the
gate, which the other arrangement can only reach with a second socket.

### 4.3 It stays in the signed manifest, and an operator cannot grant one

An application **asks** to be reachable without a caller certificate. An
operator cannot **grant** it. There is no setting, no key in the desired state
document, and no file on the machine an edit could open a prefix through — which
is what `cmd/rivet-proxyd`'s own refusal means by *it takes a new artifact*.

The reason is sharper for a prefix than for a whole application. A whole-route
relaxation is visible in one word on the index and in one line of the journal; a
prefix is a route that reads as authenticated everywhere it is summarised, with
one door in it. If that door were editable on the machine, a text edit would
publish somebody's personal records — and the edit would look like configuration.

So the same containment applies, and the machine says so out loud instead:
**every open prefix is journalled by name**, at each derivation and at each
proxy start, with the whole address rather than the suffix, and is labelled
beside its route on the index. A signed declaration nobody can turn off is worth
having only if somebody can see it.

### 4.4 The routing table carries the rules, and a row is read whole or not at all

`ADR-046` §5's table gains fields after the fourth: one per rule, in the table's
own two words rather than the manifest's. `rivet-core/src/proxy.rs` holds both
the shape and the matcher, in `rivet-core` for the reason the table is there at
all — the supervisor writes it, the proxy reads it, and a tool reporting what is
routed reads it too.

**A row this build cannot read whole is dropped whole**: an unknown verdict, a
prefix that would escape its application, a set out of order, a prefix named
twice. A partially-understood security row is a gate with a hole in it, and a
route that is absent answers `404` rather than serving something nobody
declared.

### 4.5 Four verdicts: `operator`, `none`, `app` and `plain`

`.rulings/` R-221, designed in `ADR-061` §7:

| verdict | at the proxy on 443 | on port 80 | what the declaration says |
| :-- | :-- | :-- | :-- |
| `operator` | an operator certificate is required | not served | only the operator |
| `none` | forwarded, no certificate asked | not served | anyone may read this |
| `app` | forwarded exactly as `none` | not served | **the application checks a credential here** |
| `plain` | forwarded exactly as `none` | **served, `GET` and `HEAD` only** | anyone may fetch this before they can check the machine's certificate |

**`app` changes nothing the proxy does, and that is deliberate.** It exists so
a manifest, the journal and the index can tell a public path from an
application-authenticated one without reading the application. A phone that
signs a token with its hardware key reaches such a path through the one front
door, and the `Authorization` header passes through unchanged. **This is the
pattern for phone traffic**, and it is what §3's two-socket arrangement migrates
to.

**Declaring `plain` is what opens port 80**, on the machine where it is
declared, and nothing else opens it (`ADR-046` §7). The matching of §5 and the traversal refusal apply on
80 exactly as on 443. Both new verdicts relax, so both are journalled by name
and labelled on the index, as §4.3 requires of every open prefix.

**A proxy that predates these verdicts drops a row carrying them**, per §4.4,
so the application answers `404` until the proxy is updated. That is §6's
fail-closed direction, and the proxy ships in the image (`ADR-060`), so the
update that teaches it these verdicts is a system update.

---

## 5. Segment matching, longest match, and no normalisation

This section is the one that decides whether the mechanism is worth having. An
open prefix that reaches further than its author meant is invisible after it is
written, and the service it sits on holds sensitive personal data.

### 5.1 Matching is on whole segments, never on characters

An open `pair` answers for `/pair` and for everything under it. It does **not**
answer for `/pairings`, `/pair-admin`, `/Pair` or `/%70air`, none of which is
the same sequence of segments — and a `starts_with` on the raw string opens the
first two.

Two properties fall out of the prefix grammar rather than out of the matcher,
and both fall the safe way:

* **Case-sensitivity.** The grammar is lowercase, so an upper-case spelling
  matches no rule and takes the route's own gate. A caller who shouts is gated.
* **Percent-encoding.** Nothing is decoded before matching, so `%70air` is not
  `pair`. An encoded spelling of an open prefix is refused rather than served,
  which costs a caller a `403` rather than costing the machine.

### 5.2 The deepest matching rule answers, and no rule reads an order

A gated `pair/admin` beats an open `pair`, so a specific decision inside a
relaxed subtree holds. **Nothing consults a declaration order** — not the
manifest's, not the table's, not the order two rules were considered in. A rule
whose result depended on where it was written is a rule nobody can audit, and
the emitter's canonical sort exists so that the question cannot even be asked of
an artifact.

**The manifest asks the same question before an artifact is signed.** The
longest-match is one function, `rivet_core::proxy::requires_operator_under`,
called by the proxy's `Route::requires_operator_for` on a live request and by
`Manifest::requires_operator_at` over a manifest's own `proxy-auth` and
`proxy-auth-paths`. `ADR-051` §12 is the second caller: a `restore-path` whose
answer is *not required* is refused, so the rule that refuses an open restore
route and the rule the proxy enforces cannot disagree.

### 5.3 A path that would have to be normalised is refused

`/reliquary/pair/../blobs` matches an open rule at `pair` segment by segment,
and a backend that resolves the dot segment then serves `/blobs`. The gate and
the server would be answering about two different paths and the gate's would be
the wrong one.

So a request whose path holds a segment that is, or percent-decodes to, `.` or
`..`, or a segment holding an encoded separator, is answered `400` **before any
route is found**. Three things about that placement are deliberate:

1. **Refusing rather than normalising.** A proxy that normalised first would
   have to agree with every backend about a normalisation nobody wrote down, and
   the failure would be silent on both sides.
2. **Before the route lookup and before the gate**, so it binds a gated route as
   well as an open one. A traversal that today's verdict happens to refuse would
   otherwise become live the day that route relaxed — the guard would have been
   load-bearing without anyone having decided it was.
3. **`400`, not `403` or `404`.** The reader is usually a program that built a
   URL by joining strings, and the refusal names the segment shape so the fix is
   findable in the joining.

---

## 6. Reaching a board costs an image, and the skew is the reason to care

`cmd/rivet-sysd` derives the table and ships **inside the system image**;
`cmd/rivet-proxyd` reads it and ships **as an application**. So the two halves
of this change arrive by different routes, and the shape of the table is what
crosses between them:

| | how it reaches a board | what it carries |
| :-- | :-- | :-- |
| `cmd/rivet-sysd` | a system image, an A/B update, a trial boot and a commit | the rules, derived from manifests, written into the table |
| `cmd/rivet-proxyd` | `cargo rivet deploy` and a service restart | the matcher, the traversal guard, the index label |
| an adopter | `cargo rivet deploy` of that application | the manifest that declares a prefix |

**A supervisor that predates this ADR writes no rule fields**, so a proxy that
understands them reads whole-route rows and every route takes its own gate —
which is what it did before. That direction is safe by construction.

**A proxy that predates this ADR drops a row carrying rule fields**, because a
fifth field is a row it cannot read whole (§4.4). The application is then
unreachable rather than served at whichever verdict its root happened to carry.
That is the fail-closed half of the skew and it is deliberate: an application
that asked to be split is not one to serve undivided.

**The two orders therefore differ in what they cost, not in whether they are
safe.** Deploying the proxy first is free. Shipping the image first is free as
well until an adopter declares a prefix; from that moment until the proxy is
redeployed, that application answers `404`.

---

## 7. Consolidating onto 443 closes `ADR-046` §4.4's escape hatch

`ADR-046` §4.4 records that the proxy knows which operator authenticated and the
backend does not, and offers a way out: an application that must make
per-operator decisions *"should terminate its own TLS and sit outside the
proxy"*.

**That hatch survives mechanically and stops being usable in practice.** An
application can still declare a port the proxy does not route and check
certificates itself — nothing forbids it. But this ADR removes every reason
anybody had to reach for it, and §3 names the two-socket shape it depends on as
transitional. A design whose only remaining users are the arrangement being
retired is a design that will fall out of use without anyone deciding to retire
it.

So the identity question becomes the **proxy's** to answer rather than an
application's, and it is worth naming now while nothing depends on the answer:

**The recommendation is to build nothing yet, and to build a forwarded identity
header when a second operator arrives.** Three reasons, in the order they
matter:

1. **A header is an authorisation decision made by a header.** With one operator
   it carries no information; with two it carries all of it, and every backend
   would have to be trusted to check that the header came from the proxy rather
   than from the caller. The proxy would have to strip the header from every
   inbound request, unconditionally, and that stripping would be the whole
   security property — one line, in one program, whose absence is invisible.
2. **`ADR-046` §3's rejected alternative remains the honest answer for an
   application that genuinely needs the caller's identity**, and it costs a port
   in a URL rather than a trust assumption in every backend. An application that
   needs identity badly enough to pay that is an application that should say so.
3. **The question the maintainer has open is authority rather than identity** —
   both operator keys are equal and nothing grades what a pin may do. A
   forwarded name is worth little until there is something to grade, and
   designing the header first would settle the smaller half of a question whose
   larger half is unruled.

**A phone is not an operator, and needs none of this.** An application that
must know which phone is calling verifies the phone's own signed token on an
`app:` path (§4.5). That settles the phone half of the identity question
without a forwarded header; what remains open here is the operator half.

**What this ADR does commit to:** when identity is forwarded, it is forwarded by
the proxy, from a chain the proxy verified, over a header the proxy strips
inbound — and this section is revisited rather than extended.

---

## 8. What this does NOT do

* **It does not tell an application its base path.** `ADR-046` §8's first bullet
  is untouched: an application mounted at a prefix emits relative links or emits
  wrong ones.
* **It does not authorise.** It establishes what a caller presented, per path.
  What that caller may do is the application's question, and an open prefix is
  a decision that the application will do its own checking there — a pairing
  endpoint is unauthenticated because the code it redeems is the credential.
* **It does not decode, normalise or rewrite a path.** The remainder handed to a
  backend is the caller's own spelling, minus the route prefix.
* **It does not remove an application's ability to open a second socket**, and
  `ADR-046` §9.4's larger point stands: nothing in the platform stops an
  application binding a port it never declared.

---

## 9. What is built, what is proved, and what is unproven

### 9.1 Built and tested

The key and its grammar, the canonical order, the table's rows, the matcher, the
longest-match rule, the traversal guard, the gate's use of all of it, the
journal lines at derivation and at proxy start, and the index label.

**How the matcher was proved, and by what method.** `rivet-core`'s suite attacks
it directly and `cmd/rivet-proxyd`'s attacks the decision, because a correct
matcher consulted at the wrong moment is a hole with a green unit test behind
it. Three of the attacks:

| the request | what happens | what it proves |
| --- | --- | --- |
| `/reliquary/pairings` against an open `pair` | `403` | matching is on segments; a `starts_with` serves this |
| `/reliquary/pair/admin` against an open `pair` and a gated `pair/admin` | `403` | the deeper rule answers, in either declaration order |
| `/reliquary/pair/../blobs` | `400` before any route is found | the gate and a normalising backend cannot be made to disagree |

**Every one of them was watched to fail.** Replacing the segment comparison with
a string prefix fails five tests; making the first matching rule win instead of
the deepest fails two; making the traversal guard admit everything fails two;
making the gate ask the route rather than the path fails four, including the one
that drives a real TLS handshake. Each was then restored and re-run.

**The end-to-end shape is proved in-process over real TLS**: one gated
application, one backend, one open prefix served to a caller presenting no
certificate, and the record beside it refused to the same caller on the same
connection settings — the two requests differing only in their path.

### 9.2 Unproven

**No machine has served a per-path route.** Nothing here has been deployed, no
image carrying the supervisor's half has been built, and no adopter declares a
prefix. What is written is the mechanism and the record; migrating Reliquary off
its two sockets and giving Trends a door are separate passes.

**The version skew of §6 is reasoned rather than observed.** The fail-closed
direction is asserted by `rivet-core`'s table tests against rows this build
writes; no board has run a proxy of one vintage against a supervisor of another.
