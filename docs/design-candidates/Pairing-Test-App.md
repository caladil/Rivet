# The pairing harness: an Android application a person and an agent both drive

* **Status:** Design candidate — proposed, not accepted
* **Date:** 2026-09-07
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Why it is in Rivet's record:** `STATE.md` §3.4 carries the pairing harness
  as queued work and `docs/design-candidates/Phone-Pairing.md` plans the
  protocol side of it. This document is that plan's **Phase B**, written out.
  The *work* lands in Kotlin, outside this repository; what lands here is the
  finding that decides whether it is worth doing, which is `ADR-046` §4.
* **Method:** every claim in §2 was read out of source on 2026-09-07 —
  `Logbook`, `Trends`, `Reliquary` and one more of the maintainer's working trees, plus this
  repository's `ADR-046` and the two Rivet manifests in `Trends` and
  `Reliquary`. No board was touched, no service was started, nothing outside
  this file was modified.

---

## 1. Context

### 1.1 What was asked for

The maintainer wants an Android application he can install and drive by hand,
that pairs to his servers the way his phone will have to pair to them once they
run on Rivet, and that pushes data at them afterwards. His argument for it is
that a proven ceremony is what makes the migration's integration surface
tractable rather than a surprise. He asked for Kotlin so it matches Logbook,
and he handed the interaction design away deliberately: a **Pair** button, a
camera, a QR code, paired.

**Kotlin is agreement with the tree rather than a choice.** Both Android clients
in the family are Kotlin and Jetpack Compose already, both use
`HttpsURLConnection` with a pinned trust manager and no HTTP library, and both
store their pairing under an AndroidKeyStore-held AES key. The harness inherits
a house style that is written down in working code.

### 1.2 The premise that was corrected

An earlier draft of `Phone-Pairing.md` assumed the phone client at issue was
Trends'. **Reliquary serves Logbook**, so Logbook is the application
that pairs to Reliquary and does so today. The maintainer's direction is that
Reliquary becomes the backup custodian for all of his personal-data services,
Trends among them.

**The next server in that line is not a Reliquary adopter yet, and its tree
says so two ways.** A repository-wide `git grep -i reliquary` there returns
three files — its `docs/ADR-009`, `docs/brand/BRAND.html` and
`docs/brand/brand.json` — and the ADR's single hit is one sentence about
Reliquary being HTTPS-only, used as a premise in an argument about something
else. A second search over `cmd/` and the library crates for `backup` and
`pairing` returns nothing that talks to a backup service. So that integration
is unwritten, and the belief that it is finished should be corrected before it
is built on.

---

## 2. What exists, measured

### 2.1 A working Kotlin pairing client, in Logbook

`Logbook/app/src/main/java/…/data/remote/`
carries the whole ceremony against Reliquary and it runs against the real
service:

| File | What it holds |
| :-- | :-- |
| `ReliquaryPairings.kt` | the composition root — typed address plus typed code becomes a stored pairing, once, or not at all; plus `check` and `readdress` |
| `ReliquaryPairingStore.kt` | the pairing under an AndroidKeyStore AES-GCM key that cannot be extracted |
| `CertificatePin.kt` | SHA-256 over the leaf's SubjectPublicKeyInfo, rendered `sha256/` + padded base64, and `PinnedTrustManager` with its two factory functions |
| `ReliquaryEndpoints.kt` | the five URLs, `https` enforced, port defaulted to 8443 |
| `ReliquaryTransport.kt`, `HttpUrlConnectionReliquaryTransport.kt` | the network seam, so the flow is drivable on the JVM |

Two properties of that code decide most of this document.

**The pin is learned in the handshake that redeems the code.** One connection,
not two: `PinnedTrustManager.learning()` accepts the first key it is shown and
records it, and every request afterwards uses `pinnedTo(pin)`. The residual —
an attacker in the path at that exact moment — is argued in `CertificatePin.kt`
rather than glossed, and it is bounded by the code's ten-minute life and its
five-attempt budget.

**The platform trust store is never consulted.** The transport installs its own
`SSLSocketFactory` from that trust manager and a `HostnameVerifier` that returns
true **behind it**, with the argument written at the constant: a pin to one
public key admits exactly one machine, which is a stronger claim than any name
check, and a certificate the maintainer minted for his own machine will not
carry a name matching whatever he typed.

### 2.2 The Trends client, and the exact gap

