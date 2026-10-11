# SPEC-04: Aura Cards — what an addon may say to the screen

* **Status:** Normative for `aura-card` `0.1.61` and later. Every row in §12
  marked *built* compiles and its tests pass; **parts of it run on a Raspberry
  Pi 4** — and §12 says which,
  line by line, with the method beside each. (This line said *nothing here has
  run on hardware* and was corrected 2026-09-02, the same day the sentence
  stopped being true; §12's own copy of it was corrected and this one was not,
  which is `AGENTS.md` §2 rule 4 inside one file.)
* **Date:** 2026-09-01, amended 2026-09-02 (§4.5, §5.6.1, §5.8 — `.rulings/`
  R-126) and **2026-09-03 (§1.0, §4.1, §4.2, §12 — `PLAN.md` §119)**
* **What the 2026-09-03 amendment corrected, because it is the kind a spec
  should not survive quietly:** this document described the renderer as if
  cards were its only input. They are not, and the omission meant Aura's whole
  `cmd` channel — brightness, the screensaver theme, the weather toggle, corner
  rotation, sleep and wake — had nowhere to live and was never ported. §1.0 now
  states the boundary. The weather is no longer an addon (§4.2), and
  `aura-bridge` is deleted.
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Artifact kind:** `application`
* **Decides:** the vocabulary an Aura addon speaks, and the transport it speaks
  it over. It does **not** decide whether Aura ships enabled, what it looks
  like, or which addons exist.
* **Builds on:** `ADR-015` (packaging and sandboxing), `ADR-036` (the SDK),
  `SPEC-03` (what a Rivet hands an application), `ADR-015` §5.3
  (`provides`/`connects`), `PLAN.md` §106 (the display lease)
* **Derived from:** Aura, a separate repository — `cpp/src/message.hpp`,
  `cpp/src/widgets.hpp`, `cpp/src/main.cpp`, `cpp/src/ss_solid.hpp`,
  `cpp/src/ss_matrix.hpp`, `cpp/src/ss_boxes.hpp`, `docs/ipc-schema.md`,
  `cmd/aura-controller/main.go`, `internal/portal/portal.go`. Every shape,
  role, state and glyph below names the code it came from.

---

## 1. Scope

**This is the contract between the Aura renderer and anything that wants to put
something on the screen it holds.**

It is language-neutral by construction, and it is held to `SPEC-03` §1's
standard for a Rivet contract: everything below is a socket, a line of text, or
a manifest key, and a second language should be able to consume it in about two
hundred lines. `aura-card` is one implementation of this document, for Rust.

### 1.0 THIS IS THE ADDON WIRE, AND IT IS NOT THE ONLY ONE THE RENDERER SPEAKS

**Added 2026-09-03, because the omission cost a day.** The renderer has two
inputs and this document governs one of them:

| | Wire | Governed by |
| :-- | :-- | :-- |
| **addons** | cards on slot sockets | **this document** |
| **the controller** | `state` and `cmd`, newline JSON on `aura-controller:kiosk.sock` | `Aura/docs/ipc-schema.md` |

**What went wrong when that was not written down.** This document models cards
and only cards. A renderer built to it, and to nothing else, has nowhere to put
a *directive* — and Aura's controller sends six of them (`brightness`,
`set_screensaver`, `show_weather`, `rotate_corner`, `display_sleep`/`_wake`,
`screenshot`). The whole `cmd` channel was therefore not deferred or stubbed but
**absent**: the portal saved a theme and a brightness that reached nothing, and
the screen stayed green while the configuration said *Ice Blue*. `PLAN.md` §119.

**So: a card is not the only thing that may reach the renderer, and this
document must not be read as if it were.** What a card can say is bounded here.
What the controller may say is bounded in Aura's own schema, and the renderer
speaks it directly — there is no translator between them, and the one that
briefly existed is why the gap went unnoticed.

**An addon still has no directive channel, and that is deliberate.** An addon
publishes a card and nothing else; brightness and the background belong to the
machine's owner through the controller's portal, not to whoever wrote a card
producer. If an addon ever needs one, it is a change to this document and
should be argued here rather than added quietly.

### 1.1 What an addon author should read first

`SPEC-03` — **the whole of it**, and §1.1 of it first. An addon is an ordinary
Rivet application: it is handed its sockets, told where it may write, and
started by a supervisor that composes its whole environment. Nothing in this
document works around anything `SPEC-01` §5.2 says a Rivet does not have.

Then `SPEC-03` §3.4, because the slot model is `connects` and nothing else.

---

## 2. Context, and the rulings this implements

Aura is a smart ambient kiosk display: a clock, the weather, an indoor
temperature and one more reading from an addon, drawn over an animated
background on a television that is never switched off. Today it is a Go controller pushing
newline-delimited JSON to a C++/raylib renderer over a Unix socket, and it runs
on a Rivet (`PLAN.md` §104.29).

**The maintainer has ruled that Aura becomes Rivet's built-in demo, rewritten in
Rust as a Rivet application, and that the shipped version carries only two
cards: date/time and weather.** The corner reading card's source and the
indoor temperature move out into separate addon projects outside this
repository, and Aura's host knows nothing of what the reading card shows
(`.rulings/` R-375 D1).

That ruling creates a problem this document is the answer to: **an addon in
somebody else's repository has to be able to put something on a screen it can
never draw on.** Four rulings settle the shape, and none of them is reopened
here:

| Ruled | Consequence in this document |
| :-- | :-- |
| **An addon publishes a card, not pixels.** The launcher holds DRM master and cuts exactly one lease to exactly one application (`PLAN.md` §106), so there can never be a second process drawing on that screen | §4, §6 — an addon is an ordinary application, is handed a socket, and sends a declared card |
| **No `dlopen`.** Loading third-party code into the process holding the display lease is not acceptable | The transport is a socket. Nothing in this document is a plugin interface |
| **Slots and shapes, not pixels.** An addon chooses from a closed vocabulary and occupies a slot; it cannot invent a visual | §3, §5. A future addon needing a shape §5 lacks is a change to Aura, not a licence to draw |
| **An addon never sends a colour — it sends a meaning** | §5.1. Five roles; the theme resolves them |
| **There are TWO KINDS of addon.** One claims a slot and produces its own card; the other claims nothing, draws nothing, and produces data for a producer that owns a card (`.rulings/` R-126, 2026-09-02) | §4.5 — the second kind is a **contributor**, declared with `provides`/`connects` one level down. It is what let §5.8's two-column panel exist without §5.2's rule being relaxed |

---

## 3. Slots

### 3.1 Four, and why four

| Slot | Wire name and endpoint name |
| :-- | :-- |
| the largest card | `primary` |
| the second | `secondary` |
| the third | `tertiary` |
| the fourth | `quaternary` |

**Four because Aura has four corners and rotates through them.**
`cpp/src/main.cpp` moves the clock to a different corner every two to eight
minutes (`RandomCorner`, `RandomInterval`) because the panel is on all day and a
static bright rectangle burns in. The clock and the weather panel own one
diagonal between them and the original renderer's corner rule puts the reading
card a quarter-turn from the clock, so the real arrangement has always been at
most four cards. A
fifth would have nowhere to go.

### 3.2 A slot is a prominence, never a position

The position rotates, so an addon that asked for *the top left* would be asking
for something that is true for four minutes at a time — and the burn-in rule
would become the addon's business rather than the renderer's. The ordinal names
say the one thing an addon has a legitimate opinion about: how much of the
screen this deserves.

### 3.3 Claiming a slot is a manifest declaration, not a runtime negotiation

The renderer declares:

```toml
provides = ["socket:primary", "socket:secondary", "socket:tertiary", "socket:quaternary"]
```

An addon declares one of:

```toml
connects = ["aura-renderer:secondary"]
```

**The endpoint name IS the slot's wire name.** `aura-card` asserts it
(`a_slots_wire_name_is_a_legal_manifest_endpoint_name`), so the two cannot
drift apart.

Three consequences follow from `SPEC-03` §3.4 and **Aura implements none of
them** — the platform already does:

* **Two addons wanting one slot.** `provides`/`connects` is enforced as a
  `chown` to one uid, so it is **one consumer per endpoint**. Two addons
  declaring the same slot are **both** refused by name at launch and **neither
  is started** (`ADR-015` §5.3): the supervisor does not pick a winner, so an
  addon installed later cannot take a slot from the one holding it. Aura
  arbitrates nothing and has no code for it.
* **An addon reaching a slot it was not granted.** The socket is `0600` and
  owned by somebody else; the kernel refuses it. The same harness's interloper is the
  proof: same machine, same binary, same hard-coded path, refused — the only
  difference is the manifest.
* **An addon whose slot does not exist.** `connects` that cannot be satisfied
  stops the service *starting*, with a sentence (`SPEC-03` §3.4). An addon
  installed on a machine with no Aura does not run and says why.

### 3.4 An addon that goes silent

**Every card carries its own lifetime and the renderer withdraws it when that
runs out.** `Card.ttl` is required; there is no heartbeat and no liveness probe.

The reason is Aura's own, twice over. `IndoorData.online` exists because *a dead
sensor and a perfectly stable room produce the same flat line, and nothing but
this flag tells them apart*; the status on the original renderer's reading card
exists so that *a renderer that ignored the status still could not put a stale
number on the wall*. A silent producer is indistinguishable from a producer
whose answer has not changed, so the producer states in advance how long its
answer stays true — a reading polled every five minutes might say 900; a
forecast refreshed every twenty might say 5400.

**When a card expires the slot goes empty.** The renderer does not dim it,
freeze it, or draw a staleness marker: that is `state: stale`, which is the
producer's word (§5.4), and a renderer inventing it would be inventing a claim
about data it does not hold. The expiry is written to the journal naming the
slot, because a slot that emptied is a producer that stopped.

A producer that is stopping cleanly should send `withdraw` (§6.3) rather than
leave a card on the wall for the rest of its TTL.

---

## 4. Producers

### 4.1 A card's producer is the nearest thing that already has the fact

This is the principle the rest of this section follows from. A network client
has the weather; a relay client has the corner card's reading; a sensor client
has a room's temperature; **the renderer has the time**, because it is drawing frames.
One mechanism, different distances.

**Amended 2026-09-03: the nearest thing is sometimes not an addon at all.** On
this machine `aura-controller` already holds the weather, the room and the
corner card's reading, and the renderer reads them from it directly — so for those
three the nearest thing is a socket the renderer already has, and interposing a
card producer added a hop and lost the `cmd` channel (`§1.0`, `§4.2`). The
principle is unchanged and it is what settles the question: an addon is the
right shape when it is nearer the fact than the controller is, which is what the
reading-card and indoor-temperature addons become when they hold their own accounts
and sensors rather than re-reading Aura's.

### 4.2 Weather is NOT an addon — corrected 2026-09-03

**The weather is not an addon and does not arrive as a
card.** It comes from `aura-controller` in a `state` message, and the renderer
reads that socket itself (`§1.0`, `PLAN.md` §119). There is no weather producer
and no `secondary` claimant; the panel is composed by the renderer from the
controller's own fields.

**How this section came to be wrong is the part worth keeping.** It described
`examples/aura/aura-bridge` — a program that read the controller's JSON and
re-emitted it as cards, so that the weather would arrive through the same door
as an addon. It was deleted on 2026-09-02. It existed only because the renderer
had been built to consume cards and nothing else, and it is the reason `cmd`
had nowhere to go: a translator that relayed cards necessarily dropped
directives. **The maintainer had asked for a port and this was a rewrite.**

**The principle below survives the deletion and is the reason it is not simply
removed.** It was written about the weather; it is now the rule for every real
addon — the reading-card, indoor-temperature and bus producers, which live in
separate repositories and DO arrive as cards.

Same socket, same vocabulary, same slot rules, same refusals. An addon ships
in the box or does not, and either way it has **no privileged path into the
renderer**.

That is a deliberate constraint and it costs something. It is refused because
**a private fast path is how an addon API rots**: the shipped cards quietly
acquire reach that a third party cannot get, nobody notices while the only cards
are the shipped ones, and it is discovered when the fourth card is written and
the vocabulary turns out to have been sized for three.

**The test of this is stated as a property of the code**: it must be impossible
to tell, from the renderer's ingestion path, whether a card came from a shipped
producer or from an addon written by somebody Aura has never heard of. Read
`aura-renderer/src/ingest.rs`; there is no branch on who is calling. The only
thing it knows about a peer is which socket it arrived on.

**`crate::controller` does not weaken that and must not be mistaken for a
private path.** It is a second INPUT, not a second door into the slots: what it
carries is the machine owner's own configuration and the facts the controller
already holds, on a socket no addon is granted. Cards still arrive one way.
The property above is about `ingest.rs` and is unchanged — but note the
distinction is now load-bearing, so `ingest.rs` is the file to keep honest.

### 4.3 The clock is not an addon, and the reason is distance rather than
privilege

**Interposing a process and a socket between the renderer and the current time
adds a failure mode to the only card that cannot otherwise fail.** The failure
is unusually visible, too: a stale clock reads as a broken display in a way a
missing reading card does not.

**That is not hypothetical — Aura has the defect today**, and it was verified in
the source rather than assumed:

* `cmd/aura-controller/main.go`'s tick loop formats `time.Now()` into a string
  once a second and pushes it (`st.SetClock`).
* `cpp/src/main.cpp` assigns `time_str` **only** when a state message arrives.
  Its initial value is `"00:00"`.
* `cpp/src/ipc_service.hpp` has no concept of staleness at all — no timeout, no
  last-seen, nothing. Grepped for `stale`, `timeout`, `last_`; the only hit is
  the name of the accessor.

So a controller that stalls leaves the wall showing the minute it stalled at,
indefinitely, while every other card still looks plausible.

### 4.4 One drawing path, no special cases

The renderer's clock is **still a card**: it synthesises an ordinary `Card` in
this vocabulary and hands it to the same store, the same layout and the same
theme as everything else. `aura-card`'s
`the_synthesised_clock_is_a_card_the_protocol_admits` asserts that it survives
`encode` → `decode` unchanged, so the claim is checked rather than asserted.

The only difference is that nothing had to arrive over a socket.

**A slot's socket still exists for `primary`.** An addon that declares
`connects = ["aura-renderer:primary"]` is granted it and its cards replace the
synthesised clock while it is live. That is a consequence of §4.2's *no
privileged path* rather than a feature anyone asked for, and §11 records it as
something the maintainer may want to close.

### 4.5 There are two kinds of addon, and conflating them is what deadlocked §5.2

* **Status: RULED 2026-09-02, SPECIFIED, NOT BUILT.** No contributor exists and
  no producer reads one; §12 says so line by line. The mechanism it rests on is
  the platform's and **is** built — see *"Verified rather than assumed"* below.

