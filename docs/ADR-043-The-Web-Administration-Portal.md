# ADR-043: The Web Administration Portal

* **Status:** Accepted
* **Date:** 2026-09-05
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

`cmd/rivet-fleetd` served a **machine status page**: a sidebar of the register
and a dashboard of one machine's own numbers. It read, and it did nothing else.

The maintainer asked, on 2026-09-05, for it to manage the box, and named five
things — stated here in this document's own words:

1. **A row of buttons along the top right** of the machine status page, from
   which the rest is reached.
2. **A Software page** listing applications, network services, shared Rivets and
   system extensions, with enable and disable for the ones that can be enabled
   and disabled.
3. **A button that fetches a screenshot** from the device and downloads it to
   the browser as a PNG.
4. **Buttons that restart, trial-restart and shut down** the machine, and one
   that makes a trial boot the default going forward.
5. **A journal viewer** — named as no small task, with the streaming and the
   filtering called out as the hard parts, and put in scope anyway.

This ADR records what that turned into, and — more usefully — the four places
where building it produced a *finding* rather than a feature. Two of those are
gaps in the machine's own surface and one of them is the maintainer's to rule
(§7).

`ADR-006` §5.1 has always foreseen a dashboard; `ADR-034` §2.2's Remote mode is
what serves it; `ADR-035` is the transport underneath. Nothing here is a new
kind of thing. What is new is that a **browser** is now an operator, and an
operator surface has obligations the read-only page did not have.

---

## 2. The two principles were constraints, and neither was spent

`cmd/rivet-fleetd/src/page.rs` opens with two, and they were written before
anything could change a machine:

| | The principle | What it forbids |
| :-- | :-- | :-- |
| 1 | **The register and the machine are two authorities kept visibly apart** | a page that blends *what the fleet was told* with *what the machine says*, and so has no vocabulary for their disagreement |
| 2 | **Nothing is cached and nothing is summarised** | a fleet server that computes a figure — which would be a second opinion about somebody else's state (`ADR-011`) |

Both survive, and the way they survive is worth stating because it was not
guaranteed:

* **The server grew no new route.** `GET /`, `GET /api/fleet` and
  `POST /mcp/<device>` are the whole surface, and they were the whole surface
  before. Every one of the four new pages is a catalogue operation carried on
  the third of those.
* **The server holds no new state.** There is no session, no job, no queue and
  no record of what an operator did. What happened to a machine is in that
  machine's journal, which is where it already was.
* **Nothing on a page is a figure this project composed.** The plan an operator
  confirms against is the machine's own prose; the software table's columns are
  the machine's own fields; the screenshot is the machine's own bytes.

### 2.1 The one place a summary was tempting, and where it went

The journal's severity, service, correlation and text filters. `rivet get log`
selects by boot and by count and by nothing else (`ADR-022` §9, and
`rivet_core::journal::Filter` implements the rest without being wired to an
argument), so a viewer either filters in the browser or does not filter.

It filters in the browser, and **the page says which half of the answer is
which**: *"the boot selector and the limit are the MACHINE'S selection;
severity, service, correlation and text are this page's, applied to what
arrived."* That sentence is the whole of the concession. A filter that hid its
own scope would be the summary this principle forbids — see §10.

### 2.2 What the browser is allowed to compute

Decoding is not summarising. The browser turns base64 into bytes, bytes into
QOI, and QOI into pixels; it reads a manifest's `key = value` lines; it parses
JSONL records. Every one of those is **a representation change over the
machine's own bytes**, and none of them produces a fact the machine did not
send. The test is whether re-asking the machine could disagree with the page: it
cannot, because the page holds no answer of its own.

### 2.3 Two facts the machine had nothing to say, and now does — added 2026-09-13 (`.rulings/` R-163, R33, R35)

Wave 6 (`docs/PLAN-Feature-Set-2026-09-12.md` §8) asked the Overview page for
the running image's version and dropped both of the old page's disk cards
without meaning to lose `/state`'s. Both were §2.1's rule working correctly
rather than a page falling short: **no `get` subject answered either
question**, so the honest choice was to say so on the card rather than
compute an answer the machine never sent — a version guessed from a card
image's own filename, or a used/total split re-derived from `get file`'s
directory listing, would both have been exactly the summary §2 forbids.

**Machine-side changes closed both**, and the page changed nothing about what
it is allowed to do:

* `get status` now carries `version`, so the Boot card reads
  `status.version` in place of the fixed sentence it used to show.
* The `/state` card reads `get storage`'s own `filesystems[].holds`, the
  same field the retired disk cards read by `mount-point` — matched on what a
  filesystem **holds** rather than where it is mounted, so the card does not
  go silent on a machine where `/state` shares a filesystem with `/`.

