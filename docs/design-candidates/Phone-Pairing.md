# Phone Pairing: a plan of action for Reliquary and Trends

* **Status:** Design candidate — proposed, not accepted
* **Date:** 2026-09-07
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Why it is in Rivet's record:** `STATE.md` §3.4 carries phone pairing as one
  of the things standing between here and the fleet server, and this document is the plan
  that item was missing. The *work* is in `Trends` and `Reliquary`.

---

## 1. Context — what is actually built, measured rather than assumed

Both projects already implement the same ceremony, independently, and both
implementations are tested. **Nothing below needs inventing; the gap is
narrower than "phone pairing" makes it sound.**

| | Reliquary | Trends |
| :-- | :-- | :-- |
| Enrollment code | 8 chars, Crockford Base32 | 8 chars, Crockford Base32 |
| Alphabet | `0123456789ABCDEFGHJKMNPQRSTVWXYZ` | the same, from `trends_core::ports` |
| Misreading folded on input | `normalise` — `I`/`L`→`1`, `O`→`0` | `canonicalise` |
| Lifetime | 10 minutes, deliberately not configurable | its own `CODE_LIFETIME`, same argument |
| Guess budget | 5 attempts per outstanding code | same shape |
| Redemption | once, for a device token | `POST /api/v1/pair`, once |
| Server keeps | SHA-256 verifier of 256 CSPRNG bits | the same |
| Class/scope at mint | — | `--class tray` \| `phone` \| `dashboard` |
| Certificate pin | **none** | `sha256/` + standard padded base64 |
| QR producer | **none** | Flutter dashboard, `trends://pair?…` |
| QR decoder | **none** | **none** |

### 1.1 The two projects are already tracking each other by hand

`reliquary-core/src/tokens.rs` says, in a doc comment, *"Trends hit the same
wall and recorded it against its own `CODE_LIFETIME`."* Two codebases keeping a
protocol in step through prose is the duplication `STATE.md` §3.4 means when it
says **extract the protocol into a shared library; do not husk a client.**

### 1.2 The divergences — and Phase A found more than this section first claimed

**Reliquary has no certificate pin.** `sha256/` appears nowhere in it. Trends'
QR payload carries `pin` as an optional field — `'pin': ?pin`, absent entirely
rather than empty when the daemon serves no TLS — so the payload shape already
accommodates a service without one.

**Phase A then read both implementations side by side and the table above was
too generous.** These are not one protocol with two spellings. They are two
protocols that agree on the parts a person sees and disagree on the parts that
decide whether the design is sound:

| | Reliquary | Trends |
| :-- | :-- | :-- |
| Alphabet, length | Crockford, 8 | **identical** |
| Misreadings folded | `normalise` | `canonicalise` — **same rule** |
| **Code lifetime** | **10 minutes** | **15 minutes** |
| **Where the lifetime lives** | `reliquary-core` — **domain policy** | `trends-adapter-sqlite` — **a storage detail** |
| **Guessing defence** | `CODE_ATTEMPTS = 5`, a per-code **quota** | `PairThrottle`, **2 attempts/second**, a rate |

**The third row looked like two designs and is one design and a gap.** Reading
`trends-api`'s `throttle.rs` settles it, in its own words:

> *"It is not a guessing defence and must not be mistaken for one."*

So Trends has **no per-code guessing defence**. The throttle is a resource floor
— 500 ms with one permit, two attempts a second globally — and it exists because
an unauthenticated endpoint doing a SHA-256 and a database read per call is a
way to make the daemon busy from off-box. That is a real and separate concern,
and Reliquary does not address it at all.

**And the reasoning that excused the absence rests on the wrong secret.** The
same paragraph says *"a code is 256 bits of CSPRNG output… no delay is what
stands between an attacker and that."* **The enrollment code is 40 bits** — eight
Crockford symbols at five bits each. 256 bits is the **device token**, which is
what redemption yields rather than what guards it. At 256 bits the claim is
true; at 40 it is not, and the two are one function apart.

**The two files also disagree with each other.** `tokens.rs` justifies eight
symbols by citing the throttle as bounding guesses — *"1,800 guesses against
2⁴⁰"* — while `throttle.rs` disclaims exactly that role. One of those comments
has to be wrong, and it is the one that credits a performance floor with a
security property.

**Which is why one shared implementation is not a compromise but a correction.**
There was never a second design to reconcile.