| | Claims | Produces | Draws | Example |
| :-- | :-- | :-- | :-- | :-- |
| **Slot addon** | a slot (§3.3) | its own card | one slot's worth of screen | a river-gauge relay; a departures board |
| **Contributor** | nothing | one reading, for a producer | **nothing at all** | an indoor thermometer feeding the weather producer |

**§5.2's objection is satisfied by a contributor rather than violated by it.**
*An addon cannot be half of somebody else's card* — and a contributor is not
half of one. It sends a measurement to another application, which decides
whether to use it, how to compose it, and whether to send a card at all. One
card, one owner, and the owner is the producer.

**This is Aura's own arrangement and not a new idea**, which is the strongest
argument for it. `cpp/src/message.hpp`'s `IndoorData` carries `bool
online{false}`; `cpp/src/main.cpp` reads `if (msg->indoor && msg->indoor->online)`
and otherwise clears the column. The indoor sensor has never drawn anything on
Aura's screen and has never owned a card. It hands the controller a number, and
the controller composes a panel. What changes is only the distance: the number
now arrives over a socket from a separately packaged application instead of from
a goroutine.

#### 4.5.1 How a contributor is declared and connected

**`provides`/`connects`, one level down.** A contributor connects to a producer
exactly as a producer connects to the renderer:

```toml
# the producer that owns the card
provides = ["socket:indoor"]
connects = ["aura-renderer:secondary"]