---

## 3. The shape

One shell and five views, in `cmd/rivet-fleetd/src/web/`.

| File | Holds |
| :-- | :-- |
| `core.js` | the transport, the catalogue binding, the router, the operation row |
| `plan.js` | the confirmation gate (§5) |
| `overview.js` | the machine status dashboard, unchanged in substance |
| `software.js` | applications, services, shared Rivets, extensions, the drawer's package facts, and the row's capture button (§6) |
| `power.js` | restart, trial-restart, power-off, the A/B slot (§9) |
| `journal.js` | `get log`, filtered and followed (§10) |
| `config.js` | the schema-driven configuration screen (§6.4) |
| `start.js` | the first call, last in the concatenation |

`page.rs` is the assembler. It was two Rust raw strings holding the whole
client; the client outgrew a string literal at the second page, and — the reason
that decides it — **each file now carries its own reasoning where a reader of
that code will find it.** The argument for why the capture button does not
press the capture belongs at the top of `software.js`, not in a Rust doc
comment three hundred lines away.

Two tests in `page.rs` hold the assembly together, because nothing at run time
checks it and the failure mode is a button that clears the screen: every entry
in the operation row has a view registered for it, and the whole document is one
file with one `<script>`.

### 3.1 The operation row is a frame, not a menu

It is drawn by the shell, once, above every view. A page that drew its own
navigation would let one view lose it.

Every dangerous button's plan opens **in place, under it**, rather than on a
page of its own — the same reason a capture, since 2026-09-13, renders inside
the Software row it was pressed from instead of navigating anywhere (§6.3).

---

## 4. A command, bound in a browser

**An operation's inputs are positional and an MCP tool call names them.** The
name is a label for a position, which `rivet_core::operation::Input` says in as
many words. So `apply restart confirm` is not
`{"subject":"restart","confirmation":"confirm"}`: it is the word vector
`["restart","confirm"]`, and `confirm` binds to whatever the *second* declared
input happens to be called — which for `apply` is `slot`.

`rivet_remote::command::bind` is the rule. `bindWords` in `core.js` reproduces
it: words fill the declared inputs left to right, and the one input a verb may
declare several-word absorbs whatever is left after reserving a word for each
remaining single-word input.

**It binds against the machine's own `tools/list`, not a table written in the
page.** Two reasons, and the second decides it:

1. A table here would be a second copy of the catalogue — `ADR-011`, and the
   same reason `rivet_remote` refuses to hold one.
2. **A fleet holds machines running different builds.** Each answers with the
   catalogue generated from its own types, so the only correct binding for a
   given machine is the one that machine published.

The cost is one small call when a machine is selected. It is dropped and read
again on the next selection, because a binding held across machines would be
the cache principle 2 forbids.

### 4.1 A refusal is not an error, and reading it as one loses the plan

`cmd/rivet`'s MCP adapter makes a refusal **a successful call carrying
`isError`**, so that an agent can tell *Rivet is broken* from *Rivet said no*.
The status page read only `content[0].text` and would have rendered a refusal as
an answer.

That is not a tidiness point. **For a confirming operation the refusal IS the
plan** — `E_CONFIRMATION_REQUIRED` with the plan in `ADR-006` §4.1's structured
payload slot. A client that collapsed the two would have thrown away the one
thing the operator has to read.

---

## 5. The confirmation gate

`ADR-012` §7.1 requires confirmation before apply, and the machine already
enforces it: an operation whose `confirms` is true does nothing without the
literal last word `confirm`, and answers with its plan instead.

The portal's whole addition is `plan.js`:

1. send the command **without** the confirmation word;
2. render the plan the machine answered with;
3. send it again **with** the word, only after somebody has pressed a button
   beside that plan.

**Nothing in the renderer composes a consequence.** It walks the plan's fields —
`consequence`, each `effect`, `untouched`, `takes-effect`, `confirm-with` — and
prints them. A page that summarised a reboot's consequences would be a second
account of them and the operator would be confirming against the wrong one.

Three details that are decisions rather than rendering:

* **`requires-reboot` absent is a third answer**, not a "no". `TakesEffect`
  omits the field rather than defaulting it, exactly so that a caller cannot
  read *not asserted* as *no restart needed* (`ADR-034` §5.3), and the gate
  marks that case rather than rendering nothing.
* **A refusal that is not a plan is shown as the machine's own refusal**, not as
  a failure. `apply service-restart` on a disabled service is the ordinary case,
  and its refusal says what to do instead.