**The second row is an architecture disagreement, not a number.** One project
treats how long a credential lives as a domain rule; the other treats it as a
property of the table it is stored in. A shared crate has to place it somewhere,
and that placement contradicts one of the two codebases.

**And §1.1's coupling is now visibly false in a specific way.**
`reliquary-core` says *"Trends hit the same wall and recorded it against its own
`CODE_LIFETIME`"* — true about the **configurability** question they both faced,
and written beside a number that differs from Trends' by five minutes. The
comment asserts kinship the code does not have. That is precisely the failure
mode of keeping two implementations in step through prose: the sentence stays
true about the argument while the values drift underneath it.

**This is the Phase A result, and it is the useful one.** Had the extraction
been done first, it would have silently imposed one project's security model on
the other — which is the reason §3.1 puts the vectors before any code moves.

### 1.3 The photograph argument, which the QR appears to contradict

Reliquary's own reasoning for an 8-character code is that *"a code long enough
to mistype twice is a code that gets photographed, and a photographed credential
outlives its window."*

A QR **is** a photograph, so this has to be answered rather than stepped around.
The answer is that the argument is about **code length driving a person to
photograph a screen and keep the photo**, not about optical transfer as such:
the code is single-use, lives ten minutes, and burns one of five attempts per
guess. A scan consumes it immediately. **But it should be ruled explicitly**, in
whichever spec grows the QR payload, rather than left for a future reader to
notice the tension and assume nobody saw it.

---

## 2. What is actually missing

Three things, in increasing size:

1. **A decoder on the phone.** `PairingScreen.kt` says so itself — *"No camera,
   and the QR code is not read here."* No barcode dependency, no camera
   permission, no `trends://` intent filter. Reliquary has no phone client to
   put one in.
2. **A QR producer for Reliquary**, if it is to be paired the same way.
3. **The shared module**, so the two protocols cannot drift apart — and the
   conformance harness that proves they have not.

**These are separable, and that matters**: (1) alone gets a phone scanning a
code this week; (3) is the one that keeps it true a year from now.

---

## 3. The plan, in four phases

The order is chosen so that **each phase is provable on its own** and the
maintainer's time is spent only where a machine genuinely cannot stand in.

### 3.1 Phase A — conformance vectors, before any code moves

**Write the protocol down as test vectors, in both repositories, from the
implementations that exist.** Codes that must normalise to the same value;
codes that must be refused; a redemption that must succeed exactly once; four
that must fail with the same `401`; an expiry; a budget exhaustion.

**Why this is first and not last.** It is the cheapest possible proof that the
two implementations agree *today*, and if they do not, that is a finding worth
having before a shared library is built on the assumption that they do. It also
becomes the acceptance test for Phase C: a library that passes both vector sets
unchanged has not husked either client.

*No maintainer time. Fully testable in CI in both repos.*

### 3.2 Phase B — the decoder, on the phone, for Trends only

The smallest thing that puts a working scan in the maintainer's hand:

* a barcode dependency and a camera permission in `client/app/build.gradle.kts`
* an intent filter for `trends://pair`
* parse the URI into the `onPair(address, code)` that **already exists**
* the pin, compared and displayed, through the `CertificatePin.kt` already there

**Nothing server-side changes.** The dashboard already renders the payload and
already refuses to draw it on a loopback address.

*This is where the maintainer's phone is first required, because an optical
scan and a radio are the two things a headless harness cannot stand in for.*

#### 3.2.1 CONFIRM the code; do not type it — ruled 2026-09-07

> *"I prefer CONFIRMING a code over TYPING a code."*

So the scan does not pair silently. It reads the payload, shows what it found,
and waits for one tap:

1. The page displays the QR **and** the code as text, grouped `ABCD-EFGH`.
2. The phone scans, decodes, and **displays the same code back** beside the
   service it is about to pair with.
3. He compares two screens and taps **Confirm**. Only then is the code redeemed.

**This is Bluetooth's numeric comparison, and it buys a real property rather
than only comfort.** A transcribed code proves he can read a screen. A *matched*
code proves the payload the phone decoded is the one **that page** produced —
so a photograph of somebody else's QR, a stale code left on a second monitor, or
a symbol substituted between the screen and the lens all fail at the compare
instead of pairing something wrong.

**It also costs nothing on the wire.** The code is already in the payload; the
step is a render and a tap, and the redemption request is unchanged.

**Typing stays as the fallback and stops being the main path** — a camera that
will not focus, a cracked lens, a page on a machine with no screen to hold up.
`ReliquaryPairings` takes an address and a code either way, so both routes reach
the same function with the same arguments, which is what keeps the fallback
honest rather than a second implementation.