# the contributor
connects = ["aura-weather:indoor"]
```

Nothing new is declared, nothing new is parsed, and nothing in the platform is
told that Aura exists.

**Verified rather than assumed**, because the design would be different if the
supervisor only wired endpoints to a renderer. It does not; the mechanism has no
idea what a renderer is. Read in this order:

1. `cmd/rivet-sysd/src/applications.rs` — every `provides = ["socket:<name>"]`
   in **any** application's manifest is folded into that service's `listen` list
   and bound by the one path that binds listeners. The provider is handed the
   listening descriptor, named with the endpoint name (`SPEC-03` §3.1).
2. `cmd/rivet-sysd/src/endpoints.rs`'s `resolve` — takes *(name, provides,
   connects)* for **every enabled service** and pairs them by name. It branches
   on nothing but the supervisor's own kiosk endpoint, which is skipped because
   it is granted elsewhere.
3. `cmd/rivet-sysd/src/supervise.rs` — chowns each granted socket file to the
   consumer's uid before the spawn, and puts the path in `RIVET_CONNECT`.

So `rivet_sdk::app::endpoint("aura-weather", "indoor")` on the contributor's side
and `rivet_sdk::listen::socket("indoor")` on the producer's are the whole of it,
which are the same two calls the renderer and a producer already make.

**Three consequences, and each is the platform's answer rather than Aura's:**

* **One consumer per endpoint.** The grant is a `chown` and a `chown` names one
  uid, so a producer wanting two contributors declares **two named endpoints**.
  Two contributors on one endpoint are both refused by name and neither is
  started (`ADR-015` §5.3), so adding a second contributor to a working one
  stops the first as well — a conflict both journals name.
* **A contributor whose producer is not installed does not start**, and says so.
  `NoSuchProvider` is permanent, so the service is failed with a sentence rather
  than restarted for ever (`SPEC-03` §3.4).
* **A contributor cannot reach a producer that did not name it.** The socket is
  `0600` and owned by somebody else.

One asymmetry is worth writing down because it will surprise somebody. The
renderer is a **kiosk** application, so it is enabled only while it is the
selected display application (`PLAN.md` §106). A producer whose renderer is not
selected is therefore refused with `NoSuchProvider` and does not run. **A
contributor is not affected by that** — its producer is an ordinary daemon — so
on a machine showing some other kiosk application the contributor keeps running
and the producer does not. Nothing is drawn either way; it is stated so that a
journal reading *"the weather producer was not started"* beside a healthy
thermometer is diagnosable rather than mysterious.

#### 4.5.2 What a contributor sends: a card without a slot

**The same vocabulary, minus the one field that means *screen*.** §5.7 says a
card is *a slot, a `ttl`, and one to four blocks*; a **contribution** is a `ttl`
and one to four blocks. Everything else in this document applies unchanged —
the shapes of §5, the roles of §5.1, the states of §5.4, the grammar, refusals
and bounds of §6, and the versioning of §8.

That is the whole design, and the alternatives were worse in ways worth naming:

| Considered | Why not |
| :-- | :-- |
| **A second, smaller protocol** — a bare `key: value` reading with no envelope | Two wire formats to implement, two to version, two to diagnose. A contributor would then need a *different* library from the addon beside it, for a message that is a strict subset of one this document already defines |
| **A `reading` with no envelope at all** | No `ttl`, and §3.4's argument for `ttl` is *stronger* one level down: a producer cannot tell a silent thermometer from a room that has not changed, which is the exact sentence `IndoorData.online` exists to answer |
| **Reuse `card` and let `slot:` name the endpoint** | A slot is one of four fixed names (§3.1) and an endpoint is any name a producer chose. Overloading one key with two vocabularies is the kind of ambiguity §6.5 refuses cards for |

The verbs are `contribute` and `retract`, and the key that replaces `slot:` is
`endpoint:` — §6.3 carries the grammar and §6.4 a worked example.

**Why `endpoint:` is required when the socket already says which one it is.**
For §7 rule 7's reason exactly, one level down: the socket is the half the
platform vouches for, and the message saying which endpoint it believes it is
lets a producer refuse a mismatch instead of composing the room's humidity into
the wind chill. It also makes a journal line self-describing, which is §6.2's
whole argument for the format.

#### 4.5.3 Absent, silent, expired — Aura already answers this

**Carried across rather than invented.** A contribution carries a `ttl`, and when
it runs out the producer holds nothing:

| | What the producer has | What it does |
| :-- | :-- | :-- |
| **absent** — no contributor installed | nothing, and never had anything | composes the card without it. `PanelColumn::drawable()` is exactly this case and it has always been reachable |
| **silent** — connected, nothing arriving | the last contribution, until its `ttl` | composes with it, then without it when it expires |
| **expired** | nothing | composes without it, and **journals the endpoint**, because an endpoint that emptied is a contributor that stopped |
| **`state` not `ok`** | a contribution the producer must not turn into a number | §5.4 binds the producer here exactly as it binds the renderer. `connecting` and `unavailable` carry no value and there is nothing to compose |

**The absent case is not new work and that is the point.** `main.cpp`'s
`if (msg->indoor && msg->indoor->online)` clears the indoor column, and
`DrawWeatherPanel` centres the remaining column as a group (§5.8.3 item 6). The
panel has drawn with and without indoor since it was written; a contributor that
never appears produces the picture Aura already draws with the sensor unplugged.

**A producer must not substitute.** It composes with what it has, and never with
what a contributor said an hour ago and has not repeated. That is `ttl`'s job and
`ttl` is required for the same reason it is required on a card (§3.4).

#### 4.5.4 What a contributor may NOT do, which is the boundary

1. **Claim a slot.** A contributor's manifest names no `aura-renderer:*`
   endpoint. If it does, it is a slot addon and this section does not apply to
   it — a program may be both, and then it is two producers in one binary and
   the platform grants it two sockets.
2. **Draw.** It declares no `display` device, is never offered the screen, and
   there is one lease and it is not its (`PLAN.md` §106).
3. **Affect layout.** It sends no `slot`, no widths, no order and no position. It
   cannot make its reading the left column, cannot make it the larger one, and
   cannot stop the producer dropping it.
4. **Be depended on.** A producer that cannot compose a card without its
   contributor has designed a card that is half somebody else's, which is the
   thing §5.2 refuses and this section does not reopen.

**The boundary is the whole value of the distinction.** A slot addon is granted
a share of the screen and everything §3 says about slots protects the renderer
from it. A contributor is granted a share of somebody's *data*, and the only
thing protecting the producer is that a contribution cannot express anything but
a measurement.

---

## 5. The vocabulary

**Four shapes — corrected 2026-09-02, and how the count was wrong is the useful
part.** This section said *three*, on the grounds that the brief's fourth known
case — *whatever the clock and weather cards actually need* — needed nothing
new, because the clock is a reading and the weather card is a reading and a
statement stacked. The fourth case did need something new. What hid that is
§5.2's superseded paragraph, which had already decided the weather card would
*become* a reading and a statement by dropping its second column. Read together,
the vocabulary was complete because the card had been trimmed to fit it. `pair`
is §5.8, ruled and built the same day.

That the composition is *a shape and a statement stacked* is still true, and is
why a **card holds an ordered sequence of blocks** rather than being one shape:
the composition belongs to the card, and the shapes are the alphabet.

### 5.1 Colour: **four** roles, and no colour ever crosses the wire

| Role | Means | Where it comes from |
| :-- | :-- | :-- |
| `normal` | the ordinary ink | `CardTheme::text` |
| `muted` | an aside: a unit beside a value, a caption under one, a rule between two columns | the C++ panel draws its divider at 35% of the text alpha |
| `warn` | **not right, yet** | `CardTheme::caution` — the amber connection question on `status: connecting` |
| `alarm` | **wrong** | `CardTheme::alert` — a reading outside its band, or a connection gone quiet for good |

**A fifth role, `accent`, was removed on 2026-09-02, and the reason it existed
is the reason it went.** The maintainer asked: *where do you need this accent colour
that the Aura screensaver themes aren't already providing?* Nowhere.
`CardTheme` in `cpp/src/main.cpp` carries `bg`, `text`, `alert` and `caution` —
**four colours, because four is what Aura draws.**

Its own entry in this table gave the game away and stood for a day anyway: the
justification was *"a theme with no way to be itself is a palette, not a
theme"*, which is an aesthetic argument rather than a requirement. It produced a
role no producer could use and no theme could resolve — `widgets::role_ink`
drew it as `normal`, and §11 (15) recorded THAT as a defect to be closed by
choosing a hue.

**Closed by deletion instead**, which costs nothing today: the vocabulary is at
version 1 and no third party has written a producer against it. The rule this
leaves behind is the one worth keeping — **every role must name something the
existing renderer already distinguishes**, and a role that cannot point at a
`CardTheme` field is a role that has not earned its place.

**Why an addon may not send a colour.** The C++ renderer already learned this
and wrote it down beside `CardTheme::alert`: contrast is part of the theme,
because a red that reads on a black ground is weak on a white one and a theme
drawing black text needs a darker red beside it. Every one of Aura's ten
palettes carries its own pair of signal colours for exactly that reason — the
two light ones ship `{186, 26, 26}` and `{150, 95, 0}` where the eight dark ones
ship `{255, 69, 58}` and `{255, 197, 0}`.

An addon cannot know which theme is on the wall, and an addon written today
certainly cannot know about a theme written next year. **This is the whole
argument for roles**, and the exact pairs above are carried into
`aura-renderer/src/theme.rs` rather than re-chosen, because they were chosen
against a real television by somebody looking at it.

**The renderer must resolve a role in exactly one function.** Two would be two
answers (`ADR-011`).

`warn` and `alarm` must be visually distinct from each other and from `normal`
in every theme, and none of them may be the colour of the card they are drawn
on. `theme.rs`'s tests assert both.

### 5.2 `reading` — one measured quantity, read from across the room

| Key | Required | Meaning |
| :-- | :-- | :-- |
| `glyph` | no | one of §5.6's thirteen marks |
| `value` | see §5.4 | the number, **already formatted by the producer** |
| `unit` | no | drawn beside the value, `muted` |
| `secondary` | no | the second line: a wind chill, a dewpoint, a humidity |
| `secondary-unit` | no | when it differs from `unit`, which it does for a room (°F over %) |
| `caption` | no | a short line underneath: the clock's date, the weather's condition |
| `trend` | no | §5.5. **Absent means unknown and draws nothing** |
| `state` | no, defaults `ok` | §5.4 |
| `role` | no, defaults `normal` | §5.1 |
| `age` | no | seconds since the reading was taken |

**Where the shape came from.** Three functions in `cpp/src/widgets.hpp` draw the
same composition with different fields present:

| card | `value` | `unit` | `secondary` | `caption` | `trend` |
| :-- | :-- | :-- | :-- | :-- | :-- |
| clock — `DrawClockWidget` | `22:15` | — | — | `Monday, September 1` | — |
| the corner reading card — the original renderer's reading card | `2.4` | `m` | — | — | `rising` |
| outdoor — `DrawWeatherPanel` | `70°` | `F` | `62°F` | `Cloudy` | — |
| indoor — the same, other column | `76°` | `F` | `54%` | — | — |

**The clock is a reading, and a separate `clock` shape was considered and
rejected**: it would carry no field this one lacks, because a time is a value
and a date is a caption. The one real difference is that a clock's digits must
not jitter as the minute changes — which is why Aura loads a *monospaced* face
for every number it draws — and that is not a field, because it applies to every
reading.

> **Renderer obligation.** A reading's `value` and `secondary` are drawn with
> **tabular figures**. A value that reflows as it changes reads as a fault in
> the display.

**`value` is a string, not a number.** The producer already owns every
formatting rule there is — a source's own rounding, °F to °C, and *"unit
conversion happens once, in the controller"* — so a renderer that reformatted
would be a second place those rules live (`ADR-011`). For the corner reading
card the producer is `aura-renderer`'s own `controller` module, and its rule
keys on no unit, because the host knows none for that card (`.rulings/` R-375
D1): the source's value is printed as the source rounded it, to at most one
decimal.

**The two-column weather panel survives whole** — §4.5 and §5.8, 2026-09-02. The
vocabulary grew a fourth shape to carry it, and `aura-renderer` draws it. This
section said the opposite: that `DrawWeatherPanel`'s side-by-side outdoor and
indoor columns did not survive the design, so outdoor would become a `reading`
plus a `statement` in `secondary` and indoor an addon's own `reading` in a slot
of its own.

**What was actually wrong with that is its middle clause, not its
conclusion.** *An addon cannot be half of somebody else's card* is still true and
is still enforced — nothing below relaxes it. What the paragraph assumed, without
noticing that it was an assumption, is that **an addon must be a thing that
claims a slot and draws a card.** There is a second kind, and the indoor sensor
is one: it claims nothing, draws nothing, owns no part of any card, and hands a
value to the application that composes the whole card itself. §4.5 names the two
kinds and §5.8 is the shape. One card, one owner — satisfied rather than
violated. (`.rulings/` R-126.)

The struck sentence is kept because its reasoning was sound for the question it
was asked (`AGENTS.md` §2 rule 3). The question was the wrong one.

### 5.3 `statement` and `list`

**`statement`** is wrapped prose: `text` (required) and `role`. One card in the
shipped set needs it — the NWS `shortForecast` in `WeatherData.statement`, which
`DrawWeatherPanel` puts through `WrapText` at 44px under the columns. It is the
only free text in Aura's entire wire schema, and it is free text because a
forecast is a sentence somebody else wrote. Its second use is an addon
explaining itself, which a `state: unavailable` reading cannot do.

> **Renderer obligation.** The renderer wraps, never the producer. The width is
> the renderer's fact and nobody else's — the same reason `WrapText` lives in
> the C++ renderer and not in the Go controller.

**`list`** is rows: an optional `caption`, an optional `glyph`, and one or more
`row`s of `lead` / `label` / `trail` / `role`. `label` is the only required
part.

#### 5.3.1 How the list was checked against a vocabulary that is secretly too narrow

The corner card's value and the indoor temperature are **both** readings. A
vocabulary derived only from Aura's existing cards could therefore be far too
narrow and still look
complete, and the honest test of the whole design is a card of a genuinely
different shape. `BusDep{route, destination, minutes}` — already in
`cpp/src/message.hpp`, reserved and never populated — is that card.

Three things were done about it, and all three are in the tree:

1. **The field names refuse to say *bus*.** `lead` / `label` / `trail` are
   positions, not nouns. `21` / `Rosedale Center` / `7 min` is one filling;
   `09:30` / `Standup` / `30m` is another.
2. **`aura-card`'s `a_list_is_not_bus_shaped` carries both**, plus the
   degenerate case — a list with no lead and no trail at all, which must still
   be a list rather than needing a fourth shape.
3. **The renderer's layout rule can be stated without either noun**: *a lead
   token in a column as wide as the widest lead; a trailing token aligned down
   the right edge; the label in what is left, truncated with an ellipsis.* Read
   `aura-renderer/src/layout.rs`'s `list`: it does not name a route, a
   destination or a minute.

`role` is per row, so *this bus leaves in two minutes* can be `warn` while the
rest of the board is ordinary.

### 5.4 `state` — how much a reading can be believed

Lifted whole from the status on the original renderer's reading card, and for
the reason `cpp/src/message.hpp` gives: *how old a reading may be before it
stops being worth showing is a rule about data*, and the producer owns the data.
A renderer deciding this would be deciding it identically for a river's level
and a room's temperature, which are not the same question.

| `state` | May carry a value | Renderer must |
| :-- | :-- | :-- |
| `ok` | yes | draw it |
| `stale` | yes | draw it **with a mark beside it** — never silently |
| `connecting` | **no** | draw the `warn` mark alone. A cold start is not a fault |
| `unavailable` | **no** | draw the `alarm` mark alone |

**A value with a state that does not admit one is refused at the wire**, in both
directions — `aura-card` will not encode it and will not decode it. Aura enforced
the same rule from the producer's side by omitting the field; stating it here as
well means a producer that sends both is refused rather than trusted.

**A reading with no value and `state: ok` is also refused**: it would draw
nothing at all, the original renderer's reading card returns early in exactly
that case, and a producer hearing about it beats an operator watching an empty
corner.

### 5.5 `trend`

`falling-fast`, `falling`, `flat`, `rising`, `rising-fast`. **There is no sixth
and no `unknown`**: an unknown trend is expressed by omitting the key, so there
is no value to be defaulted to by accident. `cpp/src/widgets.hpp` states the
reason — *a sixth state the relay grows must not arrive on screen as a
reassurance nobody sent.*

### 5.6 `glyph` — the closed icon set

`clear`, `partly-cloudy`, `cloudy`, `rain`, `thunder`, `snow`, `sleet`, `fog`,
`wind`, `indoor`, `unavailable`, `clock`, `bus`.

The first eleven are each a branch of `DrawWeatherIcon` or `DrawIndoorIcon` in
`cpp/src/widgets.hpp` — that is where the list came from. The NWS slug space is
far larger; the C++ renderer already collapses it to these, so **mapping a slug
to a glyph is a producer's job and always was**. `clock` and `bus` are new, and
each is named by the card that needs it.

**The connection marks are deliberately absent.** `cpp/src/widgets.hpp` draws
`android_wifi_3_bar_alert` and `android_wifi_3_bar_question`; those are
`state: unavailable` and `state: connecting`, and the renderer derives them. A
producer that could send the mark *and* the state could send two different
answers to one question.

`glyph: unavailable` is **not** `state: unavailable`. The first is *the weather
service said nothing*, which is itself a condition worth drawing; the second is
*this reading is not available*.

#### 5.6.1 Eleven of the thirteen are not font glyphs, and §11 (5) said they were

**Corrected 2026-09-02**, while implementing them. §11 (5) records *"thirteen
glyphs and five trend arrows need a face"* as one open question. That is right
about two of the thirteen and wrong about eleven, and the two halves have
different answers and different costs:

| | Where it comes from in `cpp/src/widgets.hpp` | What it needs here |
| :-- | :-- | :-- |
| the nine weather conditions, `unavailable`, and `indoor` | **drawn with primitives** — `DrawWeatherIcon`, `DrawIndoorIcon`, `DrawCloudBody`: circles, thick lines and rectangles, no font involved | drawing primitives, which is what `aura-renderer` now uses |
| `clock`, `bus` | neither exists in Aura | a face, or a drawing |
| the five trend arrows and the two connection marks | Material Symbols codepoints, subset into `assets/MaterialSymbols-Aura.ttf` — and the comment beside them says why: *the arrows were drawn with primitives first, and a hand-rolled arrowhead next to type set in a real typeface looks exactly like what it is* | a face, and there is no substitute; the C++ tried one and rejected it |

So the icon-face question was real for the corner reading card, and it never
blocked the weather panel. It was written as one question because the
document was listing what was missing rather than what each thing was made of.

### 5.7 A card

A slot, a `ttl`, and one to four blocks. Never zero: a card with nothing in it
is a withdrawal, and a withdrawal has its own verb so that an encoder with a bug
cannot express one by accident.

**A `pair` is one block**, not two. It counts once against the four and once
against §6.6's bound, because the thing being bounded is how much composition a
card may ask for, and a comparison drawn as one unit is one piece of
composition.

### 5.8 `pair` — two readings meant to be compared

* **Status: RULED 2026-09-02, BUILT.** Added 2026-09-02 as a shape waiting on a
  ruling; ruled and implemented the same day. `aura-card` encodes and decodes it
  and `aura-renderer` draws it — §12 says what was proven on a board and what
  was only tested.

**The fourth shape, and the vocabulary is closed again with four in it.** Three
was not a target; it was what the first four known cards needed, and this
document said so in §5's opening sentence. A fourth shape is admitted the way
the first three were — by a card that cannot be composed out of the others —
and `pair` is admitted because a comparison drawn as two stacked readings is a
different picture, not a differently spaced one.

**What this section was, and why it is worth reading the history.** It was
written on 2026-09-02 as *SPECIFIED, NOT RULED, NOT BUILT*, because §5.2 had
rejected the two-column panel outright and R-125 had ruled that Aura's
appearance is fixed and the protocol bends to it. Those two could not both
stand. §5.8.4 laid out two ways forward, said that one of them was already
refused, and closed with *"there is no third option in which both hold"*.

**There was a third option, and it is the one that was taken.** The dilemma
assumed every addon claims a slot and produces a card. §4.5 names the second
kind — a **contributor**, which claims nothing and draws nothing — and with that
distinction §5.2's rule and R-125's ruling stop competing: the weather producer
owns the whole card and the whole panel, and the indoor sensor stays a separately
packaged, separately installable application that feeds it. (`.rulings/` R-126.)

§5.8.4 is left standing below, struck where it was wrong, because a section that
argued a dilemma was exhaustive and then turned out to have missed a term is
exactly the kind of reasoning the record should keep.

#### 5.8.1 The shape, described as a shape

Two readings drawn **side by side as one unit**, with a rule between them,
their contents aligned on a common band, and the pair centred as a group.

| Key | Required | Meaning |
| :-- | :-- | :-- |
| `left` | yes | a `reading` (§5.2) |
| `right` | yes | a `reading` (§5.2) |

Nothing else. No widths, no alignment, no rule colour: those are the renderer's
(§5.1's `muted` is the rule, and the C++ draws it at 35% of the text alpha).

**Two and not N.** A row of four readings on a 1080p panel at Aura's sizes does
not fit, and a shape that admits an unbounded row is a shape whose failure mode
is a card that silently truncates. Two is what the product does and two is what
a comparison is. A producer with three facts sends a `list`.

**On the wire, `left` and `right` open a scope**, exactly as `row` does for a
`list`, and everything §5.2 admits inside a `reading` is admitted inside each of
them. That is one more scope-opening word in §6.3's grammar and no new concept:

```
block: pair
left
glyph: cloudy
value: 70
unit: °F
secondary: 62
secondary-unit: °F
right
glyph: indoor
value: 76
unit: °F
secondary: 54
secondary-unit: %
```

**Both are required, and a side with nothing to report is spelled with its
`state`.** An optional `right` and a `state: unavailable` `right` would be two
ways to say one thing, which is the second-representation failure `ADR-011`
names arriving in a wire format — and of the two, `state` is the one that says
*why*. A producer whose second sensor has gone quiet keeps sending a `pair` and
lets the reading carry `unavailable` or `connecting`; the card does not change
shape underneath the renderer, which is exactly what `main.cpp` does when
`indoor.online` goes false — it clears the column and keeps the panel.

> **Renderer obligation — which columns are drawn.** A column is drawn when its
> reading has a `value`, a `secondary`, **or** a `glyph`; a reading with none of
> the three is skipped and the other column is centred alone (§5.8.3 item 6).
>
> That is `PanelColumn::drawable()` generalised, and the generalisation has one
> visible consequence worth naming. The C++ tests *has values, or has a
> condition slug* — and its indoor column never has a slug, because the house
> icon comes from a `bool indoor` flag rather than from data. Here the house is
> `glyph: indoor` and is data like every other glyph, so a producer that sends a
> glyph and no reading gets a lone icon where Aura would have drawn nothing. It
> is unreachable for the shipped card, because a producer with no room reading
> sends no room glyph, and it is written down rather than discovered.

**The vocabulary stays closed.** Four shapes, **four** roles (§5.1, `accent` removed 2026-09-02), four states, five
trends, thirteen glyphs, and a card holding one to four blocks. `pair` adds a
shape and takes nothing away, so §8's rule holds: a card written against
vocabulary 1 is still valid, and this is vocabulary 1 — the number does **not**
advance, because nothing an older producer can write has changed meaning and
nothing it can write has become invalid.

> **Renderer obligation.** A `pair` is drawn as **one unit**: the two readings
> share a common band, are separated by a `muted` rule, and are centred as a
> group. A renderer that drew them as two adjacent readings would satisfy the
> grammar and lose the shape's whole meaning, which is that the *difference*
> between the two numbers is what a person is reading.

#### 5.8.2 What forces it, and what an addon would do with it

**The card that forces it is Aura's weather panel**: outside and inside, at the
same instant, in the same units, side by side — because the question a person
standing in front of it is asking is *do I need a coat*, and that question is
the **difference** between the two numbers. Stacked vertically or split across
two slots, the comparison stops being one glance.

**As a shape it is a comparison**, and comparisons are ordinary:

* a two-zone thermostat — upstairs and downstairs;
* today's high beside tomorrow's;
* a battery bank's charge beside its draw;
* two people's step counts, on a panel in a shared kitchen;
* the home and away score of one match.

Every one of those is *two readings whose meaning is the pair*. None of them is
weather, and a renderer drawing this shape cannot tell which of them it has —
which is the test §5.3.1 applied to `list` and is applied here for the same
reason.

**What it is not.** It is not `kind = "aura-weather"`, and the difference is not
cosmetic: a shape that named the card would be the private fast path §4.2 exists
to forbid, rebuilt with extra steps.

**The test that it stayed general**, applied the way §5.3.1 applied it to
`list`: **the definition in §5.8.1 and the geometry in §5.8.3 can be read end to
end without the noun.** `left` and `right` are positions; a column is a glyph
and a stack of two numbers; the rule is two pixels of `muted`; the group is
centred. Nothing in either says outside, inside, degree or forecast, and
`aura-renderer`'s implementation names none of them either — the panel takes a
`pair` and a `statement` and cannot tell what they are about. A third-party
addon writing a two-zone thermostat card gets the identical picture with
different words in it, and that is the property that makes this a shape rather
than a feature.

#### 5.8.3 The geometry, out of `cpp/src/widgets.hpp`

Recorded here so that whoever implements it has the numbers rather than an
impression of them. All from `DrawWeatherPanel`, at 1080p.

| constant | px | what it separates |
| :-- | --: | :-- |
| `kTempSize` | 96 | the first value |
| `kSecSize` | 62 | the second line, deliberately smaller than the value above it |
| `kStmtSize` | 44 | the statement under the pair |
| `kIconSize` | 88 | the glyph, square |
| `kIconGap` | 26 | glyph → the value stack |
| `kValGap` | 4 | value → second line |
| `kColGap` | 48 | each column → the rule, so the rule sits 48 either side |
| `kStmtGap` | 26 | the pair → the statement |
| `kLineGap` | 6 | between wrapped statement lines |
| `kCardPad` / `kCardHPad` | 20 / 30 | the card, vertically and horizontally |
| the rule | 2 wide | at 35% of the text alpha, `muted` |

And the placements, which are the part an impression gets wrong:

1. **The glyph is at the left of its column and the value stack is centred
   against it** — `icon_y = origin.y + (top_h - kIconSize) * 0.5`, where `top_h`
   is the taller of the glyph and the stack.
2. **The second line is centred under the first**, on the column's own value
   width, not sharing its left edge: `vx + (values_w - sz.x) * 0.5`. That is
   what makes the two read as one stacked reading.
3. **Every vertical measurement is trimmed by `kDescentFrac` = 0.20** before
   being used as a gap, because `MeasureTextEx(...).y` is the font size and
   digits never use their descender.
4. **A column with no glyph reserves no width for one** — added after readings
   with no condition slug left the numbers floating in an icon-sized hole.
5. **The rule spans 88% of the taller column, starting 6% down**:
   `origin.y + content_h * 0.06`, height `max(top_h) * 0.88`.
6. **The pair is centred as a group inside the block**, so a column that is
   absent leaves the other centred rather than hanging off one edge.
7. **The statement is centred on the block**, wrapped by the renderer to
   `max(content_w, 480)`.

#### 5.8.4 The two ways out, and what each costs — and the third one that was missed

**RULED 2026-09-02: (c).** Options (a) and (b) were written as exhaustive and
were not, and the *shape* of that mistake is the useful part: an option was
ruled out by a premise nobody had noticed was a premise.

**(a) One producer owns both columns.** The weather addon reads the indoor
sensor as well as the forecast and sends one card containing a `pair`. Aura's
appearance is preserved exactly. **Its cost — that the indoor sensor stops being
a separable addon — was real, and (c) avoids paying it, being (a) with the
sensor kept separable.**

**(b) Two producers, two slots.** Indoor is its own addon in its own slot, as
§5.2 says. **The cost:** Aura's screen changes shape. The rule between the
columns cannot exist, the group centring cannot exist, and the two temperatures
land in two different corners at two different times of the rotation. This is
the option the maintainer has already said no to. **Still refused.**

**(c) One producer owns the card; the sensor is a CONTRIBUTOR.** §4.5. The
weather producer composes the whole panel and sends one card containing a
`pair`; the indoor sensor is its own installable application that hands the
producer a reading over a `provides`/`connects` endpoint one level down, and
draws nothing. **Aura's appearance is preserved exactly and the sensor stays
separable**, so (a)'s cost is not paid.

**What it costs, stated so it is not read as free:** a third party who wants
their reading *next to* somebody else's now needs that somebody to have declared
an endpoint for it. A producer that declares none takes no contributions, and
nothing forces one to. That is a narrower kind of extensibility than a slot —
which is correct, because it is a share of another application's data rather than
a share of the screen, and §4.5.4 is the boundary.

**This section said there was no third option in which both hold.** There was;
it is (c), and it is what was ruled.

**And that sentence is why this subsection is kept.** It was written to stop
somebody discovering a dead end halfway through implementing it, and it was
itself the dead end: an exhaustiveness claim over a space defined by an
unexamined assumption — *an addon is a thing that claims a slot* — which is a
premise the document had never stated because it had never had reason to doubt
it. `AGENTS.md` §7 is about claims of absence, and *"there is no third option"*
is one of the largest a design document can make.

---

### 5.9 The generic composition — a slot Aura has no widget for

* **Status: RULED 2026-09-02, BUILT.** `examples/aura/aura-renderer/src/layout.rs`.
  §11 (12) is the question this closes and §12 says what was proven and by what
  method.

**A composition and an appearance are two different things, and conflating them
is what deadlocked §11 (12)** — the same shape of mistake §5.8 records, one
section earlier, about a different premise.

The question was *what does an addon's card look like in a slot Aura has no
widget for*, and it read as a choice between two bad answers. Draw a generic
composition, and a picture that is not Aura's goes on the wall, which R-125
forbids. Draw nothing, and a shape the protocol admits reaches a renderer that
throws it away — which makes §4.4's *one drawing path* a path to a dead end and
the vocabulary a vocabulary in name only.

Both answers assume that *composition* and *appearance* are one decision. They
are not:

| | What it means | Where it comes from |
| :-- | :-- | :-- |
| **the composition** | which blocks, in what order, stacked how | **generic** — the card's own ordering, §5.7. Nothing in `layout.rs` knows what a bus, a forecast or a river gauge is |
| **the appearance** | type sizes, gaps, padding, glyph drawings, the descender trim, the card behind | **Aura's** — `crate::widgets`' constants, which are `cpp/src/widgets.hpp`'s, reached by calling the same functions the weather panel calls |

So an addon's card is **composed generically and set in Aura's typeface at
Aura's sizes in Aura's palette, anchored in a corner the way Aura's own two
widgets are.** It is a member of the family rather than a second visual
language beside it.

**R-125 is untouched.** It fixes the appearance of *Aura's cards*, and Aura's
cards keep their hard-coded widgets: `primary` is `DrawClockWidget`, `secondary`
is `DrawWeatherPanel`. This path is reached only where Aura has no widget, and
there is no C++ picture of a departure board to be unfaithful to.

#### 5.9.1 The stack

A card's blocks are drawn top to bottom in the card's own order, each centred in
the card's width, `kStmtGap` apart — the gap `DrawWeatherPanel` already puts
between its columns and the statement under them, because it is the same
relationship.

| block | drawn as |
| :-- | :-- |
| `reading` | `PanelColumn` — glyph, value with its unit, second line — through `widgets::draw_column`, with `caption` wrapped under it in the `muted` role |
| `pair` | §5.8's shape: two columns, `kColGap` either side of the rule, the group centred. The rule is `muted`, which §5.1 already says it is |
| `statement` | wrapped by the renderer and centred, at `kStmtSize` |
| `list` | a `muted` caption over rows of lead / label / trail: one lead column as wide as the widest lead, trailing tokens down the right edge, labels cut with an ellipsis |

**The `pair` arm draws.** The struck text in §11 (12) recorded it returning the
pen unchanged, which meant a card with a pair above a statement drew the
statement on top of nothing.

#### 5.9.2 Two numbers that are judgements, and they are named as such

Everything above is a port. Two things are not, because **a `list` has no C++
counterpart at all** — Aura draws three cards and none of them is one — and they
are settled by reusing Aura's nearest existing number rather than by choosing a
new one:

* **A row is `kStmtSize` (44), not `kSecSize` (62).** A row looks like a small
  value, so 62 was the first answer; drawn, it was plainly wrong. A row is three
  things across where a reading's second line is one, so the label is the only
  elastic column and it collapses — `"Ann Arbor via State"` came out as
  `"Ann A…"` with half the card empty. A row is read the way the statement under
  the panel is read.
* **A corner widget may be half of `screen_w - 3 × pad`.** Prose and labels have
  no intrinsic width and need a ceiling; the panel's own `max(480.f)` is a floor
  and not one. The bound is not a taste: two widgets share a row, each `pad`
  from its outer edge and `pad` from each other. A `pair` of two wide readings
  is drawn at its own width regardless, because a column of digits has an
  intrinsic size and clipping it would be worse than overhanging.

#### 5.9.3 The fourth corner

`tertiary` takes the reading card's corner — the original renderer's rule, one
quarter-turn clockwise from the clock (`Corner::quarter_turn`) — and
`quaternary` takes the corner opposite that. The clock and the panel own one
diagonal, so the quarter turn lands on the other and its opposite is the one
seat left. `the_four_widgets_fill_the_four_corners_at_every_rotation` asserts
both halves: no two share a corner, and none is left empty.

#### 5.9.4 What it still does not draw

**Trend arrows and connection marks are omitted**, for §11 (5)'s reason and on
§7 rule 5's terms: no icon face ships, and a mark that is absent is diagnosable
where a mark that is wrong is not. `Glyph::Clock` and `Glyph::Bus` are in the
vocabulary and in no Aura source, so a column carrying one reserves no width for
it (`widgets::icon_is_drawn`).

**There is no `accent` role to fail to draw.** An earlier version of this
section recorded that `Role::Accent` drew as `Role::Normal` and called it a
silent degradation. It was — and the fix was to remove the role rather than to
find it a hue, because `CardTheme` carries four colours and Aura draws four.
§5.1 and §11 (15) carry the reasoning.

`widgets::role_ink` is the one place a role becomes a colour — the function
`theme.rs`'s `Theme::ink` was invented to be, rebuilt on the palette the product
ships. `muted` is the ordinary ink at 35% alpha, which is `DrawWeatherPanel`'s
own rule colour; `warn` and `alarm` are `CardTheme::caution` and
`CardTheme::alert`, and with no card theme at all they fall back to
`{255,197,0}` and `{255,69,58}` — not an invention, but `CardTheme`'s in-class
defaults in `cpp/src/main.cpp`.

---

## 6. The wire

### 6.1 Transport

A `SOCK_STREAM` Unix socket per slot, **bound by the supervisor and handed to
the renderer as a listening descriptor** (`SPEC-03` §3.1). The renderer binds
nothing and the addon guesses nothing: it is told the path in `RIVET_CONNECT`
(`SPEC-03` §3.4), and Aura's C++ renderer is the cautionary case for doing
otherwise — it hard-coded `/run/aura/kiosk.sock`, which is not where a Rivet
puts it.

The `provides`/`listen` fold makes a provided socket the same bind and the same
handoff as a declared listener (`cmd/rivet-sysd::endpoints`), so the descriptor arrives
named with the endpoint name and `rivet_sdk::listen::socket("secondary")` finds
it.

**Traffic is one-way: addon → renderer.** There is no reply and nothing to read.
An addon that wants to know whether its card was refused reads the machine's
journal, like everything else.

**One producer per socket at a time.** The grant is to one uid; a second
connection on the same socket is that producer having restarted, and the
renderer takes the new one and drops the old.

### 6.2 Why a line format and not JSON

`rivet_core::json` already argued this for the control plane and the argument
transfers: this workspace carries no serialisation framework, and *every wire
format in it is a function that writes and a function that reads, both visible*.

Nothing here forces JSON. The one real pressure is `SPEC-03` §1's standard —
a second language must be able to consume the contract in about two hundred
lines — and `key: value` lines between two sentinels meets that in any language
with a string split. JSON would meet it in languages that ship a parser and not
at all in the ones that do not.

The deciding property is legibility: a card that arrives wrong will be diagnosed
by a person reading a journal line, and `value: 70` beats `{"v":"70"}` at three
in the morning.

### 6.3 The grammar

A message is a sequence of `\n`-terminated lines. A trailing `\r` on any line is
ignored, so a producer written on Windows works.

```
card <vocabulary>            the first line of a card
withdraw <vocabulary>        the first line of a withdrawal
contribute <vocabulary>      the first line of a contribution     (§4.5)
retract <vocabulary>         the first line of a withdrawn one    (§4.5)
slot: <slot>                 required, `card` and `withdraw`
endpoint: <name>             required, `contribute` and `retract`
ttl: <seconds>               required, `card` and `contribute`
block: <shape>               opens a block; keys after it belong to it
row                          opens a row; `list` blocks only
left | right                 opens a reading; `pair` blocks only
<key>: <value>               a key of whatever is currently open
end                          the last line
```

**Four verbs and two envelopes, not two protocols.** `contribute` and `retract`
differ from `card` and `withdraw` in exactly one key — `endpoint:` where a card
has `slot:` — and carry the same blocks under the same rules, the same bounds
and the same version number. §4.5.2 says why that is one key and not a second
format. A reader that implements cards implements contributions by admitting two
more verbs.

**Five rules, and they are the whole format:**

1. **UTF-8, and no control character anywhere** — nothing below `0x20` except
   the `\n` that ends a line, and not `0x7f`. This is *not* restricted to ASCII
   the way a Rivet manifest is: a weather statement is prose somebody else
   wrote, and `°` is in every temperature Aura has ever drawn.
2. **The separator is a colon and exactly one space.** Everything after it to
   the end of the line is the value — no quoting, no escapes, and therefore
   nothing for two implementations to disagree about.
3. **A value is never empty.** Absent is spelled by omitting the key — the same
   rule `SPEC-03` §3.0 states for the handed environment: *absent, not empty*,
   throughout.
4. **A key never repeats within its scope.** Two answers to one question is not
   a card.
5. **Key order is not enforced.** This is a deliberate departure from
   `rivet_core::package::application::Manifest`, which demands the canonical
   order because a manifest is *emitted* and anything else means a second
   producer. A card is emitted by programs Aura did not build, in languages Aura
   did not choose, and refusing a correct card because its author sorted the
   keys differently would be refusing for a cosmetic reason. An encoder still
   writes one canonical order, so Aura's own output is stable and diffable.

### 6.4 Worked examples

The shipped weather card — values from `Aura/docs/ipc-schema.md`'s own example:

```
card 1
slot: secondary
ttl: 5400
block: reading
glyph: cloudy
value: 70
unit: °F
secondary: 62
secondary-unit: °F
caption: Cloudy
state: ok
role: normal
block: statement
text: Chance Rain Showers
role: muted
end
```

A departures board — the card that proves §5.3.1:

```
card 1
slot: tertiary
ttl: 90
block: list
caption: Snelling & University
glyph: bus
row
lead: 21
label: Rosedale Center
trail: 7 min
role: warn
row
lead: 63
label: Grand Ave
trail: 21 min
role: normal
end
```

Going away cleanly:

```
withdraw 1
slot: tertiary
end
```

**The shipped weather card as it is actually composed** — §5.8's `pair` and the
forecast under it, which is `DrawWeatherPanel`'s three arguments in order:

```
card 1
slot: secondary
ttl: 5400
block: pair
left
glyph: cloudy
value: 70
unit: °F
secondary: 62
secondary-unit: °F
right
glyph: indoor
value: 76
unit: °F
secondary: 54
secondary-unit: %
block: statement
text: Chance Rain Showers
end
```

The `right` reading is the one a **contributor** supplied (§4.5). It reaches the
producer like this, and the producer copies it into the card above:

```
contribute 1
endpoint: indoor
ttl: 900
block: reading
glyph: indoor
value: 76
unit: °F
secondary: 54
secondary-unit: %
end
```

**Nothing on the renderer's side can tell.** The card names one producer, one
slot and one composition; that the room's temperature came from another
application is invisible to it, which is §4.5's whole claim about who owns the
card. And when the thermometer stops, the producer sends the same card with its
`right` reduced to `state: unavailable`, the panel skips that column and centres
what is left — the picture Aura draws today with the sensor unplugged.

### 6.5 Refusals, and the two kinds

**A malformed or unknown card is refused whole.** It is never drawn with the
part the renderer did not understand skipped — which is what Aura's JSON reader
does today (*"unknown types, and unknown fields within a known type, are ignored
by the renderer"*).

The reason is Aura's most expensive rendering bug, and it was not about fonts.
`.` was missing from the C++ bold atlas; raylib draws an absent codepoint as the
atlas's *first* glyph, which was `0`; a reading of `6.5` therefore appeared on
the wall as `605`. *Nothing failed and nothing logged; the number
was simply wrong.* **A partial answer that looks whole is worse than a
refusal**, and `aura-card` asserts the choice
(`an_unknown_key_refuses_the_card_rather_than_drawing_the_rest_of_it`) so that a
later kindness cannot quietly reverse it.

Refusals come in two kinds and a renderer must treat them differently:

| | What it means | What the renderer does |
| :-- | :-- | :-- |
| **content** — an unknown key, an unknown value, a missing key, a contradiction, a vocabulary from the future | one bad card | refuse **that card**, say so once in the journal, leave the slot holding whatever it held, read the next message |
| **fatal** — bytes that are not text, a message that overran its bound without ending | the framing failed | there is no position to resume from; close the connection and let the supervisor restart the producer |

Every refusal names the key, the shape or the number (`AGENTS.md` §5). A reader
that says *"bad card"* sends its author back to the encoder with nothing to look
for.

### 6.6 Bounds

The bytes reaching a reader were written by something else, so it is strict
rather than generous and bounded in ways a hand-written parser usually is not
(`rivet_core::json`'s rule).

| Bound | Value | Sized on |
| :-- | :-- | :-- |
| a message | 8 KiB | the largest card §6.4 can express, several times over |
| a value | 256 bytes | the longest thing Aura has put on the wire is an NWS `shortForecast` under sixty |
| lines in a message | 128 | four blocks of twelve rows |
| blocks in a card | 4 | the richest shipped card is two; five wants a slot of its own |
| rows in a list | 12 | a board that does not fit in a quarter of a 1080p screen has stopped being glanceable |

The message bound is enforced on the **unfinished** message too. A producer that
never sends `end` is answered with a sentence rather than an out-of-memory kill.

---

## 7. What the renderer owes in exchange

A renderer claiming conformance with this document must:

1. **Resolve every role in exactly one function** (§5.1), and in no theme draw
   two roles the same, or any role the colour of the card it sits on.
2. **Draw a reading's `value` and `secondary` with tabular figures** (§5.2).
3. **Wrap statements itself** and truncate labels itself, never asking a
   producer to (§5.3).
4. **Honour `state`** — never draw a `stale` value without its mark, never draw
   a value the state does not admit (§5.4).

   > **Amended 2026-09-02, and this is a deviation rather than a
   > reinterpretation.** All three marks are Material Symbols glyphs and the
   > face is not shipped (§11 (5), §5.6.1). `aura-renderer` therefore draws a
   > `stale` value **without** its mark and says so in the journal, naming the
   > slot, once per card rather than once per frame. That is a knowing breach of
   > the first clause, taken because rule 5 already settles the choice between
   > the two available failures: *omitting the mark is diagnosable; drawing the
   > wrong one is not*. The second clause is honoured exactly — no value the
   > state does not admit is drawn, ever. The breach closes when the face ships
   > and not before, and §12 carries it as a row rather than a footnote.

5. **Never draw a glyph or a trend it does not have.** Substituting a stand-in
   is the `605` failure. Omitting the mark is diagnosable; drawing the wrong one
   is not.
6. **Withdraw an expired card** rather than dimming or freezing it (§3.4), and
   name the slot in the journal when it does.
7. **Refuse a card that names a slot other than the one its socket was granted**
   (§3.3). The socket is the half of that pair the platform vouches for.
8. **Take no branch on who is calling** (§4.2).
9. **Draw a `pair` as one unit** (§5.8) — a shared band, a `muted` rule between
   the columns, and the group centred, so that a skipped column leaves the other
   in the middle rather than against an edge. Added 2026-09-02 with the shape.

---

## 8. Versioning

Every message carries the vocabulary it was written against, on its first line.

* A renderer **must accept every vocabulary at or below its own.** Growing the
  vocabulary means adding shapes, roles, glyphs and *optional* keys, so a card
  written against 1 is still a valid card under 2. **An addon built years ago
  keeps working**, which is the direction that matters, because the addons are
  in somebody else's repository.
* A renderer **must refuse a card declaring a vocabulary above its own**, by
  number, in the journal, and keep drawing whatever that slot last held. It
  cannot know what it is not being told, and this is a content refusal (§6.5) —
  one card from the future is not a producer that should be disconnected.

**Vocabulary 1 is this document.** A change that would break an older card —
removing a shape, making an optional key required, changing what a role means —
requires a new number and a renderer that keeps the old behaviour for old
numbers.

Aura's current schema is unversioned, and `Aura/docs/ipc-schema.md` says why
that was acceptable: *the controller and renderer are built and installed
together … so a mixed pair is not a state the system reaches on its own.* **That
premise is exactly what the addon model destroys.** A version number is the cost
of third parties existing.

---

## 9. The background, and what things are called

**The background is not an alternative to the dashboard — it is behind it.**
`cpp/src/main.cpp`'s draw block is a `// Background / screensaver` switch
followed by the overlay widgets, every frame. `ss_solid` reports
`isAnimated() == false` and the C++ renderer's `needs_redraw` genuinely lets the
screen go still; the other two force a redraw every frame
(`if (ss_animated) needs_redraw = true`). So *"the dashboard is static and the
screensaver is the animated case"* is false: the animated case is the ordinary
one whenever a person has chosen an animated background.