`Trends/client/` is the same design, transcribed. `store/api/CertificatePin.kt`
is Logbook's file with the argument carried across; `store/pairing/
PairingStore.kt` says so in as many words; `store/api/
HttpUrlConnectionTransport.kt`'s permissive `HostnameVerifier` is annotated
*"Transcribed, argument included, from Logbook's
`HttpUrlConnectionReliquaryTransport.acceptPinnedHost`."*

What it lacks is the camera and the URI. `ui/pairing/PairingScreen.kt` documents
its own gap in a heading — *"No camera, and the QR code is not read here"* — and
the manifest confirms it from the other side: `AndroidManifest.xml` declares
`INTERNET`, eight platform data permissions and Logbook's signature
permission, and **no `CAMERA` permission and no `<data android:scheme>` filter
of any kind.** `client/app/build.gradle.kts` names no barcode or CameraX
dependency.

`TrendsClient.kt` already carries `pair`, `streams` and `pushReadings`, and
`domain/Endpoints.kt` normalises an address to `https` on port 8787.

### 2.3 The QR producer, and its payload

`Trends/dashboard/lib/ui/pair_phone_dialog.dart` mints a code on open
(`POST /api/v1/pair/code` with `{"class":"phone"}`, authenticated by the
browser's own `dashboard` token) and renders:

    trends://pair?address=<percent-encoded base>&code=<ABCD-EFGH>&pin=<sha256/…>

`pin` is null-aware — the key is **absent entirely** rather than present and
empty when the daemon serves no TLS. The dialog refuses to draw the code at all
on a loopback address, because a phone scanning `localhost` dials itself and
reports the daemon down.

Reliquary has no QR producer. Its `reliquary-tray` on Windows mints through
`POST /api/v1/enrolment-code` against a local admin secret and displays the code
as text for typing.

### 2.4 The camera stack already exists, in the other application

Logbook ships CameraX and ML Kit barcode scanning — `androidx.camera.core`,
`camera2`, `lifecycle`, `view` and `mlkit.barcode.scanning` in
`app/build.gradle.kts`, `android.permission.CAMERA` in its manifest, an
`ImageAnalysis.Analyzer` in `ui/scan/DataMatrixAnalyzer.kt`, and `zxing-core` on
the instrumented-test classpath so a symbol can be **generated** and fed to the
decoder rather than held in front of a lens.

Two details of that analyzer transfer directly. It requests
`Barcode.FORMAT_ALL_FORMATS` for detection and narrows acceptance afterwards,
because a format-restricted decoder and an unreadable frame produce the same
silence. And it inverts alternate frames, because ML Kit assumes dark modules on
light and the maintainer's cartons are reverse-printed — a QR on a white
background needs none of that, which is worth knowing before somebody copies the
inversion across.

### 2.5 Both protocols, side by side

| | Reliquary | Trends |
| :-- | :-- | :-- |
| Code | 8 Crockford symbols, ~40 bits | identical, `CODE_LENGTH = 8` |
| Folding | `normalise` | `canonicalise`, same rule |
| Lifetime | 10 minutes | 15 minutes |
| Guessing defence | `CODE_ATTEMPTS = 5`, per outstanding code | none; `PairThrottle` is a rate floor and disclaims the role |
| Redemption | `POST /api/v1/pair` `{code}` → `{token}` | `POST /api/v1/pair` `{code}` → token plus class |
| Token | 256 bits hex | 256 bits hex |
| Port | 8443 | 8787 |
| Pin in the payload | absent | present when the daemon serves TLS |
| Code minting, headless | `POST /api/v1/enrolment-code`, admin secret | `POST /api/v1/pair/code`, dashboard token |

**One correction to add to `Phone-Pairing.md` §1.2's list.** Reliquary's
`reliquary-core/src/tokens.rs` states, in its module header, that *"Trends'
enrolment code is 256 bits like its token."* `trends-core`'s
`ports/tokens.rs` fixes `CODE_LENGTH = 8` over a 32-symbol alphabet, which is
40 bits. That is a second cross-project claim that was written from memory of
the other repository rather than from it, in the same paragraph family as the
one `Phone-Pairing.md` already caught — and it is the same failure in the
opposite direction, since it credits Trends with a stronger credential than
Trends has.

### 2.6 The finding that matters most: a Rivet-hosted Trends refuses the phone

This is `ADR-046` in force, read against the two Rivet manifests.

`Trends/server/rivet/cmd/trends-rivet/Cargo.toml` declares
`proxy-path = "trends"` and leaves `proxy-auth` out, which `ADR-046` §4.3.1
reads as **gated**: `rivet-proxyd` requires a client certificate chaining to the
fleet authority (`ADR-046` §4.1) and answers `403` to a caller that presents
none. The manifest's own comment says the phone is unaffected because
`listen = ["tcp:8787"]` is unchanged — but
`trends-adapter-rivet/src/tls.rs` calls
`requiring_a_caller_certificate_from`, so **that door demands a fleet-issued
client certificate too.**

The supervisor binds an application's declared TCP socket on `0.0.0.0`
(`cmd/rivet-sysd/src/kernel.rs`), so the port is reachable from the LAN; what
refuses the phone is the daemon's own client-certificate requirement, not the
address it listens on.

**So a phone holding a device token and a pin has no route to a Rivet-hosted
Trends.** Both doors want an X.509 client certificate the pairing ceremony
issues nothing like.

**Reliquary is the opposite case and is pairable today.** Its
`reliquary-rivet` manifest declares `listen = ["tcp:8443"]` and
`proxy-path = "reliquary"`, and `reliquary-api/src/tls.rs` builds its server
configuration `with_no_client_auth`. The proxy route `/reliquary/` is gated for
a browser; the application's own port is open to the LAN and gated by the device
token alone. That is a live route for the harness and it is also a fact the
maintainer should see written down, because the front door and the archive's own
door disagree about who may knock.

### 2.7 What has to be written

1. A QR decoder on a phone, in any of the three applications.
2. A URI intake — an intent filter and a parser — in any of them.
3. A Reliquary QR producer.
4. A way for a phone-shaped credential to get past `ADR-046`'s gate, or a
   ruling that it need not (§10.1).
5. The harness itself: the screens, the synthetic payloads, the command surface.

---

## 3. Design problem 1 — how an agent drives a camera it does not have

### 3.1 The rule the design has to satisfy

A human points a camera at a QR code. An agent has none. **If the agent's path
and the human's path diverge anywhere but at the very first step, a green agent
run says nothing about the human path** — which would make the harness a machine
for proving its own test hooks.

So: the camera and `adb` are **two transports for one payload**, and everything
downstream of the decoded URI is a single function that neither knows about.

    camera → ML Kit → String                    ┐
                                                 ├→ Uri → PairingIntake.accept(uri) → …
    adb → Intent VIEW → intent.data → Uri       ┘

`PairingIntake.accept` parses, validates, dispatches to the right service
client, redeems, learns or checks the pin, stores the pairing, and publishes the
outcome. Nothing else in the application writes a pairing, exactly as
`ReliquaryPairings` holds that role in Logbook.

The QR payload is already a URI, which is what makes this available at no design
cost — and `pair_phone_dialog.dart` says a URI was chosen partly so that *"the
Android client [has] a shape it can register an intent filter against later."*
This is that later.

### 3.2 What the application must declare

```xml
<activity
    android:name=".PairingIntakeActivity"
    android:exported="true"
    android:launchMode="singleTask">
    <intent-filter>
        <action android:name="android.intent.action.VIEW" />
        <category android:name="android.intent.category.DEFAULT" />
        <category android:name="android.intent.category.BROWSABLE" />
        <data android:scheme="trends" android:host="pair" />
        <data android:scheme="reliquary" android:host="pair" />
    </intent-filter>