**What an agent drives is the confirm, not the camera.** The URI arrives by
intent, the same screen renders, and the tap is an `input` event — so the
unattended path exercises the comparison step rather than skipping it, and the
only thing left uncovered is the optical decode itself.

### 3.3 Phase C — the shared module

Extract the protocol — alphabet, normalisation, length, lifetime, attempt
budget, verifier derivation, the payload URI — into one Rust crate both
daemons depend on, and one Kotlin equivalent (or a binding) the client uses.

**Ruled already and worth restating: do not husk a client.** The extraction
takes the protocol, not the UI, not the transport, not the storage.

**ONE implementation, and §1.2 is why it is available.** An earlier draft of
this section proposed making the guessing defence a policy each adopter supplies,
on the reading that a quota and a rate limit were competing answers. They are
not. `throttle.rs` disclaims the guessing role in its own words, so what looked
like two models is one model and one gap.

So the collapse takes the better half from each, and neither is a concession:

| | from | why |
| :-- | :-- | :-- |
| **Attempt quota**, per outstanding code | Reliquary | Trends has none, and its excuse cites a 256-bit secret that is not the code |
| **Rate floor**, one permit and 500 ms | Trends | Reliquary leaves an unauthenticated SHA-256-and-a-read endpoint unbounded |
| **Lifetime in the domain**, not the storage adapter | Reliquary | a second backend would otherwise get a different lifetime silently |
| **The lifetime value** | either | see below |

**The two defend different attackers and a serious design has both** — the quota
bounds guesses against one secret, the floor bounds what an unauthenticated
endpoint costs. Each project built one and left the other's job undone.

**Once a quota exists the lifetime stops doing security work**, which dissolves
the 10-versus-15 disagreement rather than settling it. Fifteen minutes was
Trends' number *because* the lifetime was its only bound; with a quota, either
value is a question of how long a person needs to walk to a phone. Ten is the
shorter and the argument for it is now convenience rather than safety.

**What must not be lost in the collapse:** Reliquary's reason for keying the
budget to the *code* and never to the source address — an address budget is one
an attacker resets by changing interface, and it would let a hostile client on
the LAN lock the household's own phone out of pairing.

**And the cost, stated rather than discovered later:** a per-code quota means
somebody who knows a code is outstanding can burn it with five wrong guesses.
Reliquary accepted that deliberately, because codes are cheap to mint and the
alternative is worse. Trends will inherit that trade and should be told.

*No maintainer time, but it restructures two live codebases, which
`STATE.md` §3.4 says is not an unattended act. It needs his go-ahead, not his
keyboard.*

### 3.4 Phase D — Reliquary's half

Reliquary gets the pin (§1.2's decision), a QR producer, and the phone client
learns a second service. **This is the phase that can wait**: Reliquary is
paired today by typing eight characters, which works.

---

## 4. What this does and does not unblock for the fleet server

**It is not on the fleet server's critical path.** That is a fleet *server*
migration; the certificate ceremony, DNS and the human-run provisioning walk are
what stand in
front of it. Phone pairing is a client-side capability of two applications that
happen to run on Rivet.

**What it does block** is the pairing test harness in `STATE.md` §3.4 — the
headless conformance layer on QEMU plus a dumb phone app over `adb` — because
that harness is Phase A and Phase C wearing a different hat. Phase A is most of
its value at a fraction of its cost.

---

## 5. The decisions owed before Phase C starts

0. **Confirm the collapse, and that Trends gains both a quota and the burn
   trade.** Phase A raised this as *"quota, rate, or the adopter's choice"* and
   §3.3 has since answered it: there is one design and one gap, so both adopt
   quota **and** floor. What still needs saying out loud is that this **adds a
   guessing defence to a live service that lacks one**, and that Trends
   inherits the cost — a code can be burned by five wrong guesses. That is a
   change to how Trends is attacked, so it is ruled rather than merged.
1. **Does Reliquary get a certificate pin?** Decides one payload shape or two.
2. **Is the QR payload `trends://pair` generalised**, or does each service keep
   its own scheme? A shared decoder wants one scheme with a service field.
3. **One phone app for both, or two?** The client is already
   `io.github.caladil.trendsclient` and is Trends-shaped.
4. **Is the photograph argument (§1.3) answered by single-use plus ten minutes**,
   on the record, in the spec that grows the payload?