* **An operation that ANSWERS instead of planning is reported as a defect in the
  machine's build.** Not reachable through anything in this build; rendered
  anyway, because if it ever happens the operator has to be told the machine
  acted without being confirmed.

### 5.1 Why a bare button would have been worse than the CLI

`cargo rivet run -- apply restart` prints a plan and refuses. A portal that
fired a reboot from one click would be a **worse** operator surface than the
command line it is meant to improve on. That is the argument for building the
gate before building any button that uses it, and it is why the gate is its own
file with its own tests-by-inspection rather than a helper inside `power.js`.

### 5.2 A write another site caused never reaches the gate

The browser presents the operator's certificate to any request to the portal,
whichever page caused it, so a page on another site could `POST /mcp/<device>`
or `POST /api/commit/<device>` as `text/plain` — a request that needs no CORS
preflight — with the operator's identity. `cmd/rivet-fleetd/src/main.rs`'s
`drivable` refuses both routes unless **both** of these hold:

* **`Content-Type: application/json`.** A page on another site cannot send it
  without a preflight, and this server answers none. This half is the portal's
  own policy about its administrative endpoints and stays in `rivet-fleetd`
  (`ADR-040` §8).
* **No other site caused it**, by `rivet_core::cross_site::is_cross_site`:
  `Sec-Fetch-Site` must be `same-origin` or `none` when the browser sent it;
  with no `Sec-Fetch-Site`, an `Origin` must equal this server's own origin,
  port included; a request carrying neither passes, because `curl` and every
  other non-browser client sends neither.

**The second half is the rule `rivet-proxyd` applies to every application**
(`ADR-046` §4.5, `.rulings/` R-186 D1), in `rivet-core` so there is one copy of
it. A refusal answers JSON-RPC error `-32600` before any handler, register read
or commit runs, and is printed with the request line. The tests in
`cmd/rivet-fleetd/src/main.rs` drive `drivable` and `dispatch` with request
text: each safelisted content type refused, `Sec-Fetch-Site: cross-site`
refused even with JSON, an `Origin` from another site (or from the proxy's
origin on 443) refused, the page's own request and a header-less JSON call
allowed.

### 5.3 The operator is a client certificate that names no host

`.rulings/` R-187 D1. **The 4443 handshake establishes that the fleet authority
signed the caller's key, and that alone does not make it the operator.** The
authority signs two kinds of leaf: every fleet machine's `rivet-proxyd` front door
(`cmd/rivet-provision/src/stage.rs`, through `issue_from`), and the operator's
browser certificate (`Authority::operator`). Neither carries an extended key
usage or any other mark. What tells them apart is what each is for: a machine's
leaf is presented to browsers under the names that machine answers to, so it
always carries them in `subjectAltName`; the operator's identifies a person and
carries none.

**So `rivet-fleetd` serves a caller whose certificate names no host, and refuses
every other.** `serve_one` classifies the verified leaf with
`rivetos_tls::classify_caller` — the one predicate `rivet-proxyd` asks too
(`ADR-046` §4.1) — and hands it to `admit` before anything else runs:

* **The rule is the presence of `subjectAltName`**, whatever its entries: DNS
  names and IP addresses are what a machine's leaf carries, and any other kind is
  still not the operator's shape.
* **The subject's `commonName` is not consulted.** Nothing authenticates by the
  subject (`ADR-028` §4.5), and an operator's name is whatever was typed at
  minting, a dotted one included. A machine leaf with no `subjectAltName` would
  be accepted as that machine by no browser or rustls client, and `issue_from`
  refuses to mint one.
* **A certificate that cannot be read as far as its extensions is not the
  operator's.** Truncation, trailing bytes, a missing version, a malformed
  extension, two `subjectAltName`s, an empty one or an undefined entry tag each
  refuse.
* **Every request is gated, the portal page included.** `/api/*` and `/mcp/*`
  drive the fleet and the page is the operator's own; no fleet machine has a
  reason to ask this server for any of it. A gate on the whole server cannot be
  missed by a route added under a new prefix, and a machine's key learns nothing
  here but the refusal.
* **`dispatch` takes an `Operator`, which only `admit` makes**, so no handler can
  be reached with a caller nobody classified.
* **The refusal** is `403 Forbidden` with JSON-RPC error `-32003`, naming the
  host the certificate names, and a `REFUSED` log line with the certificate's
  fingerprint and the request line, as §5.2's refusal is logged.