</activity>
```

Four points about it, each of which is a way to get this wrong:

* **`DEFAULT` is required.** An implicit intent delivered through
  `startActivity` resolves only against filters carrying it, and `am start`
  sends an implicit intent. A filter with `VIEW` and no `DEFAULT` matches
  nothing and the failure reads as *no activity found*.
* **`BROWSABLE` is what lets the same URI arrive from a browser or a messaging
  app**, which is the human's fallback when a camera will not focus. It costs
  one line and it widens what the one intake path is exercised by.
* **`exported="true"` is unavoidable and is the whole exposure.** Any
  application on the phone can send this activity a `trends://pair` URI. That is
  acceptable here for a reason that would not hold in a production client: the
  payload carries a single-use code that a hostile sender would have to have
  already read off the maintainer's screen, and the harness holds no real data
  to lose. A production client adopting this should say the same sentence or a
  better one.
* **Two `<data>` elements in one filter cross-multiply.** Scheme × host is the
  match rule, so this filter also accepts `trends://pair` and
  `reliquary://pair` in every combination — harmless because both hosts are
  `pair`, and worth knowing before a third service arrives with a different one.

### 3.3 The exact invocation

From the workstation, with the device attached:

```console
$ adb shell "am start -a android.intent.action.VIEW -d 'trends://pair?address=https%3A%2F%2F192.168.50.60%3A8787&code=ABCD-EFGH&pin=sha256%2FYUJDRC…%3D' -p io.github.caladil.pairingharness"
```