**Nothing in the background is a shader, and nothing ever was.** Verified two
ways: `find` turns up no `.vs`/`.fs`/`.glsl`/`.frag`/`.vert` anywhere in the
Aura tree, and `cpp/assets/shaders/` is an empty directory; the only mentions of
the word in `cpp/` are `CMakeLists.txt` patching *raylib's own* default shaders
from `#version 330` to `#version 140` for the Pi's GL 3.1. Digital Rain is
falling glyph columns; Boxes is animated rectangles. Both map directly onto
`fill`, `fill_rounded` and text.

**That is what makes §10 the decision and not a curiosity.** If nothing in the
visuals needs a GPU, then GLES-via-GBM is purely a throughput choice, and a
measurement decides it rather than an argument about capability.

### 9.1 Names

Ruled by the maintainer, 2026-09-01: **the effect is *Digital Rain***, and the
criterion is that the term describes what is on the screen rather than naming a
title anyone owns.

| Background | Palettes |
| :-- | :-- |
| Digital Rain | **Phosphor Green** (default), Ice Blue, Born This Way |
| Boxes | Ice Blue (default), **Chartreuse**, Dark, Born This Way |
| Solid | Black (default), White, Navy |

**The two greens are named from their own channel values**, which is what
settled which name went where. Digital Rain's is `{180,255,180}` head over
`{0,200,50}` trail over `{0,100,20}` dim (`cpp/src/ss_matrix.hpp`
`set_palette`): no red at all in the trail or the dim, and more blue than red in
both, which is the cool green of a monochrome CRT — the thing the effect is
evoking in the first place. Boxes' is a ramp through `{10,28,0}` forest,
`{35,80,0}` olive, `{80,160,5}` and `{145,230,60}` (`cpp/src/ss_boxes.hpp`
`kBoxGreenPalette`): red climbing and blue near zero, which is chartreuse — and
the file's own comment already calls that stop *"bright chartreuse"*. Every
value above was read out of the two files.