**Provisioning keeps the rule true.** `issue_from` refuses an empty or blank
name list, so no machine leaf it mints can pass as the operator, and tests pin
that every certificate provisioning mints for a machine names its host — every
`issue_from` caller and every role — and that the operator's names none
(`cmd/rivet-provision/src/ca.rs`, `stage_tests.rs`, `main.rs`). An explicit
operator marker replaces this inference when browser certificates are next
minted (`ADR-052` §10); until then no certificate is reissued for it.

`cmd/rivet-fleetd/src/gate_tests.rs` proves it through `serve_one` over real TLS,
with certificates minted by `cmd/rivet-provision`'s own authority module: the
operator reaches `GET /api/fleet`, `POST /api/commit/<device>`,
`POST /mcp/<device>` and the page; a member's front-door leaf and this server's
own are refused each with `403`; a certificate from another authority, or none,
is refused at the handshake.

---

## 6. Software: installing is not running

`ADR-015` §3.2.2 splits the two authorities and the page keeps them split:

| Fact | Whose | How the page reads it |
| :-- | :-- | :-- |
| what an application IS — restart policy, what it listens on, what it connects to | the signed manifest | `get file /state/app/<n>/current/manifest.toml` |
| whether the operator wants it running | the desired-state document | `get status`, and `get desired-state` for the text |
| what is installed at all | the installer's directory | `get file /state/app` |
| shared Rivets | the installer's library root | `get file /state/lib` |

So a row carries both, and the two can legitimately disagree: a thing can be
installed and disabled, or enabled and not installed. The second is ordinary —
a fleet layer writes configuration for software it is about to install, and
`rivet-config`'s `services` module argues that at length.

**Manifests are read one at a time after the table is on screen.**
`rivet-fleetd` holds one connection per request against a cap of sixteen, at
most twelve of them from one address (`ADR-035` §12), and a machine with twenty
applications asked in parallel is twenty connections. The row shows what it has
and fills in the rest.

### 6.1 Enabling and disabling

There is no `set` verb (`ADR-006` §6.4 has one and this build does not
implement it) and no desired-state `apply`. So the change is `apply edit` on
`/state/config/machine.toml`, composed against the document's own text:

* the exact `[services.<name>]` header and its `enabled` line **as they appear**
  become `--expect`, and the same text with the value flipped becomes
  `--replace`;
* the browser counts the occurrences before anything is sent, because the
  operation refuses text that occurs other than exactly once and a refusal the
  page can predict is one the operator should not have to read a plan to
  discover;
* a service the document does not mention **at all** gets `--append` instead —
  and only then, because appending a second `[services.<name>]` would make the
  document refuse to parse.

`apply reload` is a separate, explicit act on the same page. An edited document
is not a changed machine.

### 6.2 System extensions

`ADR-016`'s extension is designed and is not built. This build's package kinds
are `update`, `application` and `library`; there is no extension token and no
operation installs one. **The page carries the section and says that**, because
a missing section reads as an oversight and the maintainer asked for extensions
by name.

### 6.3 The drawer, and the capture on the row — added 2026-09-13

`BRIEFING-001`'s review of the first cut found the drawer added nothing the
collapsed row did not already show, apart from two buttons. The fix put the
buttons ON the row and put what was actually missing INTO the drawer: the
package's version and when it was installed (read from `current` and the
version directory it points at — `crate::package::install` names a version
directory for the version, so the symlink's target IS the version), its
location on disk, and the manifest's own declarations spelled out rather than
only summarised as pills.

**The size is read, not walked** — R34, `.rulings/` R-163, 2026-09-13. It
used to sum the version directory one level deep itself, because `get
file`'s own `size_bytes` is a directory's inode weight and not what is
inside it, and named the approximation: a directory nested deeper than one
level went uncounted. The installer now records the true total once, at
install, in a fifth file beside the retained header — it already reads every
member's bytes to write them, so it sums there instead of walking a second
time — and the drawer reads that one file. **An application installed before
this existed has no record and the drawer says so; there is no fallback
walk** (`.rulings/` R-163.1).

**The Screenshot page (formerly §8) is retired into this row.** Every
application's capture lives at a path that is a function of its name alone —
`/state/app/<name>/data/screen.qoi` — so a dedicated page whose only job was to
pick a path and fetch it had nothing left to justify existing once the row
itself could offer the button. The reasoning for why the button converts and
does not press stands unchanged from the retired page and from
`cargo rivet screenshot`'s:

1. **The launcher refuses to capture while an application holds the display**,
   correctly — the refusal goes to the launcher's own log and not to the
   control plane, so `apply input capture` still succeeds. A button that
   pressed first and fetched second would report success and hand back a real
   picture of a moment nobody asked about.
2. **There is no freshness signal this end can trust.** `modified_ms` is the
   board's own clock and a Pi has no RTC.