* **The double quotes are the workstation's shell; the single quotes are the
  device's.** `adb shell` joins its arguments and hands them to `sh` on the
  phone, so an unescaped `&` splits the command **on the device** and the URI
  arrives truncated at the first query parameter. The symptom is a pairing that
  reports a missing code rather than a shell error, which is why it costs an
  afternoon the first time.
* **`-p <package>` rather than `-n <component>`.** `-n` names the component and
  bypasses filter matching entirely, which would let the harness pass this test
  with a broken manifest. `-p` restricts resolution to one package and still
  makes Android resolve the intent against the filter — so the filter itself is
  under test, which is the point.
* **The value of `address` is percent-encoded**, and so is `pin` — `sha256/`
  and standard padded base64 both carry characters that are not URI-safe. The
  Flutter producer encodes them; the harness must round-trip them byte for byte,
  because a pin that came back re-encoded matches nothing and the failure looks
  like a wrong certificate.

An agent that must also **mint** the code first has a fully headless path for
each service: `POST /api/v1/pair/code` with a dashboard token for Trends, which
returns the code and the pin together; `POST /api/v1/enrolment-code` with the
admin secret for Reliquary, which returns the code and no pin. It then composes
the URI itself — the same string the dashboard would have drawn.

### 3.4 What this does NOT prove, said here rather than at the end

**The optical decode is untouched by every agent run.** Frame capture, focus,
exposure, the inversion question of §2.4, ML Kit's decode of a QR at whatever
size and contrast the dashboard drew it, and a phone held at arm's length in the
light of a real room — all of that is exercised **only** by a person with a
camera. An `adb` run proves the intake, the protocol, the pin and the storage,
and it proves that the camera path has somewhere correct to hand its string.

That is a large fraction of the risk and it is not the whole of it. §9 keeps the
rest of this ledger.

---

## 4. Design problem 2 — trusting a locally minted fleet authority

### 4.1 The platform rule, and why it does not bind here

From API 24, Android ignores user-installed certificate authorities for
application traffic unless the application opts in through a network security
configuration. That rule governs code that uses the **platform's** trust
manager.

**Neither existing client does.** Both build an `SSLContext` from their own
`X509TrustManager` and install the resulting factory on the connection, so the
system trust store is never asked a question. The user-CA rule, a
`res/xml/network_security_config.xml`, and installing the fleet root on the
phone at all are therefore beside the point for this design.

So the answer to *network security config, pinned certificate, or both* is
**the pin, and no configuration file.**

### 4.2 Why the pin rather than the fleet authority

Both would work. They are not equivalent, and the difference is what each one
admits:

| | Ship the fleet root, validate the chain | Pin the leaf's public key |
| :-- | :-- | :-- |
| Admits | every leaf that authority ever signs — every board in the fleet, and anything a compromise of the root produces | exactly one key, therefore one machine |
| Needs the leaf to carry | a subject alternative name the phone dials, which forces DNS or a hosts entry to be right before pairing can work at all | nothing |
| Survives a routine reissue from the same key | yes | yes — the pin is over the SubjectPublicKeyInfo, not the certificate |
| Survives a re-mint with a new key | yes | no, and that is correct: a new key is a different machine as far as the phone can tell |
| Ships in the APK | a root certificate, which then has to be rotated in a release | nothing |

The pin wins on the first row alone. The harness therefore carries no
certificate material at all, which also keeps it inside `AGENTS.md` §8.3 — there
is nothing to scrub because nothing was committed.

**Hostname verification is deferred to the pin and only behind it.** A
permissive `HostnameVerifier` in front of a pinning trust manager is a defect; a
permissive one behind it is the narrower claim, argued in both existing clients
and transcribed rather than re-derived.

### 4.3 What the harness does when the payload carries no pin

Reliquary's payload has no `pin` field. Trends' has one that is absent rather
than empty when the daemon serves no TLS. Both cases arrive at the same branch:

| Payload | What happens |
| :-- | :-- |
| `pin` present | `PinnedTrustManager.pinnedTo(pin)` for the redemption handshake itself. **No trust-on-first-use at any point** — the phone knows the key before it speaks to the machine, and a mismatched key is refused before the code is spent. |
| `pin` absent | `PinnedTrustManager.learning()` for the redemption handshake, and `pinnedTo(learned)` for everything after. Bounded trust-on-first-use, exactly Logbook's window. |

Three things the harness must do that a production client can be laxer about:

1. **Display the learned pin** in the rendered `sha256/…` form, so a person can
   compare it by eye against what the machine reports.
2. **Record it in the run's status**, so an agent can compare it against the pin
   the mint endpoint returned — which turns the trust-on-first-use window into
   something a headless run can *assert on* rather than merely accept.