The old labels were the other way round, and `internal/portal/portal.go`'s
`ssThemeNames` gave the Boxes ramp the name *Matrix Green* while
`internal/portal/web/index.html` called the Digital Rain default *Classic
Green* — so neither said anything about the colour.

**Born This Way stays**, and is not the same kind of question. Its internal key
is `pride`, no reference exists anywhere in the source, and the label carries a
meaning rather than imitating anything.

**The rewrite carries no `matrix` and no `construct`.** Aura's portal already
says *Digital Rain* everywhere a person can see it; the old word survives only
in `internal/config/config.go`'s stored key, the file name `ss_matrix.hpp`, and
three portal labels. `cpp/src/main.cpp` renamed the modes to
`ConstructGreen`/`ConstructIce`, which swapped one reference from the same film
for a more obscure one — descriptive names replace both.

**No legacy aliases.** `main.cpp` accepts `matrix_green` and `matrix_ice`
alongside the new spellings so that stored configurations keep loading. The
rewrite is a new product with no stored configurations to migrate, so it accepts
neither.

### 9.2 The card behind the widgets is the background pack's to describe

The renderer draws its own cards, and the background pack draws the ground
under them, so whether a card is legible depends on both. The pack knows what
its ground does; the renderer does not. **Each theme therefore says which card
the renderer draws over it**, in its `Describe` answer
(`rivetos_addon::Theme::cards`), and the renderer holds no table of its own.

A theme gives one of three answers, and the renderer must honour each
(`ADR-053` §5.1 has the wire spelling and the version reasoning; `.rulings/`
R-210 D23):

1. **A card** — fill, opacity, ink, the rainbow flag and the two signal
   colours. Drawn as described.
2. **No card** (`"cards": null`) — no rectangle at all, and the widgets' own
   ink used as it is: the C++ renderer's `card = nullptr`. **This holds over
   every background, including one whose `prefers_dark_cards` is set.** Solid
   Black is the theme that asks for it, and it is Aura's default picture.
3. **No opinion** (`cards` omitted) — what a pack built before the field
   existed sends. The renderer falls back to `prefers_dark_cards`: a plain,
   legible dark card when it is set, and no card when it is not. The fallback
   is deliberately neutral rather than a copy of one of the pack's cards,
   because a renderer holding the pack's table would have to be edited before
   the pack could add a theme.

A pair the pack does not have has no appearance at all, so the renderer never
mistakes *unknown* for *no card*.

---

## 10. The CPU rasteriser: what a frame costs

The C++ renderer draws through raylib on its DRM backend, which is OpenGL. The
Rust renderer draws on the CPU into a buffer and copies it to the display
(`rivet_sdk::draw`, `rivet_sdk::screen`). **Whether full-screen animation at
1080p is realistic that way on a Raspberry Pi 4 is an open question, and this
section does not close it.**

### 10.1 What is already designed out, and it is the pathological case

`rivet_sdk::screen` is *one dumb buffer, mapped once*, and every drawing
primitive in `rivet_sdk::draw` operates on `canvas: &mut [u8]` — an ordinary
`Vec` in the application's own memory. Only `Screen::blit` touches the mapping,
and it is a single sequential `copy_nonoverlapping`.

**So every read-modify-write happens in cached RAM**: alpha blending, `dim`, a
screensaver fade. The catastrophic version of CPU rendering — read-modify-write
against write-combined framebuffer memory, where a read costs an uncached round
trip — cannot happen here. Confirmed by reading both files.

What remains genuinely unknown is the sequential 8.29 MB write per frame to WC
memory, which is the case WC exists for but is still bandwidth-bound.

### 10.2 Measured on the host, before and after

`tools/container/exec.sh`, x86_64 Linux, Intel i7-14700KF, one thread,
`opt-level = 3` with LTO, `1920×1080×4`. Stable to within 2% across three runs.
**Both columns come from the same harness linking the same crate**, with only
`draw.rs` and `text.rs` differing — the first run against the code as it stood,
the second against §10.3's changes.

| Phase | before | after |
| :-- | --: | --: |
| **FLOOR** `canvas.fill(0)` — touching every pixel once | 0.16 | 0.16 |
| **FLOOR** `blit` — 8.29 MB copy, cached memory | 0.29 | 0.30 |
| `draw::fill`, full screen | 0.52 | 0.51 |
| `draw::fill`, 64 cells (a Boxes frame) | 1.28 | **0.55** |
| `draw::fill_rounded`, 760×420, r=24 | 2.62 | **0.36** |
| `draw::stroke_rounded`, the same, 3px | 1.66 | 1.63 |
| `draw::dim`, full screen | 0.38 | 0.37 |
| 5 glyphs at 292px (the clock) | 0.17 | 0.13 |
| 26 glyphs at 60px (the date) | 0.047 | **0.028** |
| 6360 glyphs at 18px (a full Digital Rain grid) | 2.40 | **1.30** |
| 1590 glyphs at 18px (a quarter lit — the steady state) | 0.64 | **0.34** |
| **FRAME** solid background + 2 cards + type + blit | 7.80 | **1.87** |
| **FRAME** Digital Rain + 4 cards + type + blit | **15.63** | **3.31** |

**The two floor rows are what make the rest readable.** Any full-screen frame
must touch every pixel once (0.16 ms) and reach the display (0.30 ms), so
**0.46 ms is the floor** and the optimised Digital Rain frame is about seven
times it. There is more to win; there is not an order of magnitude more.

**A Raspberry Pi 4 is not this machine, and no honest scaling factor exists
without measuring one.** A Cortex-A72 at 1.5 GHz against a 14700K P-core is
roughly 3.5× on clock alone, and scalar IPC on this kind of work is plausibly
another 2–3×. That puts the optimised Digital Rain frame between **26 ms and 40
ms against a 33.3 ms budget** — which still spans the answer, and is why §10.4
is an instrument rather than a conclusion. What the change does settle is the
*before*: at 15.6 ms on this host, the same arithmetic would have been between
110 ms and 175 ms on a board, and no amount of tuning elsewhere would have
rescued it. Memory bandwidth is the other half and moves the same way: a Pi 4
sustains a few GB/s where this host sustains tens.

### 10.3 Where the cost was, and what changed

All of it is in the SDK rather than in Aura, so every kiosk application on this
platform was paying it.

* **`draw::blend` had no opaque fast path.** It is called per pixel, recomputes
  `y * stride + x * bytes_per_pixel` every time, takes three early-return
  branches, and did three `f32` multiply-adds plus three multiplies **even at
  alpha 1.0**, where the answer is the colour. It now returns the colour
  directly, which is exact rather than approximate: at `a == 1.0` the general
  expression is `(fg + 0.5) as u8`, and that truncates back to `fg`.
* **`draw::fill_rounded` ran the full signed distance, `hypot` included, at
  every pixel of a shape whose interior is by definition solid.** Coverage is
  now computed only in the four `ceil(r)`-wide corner squares. **2.62 ms to 0.36
  ms.**
* **`draw::fill` blended pixel by pixel.** It clips once and writes each row as
  a slice now. The full-screen case barely moved — the compiler was already
  vectorising it — but the 64-cell Boxes frame more than halved.
* **`text::Face::draw` re-rasterised every glyph on every call**, with no cache.
  A `(char, size)` cache, bounded at 1024 entries, roughly halves both Digital
  Rain figures and takes a third off a line of date text.

**`draw::dim` was tried and reverted, and the measurement is the finding.** It
looks like the most obvious target of all — a multiply, a constant divide and a
*fallible* `u8::try_from` per byte across eight megabytes, with the fallible
branch dead. Replacing it with a 256-entry lookup table made it **3.2 times
slower: 0.36 ms to 1.17 ms.** The arithmetic auto-vectorises, because the
compiler lowers the constant divide to a multiply and a shift and does sixteen
bytes at a time; **a table lookup cannot be vectorised at all.** The function is
bandwidth-bound as written and the table traded a SIMD stream for a gather. The
attempt is recorded in the source rather than deleted, because the next reader
will have the same idea.

> **Every one of these is a change that alters output by a bit per channel if it
> is wrong, on an edge pixel, in one theme — the kind of thing that is
> discovered by comparing two photographs of a television.** So the originals
> are kept verbatim as reference implementations in `draw.rs`'s test module, and
> `the_fast_paths_are_byte_identical_to_the_general_ones` runs every case
> through both and compares the buffers **byte for byte**, on a canvas that is
> not blank, at four sizes, six radii, four placements including past every
> edge, and ten `dim` strengths. The glyph cache has its own equivalence test
> against a face that has never seen the string. Both were confirmed to fail
> when the fix is broken by one bit, which is the only thing that proves a
> regression test is wired to the defect.

### 10.4 The instrument: `examples/aura/aura-probe`

**Built and never run.** It is a Rivet kiosk application, so a person standing
in front of the television selects it from the launcher menu, and an agent
selects it over `apply input`. It measures on the machine it is running on and
prints its table to the journal, draws it on the screen, and captures it as QOI
so somebody who is not in the room can read it.

It exists because the maintainer's method is **measure with the known problems
present, fix them, measure again** — which needs one instrument used twice, not
two.

Four things about how it is built are load-bearing:

* **It measures the floor first.** `canvas.fill(0)` is the memory-bandwidth cost
  of touching every pixel once; a bare `blit` of an untouched canvas is the cost
  of getting 8.29 MB to the display and nothing else. Without those, *"`fill`
  went from 40 ms to 6 ms"* is unreadable — nobody can tell whether 6 ms is
  near-optimal or still five times off. **The floor is what says when to stop
  optimising.**
* **It reports against budgets, not frame rates.** Every phase is scored against
  33.3 ms (30 fps), 25 ms (40 fps) and 16.7 ms (60 fps), because the decision is
  *does this fit, with headroom* and not *what number does it reach*.
* **It runs to thermal steady state.** A Pi 4 throttles under sustained
  full-screen drawing and a two-second benchmark measures the boost clock. Each
  phase runs for a window and reports the **first second** and the **sustained
  remainder separately**, and the whole cycle repeats for ever with the elapsed
  time printed beside it — so throttling shows up as the same phase getting
  slower cycle over cycle rather than hidden inside an average.
* **It is uncapped, and the renderer is not.** Raw per-phase cost is the whole
  measurement.

The phases are the floor, then the primitives, then the two backgrounds as they
actually are, then **whole frames** — a full-screen animated background *plus*
four cards, because §9's finding means "the cards are static and therefore
cheap" is false. It also measures a full-screen alpha composite, which is what a
layered design (cards cached in their own buffer, composited over a redrawn
background) would cost per frame instead of redrawing every rounded rectangle
and glyph. That composite is not free, so the comparison is measured rather than
assumed.

### 10.5 Two platform findings the measurement turned up

**`Screen::blit` cannot copy a region.** It copies from the start of the mapping
for `canvas.len()` bytes, so the only partial copy expressible is *the top N
rows*. A clock in a rotating corner is a rectangle, not a prefix, so damage
tracking is not available to a kiosk application today: a one-pixel change costs
the whole 8.29 MB. Whether the SDK grows a `blit_rows` or a rectangle blit is
the maintainer's call; the fact is recorded here because it decides whether
redraw-on-change is worth anything for a dashboard.

**There is no double buffer and no vblank, so there is tearing.** `screen.rs`
says so itself and accepts it — for `kiosk-demo`, whose job was to prove a
capability. For a dashboard repainting once a second it is invisible. **For a
full-screen animated background it is a visible artefact that no amount of CPU
speed fixes**, because the renderer is writing into the buffer being scanned
out. This is a platform question — double buffering belongs in the SDK, not in
Aura — and it is stated here so that it is heard rather than discovered on a
television.

### 10.6 The frame budget, and a third finding

`aura-renderer` sleeps whatever is left of a 33.3 ms budget after its work,
rather than sleeping a fixed amount after it. 30 fps is the default because
Aura's C++ renderer asks raylib for exactly that (`SetTargetFPS(30)`) and has
run on a wall at it for months; a clock changes once a minute, a forecast every
twenty, and the animated backgrounds tick at one to fifteen cells a second.
Nothing here is helped by 60 and nothing is helped by 180.

**Nothing tells a kiosk author to do this**, and the shape that is demonstrated
is not quite right. Verified: `examples/templates/template-kiosk` sleeps a
constant 33 ms with no comment explaining it, and `cmd/rivet-launcher` sleeps a
constant `TICK` of 50 ms. Both are *after* the work, so the real interval is
`work + constant` and stretches silently as drawing gets heavier. Nothing in
`rivet-sdk` mentions frame pacing at all, and `SPEC-03` §3.7's Rust example is
three lines with no loop. An author who writes the obvious loop from that
example gets one that spins as fast as the CPU allows, on a board that runs
twenty-four hours a day.

### 10.7 Measured ON THE BOARD, with the real background drawing

Everything above §10.6 was measured on an x86-64 host or not at all, and the one
hardware number in circulation was **35.7 ms against the 33.3 ms budget** — taken
from `aura-renderer` 0.2.0 with a nearly static screen and **no animated
background**, so it was the optimistic case and it still looked like a failure.

It was not a failure, and it was not measuring what it looked like it was
measuring.

`aura-renderer` 0.3.2 reports its own cost every 600 frames. Digital Rain,
Phosphor Green, the 292 px clock on its card, 1920x1080, Pi 4:

| phase | mean | max |
| :-- | --: | --: |
| background — Digital Rain, whole screen | 12.4 ms | 15.7 ms |
| widgets — the clock and its card | 5.4 ms | 9.8 ms |
| **drawing (the two above)** | **17.8 ms** | — |
| `blit` — the copy **and the wait for vblank** | 10.6 ms | 25.2 ms |
| whole frame | 28.4 ms | 43.7 ms |

**17.8 ms of the 33.3 ms budget. 53%, with the animated background running.**

**The 35.7 ms was mostly a page-flip wait.** `Screen::blit` copies into the back
page and then **blocks until the flip completes**, which is the panel pacing the
loop and is what it is for. Counting that as drawing cost is the error the split
above exists to prevent, and it is the error the earlier figure invited.

**Thermal steady state was watched, not assumed** — because a two-second
benchmark measures a Pi 4's boost clock and nothing else.

Twenty-four consecutive windows, eight minutes: background 12.24, 12.20, 12.10,
12.24, 12.22, 12.42, 12.39 ms. A board dropping off its clock would show as a
step between two lines; there is no step. The first window after a start is the
only outlier (15.7 ms max), and that is a cold glyph cache rather than a warm
CPU.

**And the board says the same thing from the other side.** `rivet get thermal`
after eight minutes of this:

* `cpu-thermal` **60.4 °C**;
* `throttled_since_boot: false`, `frequency_capped_since_boot: false`,
  `under_voltage_since_boot: false` — the firmware's own flags, which are the
  authority on whether this board has been held back at any point this boot;
* the ceiling still at 1.5 GHz, and the **current clock 1.0 GHz** under the
  `ondemand` governor.

That last number is the interesting one. **The 53% figure was measured at two
thirds of this board's maximum clock**, because the governor never needed the
rest. The headroom is larger than the table says, and how much larger has not
been measured — forcing the `performance` governor would answer it, and nothing
here needs the answer.

**The maxima are the number to watch, not the mean.** 43.7 ms is one frame in
600 landing 10 ms over budget — a dropped frame, once every twenty seconds,
which on a wall panel showing rain is not visible. It is recorded because a mean
would have hidden it.

**Nothing in Aura's visuals needs a GPU** — they are rectangles and glyphs, and
`cpp/assets/shaders/` is empty. What GLES would buy is a decision for the
maintainer and this section is not an argument for either answer; it is the
number he asked for.

### 10.8 What it costs to draw the background through an ADDON

§10.7's background is code inside `aura-renderer`. `.rulings/` R-199 D1 moves
it into a pack the renderer loads, and R-199 D2 makes the frame cross as a
`#[repr(C)]` descriptor. **The question that decides whether that is affordable
is what one indirect call across that boundary costs**, and it was measured on
`rivet-06f5c` — the same board as §10.7 — on 2026-09-19, by
`examples/probes/frame-probe-host` drawing through
`examples/probes/frame-probe`. Both halves built `opt-level = 3` for
`aarch64-unknown-linux-musl`, 1920x1080, stride 7808.

Three phases, because a number with no floor under it is unreadable — the same
reason §10.4 measures `canvas.fill(0)` before it measures anything else:

| phase | round 1 | round 2 | round 3 |
| :-- | --: | --: | --: |
| the timing loop and the clock, empty | 5 ns | 5 ns | 5 ns |
| **one frame across the boundary, drawing nothing** | **271 ns** | **270 ns** | **270 ns** |
| a whole-surface fill, across the boundary | 4.576 ms | 4.329 ms | 4.562 ms |
| the identical fill, in the host, crossing nothing | 4.265 ms | 4.608 ms | 4.584 ms |

**One crossing costs about 265 ns**, floor subtracted: **0.0007% of the 33.3 ms
frame budget, and 0.002% of §10.7's 12.4 ms background phase.** Two thousand
crossings were timed per round. **Across four runs of the whole article, from
four separately built pairs, every round landed between 261 ns and 290 ns** —
the spread is the `ondemand` governor moving the clock under a loop that runs
for half a millisecond, and it is quoted as a range rather than averaged away
because 290 ns is as true as 265.

**The second pair is the one to read carefully, and what it says is that the
difference is not measurable in a real frame.** The same 8.29 MB fill is
*faster in the host* by 7% in round one and *faster across the boundary* by 6%
in round two — and across the three runs the sign of that difference flipped
both ways. Neither direction can be true of a crossing. It is what it looks
like: two compilations of one loop, inlined differently, on a board whose
governor moves the clock while the loop runs. The spread is around **a thousand
times** the 265 ns the crossing actually adds, so **a per-frame difference
cannot be established from this pair at all** — which is why it is reported as a
pair rather than subtracted, and why the per-call figure above is the answer
instead.

**What this settles**: a background drawn through an addon spends the same
12.4 ms it spends today, plus a quarter of a microsecond. The frame budget is
not what moves when drawing crosses this boundary — what moves is **where a
fault lands**, which is `docs/design-candidates/Background-Addons-2026-09-19.md`
§4's point and is not a number.

**What it does not settle**: a pack with several backgrounds may draw
differently from the renderer's own code for reasons that have nothing to do
with the crossing. `PLAN-Addon-Feature-Set-2026-09-19` T6 compares the two byte
for byte, and T8 measures each new background against the budget separately.

---

## 11. Open questions

Recorded rather than answered.

1. **The clock's time zone — CLOSED 2026-09-02.** This item read *the clock has
   no time zone*, because a Rivet had no `/usr/share/zoneinfo` (`SPEC-01` §5.2)
   and so nothing on the machine to ask. **The PLATFORM half was ruled that day
   — `ADR-021` §9**: a Rivet's document carries a `timezone` key and the
   supervisor projects `/etc/localtime`.

   The renderer's half was then framed as a choice between a tzfile reader of
   its own — RFC 8536, and a real amount of code for a clock — and `chrono`,
   whose `Local` reads exactly that path and which is a dependency `ADR-009`
   would want an argument for.

   **That framing was the mistake, and the item closed the same day.** Both
   options were answers to a question nobody had to ask: **musl already contains a
   tzfile reader.** It is `localtime_r(3)`, it reads exactly the
   `/etc/localtime` the supervisor projects, and `cmd/rivet-sysd`'s
   `Kernel::local_time` was already calling it — written the same morning, in
   the same tree, with the reentrancy argument already worked out. `tm_gmtoff`
   is the whole of the renderer's half.

   `examples/aura/aura-renderer/src/clock.rs` calls it. The renderer shows
   local time and the constant is gone. `libc` is the only dependency it added,
   and `ADR-009` has no argument to make about `libc`.

   **How it was found:** the maintainer looked at the wall, saw UTC on a board
   set to `America/Chicago`, and said so. It had been recorded as an open
   question rather than tried.
2. **An application on a Rivet has no configuration surface at all.** No `env`
   key in a manifest, no argv, and no control-plane verb that writes a file an
   application could read — `rivet` has `get file` and no counterpart. So the
   frame budget, the brightness, the theme, the twelve-hour clock and the probe's
   phase length are all *a constant in one file, changed and rebuilt*. Aura's Go
   controller exposes every one of these through an HTTP portal today. **This is
   the largest gap between what Aura is and what a Rivet application can be**,
   and it is a platform decision.
3. **Who holds Aura's settings?** Following from (2): the natural answer is that
   the portal survives as its own Rivet application with `listen = ["tcp:8080"]`,
   and that it needs a channel to the renderer that this vocabulary does not
   provide — cards go one way and carry no commands. Aura's existing `cmd`
   messages (brightness, screensaver, sleep, screenshot) have no home in this
   design yet.
4. **`primary` is claimable.** §4.4: an addon may take the clock's slot. That
   falls out of *no privileged path* and nobody asked for it.
5. **The icon face — corrected 2026-09-02, §5.6.1.** This item said thirteen
   glyphs and five trend arrows need a face. **TWO of the thirteen need a face,
   and eleven
   do not**: the nine weather conditions, `unavailable` and `indoor` are drawn
   with primitives in `DrawWeatherIcon` and `DrawIndoorIcon` and are drawn with
   primitives here, so the weather panel was never blocked on this. What still
   needs a face is `clock`, `bus`, **the five trend arrows and the two
   connection marks** — and the C++ says the arrows must not be hand-drawn,
   having tried it. The platform's `ICON_FONT` is a five-glyph Material subset
   cut for the launcher,
   and Aura ships its own subset today.

   **Whether Aura carries its own asset or the image's icon font grows is not a
   decision, and never was — corrected 2026-09-02 on
   the maintainer's question: *what font decision needs to be made that wasn't
   already made in the original Aura?*** None. `Aura/assets/MaterialSymbols-Aura.ttf`
   exists, is already subset to exactly these marks, and `cpp/src/main.cpp:209`
   already loads it. The renderer already ships three faces as `ADR-015` §3
   package assets — Source Code Pro Bold and Regular, and a 105-codepoint
   IPAGothic subset — so the precedent is established three times over.

   **This item became unbuilt work rather than an open question, and was DONE
   the same day for the half that had a home.** The face ships; `crate::icons` carries the seven codepoints;
   the generic composition draws the marks and the arrows. Re-opening a decision
   the product had already made is what turned a task into a question, and it is
   the same mistake `theme.rs`'s invented palettes were — the whole thing took
   an afternoon once it stopped being a question.

   **What is left is the original renderer's reading card itself**, which is a
   composition and not a font: `secondary`'s panel still draws no mark, because
   `DrawWeatherPanel` has none and R-125 fixes its appearance. That is a ruling
   being obeyed rather than a gap.

   **What the half that still needs a face costs, stated because it is the
   interesting part:** the mark §5.4 requires beside a `stale` reading, and the
   `warn`/`alarm` mark it requires *instead of* a value for `connecting` and
   `unavailable`, are both in it. So until the face ships those three states
   draw without their mark — §7 rule 4's note says what the renderer does and
   why omitting is the lesser of the two failures §7 rule 5 names.
6. **Whether `aura-card` is published.** An addon in a private repository cannot
   depend on a path. crates.io, a vendored copy, or a git dependency — and the
   name `aura-card` is unclaimed as far as this work checked, which it did not.
7. **Cards should show a `title`.** `PLAN.md` §104.45.4 already asks for the
   launcher to draw an application's declared `title`; Aura declaring one is the
   other half.
8. **Whether Aura ships enabled.** The same question `PLAN.md` §104.45.4 asks
   about the launcher, one level down.
