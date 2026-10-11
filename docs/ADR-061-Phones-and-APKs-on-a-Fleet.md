# ADR-061: Phones and APKs on a Fleet — what Rivet provides for Gantry

* **Status:** Accepted, **built, and running on the fleet server** except
  §6.4, which waits on a route in Gantry. §9 names how each part was proved,
  and `tools/qemu/adr-061-gantry-proof.sh` is the proof that runs again.
* **Date:** 2026-09-27
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Ruled:** by the maintainer (`.rulings/` R-220, R-221, R-222, R-223)
* **Extends:** `ADR-029` §11 (a fifth kind), `ADR-028` §5.5.2 (the application
  key's mask), `ADR-034` (a second kind of record), `ADR-046` §7 and §4.5 (a
  plain listener), `ADR-048` §4 (two new verdicts), `ADR-052` §7 and §7.4 (the
  `apk` kind in both stores, and `--sync`)
* **Specification:** `SPEC-06-The-Fleet-Servers-Gantry-Socket.md` is the
  normative wire format of §6
* **Source of the requirements:** Gantry's `docs/Rivet-Requirements.md`, in the
  Gantry repository. Where this ADR and that file disagree, the difference is
  reported to a Gantry session rather than resolved here.
* **Answers:** *"how does a phone install and update the maintainer's own APKs
  from his fleet, and what must Rivet grow so that it can?"*

---

## 1. Context

**Gantry is an Android app and a Rivet application.** The phone pairs with a
fleet once, by an eight-character code, and from then on installs and updates
the maintainer's own APKs — Gantry itself, the Trends app and others — from
the fleet repository. Its server half is an ordinary Rivet application named
`gantry`, deployed to the fleet server beside Reliquary. Its design lives in its
own repository; this ADR is what Rivet provides it.

Rivet has to grow six things, and each one lands in a part of the system that
already has a rule about it:

| need | lands in | the rule it meets |
| :-- | :-- | :-- |
| an APK as an artifact | `ADR-029`'s kinds | a new kind is a token, an interpretation and one installer branch, with no format change |
| one command that publishes one | `ADR-052` §7 | nothing reaches a device without passing through the repository |
| a list of paired phones | `ADR-034`'s register | the register belongs to the fleet server |
| a way for `gantry` to read the store and the phones | `ADR-015`'s endpoints | a provider is reached over a socket the supervisor grants |
| an install page a phone reaches before it has paired | `ADR-046`'s proxy | the proxy was TLS only |
| an API a phone reaches with its own credential | `ADR-048`'s verdicts | an application needing its caller's identity had to run its own port |

**Why the phone-facing half is not inside `rivet-fleetd`.** `rivet-fleetd` is
the operator-only administration surface and holds the fleet's certificate
authority. A listener that faces phones and carries an open pairing endpoint
does not belong in that process. So `rivet-fleetd` keeps ownership of the data
and `gantry` asks for it (§6).

**The pattern that retires.** Reliquary pairs a phone over a second, directly
reachable TLS port (`ADR-048` §3). Gantry is the first application that
authenticates its own callers **behind** the proxy on 443, and that is now the
pattern for phone traffic (§7.2). `design-candidates/Phone-Pairing.md`'s survey
of the Reliquary and Trends ceremonies is the background Gantry's pairing
design was drawn from.

---

## 2. Decision

1. **An APK travels as an `.rvt` of kind `apk`** (§3). The application key may
   sign one, a fleet server's store admits one, and a machine refuses to
   install one.
2. **`cargo rivet fleet publish` takes an `.apk` as well as an `.rvt`** (§4),
   reading, checking, wrapping and signing it with Rivet's own APK reader.
   **One flag, `--sync`, carries any publish on to the fleet's server**, for
   every kind.
3. **A paired phone is a record in the fleet server's register** (§5), keyed by
   a hash of its P-256 public key.
4. **`rivet-fleetd` provides one socket, `gantry`**, answering only the `gantry`
   application's uid, with the operations of §6 and the wire format of
   `SPEC-06`.