3. **Refuse a pairing that completed with no key recorded.** Unreachable over
   TLS, and refused rather than stored blank, because a pairing with no pin is
   the one shape that leaves every later request unverified.

**What would force a network security configuration after all:** any component
that opens its own connections through the default stack — a `WebView`, an
image loader, a Flutter engine, an HTTP client library. The harness therefore
uses `HttpsURLConnection` with an explicit factory and adds no HTTP dependency,
which is what both existing clients do and the reason they can.

---

## 5. Design problem 3 — one application or two

**One new application. Not a variant of `io.github.caladil.trendsclient`, and
not that client grown a test surface.**

The maintainer's own razor is *converge first, parameterize as a second choice*
(`STATE.md` §3.4a). Applied honestly it argues **for** a separate binary, because
the thing that must not diverge is the **pairing module**, and the thing that
must diverge is **who holds a real credential**.

What a variant would put inside the application that holds a device token for
his live personal-data daemon:

* an **exported** activity that accepts a pairing URI from any application on
  the phone (§3.2);
* an **exported** command receiver an agent drives unattended (§7.2);
* a generator of synthetic rows, one mis-tap away from his real records.

And a variant would be proving the variant. `AGENTS.md` §6 is the standing rule
that "wired up" is never "works"; a build flavour that is green is evidence
about a build flavour, and the artifact he installs is the other one.

**The convergence is honoured by where the code goes, not by where the binary
goes.** The harness is written as two Gradle modules — a `pairkit` library
holding the payload grammar, the pin, the trust manager and the store, and an
`app` module holding the screens and the command surface. `pairkit` is the
Kotlin half of `Phone-Pairing.md`'s Phase C: the harness is where a converged
implementation is proved against both services before it is imposed on two
personal-data clients that work today.

**The cost, stated rather than discovered later.** Until Trends' client and
Logbook adopt `pairkit`, there are three copies of `CertificatePin` in the
family instead of two. That is the price of proving the third copy before
replacing the first two, and it is bounded by a named next step rather than
open-ended. Adopting it across repositories needs a published artifact — the
Kotlin analogue of Ignition's public mirror and pinned tag (`AGENTS.md` §8.4) —
and that is its own piece of work, not a free consequence of this one.

**Where it lives: its own repository**, alongside the maintainer's other
projects. It belongs to neither service, it tests both, it holds no
real data, and an agent may drive it unattended without touching a tree that
holds his personal-data code. Suggested `applicationId`
`io.github.caladil.pairingharness`; nothing depends on the name, so renaming it
before the first commit costs nothing.

---

## 6. Design problem 4 — what "push simulated data" means

### 6.1 The rule for every payload below

**A reader must be able to tell a harness row from a real one at a glance, and
select every harness row with one query.** Two mechanisms, because the first is
for a person and the second is for whoever has to clean up:

* a **visible marker** in a field that any rendering of the row shows, and
* a **selectable prefix** in the field that a database index or a catalogue
  listing is keyed on.

And a third rule that costs nothing and removes most of the risk: **the default
proof writes no rows at all** (§6.2).

### 6.2 Trends — `POST /api/v1/sync/push`

`TrendsClient.pushReadings` already exists and takes a device id and a list. The
harness uses it in three escalating steps, and the first is the one an agent runs
by default:

**Step 1 — the empty batch.** `pushReadings(deviceId, emptyList())` is a
legitimate call and `TrendsClient.kt` argues for it: it is the cheapest proof
that network, address, token, class and database all work end to end, and it
advances the daemon's own `record_push` watermark so the heartbeat is visible on
the desktop as well as on the phone. **It writes no reading.** This is the
harness's default push and the one an unattended run performs.

**Step 2 — resolve the registry.** `GET /api/v1/streams`, then map the key
`weight` to its `stream_type_id`. Resolving by key rather than hard-coding an id
is required: ids are assigned by a migration and a client that assumed `1` would
be asserting something about the daemon's migration history that nothing on the
phone can check.

**Step 3 — synthetic rows, behind an explicit command.** One `Reading` per
row, with:

| Field | Value | Why |
| :-- | :-- | :-- |
| `id` | UUIDv4 | required stable across sync; minted by the client |
| `stream_type_id` | from step 2 | never a literal |
| `ts` | RFC 3339 UTC, seconds, deterministic from the run id | a fixed shape is what makes `source_ref` stable across two builds |
| `value` | `1.0` for `weight` | physiologically impossible, so a chart shows it as an obvious artefact |
| `source` | `manual` | see below |
| `source_ref` | `pairing-harness:<run-id>:<n>` | the **selectable prefix** — `idx_reading_dedup` is unique over `(stream_type_id, source, source_ref)`, so a replay is a no-op and a cleanup is one `LIKE` |
| `device` | `pairing-harness` | the **visible marker** |
| `meta` | `{"synthetic":true,"harness_run":"<run-id>"}` | machine-readable, for anything that reads meta |