9. **Which background runs is a build-time constant, and this build does not
   ship Aura's default.** `cpp/src/main.cpp` starts at `ScreensaverMode::Solid`
   — plain black, no card, 48 px padding — and the portal is how a person
   changes it. `aura-renderer` has no portal (see (2)), and the complaint that
   opened this work was that *Digital Rain is named but does not run*. So
   `BACKGROUND` in `main.rs` is `DigitalRain(PhosphorGreen)`. **That is the one
   thing in the port that is not what Aura does**, it is a default rather than
   an appearance, and it is named here rather than left to be discovered.
10. **`SolidScreensaver::Warm` is unreachable in Aura and is not ported.**
   `ss_solid.hpp` defines `{30, 20, 10}` on `{255, 220, 160}`; `ParseScreensaver`
   in `cpp/src/main.cpp` has no string that selects it, and neither does the
   portal. It is a preset that exists in a header and has never been on a
   screen, so porting it would be inventing a background rather than copying
   one. The ruled palette table in §9.1 also omits it, which is consistent —
   but the two facts were arrived at separately and somebody should decide
   whether it is dead code in Aura or a feature that was lost.
11. **Aura constructs and updates all ten screensavers on every frame.**
   `cpp/src/main.cpp` holds ten objects, calls `init` on seven at start-up and
   `update` on seven per frame, and draws exactly one. Ten `std::mt19937`s.
   `background.rs` builds only the selected one, because the unselected ones
   cannot reach the screen and reproducing the waste would cost a Pi 4 frame
   time to reproduce nothing visible. **That is a deviation from the C++** and
   it is recorded here rather than presented as fidelity.
12A. **What was NOT reproducible in the panel, reported rather than
   approximated** — added 2026-09-02 with §5.8's implementation, in the shape
   `aura-renderer/src/widgets.rs`'s own module note takes.

   * **Every curve is smoother here than on the wall.** raylib draws a circle as
     a 36-sided polygon and a thick line as one quad, both through the GPU with
     no multisampling; `rivet_sdk::draw::circle` and `line` compute exact
     coverage. This is the same difference `fill_rounded_alpha` already carries
     against `DrawRectangleRounded`, in the same direction, and closing it would
     mean writing an aliased polygon rasteriser.
   * **The 96 px and 62 px numbers are sharper here.** raylib rasterises one
     atlas at `Font::baseSize` and draws every other size as a bilinear resample
     of it, so the panel's temperatures on the wall are resampled 292 px glyphs.
     `typeface.rs`'s note already recorded this and named the panel as the only
     place it bites; it now bites.
   * **The C++'s unknown-slug fallback is not ported.** `DrawWeatherIcon`'s
     `else` draws a dotted circle for a slug it does not recognise. In this
     vocabulary a producer has already collapsed the slug space to §5.6's
     thirteen names, so the branch is unreachable — and a glyph with no drawing
     is omitted rather than substituted (§7 rule 5).
   * **A glyph alone can now draw a lone house.** §5.8.1's renderer obligation
     carries this: the C++ chooses `DrawIndoorIcon` from a `bool indoor` flag
     and here the house is `glyph: indoor`, which is data. Unreachable for the
     shipped card, because a producer with no room reading sends no room glyph.

12B. **The outdoor column drew with NO ICON on the board, and that is correct.**
   The capture that proves §5.8 shows a house on the right and nothing on the
   left, which looks like a defect and is not: `cmd/aura-controller` was sending
   an **empty** condition slug at the time — its own journal line reads
   `weather: 64°F dewpoint=64°F  |`, with nothing between the dewpoint and the
   forecast — and `PanelColumn`'s rule is that a column with no glyph reserves
   no width for one (§5.8.3 item 4). So the ten drawn weather conditions are
   **tested and not proven**: nothing has put one on a screen. The cheapest way
   to close that is to look at the wall when the slug is next non-empty; the
   controller's journal says when that is.