5. **Phones are managed from two surfaces with one set of operations** (§6.3):
   Gantry's portal over the socket, and an agent through `cargo rivet fleet
   phones`. A phone is created only by pairing.
6. **The proxy gains two verdicts, `plain:` and `app:`** (§7). A `plain:` route
   opens port 80 on that machine; an `app:` route is one the application
   authenticates itself.

---

## 3. The `apk` kind

### 3.1 The header

| field | value |
| :-- | :-- |
| `kind` | `apk` |
| `product` | a name of at most 32 bytes: `gantry`, `gantry-dev`, `logbook` (§4.3) |
| `version` | the APK's `versionName`, for example `0.1.17` |
| `arch` | `any` |
| `platform` | `android` |

Nothing in `ADR-029`'s format changes. `apk` is a token that passes
`rivet_core::package::kind`'s validation, one module that interprets its
members, and one branch in the installer that refuses it (§3.4). That is
`ADR-029` §11's claim — a kind the format did not anticipate costs no change to
the verifier, the builder or the layout — tested by a fifth kind.

### 3.2 The members

| member | stored | holds |
| :-- | :-- | :-- |
| `app.apk` | **uncompressed** | the APK, byte for byte as the build produced it |
| `apk.json` | yes | the metadata below |
| `icon.png` | optional | the launcher icon at 192×192, **designed in and not produced by any build** |

`apk.json`'s keys, every one read from the APK and none typed by hand:

| key | from |
| :-- | :-- |
| `application_id` | the manifest's package name |
| `version_code` | the manifest's `versionCode` |
| `version_name` | the manifest's `versionName` |
| `label` | the application's label, resolved through the resource table where it is a reference |
| `min_sdk`, `target_sdk` | the manifest's `uses-sdk` |
| `signer_sha256` | lowercase hex SHA-256 of the DER certificate the APK is currently signed with |
| `development` | `true` when `application_id` ends in `.dev` |

**`app.apk` is stored rather than compressed** because an APK is already a zip
of compressed entries. Compressing it again spends time on both ends for
nothing, and a stored member can be streamed to a phone as it lies.

**The icon is optional because a launcher icon is usually vector XML.**
Turning an adaptive icon into a PNG needs an Android-grade vector renderer,
and the icon is decoration rather than function. So the member's name and shape
are fixed now, a reader that finds it absent shows a generic icon, and a later
`--icon FILE`, or a PNG found in the APK's own resources, can fill it without
changing the kind.

**There is no flag for irreplaceable data.** Whether an application's data
survives an uninstall depends on whether it was backed up, not on a label baked
into its APK. Gantry's uninstall warning is therefore the same for every
application and says that anything not backed up is lost (`.rulings/` R-222).

### 3.3 Trust

**The application key's trust entry gains `apk`.** An APK is the maintainer's
own application in the same sense an `application` is, so it joins the
application key's mask rather than getting a key of its own (`ADR-028`
§5.5.2). An `apk` signed by the system key is refused by name, exactly as an
`application` signed by it is. The trust store is inside the image, so this
reaches a machine as a system update.

### 3.4 Who holds one, and who refuses one

* **The workstation's repository** holds `apk` artifacts under the rule it
  applies to every kind: the three newest per product, kind, platform and
  architecture (`ADR-052` §7.2.2).
* **A fleet server's store** admits `apk` beside `application` and `library`
  (`ADR-052` §7.4.3), and keeps what the pushed manifest names.
* **A machine refuses to install one**, by name, at the installer's kind branch,
  as it refuses a `card`. An APK has no meaning on a Rivet machine; the fleet
  server holds it only so that `gantry` can hand it to a phone.

---

## 4. Publishing

### 4.1 The command

```text
cargo rivet fleet publish FILE.apk [--product NAME] [--signing-key PATH] [--at DIR] [--sync [confirm]]
cargo rivet fleet publish FILE.rvt --trusted-key PATH [--at DIR] [--sync [confirm]]
```

**One flow, for every kind** (`.rulings/` R-222). A publish always lands in the
workstation's repository first: the object is copied into `objects/`, the entry
appended to `manifest.toml`, and **only `manifest.toml` is committed**. A publish
does not sweep up whatever else the index happened to hold.

**A publish into the workstation's repository needs no `confirm`**, for either
kind: it changes one git repository on the workstation, and one revert undoes
it. **Reaching the server is what asks for `confirm`**, as `fleet push` and
`fleet sync` already do: `--sync` without `confirm` publishes locally and prints
what it would send; `--sync confirm` sends it (§4.4).

**A file is an APK when it is named `.apk` and begins with a zip's local-header
signature**; either without the other is refused. `--product` and
`--signing-key` belong to an APK and `--trusted-key` to an `.rvt`, and each is
refused with the other kind.

Handed an `.apk`, the command:

1. **reads the APK** (§4.2): package name, `versionCode`, `versionName`,
   label, SDK levels, the current signing certificate, and the v3 rotation
   lineage if one is present;
2. **refuses a `versionCode` not higher than the product's newest** in the
   repository, naming both. Android will not install a lower one as an update,
   and a publish that could never be installed is refused before it is made;
3. **refuses a signing certificate that is neither the product's last one nor
   linked to it by the APK's rotation lineage**, naming both fingerprints.
   Android would refuse the update on the phone; refusing it here moves the
   refusal to where the build is;
4. **pins the key on a product's first publish**: with no earlier artifact of
   that product there is nothing to compare, so the certificate is accepted and
   its fingerprint printed as the key the product is now held to. That is what
   Android itself does on a first install;
5. **wraps and signs** per §3 with the application seed, found the way every
   application publish finds it (`ADR-052` §7);
6. **publishes** as above, and with `--sync` carries on (§4.4).

**The product's newest version code and last signer come from every `apk`
object the repository holds for that product** — at most three (`ADR-052`
§7.2.2) — each verified against the application key before its `apk.json` is
read. The one with the highest `versionCode` is the newest, and its signer is
the pinned one. Ordering by `versionName` instead would be wrong, because
Android compares `versionCode` and a `versionName` need not agree with it. An
entry whose object is missing from `objects/`, or whose object does not verify,
is refused rather than treated as absent, since treating it as absent would
re-pin the product to whatever key came next.

**`versionCode` is Android's full comparison value**: a `versionCodeMajor`, when
present, is folded in as the high 32 bits.

**A `versionName` must be one the repository can order**, as every other
artifact's version is (`ADR-027`): dot-separated numbers, with everything from
the first `-` onward ignored. So `1.0` and `0.1.17-beta` order, and `v1.0` and
`1.0b` do not. It must also order the same way as `versionCode`, and strictly
above the one it replaces, so two builds differing only after a `-` cannot both
be published. The header's version is `versionName`, and the repository's
three-newest retention orders by it, so a name it could not read would let the
retention keep the wrong three. Such an APK is refused at publish, naming the
rule.

### 4.2 Rivet reads the APK itself

The command reads four structures, all well specified and none large: the zip
central directory; the binary XML of `AndroidManifest.xml`; enough of
`resources.arsc` to resolve a label reference; and the APK Signing Block's v2
and v3 signer certificates and v3 proof-of-rotation lineage.

**It reads and does not verify.** Android verifies an APK's signature when it
installs it and refuses one that does not verify; Rivet's check is about which
key, and a certificate's SHA-256 answers that without any cryptography. The one
dependency is **`miniz_oxide`**, pure-Rust inflate, because the entries inside
an APK are deflated. It brings one crate with it, `adler2`, a checksum with no
dependencies of its own. It is linked into
`cargo-rivet` only and ships on no board (`.rulings/` R-222).

**Why not Android's own tools.** `aapt2` and `apksigner` are authoritative, but
calling them would put the Android SDK on the path of every machine that
publishes, and would mean parsing human-readable output. The formats are stable
enough that reading them directly is the smaller dependency.

**Fixtures are real APKs from the maintainer's own builds**, labelled as such,
beside hand-composed vectors for the refusals (`AGENTS.md` §4).

### 4.3 The product name

**Derived from the application id**: its last dotted segment, with a trailing
`.dev` becoming `-dev`, so `…gantry.dev` is `gantry-dev` and `…logbook` is
`logbook`. `--product NAME` overrides it. A derived name that breaks the
32-byte limit or `ADR-027`'s name grammar is refused, and the refusal names
`--product`.

### 4.4 `--sync`: files, then the list, to the fleet's own server

`--sync` does, in this order, what `fleet sync` and `fleet push` do today:
**it sends every object the manifest names that the server lacks, then pushes
`main`**. That order is the point. A pushed list that names a file the server
does not hold leaves the server refusing to prune until the file arrives
(`ADR-052` §7.4.4), so pushing the list alone is a half-finished state, and
there is no flag that produces it.

**It needs no host.** It dials the register's one record with `role =
"fleet-server"`, the same record `rivet-fleetd` finds itself by (`ADR-034`).

**It carries every committed record, and that is safe.** Nothing on the server
acts on a pushed desired-state document: the server reads `manifest.toml` out of
the pushed repository to decide what it may prune, and nothing else, and the
component that applies desired state is forbidden from reading this repository
(`ADR-044`). A card's provisioning document is committed only after the card has
been written, and a draft stays in `~/.rivet/provision/` until then. So an
unpushed commit is a record of something already done, and sending it early
changes no machine.

**Uncommitted files do not block it**, because they never travel. `--sync` names
any it left behind, so nobody believes they went. The standalone `fleet push`
keeps its stricter refusal, which exists for an operator pushing by hand.

**For an APK, `fleet publish app.apk --sync confirm` is an agent's entire
deployment step.** The maintainer then refreshes Gantry and taps Update. An
agent never installs a build on his phone with `adb`: publishing is the route.

---

## 5. Phones in the fleet register

### 5.1 The record

| field | value |
| :-- | :-- |
| `identity` | base64url, unpadded, of the SHA-256 of the phone's DER SubjectPublicKeyInfo |
| `label` | at most 64 characters, no control characters: the device model, or a name |
| `public_key` | the DER SubjectPublicKeyInfo of an EC **P-256** key |
| `enrolled` | an RFC 3339 UTC timestamp |

**P-256 and not Ed25519**, which every other key in Rivet is: Android's
hardware-backed keystore cannot hold an Ed25519 key for this use, and a phone's
key that could leave its hardware would make its credential only as strong as
the phone's filesystem.

**The identity is derived, never supplied.** `rivet-fleetd` computes it from
the key it is handed, so a caller cannot name one phone's identity with another
phone's key.

### 5.2 Where it lives

`register/phones/<identity>.toml` inside `rivet-fleetd`'s data directory,
beside `register.toml` and `devices/`, written through `rivetos_register` like
every other register file and atomically replaced the same way. **One file per
phone** is why enrol, rename and revoke need no lock against each other: each
touches exactly one file, and a list is a directory read.

**A phone belongs to the fleet, not to a machine or an application.** A device
pairs with the fleet server, and any application that later checks a phone's
token checks it against this one list (§8). The fleet server's register is the
authority (`ADR-011`). The workstation's register holds no phones and does not
learn about them, so the question of which of the two copies is right never
arises: only one copy exists.

**A register with no `phones/` directory is a register with no phones.** Every
register written before this ADR reads unchanged, and a register written after
it reads unchanged by an older `rivet-fleetd`, which never looks there.

**Revoking removes the file.** Nothing is tombstoned, because nothing reads a
phone that no longer exists: a token signed by a revoked key fails the moment
its key is no longer in the list.

---

## 6. The two surfaces over one set of phones

### 6.1 `rivet-fleetd` provides one socket, `gantry`

`rivet-fleetd`'s manifest declares `provides = ["socket:gantry"]`, and
`gantry`'s declares `connects = ["rivet-fleetd:gantry"]`. It is an ordinary
single-consumer endpoint: the supervisor makes it `0600` and gives it to the
one application that declares the connection (`ADR-015`).

**`rivet-fleetd` also checks the caller.** Each connection's `SO_PEERCRED` uid
must be the uid `rivet_core::service::app_identity` derives for `gantry`, or the
connection gets one refusal and is closed. The supervisor's grant is the first
lock and the uid check is the second: an endpoint is given to whichever
application declares it, so a second application that declared this one first
would otherwise be answered.

**It never listens on a network port.** Gantry faces phones; `rivet-fleetd`
faces only Gantry and the operator.

**The operations**, whose wire format is `SPEC-06`:

| operation | does |
| :-- | :-- |
| `fleet` | returns the fleet's name |
| `artifacts` | lists every held `apk` artifact: its BLAKE3 digest and its 4096-byte header |
| `member` | returns one member's stored bytes, by digest and member name |
| `phones` | lists every phone record |
| `enrol` | adds a phone from its public key and label, returning the record |
| `rename` | changes a phone's label |
| `revoke` | removes a phone |

`rivet-fleetd` reads the store's `objects/` and the register's `phones/` at the
point of each request. It caches neither, so what Gantry is told is what the
disk holds (`AGENTS.md` §5).

### 6.2 Why this is a socket and not the control plane

`rivet-fleetd` itself reaches `rivet-mcpd` over the control plane (`ADR-034`),
because it is confined and the control plane is how a confined program asks
for what its confinement withholds. Gantry needs nothing withheld from
`rivet-fleetd`: the store and the register are both inside `rivet-fleetd`'s own
data directory. The question is only whether a second application may read
them, and a supervisor-granted endpoint is exactly that answer (`ADR-015`).

### 6.3 The agent's surface: `cargo rivet fleet phones`

Parity between a person and an agent (`ADR-012`) means Gantry's portal and an
agent get the same operations:

| operation | Gantry's portal | an agent |
| :-- | :-- | :-- |
| create | shows a pairing code; the phone enrolls over the socket | `cargo rivet fleet phones pair` (§6.4) |
| read | lists phones | `cargo rivet fleet phones list` |
| update | renames | `cargo rivet fleet phones rename ID LABEL [confirm]` |
| delete | revokes | `cargo rivet fleet phones revoke ID [confirm]` |

**A phone is created only by pairing**, because only the phone holds its key.
Nobody, person or agent, types a phone into the register.

**The agent's verbs go over the control plane**, as every other `cargo rivet`
verb against a machine does. They are board-side operations in `rivet-core`
(`get phones`, `apply rename-phone`, `apply revoke-phone`), answered by the
fleet server over `rivet-mcpd`, which writes the register's phone files the
way a member's enrollment record already reaches that register.

**The board moves phone records as bytes; the workstation reads them.**
`get phones` returns each record file whole, and `rename-phone` writes one
whole record back, exactly as `enroll-member` and `remove-member` move a
device record. `rivet-core` cannot depend on `rivetos_register` (`ADR-014`),
and a second TOML reader of one format on the board would be a second
authority for it (`ADR-011`). So `cargo rivet fleet phones`, on the
workstation, decodes and renders each record with `rivetos_register::phones`,
the one reader and writer of the format. A rename reads the record, changes
only its label, and sends the whole record back, and always sends `confirm`:
without it, the verb prints a preview built from the record it read and sends
nothing. Two writers of
one directory stay safe because each operation replaces or removes one file
atomically, and neither writer keeps a copy. On a machine that is not a fleet
server, each operation refuses and says so.

### 6.4 An agent starting a pairing

`cargo rivet fleet phones pair` calls an **operator-gated route on `gantry`**
through the proxy, with the operator's certificate, as `cargo rivet backup`
reaches an application's own route. Gantry mints a pairing code exactly as its
portal's button does and returns the code, the install URL and the QR code. The
verb prints the code and URL, draws the QR in the terminal, and writes it as a
PNG through `cargo-rivet`'s own PNG writer, so an agent in a desktop client can
show it and the maintainer scans it.

**There is one QR generator, and it is Gantry's** (`.rulings/` R-223). Gantry
returns the QR as an SVG for its portal and as the raw module grid for this
verb, and the verb only renders what it is given. So Rivet takes no QR
dependency, and the portal and the agent always show the identical code.

This verb depends on Gantry's route. It is the last part of this ADR to be
built.

---

## 7. The proxy's two new verdicts

`ADR-048` §4.1's rules gain two verdicts beside `none` and `operator`:

    proxy-path = "gantry"
    proxy-auth-paths = ["app:api", "app:pair", "operator:admin", "plain:install"]

### 7.1 `plain:` — served over HTTP, and the only thing that opens port 80

A `plain:` prefix is served with no authentication on 443, **and also on port
80 over plain HTTP**. It exists for exactly one situation: a phone that has not
paired cannot check the fleet's private certificate, so an HTTPS install page
shows it a warning to click through and gains it nothing. Gantry verifies its
own authenticity after pairing, which is where that check belongs.

* **Port 80 exists only on a machine where an installed application declares a
  `plain:` route.** Every other machine is exactly as it was: no listener, no
  redirect, nothing answering (`.rulings/` R-221).
* **On port 80, everything that is not a `plain:` route is `404`**, the index
  included, and **nothing redirects to HTTPS**. Port 80 never looks like a
  second front door.
* **Only `GET` and `HEAD` are served on port 80.** A plain route is for fetching
  a page and a file; a write over plain HTTP is refused `405`. A request that
  must change something is made over 443, after pairing.
* **`ADR-048` §5's traversal refusal binds port 80 too**, before any route is
  found.
* **Every plain route is journalled by name and labelled on the index**, for the
  reason every open prefix is (`ADR-048` §4.3).

`ADR-046` §7 carries the port and §4.5 the cross-site rule's origin on port 80.

### 7.2 `app:` — the application authenticates its callers

An `app:` prefix is **forwarded exactly as `none:` is**: no caller certificate
is asked for, and the `Authorization` header passes through unchanged, as every
header except the hop-by-hop ones does. What differs is what the declaration
says. `none:` means *anyone may read this*; `app:` means *the application checks
a credential here*. A reader auditing a manifest, the journal or the index can
tell a public path from an application-authenticated one without reading the
application.

**This is Rivet's pattern for phone traffic** (`.rulings/` R-221). A phone holds
a hardware key and signs a short-lived token with it, and the application
verifies the token against the phone list of §5. So a phone's traffic goes
through the one front door on 443 with the rest, and no application needs a
port of its own to know who is calling. `ADR-048` §3's two-socket arrangement
and `ADR-046` §4.4's own-TLS route are both things to migrate off.

---

## 8. What this does not do

* **It provides no token-checking function in the SDK.** Gantry verifies its
  own tokens. A shared verifier, so every application checks a phone's token
  with one piece of code, is expected work, not built here.
* **It does not let other applications read the phone list.** When Reliquary
  checks tokens Gantry issued for it, it will need the list; that is a separate
  **read-only shared** endpoint, so no other application is ever handed enrol,
  rename or revoke.
* **It does not tell a phone when an application was last backed up.** Gantry's
  uninstall warning could one day name the last backup Reliquary holds; that
  needs Reliquary to answer the question, and is expected work.
* **It does not move Reliquary off its own port.** §7.2 makes that the
  direction; the move is Reliquary's own change.
* **It installs nothing on a phone.** Gantry does that, through Android's own
  installer, which verifies the APK's signature itself.

---

## 9. How each part is proved

| part | proved by |
| :-- | :-- |
| the kind, the members, the trust mask, the installer's refusal | `rivet-core` tests: an `apk` signed by the application key verifies, one signed by the system key is refused by name, an install is refused by name |
| the APK reader | `cargo-rivet` tests over real APKs from the maintainer's builds and hand-composed refusal vectors |
| publish's refusals | a second publish with the same `versionCode` refused; one signed by another key refused; a rotated key with a lineage admitted |
| `--sync` | `fleet_sync` tests: files before list, uncommitted strays named and not blocking |
| the phone records | `rivetos_register` round trips, and a restart of `rivet-fleetd` reading them back |
| the socket | a test client under `gantry`'s uid answered and under any other uid refused, in the container and under the QEMU harness |
| `plain:` and `app:` | the proxy's tests: port 80 bound only with a plain route, everything else `404`, a write `405`, `Authorization` forwarded byte for byte |
| the whole | dev-01 takes the system update that carries the trust change and the proxy; then rivet-01 takes it and the new `rivet-fleetd`, and a real APK published with `--sync` appears in its store |

**Nothing existing on rivet-01 changes.** The store gains a kind, the register
gains a directory, the proxy gains a listener only when `gantry` declares a
plain route, and `rivet-fleetd` gains a socket. dev-01 is a member and runs no
fleet server, so the fleet-server half is proved in the container and under
QEMU before it reaches rivet-01.