**`source` is `manual` because `trends_core::model::Source` has no synthetic
variant**, and adding one is a wire-format change — `AGENTS.md` §1.1's third
row, his to rule, not a harness's to take. `manual` is the least wrong of the
nine: the two phone-import variants would forge a provenance the row does
not have, and Logbook's additionally carries the right to state a
supersession verdict Trends copies without recomputing. Recommending a
`Source::Synthetic` variant is §10.2.

**Replay is a first-class assertion, not an afterthought.** Pushing the same
batch twice must accept the second and create no duplicate — `source_ref` is
what makes that true, and a lost response on a flaky link is the normal case
rather than an edge one.

### 6.3 Reliquary — `PUT /api/v1/blobs`

The request body **is** the container: `LOGBAK01`, a plaintext manifest, and
ciphertext. Reliquary parses the manifest, hashes the ciphertext and compares it
against the manifest's own `ciphertext_sha256`, and refuses a blob that fails —
so building this correctly is a real conformance test of the format rather than
a formality.

`CatalogueRow` renders `device_id`, `app_version_name`, `build_sha` and
`schema_version` straight out of the manifest, without decrypting anything.
That is where the markers go:

| Manifest field | Value | Why |
| :-- | :-- | :-- |
| `device_id` | `pairing-harness` | the **visible marker**, and it is the field the catalogue lists per row |
| `app_version_name` | `pairing-harness` | a second one, in the column a person reads first |
| `build_sha` | the harness's own HEAD, seven hex | which build wrote it, for the same reason Logbook carries one |
| `schema_version` | `999999` | **a safety property, deliberately chosen** — Logbook refuses a backup newer than it can migrate, and refuses it *before a passphrase is asked for*, so no build of it can ever be induced to try to open a harness blob |
| `payload_bytes`, `ciphertext_sha256`, `kdf`, `cipher` | honest values for what was actually written | the store checks the digest; the rest must agree with it |

**The ciphertext is a fixed synthetic plaintext**, not a database — a short
constant string encrypted under a passphrase written in the harness source.
Reliquary never decrypts, so the payload's content is its own business; making
it plainly not a Room snapshot means a file recovered from the shelf by a person
cannot be mistaken for a restorable backup, and the `schema_version` above means
no program will try.

Three assertions after the upload, in order:

1. `PUT` answers `201` with a `Location` header.
2. `GET /api/v1/catalogue` lists a row whose `device_id` is `pairing-harness`.
3. `GET /api/v1/blobs/{id}` returns the exact bytes that were sent.

And a fourth that costs one line and proves the idempotence the verb promises:
uploading the identical container twice yields one catalogue row, because the id
is minted from the manifest.

### 6.4 Where these are pushed

**A scratch instance by default.** `AGENTS.md` §9 forbids pointing a test at the
maintainer's live data home, and the markers above exist for the case where a
run is deliberately aimed at a real service to prove the real path — which is
his call, per run, and which the harness makes visible by naming the target
address on its own screen before it will push anything.

---

## 7. Design problem 5 — instrumentation for a person and for an agent

### 7.1 What he taps

One screen, in the house style — Compose, Material 3, the register of
`PairingScreen.kt`:

1. **Service** — Trends or Reliquary. It decides the endpoints and the port,
   nothing else.
2. **Pair** — opens the camera. The QR is decoded, the URI goes to
   `PairingIntake.accept`, and the sheet closes. Below it, an address field and
   a code field, always present, because a camera that will not focus, a cracked
   lens and a denied permission are all ordinary. **Reliquary's camera path is
   unavailable until it has a producer** (`Phone-Pairing.md` Phase D), so for
   Reliquary the typed path is what remains, and the screen should say so
   rather than offer a button that cannot work.
3. **Paired** — a card showing the address as normalised, the pin in its
   `sha256/…` form, the class where the service reports one, and *token held*.
   Never the token.
4. **Actions** — `Check` (a `/health` handshake against the stored pin),
   `Push empty`, `Push N rows`, `Upload blob`, `Fetch catalogue`, `Round-trip`,
   `Forget`.
5. **Journal** — the last twenty outcomes, newest first, each a line he can read
   without a cable.

Every refusal on that screen states what to do rather than what a machine saw,
and none of them invents a distinction the protocol declines to make: a wrong,
expired and already-consumed code are one answer on both services, and a screen
offering *"expired?"* as a hint would be inventing an oracle the endpoints
deliberately refuse to be.