3. **Every application's capture lives in its own data directory**, so which
   file the press produced depends on which application it is.

Whether the picture is of the machine right now depends on whether the row is
the one `status.display_leased` names; a row that is not says so, briefly,
rather than letting the operator assume otherwise.

### 6.4 The configuration screen reads the target machine's own schema — added 2026-09-13

`docs/PLAN-Feature-Set-2026-09-12.md` §5.3. The Config view asks the **target
machine**, not this server, `get schema` — through `POST /mcp/<device>`, the
same route every other view already uses — and builds its form from exactly
the JSON Schema that machine answers with: `schemaFields` walks `properties`
in the object's own declaration order and `extractGates` reads each `allOf`
block's `if`/`then`/`else` structurally, so a field whose condition is not met
(the gateway under `method = "dhcp"`) is not offered. **This server never
renders its own compiled-in schema.** The September 10th work did, and a
member running a different `rivet-config` build gets the wrong form from that
— the whole reason this is asked of the machine and not assumed.

`cmd/rivet-fleetd/src/formfields.rs` carries the same algorithm, proved in
`cargo test` against `rivet_config::schema::schema()` — the real schema, not a
hand-written fixture — because that Rust is the one place this order and these
conditions can be tested at all; `web/config.js` is the transliteration a
browser actually runs, and nothing at run time checks the two agree.

**Mode and `[kiosk]`** (`ADR-007` §7.10): the form shows `mode` and `[kiosk]`'s
own fields independently, exactly as the document does — changing one never
touches the other. When the document reads `mode = "kiosk"` with no `[kiosk]`
table, the screen says so beside the `kiosk` panel rather than showing an
ordinary empty one, on the same footing `overview.js`'s own status card
already reports it (`b64c4470`); neither refuses it.

**Submitting composes the whole document and commits it to the fleet
repository — `.rulings/` R-167 R39.** The reading, the ordering and the
conditions above are unchanged; only the write path is. This reads the
current text with `get desired-state`, regenerates it from the form, and
POSTs the result to `POST /api/commit/<device>` on THIS server —
`cmd/rivet-fleetd/src/commit.rs` — which commits it onto the branch
`fleet-repository`/`fleet-branch` name (`ADR-044` §5.4), in the repository this
server's own control plane holds.
`ADR-044` §1.1 is why: a machine's desired state lives in the repository and
the machine converges from it (`ADR-044` §6, the same poller every other
fleet-as-code write already goes through), so `apply edit` writing the
machine directly was the one place this portal disagreed with its own
source of truth. `apply edit` still exists and this screen no longer calls
it. **A section or key this screen's schema walk does not recognise — a
manifest-defined `[applications.<name>]` table, a comment, a reserved
`[network.resolver]` sub-table — is read back and written out unchanged**,
never regenerated from a shape this build cannot see.

**The route itself is tested, not only `commit::handle`** — `.rulings/`
R-169, R41. `cmd/rivet-fleetd/src/main.rs`'s `dispatch` function is
`serve_one`'s routing pulled out into a plain function of request text, so
it can be driven directly with no TLS handshake and no socket, the same way
`drivable` and `read_request` already are. Four tests prove: a same-origin,
JSON-typed `POST /api/commit/<device>` reaches the commit branch; a
cross-site request, and one with the wrong `Content-Type`, are refused by
the same guard `POST /mcp` already uses, before `commit::handle` or even
`fleet::load` runs; and a `GET` to the same path matches none of
`dispatch`'s `POST`-prefixed branches, so it is never routed to the handler
at all.

`cmd/rivet-fleetd/src/repo.rs`'s `Repository::commit_file` is the write: a
compare-and-swap on `ADR-044` §6.6's own footing — refused, with nothing
committed, if the document at the tip of the branch no longer matches what the
form read — and then a commit asked of this server's `rivet-mcpd` on that tip,
which it refuses as a stale head if the branch moved in between (`ADR-044`
§5.5). `rivet-fleetd` runs no git and holds no clone; the commit is git's
plumbing on the served bare repository, and nothing is checked out. The commit
message names what was touched, composed
client-side from the fields the operator actually changed rather than
diffed server-side from two whole documents.

**With no fleet repository configured, Save is disabled** and the screen
says which setting turns it on. `GET /api/fleet` carries `fleetAsCode` —
this server's own `fleet-repository` setting, read the same way
`crate::code::pass` already does — precisely so the Config screen can say
why, rather than offering a button that always fails once pressed; this is
this server's own fact about itself, not a second opinion about a machine,
so it does not weaken principle 2 (§2).