12. **`theme.rs` and `layout.rs` are now unreferenced.** `Theme`'s `night` and
   `day` were chosen for the generic composition rather than taken from Aura,
   and Aura's real card themes are `CardTheme` in `cpp/src/main.cpp`, ported to
   `widgets::card_theme`. Both files are kept with `allow(dead_code)` and a note
   saying so. Whether a generic composition returns at all — for an addon whose
   card is not one of Aura's three — was a design question.

   **RULED and BUILT 2026-09-02 — the maintainer's ruling is that a generic
   composition returns.** §5.9 specifies it; `layout.rs` implements it and
   `tertiary` and `quaternary` draw through it. `theme.rs` is superseded rather
   than merely unreferenced — see §5.9.4.

   **The dilemma was false, and the term it missed is the same kind of term
   §5.8 missed.** It read as *either* a generic composition (which would put a
   picture that is not Aura's on the wall, and R-125 forbids that) *or* no
   generic composition (which leaves a shape a third party can send that
   nothing draws, and §4.4 forbids that). The third option is that **a
   composition and an appearance are two different things**: the *stack* is
   generic — blocks in the card's own order, and nothing in `layout.rs` knows
   what a bus or a forecast is — while every *number* is `widgets.hpp`'s,
   reached by calling `widgets::measure_column` and `widgets::draw_column`
   rather than by re-deriving them.

   R-125 fixes the appearance of **Aura's cards**, and it is untouched:
   `primary` is still `DrawClockWidget` and `secondary` is still
   `DrawWeatherPanel`, hard-coded, because the cards that occupy them are
   Aura's. A slot Aura has no widget for has no fixed appearance to violate —
   there is no C++ picture of a departure board to be unfaithful to.
13. **A page flip was refused once, at lease teardown.** `drmModePageFlip` said
   `-2` (`ENOENT`) on crtc 112 as the launcher revoked the lease, and the SDK
   degraded that screen to one buffer and said so — correctly, and the process
   was being stopped a second later. It has not been seen in steady state
   across three runs. Recorded because the SDK's fallback is **permanent for the
   life of the process**, so the same refusal arriving mid-session would leave a
   wall panel tearing indefinitely with one line in a journal nobody is reading.
14. **`DrawRainbowCard` — the Born This Way card — is BUILT, 2026-09-02.** This
   item said `rivet_sdk::draw` had no gradient and no circle so it could not be
   ported, **and the claim was half stale when it was written down.**

   `circle` had arrived with the weather glyphs and nobody revisited the
   sentence; **only the gradient was missing**. `rivet_sdk::draw` now has
   `fill_gradient_h` and `fill_gradient_v` — raylib's two, with the ALPHA
   interpolating as well as the colour, which is not decoration: the card's
   soft edge is four gradients from opaque black to *transparent* black and a
   primitive carrying one opacity could not draw it.

   `widgets::rainbow_card` is the port, constant for constant. The clock, the
   weather panel and the generic composition all draw it now; until today all
   three skipped the card entirely for that palette, which was the right
   refusal — *drawing a flat fill in its place is not one* — and left Born This
   Way unusable.

   **The refusal cost a fortnight over one missing primitive**, and the
   primitive was two functions. That is worth more as a lesson than as a
   footnote: *"cannot be ported"* was true of the day it was written and
   nothing re-checked it.

15. **`Role::Accent` had no colour and drew as `Role::Normal`.** Added
   2026-09-02 with §5.9: `CardTheme` carries `text`, `alert` and `caution`,
   Aura never defined an accent, so there was no sixth field to reach for, and
   `widgets::role_ink` returned the ordinary ink for it with **nothing
   saying so on screen** — an addon that asked for accent silently got normal.

   **CLOSED BY DELETION the same day**, on the maintainer's question — *where do
   you need this accent colour that the Aura screensaver themes aren't already
   providing?* The honest answer was nowhere, so the role went rather than the
   hue arriving. §5.1 carries it. Everything below this line is what the item
   said before it was closed, kept because the reasoning is the useful part:

   On the two dark card themes this is nearly right by accident: `kCardGrn`'s
   text is `{180,255,180}`, the rain's head colour, and `kCardIce`'s is
   `#BECDF7` — the two colours §5.1's own `Role::Accent` note names as what
   accent is for. On `kSolidWhite` it is plainly wrong; accent there is black.

   It is recorded rather than fixed because the fix is choosing a hue that
   exists in no Aura source, which is the one thing this port does not do. The
   cheapest honest closure is a sixth `CardTheme` field with a value **per
   palette**, chosen by somebody looking at a television — which is how the
   existing `alert` and `caution` pairs were chosen, and why they are per theme
   at all.

16. **The rainbow card's corners carry a visible seam, and it is the
   ORIGINAL's.** Found 2026-09-02 by looking at `target/rainbow-card.ppm`
   rather than by a test — every test passed.

   The card is translucent shapes that OVERLAP, drawn body → strips → four
   discs → four fades. A corner disc spans one quadrant no strip reached and
   three that one did, so at `alpha = 190` the outer quadrant is one coat
   (`190/255` = 0.745) and the inner three are two (0.935). A step of 0.19 along
   a straight edge, exactly on the disc's centre lines.

   `cpp/src/widgets.hpp:307-316` draws the same shapes in the same order at the
   same alpha, so raylib produces it too — **the port is faithful and the
   artifact is inherited.** Pinned by
   `the_rainbow_corner_carries_the_originals_double_blend` so that changing it
   is deliberate.

   **What the maintainer has to say**, because nobody else can: whether the wall
   has ever looked wrong. If it has, the fix is not small — compositing the
   whole card opaque into a scratch buffer and applying `alpha` once, which is a
   *different picture* and therefore a decision rather than a repair (R-125).

---

## 12. Conformance: what is built, and what has run

**This section runs on the board, as `aura-renderer` 0.3.2**, and the rows below
say what was proven and by what method. It said *nothing in this section has run
on hardware* until 2026-09-02.

**A capture proves the pixels that were drawn. It does not prove the absence of
tearing**, which is a property of when a buffer reaches the glass. The evidence
for that is a different line in the journal, and it is in the table.

| | State | Proven by |
| :-- | :-- | :-- |
| the vocabulary and the wire (`aura-card`) | **built** | **38** tests, run on the host. Includes hand-composed bytes decoded against the writer, a list carrying a non-bus list, a pair carrying a thermostat and two football scores, and every refusal in §6.5. `a_pair_with_one_side_is_refused_by_name` was broken on purpose and watched to fail before it was kept |
| the framer | **built** | reassembles a card split at **every one of its byte offsets** — which is how the first version's UTF-8 bug was found |
| `aura-renderer` — ingest, store, expiry, clock | **built and RUNNING** | 36 unit tests now run: the build container has no host `libdrm`, and `pacman -Sy libdrm` inside it is enough to link the test binary. §12.1's split was half the answer; this is the other half |
| the clock's **time zone** | **built and PROVEN** | `localtime_r(3)` — §11 (1). The board is `America/Chicago`; the capture reads `11:33` where UTC was `04:33`, and `aura-controller` independently logs the same local minute |
| the **typefaces** | **built and PROVEN** | Source Code Pro Bold/Regular and a 105-glyph IPAGothic subset ship as `ADR-015` §3 assets. Journal: *typefaces loaded from /state/app/aura-renderer/current*. The subset's outlines, advances, `unitsPerEm` and `hhea` were checked identical to the full face with `fontTools` — zero mismatches |
| raylib-compatible metrics (`typeface.rs`) | **built and tested** | 5 tests. `stb_truetype` scales by `px/(ascent−descent)` and `fontdue` by `px/units_per_em`; for Source Code Pro that is 1257 against 1000, so *"292 px"* differed by **25.7%** and the clock was a quarter too large |
| the backgrounds — Digital Rain, Boxes, Solid | **built; Digital Rain PROVEN, the other two only tested** | ported constant for constant from `ss_matrix.hpp`, `ss_boxes.hpp`, `ss_solid.hpp`. Digital Rain is on the board and captured. Boxes and Solid have unit tests and **have not been on a screen** — nothing selects them, because there is no configuration surface (§11 (2)) |
| the clock widget (`DrawClockWidget`) | **built and PROVEN** | corner anchoring, the rotation, the 20% descender trim, the 18% card-top trim, the per-background card themes. Captured off the board |
| **the leased page flip** | **PROVEN ACCEPTED** | the highest-risk unknown in the display work, and nothing printed it until now. `Screen::what`: *"a leased display connector 42 crtc 112 mode 1920x1080 … double-buffered, page-flipped at vblank"*. **This is not the same as proving there is no tearing** — it is proof that the mechanism which prevents it was accepted |
| the frame budget | **MEASURED on the board** | §10.6. Digital Rain background 12.4 ms, widgets 5.4 ms — **17.8 ms of the 33.3 ms budget, 53%** — whole frame 28.4 ms mean, 43.7 ms max, steady across four minutes. Reported by the renderer itself every 600 frames |
| the Born This Way card (`DrawRainbowCard`) | **built and tested; NOT on a board** | 2026-09-02. `rivet_sdk::draw` grew `fill_gradient_h`/`_v` — the circle had been there since the weather glyphs, so the recorded blocker was half stale. Ported constant for constant and written to `target/rainbow-card.ppm` for a person to judge; the gradient ramp and the absence of gaps between the three strips are asserted, because a hairline of background through a rainbow is very visible. It carries the original's corner seam deliberately — §11 (16) |
| the eleven **drawn** glyphs | **built; `indoor` PROVEN, the other ten only tested** | §5.6.1. Circles and thick lines, ported branch for branch from `DrawWeatherIcon`, `DrawIndoorIcon` and `DrawCloudBody`. The house is on the board and captured. **The ten weather conditions have not been on a screen**, because the condition slug the controller was sending at the time was empty — see the row below |
| the trend arrows and the connection marks | **built and tested; NOT on a board** | 2026-09-02. `MaterialSymbols-Aura.ttf` ships as a fourth package asset — Aura's own file, already subset to these seven codepoints — and `crate::icons` maps them. The generic composition draws the mark LEFT of a reading in its own role (amber for a question, red for a fault) and the trend arrow RIGHT of it in the reading's. Both centred on the **digits' ink band**, which took three attempts and `target/generic-card.ppm` to get right: centring the line box puts an icon visibly high. `clock` and `bus` remain undrawn — they are in the vocabulary and in no Aura source |
| **the weather panel (`DrawWeatherPanel`)** | **built and PROVEN** | §5.8, ruled 2026-09-02. This row also carried the corner reading card, *not built, and BLOCKED*, until then. Both columns, the rule between them, the stacked second lines, the group centring and the wrapped statement, ported constant for constant. **On the board and captured**, drawing a real NWS observation and a real room sensor. Eight unit tests, including one that writes the panel to a file so a person can look at it without a board |
| **the corner reading card (the original renderer's reading card)** | **built and tested** | `layout.rs`'s `Piece::Headline`, ported constant for constant: a `reading` carrying a value, a unit and a trend draws as the original card — the big number, the trend arrow stacked over the unit, the state's mark to the left — now that the icon face ships (the trend-arrows row above). `aura-renderer`'s `controller` module builds it from the controller's `reading` object, which the controller fills from whatever addon provides `aura-reading-source`; the host knows nothing of what that source measures (`.rulings/` R-375 D1) |
| **the generic composition (`layout.rs`)** | **built and tested; NOT on a board** | §5.9, ruled 2026-09-02 — the answer to §11 (12). All four shapes draw, `pair` included; corner-anchored at the reading card's corner and its opposite; every constant is `widgets.hpp`'s, reached by calling `widgets::measure_column` and `widgets::draw_column` rather than re-deriving them. **8 unit tests in `layout.rs`** plus the four-corner property in `widgets.rs` — 44 in the crate before, 50 after — including one that writes the card to `target/generic-card.ppm` so a person can look at it without a board, and one that asserts the return value agrees with whether any ink reached the canvas. Cross-compiles clean for `aarch64-unknown-linux-musl`. **Nothing has sent a card to `tertiary` or `quaternary` on the board**, so this has been seen on a workstation and not on the wall — the fan-out (`PLAN.md` §112.2) is what will put it there |
| the shipped weather producer | **DELETED 2026-09-02, and there is no weather producer** | It was `examples/aura/aura-bridge`, and it read `cmd/aura-controller`'s JSON and re-emitted it as cards so the weather would arrive through the addon door. The renderer now reads that socket itself, so there is nothing left to bridge — and the bridge is why `cmd` was lost, because a relay that forwards cards necessarily drops directives (`§1.0`, `§4.2`, `PLAN.md` §119). Its `state`-to-card tests moved into the renderer with the code |
| **the two producers** (`aura-vitals` is now **`aura-vitals-test`**; `aura-vitals-go` is gone) | **the Rust half survives as a TEST INSTRUMENT; the Go half is deleted** | `PLAN.md` §112.2's fan-out, renamed and pruned 2026-09-03 at the maintainer's ruling — an instrument should be named as one. The pair was the same producer in Rust and Go: same `connects`, same `/proc` and `/sys` reads, **same bytes**, both asserting against `examples/aura/vitals-fixture`'s single `expected-card.txt`, because two suites each carrying a private copy would prove only that each matches itself. 18 tests. Cross-compiles for `aarch64-unknown-linux-musl`; has not run on hardware |
| **the wire format as a SPECIFICATION** | **PROVEN off-board, and the proof stands after the deletion** | the sharpest thing the pair bought. `aura-vitals-go/card.go` wrote §6.3 from this document and nothing else — a Go program cannot link `aura-card` — and produced the byte-identical card. Until that existed, §6.3 was a description of what the Rust crate happened to do; `ADR-036` §6.2B's *"two implementations are what prove it"*, discharged. **The Go half was deleted 2026-09-03 having discharged it**, and this row is the record that it did; the claim does not rest on code still being in the tree, but re-proving it would need a second implementation again |
| the **contributor** (§4.5) | **specified, NOT built** | no contributor exists and no producer reads one. The indoor reading reaches the card inside the controller's own message, so the panel's two columns are proven and the SEPARATION of the second one is not |
| the frame cost of the panel | **MEASURED on the board** | widgets went from 4.88 ms to 8.82 ms mean when the panel appeared; whole drawing 20.6–20.9 ms of the 33.3 ms budget (**62–63%**, up from 53%), steady across three consecutive windows. Frame 30.2 ms mean, 45.4 ms max |
| `aura-probe` — the instrument | **built, never run** | compiles clean for the target. §10.4. **Deliberately not run on this board**: it is a kiosk application, so running it would take the screen the maintainer is watching, and the renderer's own per-phase report measures the real port rather than the probe's synthetic approximation of it |

### 12.1 Two SDK defects that were hiding each other

Both were found while trying to measure §10, both are fixed, and the second
could not have been found without the first.

**A kiosk application's drawing could not be tested anywhere.** `rivet-sdk`'s
`draw` feature compiled `screen.rs`, which links `libdrm`; the build container
has no host `libdrm`, so **enabling `draw` on any host failed to link** and a
crate using it could only be built for `aarch64-unknown-linux-musl`, where its
test binary cannot execute. `cargo test --workspace` leaves `draw` off, so
`draw.rs`'s own tests — the rounded corner, the antialiased edge, the dim range
— **were compiled by nothing.** Nothing about drawing needs `libdrm`; only the
display does.

`screen` is now its own feature and `draw` is pure Rust. The cost was one word
in three manifests (`features = ["draw"]` became `["screen"]`); the return is
`cargo test -p rivetos-sdk --features draw` on any machine, which is where
§10.3's equivalence tests run.

**And then the font tests turned out to be passing by doing nothing.**
`text.rs`'s `the_shipped_faces_rasterise_a_glyph_each` loaded
`"../../assets/fonts/Geist.ttf"` — resolved from `cargo test`'s working
directory, the *package* root, so it pointed one level above the repository at a
path that has never existed. `Face::load` returned `Err`, the test's *"a tree
without them is a tree this test has nothing to say about"* branch took over,
and it passed. It is `CARGO_MANIFEST_DIR`-relative now, and
`the_font_fixtures_are_where_the_tests_look_for_them` has no skip branch at all,
because these files are tracked in this repository and their absence is a defect
rather than a circumstance.

**The pair is the argument for the split, not a footnote to it.** A test that
cannot run cannot be wrong out loud.

### 12.2 `clippy` in the build container — FIXED, it used to fail quietly

**Fixed 2026-09-02, both halves.** The container image is `rivet-card-build:3`
and carries the `clippy` component; `cargo clippy --workspace --all-targets`
exits **0**.

**The three errors were one missing line.** `cmd/rivet-sysd/Cargo.toml` denies
`expect_used` rather than warning on it — deliberately, because that crate is
the supervisor — and a `deny` reaches test code. Every other test module in the
crate lifts it in the same words; `definitions.rs`'s never got the block, so
three `expect()` calls in tests made the whole workspace lint exit 101.

**And the Dockerfile now RUNS `cargo clippy --version` at build time**, so a
component that failed to install fails the image rather than producing a lint
run that says nothing — which is the failure this section described.

**126 warnings remain and are a backlog, not a gate** — 287 when this section
was written, and **126 later the same
day**, because the two largest groups turned out to be two changes rather than
147:

* **97 `Err`-variant-too-large were one boxed field.** `OperationError` carried
  a `plan::Plan` inline, which made every `Result` in the operation surface
  **200 bytes** — moved on every call, on the success path as well as the
  refusal. A plan is carried by exactly one of the seven codes, on a path
  already about to print a diff for a human, so boxing it costs one allocation
  there and takes ~144 bytes off everything else.
* **67 `expect()` were four test modules missing the allow block** every other
  test module in the tree carries. `rivet-tls/src/tests.rs` held 32 of them and
  is the shape that explains why: it is a whole FILE declared
  `#[cfg(test)] mod tests;`, so it never got an inline module's block.

Two smaller ones were worth acting on rather than silencing. **Three assertions
between `const`s moved out of a test and into `const` blocks**, so the compiler
refuses a drift at every build — including a cross-compile by somebody who never
ran `cargo test`. The invariant they guard is `ADR-011` at its most expensive:
an account and an application sharing a uid would be two answers to *who is this
process*. Verified by inverting one and watching the build refuse it, with the
sentence rather than a number.

**126 remain and no single group dominates.** They are a backlog, not a gate.



`rivet-card-build:2` has no `clippy` component, so `cargo clippy` inside
`tools/container/exec.sh` answers:

```text
error: 'cargo-clippy' is not installed for the toolchain '1.97.1-x86_64-unknown-linux-gnu'
```

**That line begins `error:` and not `warning:`,** so a check that counts warnings
reports zero and a reader concludes the crate is clean. This document said
exactly that about `aura-renderer` and `aura-probe` for several hours before the
claim was rechecked; it is true now, and it was not then.

`rustup component add clippy` works and takes about a minute, but
`tools/container/exec.sh` starts a fresh container each time, so it must be in
the same invocation as the run:

```sh
tools/container/exec.sh sh -c 'rustup component add clippy >/dev/null 2>&1; cargo clippy ...'
```

Adding the component to `tools/container/Dockerfile` would make it ordinary.
That is the maintainer's call; recorded here because it is the second thing in
this section that made a check pass by not happening.

---

## History

Superseded text, moved here 2026-09-05 out of inline strike-throughs so the body
above states only what is true now. **The body is complete without this
section**; nothing here is a live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

Almost everything here was superseded within a day or two of being written, and
several entries were superseded on the day. This document was written while the
port was being built, so its open questions closed faster than any other in the
record.

### Status block — nothing had run on hardware

**Corrected 2026-09-02**, the same day the sentence stopped being true. The
superseded phrase was ~~**nothing here has run on hardware**~~. §12's own copy
was corrected and this one was not, which is `AGENTS.md` §2 rule 4 inside one
file.

### In §4.2 — the weather was an addon

**Struck 2026-09-03.** The superseded heading read:

~~Weather is an addon in everything except distribution~~

The weather comes from `aura-controller` in a `state` message and the renderer
reads that socket itself.

### In §5 — the vocabulary had three shapes

**Corrected 2026-09-02, and the superseded sentence is the mistake it
describes.** It read:

~~**Three shapes.** The brief for this work named four known cases — a reading, a
list, a statement, *and whatever the clock and weather cards actually need* —
and the fourth turned out to need nothing new. The clock is a reading; the
weather card is a reading and a statement stacked.~~

What hid the fourth case is §5.2's own superseded paragraph, which had already
decided the weather card would be trimmed to fit the vocabulary.

### In §5.1 — there were five colour roles

**`accent` was removed 2026-09-02.** The superseded counts, here and in §5.8.5,
were ~~five~~, and the role's own entry read ~~`accent`~~.

### In §5.2 — the two-column weather panel did not survive

**Superseded 2026-09-02 by §4.5 and §5.8: the panel survives whole.** The
superseded paragraph read:

~~**The two-column weather panel does not survive this design, and that is
correct.** `DrawWeatherPanel` draws outdoor and indoor side by side with a rule
between them; indoor is moving out to an addon, and an addon cannot be half of
somebody else's card. Outdoor becomes a `reading` plus a `statement` in
`secondary`; indoor becomes an addon's own `reading` in a slot of its own.~~

Its middle clause was the error, not its conclusion.

### In §5.8.4 — the cost of option (a), and the claim of exhaustiveness

**Ruled 2026-09-02: (c), which is (a) with the sensor kept separable.** The
superseded text read:

~~**The cost:** the indoor sensor stops being a
separable addon, and §5.2's *an addon cannot be half of somebody else's card*
becomes *so it must not be half of one* — a real narrowing of what a third party
can add, and the thing §4.2's dogfooding argument was protecting.~~

~~There is no third option in which both hold, and this section exists so that
nobody discovers that halfway through implementing one.~~

**An option was ruled out by a premise nobody had noticed was a premise**, and
that is the shape of the mistake worth keeping.

### In §11 item 1 — the clock had no time zone

**Both halves closed 2026-09-02.** The superseded text read:

~~**The clock has no time zone.** A Rivet has no `/usr/share/zoneinfo`
(`SPEC-01` §5.2), so there is nothing on the machine to ask.~~

~~**What is still open is this renderer's half**, and it is smaller than it
was. `aura-renderer` needs to turn that file into an offset, which means
either a tzfile reader of its own — RFC 8536, and a real amount of code for
a clock — or `chrono`, whose `Local` reads exactly that path and which is a
dependency `ADR-009` would want an argument for.~~

Both options answered a question nobody had to ask: musl already contains a
tzfile reader, and `cmd/rivet-sysd` was already calling it.

### In §11 item 5 — the icon face

**Corrected 2026-09-02 and then done the same day.** The superseded text read:

~~**The icon face.** Thirteen glyphs and five trend arrows need a face;~~

~~Whether Aura carries its own asset or the image's icon font grows is an
asset decision.~~

~~**This item is therefore UNBUILT WORK and not an open question**, and it is
listed here only until it is done: ship that face as a fourth asset and port
the original reading card's glyph drawing.~~ (The superseded item named that
card by the C++ function's name; it is described here rather than quoted,
because what the card shows is not this repository's to say — `.rulings/`
R-375 D1.)

**Re-opening a decision the product had already made is what turned a task into
a question**, and the whole thing took an afternoon once it stopped being one.

### In §11 item 12 — the generic composition was an open design question

**RULED and BUILT 2026-09-02; §5.9 specifies it.** The superseded text read:

~~and §5.8 is the same question from the other side.~~

~~**Narrowed 2026-09-02, not closed.** §5.8 is settled and this is not. What a
slot's composition IS remains per-slot and hard-coded: `secondary` draws
`DrawWeatherPanel` — a `pair` is its two columns, a lone `reading` is one, a
`statement` is wrapped under either — because the card that occupies it is
Aura's own and its appearance is ruled fixed. `layout.rs` grew a
`Block::Pair` arm that draws **nothing**, for the same reason it draws
nothing at all: a generic composition for a comparison would be inventing a
picture. What is still open is what an addon's card looks like in a slot Aura
has no widget for.~~

### In §11 item 14 — the rainbow card could not be ported

**BUILT 2026-09-02, and the claim was half stale when it was written down.** The
superseded item read:

~~**`rivet_sdk::draw` has no gradient and no circle**, so `DrawRainbowCard` —
the Born This Way card — cannot be ported.~~

`circle` had arrived with the weather glyphs and nobody revisited the sentence.
**The refusal cost a fortnight over one missing primitive**, and the primitive
was two functions.

### In §11 item 15 — `Role::Accent` had no colour

**Closed by deletion the same day it was added, 2026-09-02.** The superseded
item read:

~~**`Role::Accent` has no colour, and draws as `Role::Normal`.** Added
2026-09-02 with §5.9. `CardTheme` carries `text`, `alert` and `caution`;
Aura never defined an accent, so there is no sixth field to reach for.
`widgets::role_ink` therefore returns the ordinary ink for it and **nothing
says so on screen** — an addon that asks for accent silently gets normal.~~

### In §12 — nothing had run on hardware, and four table cells

**All corrected 2026-09-02 and 2026-09-03.** The superseded text read:

~~**Nothing in this section has run on hardware.**~~

~~28~~ tests for `aura-card`, now 38.

~~the weather panel and the corner reading card~~ | ~~**not built, and BLOCKED**~~
(the row named the reading card by what it showed; reworded under `.rulings/`
R-375 D1.)

~~the shipped weather producer~~ | ~~`examples/aura/aura-bridge`~~

~~`aura-vitals`~~ and ~~`aura-vitals-go`~~ — renamed and pruned 2026-09-03 at
the maintainer's ruling that an instrument should be named as one.

### In §12.2 — `clippy` was absent from the container, and the warning count

**Fixed 2026-09-02, both halves.** The superseded heading and count read:

~~`clippy` is not in the build container, and it fails quietly~~

~~**287 warnings remain and are a backlog, not a gate.**~~