### 7.2 What an agent runs

```console
$ adb install -r app/build/outputs/apk/debug/app-debug.apk
$ adb shell "am start -a android.intent.action.VIEW -d '<uri>' -p io.github.caladil.pairingharness"
$ adb shell "am broadcast -a io.github.caladil.pairingharness.COMMAND -e op status -p io.github.caladil.pairingharness"
Broadcast completed: result=2, data="{\"state\":\"paired\",\"service\":\"trends\",\"address\":\"https://…:8787\",\"pin\":\"sha256/…\",\"seq\":4}"
```

**The exit condition is the broadcast's result code**, which is an integer a
script tests without parsing anything:

| Code | Meaning |
| :-- | :-- |
| 0 | no pairing held |
| 1 | an operation is in flight |
| 2 | paired, last operation succeeded |
| 3 | last operation was refused, `data` says which |
| 4 | the harness did not understand the command |

The receiver is declared with `android:permission="android.permission.DUMP"`,
which restricts senders to callers holding it — `adb shell` does, an ordinary
third-party application does not. `-p` on the broadcast is required rather than
decorative: an implicit broadcast to a manifest-declared receiver is restricted
from API 26, and package-targeting it is what makes it deliverable.

**The bound to respect:** a `BroadcastReceiver` has roughly ten seconds before
the platform kills it, so no command blocks on the network. Every op returns the
*current* state immediately and the agent polls `status` in a bounded loop. That
is a platform constraint rather than a design preference, and a command that
waited would fail intermittently in a way that reads as a network fault.

**The file is the evidence, and the broadcast is the interface.** The harness
also writes each outcome to `status.json` and appends to `journal.jsonl` under
its own external files directory, readable without root:

```console
$ adb shell cat /sdcard/Android/data/io.github.caladil.pairingharness/files/status.json
```

They have different jobs. The result code is what a script branches on; the
files are what survives a run that ended badly, and what a person reads when the
code says `3` and the question is why. Neither is a screenshot somebody has to
interpret.

### 7.3 The one thing the agent path must never grow

**A command that stores a pairing without going through
`PairingIntake.accept`.** A `-e op pair-with-token <token>` shortcut would make
every headless run green while proving nothing about the ceremony, and it is
exactly the shape of shortcut an agent under time pressure adds. The command
surface reads state and triggers operations; the only way a pairing is created
is a URI arriving at the intake.

---

## 8. The build order

Each phase is provable on its own, and the two that need the maintainer are
named as such.

### 8.1 Phase B1 — the intake, headless, against a scratch daemon

`pairkit`'s payload grammar and `PairingIntake`; the manifest filter; the
command receiver; the two service clients transcribed from Logbook and
Trends' client. Prove: `adb`-driven pairing against a scratch Trends and a
scratch Reliquary, the pin path in both branches of §4.3, replay, and a wrong
code refused identically to an expired one.

*No maintainer time. A phone or an emulator, and `adb`.*

### 8.2 Phase B2 — the camera

CameraX and ML Kit, the permission, an `ImageAnalysis.Analyzer` that hands its
string to the same intake. Prove headlessly what can be: an instrumented test
that **generates** a QR with `zxing-core` and feeds it to the decoder, which is
the trick Logbook already uses for DataMatrix and which separates the
decoder from the optics.

*Then the maintainer, once: point the phone at the dashboard's QR. That is the
optical decode and nothing else closes it.*

### 8.3 Phase B3 — the payloads

§6's three Trends steps and §6.3's four Reliquary assertions, with the markers.

*No maintainer time against a scratch instance.*

### 8.4 Phase B4 — the round trip a person judges

Him, with the phone in his hand: mint from the dashboard, scan, push, read the
catalogue, forget, re-pair. The part no harness can stand in for is whether the
screen reads well and whether the ceremony feels like one act.

### 8.5 Phase B5 — the Rivet-hosted run

Blocked on §10.1. Against Reliquary on the board this is available today — the
finding above about the two manifests — and is worth doing first for exactly
that reason: it is the half of the question that is already answerable.

---

## 9. What this will not prove

`AGENTS.md` §6 is the standing rule, and a plan claiming a harness proves the
migration would be lying. It will not prove:

* **The optical decode**, for any run an agent made. §3.4.
* **That Logbook or `trends-client` work.** The harness is a third program.
  A green harness says the ceremony and the protocol are sound; it says nothing
  about either application until each adopts `pairkit`, and adopting it is a
  change to two personal-data clients that is itself unproven the day it lands.