**A credential-shaped `[applications.*]` setting is refused, server-side,
before a commit is even attempted.** `ADR-007` §7.3's schema has no key a
credential could be written under, but `ADR-038` §4.2's application region
is a pass-through the schema deliberately does not validate — which is
exactly the shape `rivet-01`'s own document carries, `[applications.
tempmon-rivet] write-token` in clear text, found while building this. So
`commit.rs` scans every `[applications.*]` key for a name containing
`token`, `secret`, `key` or `password` (case-insensitively —
`cmd/rivet-fleetd/src/toml_doc.rs`'s `credential_like_settings`) and
refuses the whole write if it finds one, naming the table and the key and
pointing at `apply install-secret` (`ADR-038` §5) as where the value
belongs instead. **Where `rivet-01`'s real token should actually live is
the maintainer's to decide** — this guard only ever refuses; nothing here
relocates it.

This replaces the ad hoc editor the September 10th work left behind, which
read and wrote a *git repository* — `/api/config/<name>`, since removed —
with no plan gate at all and no credential guard. `commit_file`, removed
with it at the time, is back under this section's name and this one goes
through the confirmation gate `web/plan.js` established (§5), even though it
is committing to git rather than asking a machine to plan an `apply`.

**`timezone` is a dropdown, not free text — `.rulings/` R-168.** The target
machine's own `get schema` now carries an `enum` on `timezone` when its
image ships a readable zone database: `cmd/rivet`'s `PiMachine::
configuration_schema` reads every real tzfile under `rivet_core::etc::
ZONEINFO_DIR`, checked byte for byte against musl's own test
(`rivet_core::timezone::is_tzfile`), and `rivet_config::schema::
with_timezone_zones` — pure, so `rivet-config` itself performs no I/O
(`ADR-014` §3.1) — overlays the list onto the schema this method already
built. A machine whose database cannot be read answers exactly as before:
`timezone` as free text, with its own description still explaining the
IANA-name rule, never a refusal of the whole subject over one unreadable
directory. This screen needed no change at all for the dropdown to appear —
`field.shape.enumValues.length > 0` already rendered a `<select>` for
`method`, and a schema-declared `enum` is a schema-declared `enum`
whichever property carries it.

**`version` is visibly not editable — `.rulings/` R-168.** It always was
disabled; what changed is that a locked field now gets its own bordered box
and a sentence — *"this is the settings FORMAT version this build reads, not
a setting on the machine"* — rather than a disabled input and a four-word
label a reader could miss, which is what *"can we make it more obvious"*
was asking for.

---

## 7. OPEN — the reachability set has no structural signal

**This is the maintainer's to rule and the portal is doing the conservative
thing until he does.**

`ADR-032` §3.2.1 rules that a reload applies everything except a named
reachability set — today the control plane and the interface bring-up — because
stopping one of those takes away the means of undoing it. `cmd/rivet-sysd`'s
`definitions::REACHABILITY_SET` is the list and the supervisor is its authority.

**A boot withholds nothing.** `Occasion::Boot` applies the whole document. So a
document that disables the control plane is applied at the next restart, and on
a headless board there is no second way in (`ADR-035`). The hazard is not
the reload; it is the reload's *withholding* being mistaken for a guarantee.

A portal that offered that toggle would be offering a change with no remote
undo. It therefore does not — and to know which services those are without
holding a second copy of the list, it asks:

> `apply service-restart <name>`, planned and not confirmed, carries
> `service_restart::caveat` in the plan's `untouched` for exactly the services
> in the set.

Measured on the bench board on 2026-09-05: `rivet-mcpd` and `rivet-netd` carry
it, `aura-renderer` does not.

**That is prose-matching and it is a real weakness.** `Restarting::reachability`
already exists as a boolean, is set from the supervisor's own list, and crosses
the supervisor's socket — and it is **folded into the result's `next` sentence
instead of being published in `service-restart`'s OUTPUTS.** A reworded caveat
would make the portal stop warning, silently, in the safe-looking direction.

| Option | Cost |
| :-- | :-- |
| **A** — publish `reachability` in `service-restart`'s OUTPUTS | one field in an interface contract; every adapter gets it; the portal reads a boolean. `AGENTS.md` §1 makes it the maintainer's |
| **B** — leave it, and keep matching the caveat | free today, wrong the day somebody improves the sentence |
| **C** — hardcode the list in the page | `ADR-011` §3 exactly: two places to change, and the disagreement presents as a machine that refuses a toggle the page offered |

**Recommendation: A.** The field is already computed and already crosses one
boundary; publishing it costs a line in `OUTPUTS` and removes the only place in
this portal where a safety decision is made by reading prose. C is refused
outright.

---

## 8. Retired 2026-09-13: the capture moved onto the Software page's row. See §6.3.

### 8.1 Retired 2026-09-13: folded into §6.3.

---

## 9. Power, and the operation the maintainer was reaching for

Four operations, all through §5's gate: `restart`, `trial-restart`, `power-off`,
and `trial-boot <slot>` armed against whichever slot `get slot` calls inactive.
The page will not offer an arm at all when the two authorities disagree about
the running slot, because `get slot` itself says nothing should be armed until
that is understood.

### 9.1 There is no commit operation, and PID 1 is why

The maintainer asked for the button that makes a trial boot the default. **The
word is `commit`, and no operation in the catalogue performs it.** Verified two
ways, because `AGENTS.md` §7 is about exactly this kind of sentence:

* `get catalogue` on the bench board lists `apply`'s nineteen subjects and none
  of them commits;
* `rivet-core/src/operation/` holds one file per subject and there is no commit
  among them.

**PID 1 does it, on its own.** `cmd/rivet-initd`'s `confirm` module runs four
gates — the mount phase, the configuration document, the supervisor, and
*whether anything has reached the control plane on this boot* — and then makes
the running slot the default. A trial nobody connects to is never committed, and
`ADR-018` §5.1 rolls it back on the next reset with no userspace involvement at
all. That is the safety property, not a fault.

So the honest surface is not a button with nothing behind it. It is:

1. `get slot`, so an operator can see that this boot is a trial at all, and
2. **PID 1's own record**, out of the machine's journal.

### 9.2 And that record is at the START of the boot

`get log` keeps the **newest** records when a limit cuts, and PID 1 writes its
confirmation in the first minute of a boot. So a search for it against the
200-record default comes back empty on a machine that has been up ten minutes,
and reads as *this boot was never committed* — which is `PLAN.md` §104.10's
failure pointed at a much more expensive conclusion.

The button asks for `all` and says why on its face. On the bench board on
2026-09-05, in a trial boot on slot `b`, it answered with the machine's own
line: `rivet-initd: boot confirmed: partition 2 is now the default`, found among
1218 records of the 23 614 that machine holds.

### 9.3 What is still not answerable

**No operation reports which slot is the DEFAULT.** `get slot` answers what this
boot is running and whether it is a trial; the default lives in `autoboot.txt`
on the first FAT partition, which is not mounted at runtime. So *"has this trial
been committed"* is answerable only from the journal, and only for the boot the
journal still holds. Recorded as a gap rather than worked around: a `default`
field on `get slot` would be the structural answer and it is a new reading on an
existing subject, which is the maintainer's to want.

---

## 10. The journal

The three rules in §2.1's concession, stated as behaviour:

1. **It never sends a bare `get log`.** The limit is always explicit and always
   shown in the control that chose it; `all` is among the offered words, because
   *give me nothing* and *give me everything* must not be the same request.
2. **`truncated` is rendered loudly whenever the machine sends it.** It is the
   only field that separates *the limit cut this* from *you selected one boot
   out of thirteen*, and `returned` < `held` looks identical either way.
3. **A filter that matches nothing says how many records it looked at and how
   many the machine holds.** *"No records"* and *"no records among the 2000 you
   fetched"* are different sentences and only one of them is true.

`unreadable` and its detail are rendered too. A log that answers *there is
nothing about that* when it in fact could not parse four lines is the one
failure a log must not have.

### 10.1 Following is polling, and is named as polling

`ADR-035` §4A's cursor is the mechanism and `cmd/rivet`'s `follow` module is the
same loop on a workstation: hold the cursor, call again. The viewer does that on
a three-second timer.

**It is not called live.** The operation answers once from what is on the card
at the moment it is asked; following is a property of the transport and never of
the operation, so a view that called itself live would be claiming something the
machine does not offer. A timer whose view has gone is stopped, or the page goes
on asking a machine nobody is looking at.

### 10.2 The read that made this possible at all

`node::read_one_line` took one `read` per byte through the `rustls` state
machine. `get log latest all` off the bench board is **393 644 bytes**, measured
on 2026-09-05, and the whole-journal read is not a corner case — §9.2 needs it.
It reads in chunks now, with the same frame: one request, one connection, one
line.

---

## 11. What was rejected

| Rejected | Why |
| :-- | :-- |
| A server-side job or queue for long operations | state on the fleet server, and a second record of what happened to a machine. The machine's journal already is that record |
| Server-side rendering of any machine fact | principle 2. The server would become a second opinion |
| A hardcoded reachability set in the page | `ADR-011` §3 — see §7 option C |
| Server-side QOI-to-PNG conversion | the browser has an encoder; putting one in the fleet server would add a dependency to a program that terminates TLS, for no gain |
| `--follow` as a server-held stream | the operation does not follow (`ADR-022` §9.5 makes it the transport's); a stream would need the server to hold a cursor per viewer |
| Pressing `input capture` inside the capture button | §6.3 — it reports success and returns the wrong frame |
| A Commit button for a trial boot | §9.1 — there is nothing to call |

---

## 12. What this does not do

Stated so the absence is not read as an oversight:

* **No install and no uninstall.** `apply install` takes an absolute path to a
  `.rvt` **on the machine**, and there is no operation that puts a payload
  there. Getting one onto a board is `cargo rivet deploy`'s job from a
  workstation, and a portal upload would need a route and a staging directory —
  which is a shape decision, not an implementation.
* **No fleet-wide action.** `ADR-012` §7.1 already rules that the unit of
  confirmation is the change and not the target, so one plan confirmed once
  across a scope is the right shape. Nothing here builds it.
* **No editing beyond what a schema-driven form can express.** §6.4's Config
  screen composes the whole document from the target machine's own schema
  and commits it to the fleet repository (`.rulings/` R-167 R39); §6.1's
  `enabled` toggle still composes one line and sends it with `apply edit`,
  because that remains the only write path a RUNNING machine's document has
  outside fleet-as-code (`ADR-006` §8.9). A section this build's schema
  declares no shape for — a manifest-defined `[applications.<name>]` table —
  is read back untouched rather than edited, on the same footing as before.
* **Nothing is authenticated differently.** The browser presents a client
  certificate to the fleet server and the fleet server presents the operator
  identity to each machine, exactly as before; §5.3 is how the fleet server
  tells that certificate from a machine's. **An operator who can open this
  page can already run every one of these operations** through
  `cargo rivet run`; the portal adds no authority.

---

## History

### 2026-09-14 — §5.3 was a known defect: the handshake admitted every fleet machine

§5.3 recorded, proved by a test that was `#[ignore]`d and failed when run, that
the 4443 handshake was the whole check:

> ~~`rivet-fleetd` requires a client certificate chaining to `ca.crt` and asks
> nothing more of it: `dispatch` is handed the caller only as a fingerprint for
> its log lines.~~ ~~So a fleet member's front-door key completes the 4443
> handshake and is served `GET /api/fleet`, `POST /api/commit/<device>` and
> `POST /mcp/<device>`, the last carrying this server's operator identity to any
> machine in the register.~~

**Why it changed.** Asked in chat, the maintainer chose the recommended remedy
(`.rulings/` R-187 D1): tell the operator apart by the one property the two
kinds of leaf already differ in, enforced at once in both servers, with no
certificate reissued, and an explicit marker later. No fleet machine called any
of these routes, so refusing members cost no caller.

### 2026-09-13 — §6.4's Save used to write the machine directly

Wave 3c built the Config screen's write path as `apply edit <path> --expect
<old> --replace <new>` straight to the target machine, through the same
confirmation gate every other dangerous button on this page uses. It ran
unmodified on `rivet-01` and was never proven against a fleet repository
because fleet-as-code's own write path (`ADR-044`) had not been asked to
carry a browser's edit yet.

**Why it changed.** Wave 8's review (`.rulings/` R-167 R39) found this ran
against `ADR-044` §1.1's own single source of truth: a machine's desired
state lives in the fleet repository and the machine converges from it, and
every OTHER write this server makes already goes through that repository.
A document edited by hand and sent straight to a machine was the one write
path that disagreed with the portal's own architecture — and it was also
the reason the TOML round-trip in `web/config.js` had never run against
real bytes: nobody could press Save on `rivet-01` until it was proven,
because the deployed build wrote the machine directly (`STATE.md` §0.2 item
1, before R39 landed). Building the round-trip test (now
`cmd/rivet-fleetd/src/toml_doc.rs`) against a copy of `rivet-01`'s real
document is what surfaced that the naive port of `parseDocument`/
`serializeDocument` adds a trailing newline the source did not have — see
that module's own documentation.

The same review is also why `write-token` under `[applications.
tempmon-rivet]` was found in clear text on `rivet-01`'s real document:
`ADR-038` §4.2's application region has no schema to refuse it with, so a
value that should have gone through `apply install-secret` had been
written as an ordinary setting instead. Building the commit path made that
unsafe in a NEW way — publishing it to a git repository every clone and
reviewer can read — which is why R39's build added the credential guard in
§6.4 rather than only the commit itself.