* **The phone's platform data store, the Logbook signature seam, or the relay.** The harness
  reads no platform data and schedules no work, so background execution, doze
  behaviour and the release-signing deadline on the signature permission are all
  outside it.
* **Anything about a Rivet-hosted Trends**, until §10.1 is ruled and a run has
  gone through whichever door the ruling opens. Until then the harness proves
  the ceremony against daemons on their own ports, which is honest about its
  scope and does not become a claim about the front door.
* **That a real backup opens.** The container the harness uploads is
  deliberately unopenable — §6.3 — so restore, migration and the passphrase path
  are untouched.
* **Retention, catalogue ordering over a year, or the shelf under load.** Those
  are properties of a service holding real history.
* **The migration.** A green harness reduces the integration surface; it does
  not rehearse a data move, and the two should not be confused in a status
  summary.

One measurement that is worth taking early and is easy to overlook: **the pin a
mint endpoint reports and the pin a client observes through `rivet-proxyd` are
the keys of two different certificates unless the machine serves one.** A QR
carrying the backend's pin would be wrong for a caller arriving through the
proxy. Nothing in this document settles it; a single handshake against each door
does.

---

## 10. The decisions left open

### 10.1 How a phone gets past the gate — the one that changes what gets built

`ADR-046` §4.1 requires a client certificate chaining to the fleet authority,
and `trends-adapter-rivet` requires one on the application's own port as well —
the manifest reading above. A device token and a certificate pin satisfy
neither. Three answers:

| | What it means | Cost |
| :-- | :-- | :-- |
| **A. The phone gets a fleet-issued client certificate** | pairing issues, or is accompanied by, an X.509 key pair the machine will accept | the largest piece of unbuilt work in this subject: minting for a device that is not an operator, delivering the private half to an Android keystore, and revoking it |
| **B. `proxy-auth = "none"` on the personal-data route, with the application's own token as the sole gate** | one manifest key, shipped in a signed artifact | contradicts `.rulings/` R-152, which named personal data as the case the operator certificate was for, and it would not by itself satisfy the daemon's own requirement on `:8787` |
| **C. Reach the backend directly** | the supervisor binds `0.0.0.0`, so the port is reachable | closed for Trends by `requiring_a_caller_certificate_from`; **open for Reliquary today**, which is the other half of the same reading |

**Recommendation: rule A, and treat the client-certificate issue as its own
design candidate rather than a clause of this one.** B and C each contradict a
ruling or a design that is in force, and A contradicts neither; it is also the
answer the maintainer's own end state implies — a phone that pairs to a Rivet-hosted personal-data service is a peer of the
fleet, not a stranger with a bearer token.

**And rule on C independently**, because it is live now: Reliquary's own port
answers the LAN with `with_no_client_auth`, so the archive of a year of personal
records is reachable behind a device token while the front door in front of it
demands a certificate. That may be exactly what is wanted — it is what makes the
phone able to back up at all — but it should be a decision in the record rather
than a consequence of two manifests read separately.

### 10.2 A `Source::Synthetic` variant in Trends

§6.2 uses `manual` because the enum has nothing better. A ninth variant would
make a harness row self-describing and impossible to mistake, and it would make
`WHERE source = 'synthetic'` the cleanup rather than a `LIKE` over `source_ref`.
It is a wire-format and schema change in Trends, so it is his.

**Recommendation: add it, before the first synthetic row is pushed at anything
holding real history.** The markers in §6.2 work without it; adding it later
means two generations of harness rows marked two ways.

### 10.3 One URI scheme or one per service

`Phone-Pairing.md` §5 asks whether `trends://pair` is generalised. Applying
converge-first per field rather than per string:

**Recommendation: converge the PAYLOAD, keep the SCHEME per service.** The field
set — `address`, `code`, optional `pin` — becomes one grammar with one parser
and one set of conformance vectors, which is where the drift risk actually
lives. The scheme stays `trends://pair` and `reliquary://pair`, because an
intent filter matches on scheme and host, one application registering two costs
two lines, and a shared `pair://` scheme is a namespace any other application on
the phone can claim. The client has to know which service it is talking to in
order to know which endpoints to call, so a `service` field would be a second
statement of something the scheme already says.

This also means the harness reads the **real** QR the Trends dashboard draws
today, unchanged, which is the strongest form of §3.1's convergence rule: the
transport under test is the one that ships.

### 10.4 Where `pairkit` lives once two clients want it

Named rather than answered, because it is Girder's question in Kotlin and it is
not this harness's to settle. §5 states the cost of leaving it open, which is one
extra copy of the pin for as long as it stays open.
