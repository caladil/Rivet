# ADR-051: The Application Backup Contract — what an application offers, and what fetches it

* **Status:** Accepted. **The artifact half is built, with tests.** §3, §4, §5,
  §6 and §7 are code; **§8's feed is declared and deferred**, deliberately and by
  name; **§8.5's `--service` filter is built, with tests, and reads over the
  real control plane against `rivet-01`.** **§12's restore — the `restore-path`
  key and `cargo rivet restore` — is built, with tests and a QEMU proof; TempMon,
  Reliquary and a third adopter each declare `restore-path`, and none has restored
  through the proxy on a machine.** **§8.1's transport is the control plane**
  (`.rulings/` R-234): both verbs fetch and send over port 7777 and present no
  certificate but the operator's control-plane identity. §11 and §12.9 say what
  is proved, by what method, and what is not.
* **Code:** `rivet-core/src/package/application.rs` (the four manifest keys,
  `BackupFormat`, `BackupSurface`), `rivet-core/src/backup.rs` (the derived
  table, the restore contract's constants and the validation ladder),
  `rivet-core/src/proxy.rs` (`requires_operator_under`, the longest-match §12.2
  asks), `cmd/rivet-sysd/src/runtime.rs` (`publish_backups`),
  `rivet-core/src/generated.rs` (the published path),
  `cmd/cargo-rivet/src/backup.rs` (the consumer),
  `cmd/cargo-rivet/src/restore.rs` (§12's verb),
  `cmd/rivet-mcpd/src/backups.rs` (§8.1's relay on the machine),
  `rivet-remote/src/tls.rs` (`Tls::http`, the workstation's half of it),
  `cmd/cargo-rivet/src/crate_meta.rs` (the author's side of the keys),
  `tools/qemu/adr-051-proof.sh` (§11's two boots),
  `tools/qemu/adr-051-restore-proof.sh` and `tools/qemu/restore-probe` (§12.9's
  boot)
* **Date:** 2026-09-08
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Ruled:** by the maintainer, 2026-09-08, in conversation. Validation stops at
  the format — asking whether a file is a valid database or a valid CSV is
  probably as far as it needs to go (`.rulings/` R-306). The two shapes, the
  format's declaration
  living in the manifest, and the exclusion of retention and cadence from this
  verb are his. **`.rulings/` R-183 D1, 2026-09-14:** an application restores
  its own data, at a `restore-path` it declares, and `cargo rivet restore`
  checks the file against `backup-format` before sending it (§12).
  **`.rulings/` R-186, 2026-09-14:** `rivet-proxyd` refuses a cross-site write
  for every application, and §12.7 names it (D1); a restore leaves the old data
  or the new, each whole, never a mix (D2). **`.rulings/` R-234, 2026-10-03:**
  backups and restores ride the control plane; the machine reaches the
  application over loopback and streams the bytes on the bulk endpoint's
  listener, with no new credential and no new format (§8.1).
* **Answers:** `ADR-030` §2's exclusion of application data, which was correct
  and left nothing covering it.
* **Builds on:** `ADR-015` (the manifest as a permission request, and the
  signature over it), `ADR-046` (the path proxy and the operator gate),
  `ADR-048` (a path relative to `proxy-path`), `ADR-011` (one fact, one home),
  `ADR-035` (the control plane)

---

## 1. Context — a correct exclusion that left a hole

`ADR-030` §2 rules what Rivet's own backup covers. Its table excludes
**application data**, and the reason is sound: *"it is not on the thing this ADR
backs up"* — an application and its data live together on a volume (`ADR-031`),
and whole `/state` **remains the operator's** (`ADR-020` §9).

Nothing replaced it. On `rivet-01` that meant TempMon's two years of readings
and Reliquary's blobs, among the applications' data, were covered by nothing at
all, and were *meant* to be.

**The occasion was a 403.** The maintainer's TempMonitor tray began raising
Windows notifications: `https://rivet-01.local/tempmon/api/v1/backup` answering
**403**, because TempMon's manifest had moved from `proxy-auth = "none"` to
`"operator"` hours earlier and the tray held no operator certificate. The tray
was the wrong client, not the gate the wrong decision — and the general question
underneath it was the one this ADR answers: **how does an application offer a
backup at all, and who takes it?**

### 1.1 The exclusion is unchanged, and this is not a reversal

Rivet still does not decide what an application's data is worth keeping. What
changes is that an application can now **say what it offers**, in the artifact
somebody signed, and something outside can read that offer without knowing
anything about the application. The platform carries a declaration; it makes no
retention decision, holds no copy, and schedules nothing.

**The same holds in the other direction.** An application that accepts its
backup back says so in the same signed manifest (`restore-path`, §12), and it
performs the restore itself. Rivet checks the file's format and sends it; it
stops nothing and writes nothing into the application's data directory.

---

## 2. Decision

**An application declares a backup surface in its signed manifest. The
supervisor derives a machine-wide table from every installed manifest and
publishes it root-owned. A workstation reads the table and fetches each surface
over the control plane — the machine reaches the application on loopback and
streams its answer back — and validates what arrives at the level of the FORMAT
and never the schema.**

Four keys, two shapes, one ladder, and two verbs: `cargo rivet backup` fetches,
and `cargo rivet restore` (§12) sends a checked file back to an application that
declared where it accepts one.

---

## 3. The manifest keys

| key | meaning | required with |
| :-- | :-- | :-- |
| `backup-path` | the route, **relative to `proxy-path`** — `api/v1/backup` | `backup-format`, `proxy-path` |
| `backup-format` | what one fetch returns: `sqlite`, `csv`, `json`, `tar` | `backup-path`, `proxy-path` |
| `backup-feed` | the *other shape*: a resumable collection | `proxy-path` |
| `restore-path` | where the application accepts its artifact **back**, relative to `proxy-path` — `api/v1/restore` (§12) | `backup-path`, `backup-format`, `proxy-path`; and a route that requires an operator certificate |

All four are **optional, and absence means *offers nothing***. That is the
closed end of `ADR-015` §3.3's axis and it is what every manifest emitted before
2026-09-08 says, so no `.rvt` already signed means anything different for being
read by a build that understands these.

They are appended to `rivet_core::package::application::KEYS`, which is the
canonical order a manifest is emitted in and parsed against. A key at the end
cannot make an older manifest out-of-order; a key inserted in the middle would.
`restore-path` was appended after `accepts-class`, not beside `backup-format`,
for that reason, and it renders last.

### 3.1 The path is relative, exactly as `proxy-auth-paths` is

`ADR-048` §4 established the shape: a prefix under the application's own mount,
one or more segments of lowercase letters, digits and hyphens, each beginning
with a letter, joined by single slashes, no leading or trailing slash and no dot
segment. `backup-path` and `backup-feed` take **the same predicate** —
`is_proxy_prefix` — rather than a second one that agrees today.

So `proxy-path = "tempmon"` with `backup-path = "api/v1/backup"` is
`https://<host>/tempmon/api/v1/backup`, and `..` cannot be spelled.

### 3.2 A backup surface with no `proxy-path` is REFUSED

Not ignored, and not accepted-for-later. A backup path is relative to the mount,
so with no mount it names nothing at all — and it would start naming something
the day somebody added a `proxy-path` beside it, which is a declaration becoming
live by an edit elsewhere in the file.

**It is the same defect as enabling `rivet-proxyd` without `upstream-name`**
(`SPEC-05` §5): a service configured to reach something, with no way to reach
it. `ADR-046` §4.3 already refuses `proxy-auth` attached to no route for this
reason; this is that rule a third time.

### 3.3 Either half of the artifact pair without the other is REFUSED

A route with no format is bytes a client can check only for length. A format
with no route is a claim about something unreachable. They are **one
declaration** written on two lines, so the manifest treats them as one.

The refusal names the missing key, and `cargo rivet package` refuses earlier
still — in front of the `Cargo.toml` the author just edited — because
`Manifest::parse` reads a build product and answers with a key name, which is
the right answer for a machine and the wrong one for a person.

---

## 4. Two shapes, and they are separate keys because the difference is a PROTOCOL

This is the decision most likely to be re-litigated, so the argument is here in
full.

**`backup-path` + `backup-format` is one route, one fetch, one whole thing.** A
client issues a GET and gets a file.

**`backup-feed` is a resumable collection.** A client presents a **cursor**; the
response carries a **low-water mark** — *"my oldest retained index is N"* — and a
client below it must treat that as an **error** rather than a fetch, because the
items between its cursor and N are gone and silently skipping them would produce
a collection with a hole nobody can see. Partial success advances the cursor
only to the last **contiguous** item.

**A single key with five values would have been wrong.** `sqlite`, `csv`, `json`
and `tar` all answer *"what did that one fetch return"*; a fifth value meaning
*"actually, resume"* would change what every other field of the interaction
means. A client written for one **cannot read the other by changing a string** —
it needs cursor state, a comparison against a low-water mark, and a definition of
partial success. That is a protocol difference and it earns its own key.

**The manifest refuses both**, and the exclusion is structural rather than
checked: `BackupSurface` is an enum with no value carrying a format and a feed at
once, so nothing downstream has a rule to get wrong.

### 4.1 A feed needs no format validation at all

Its item ids carry a **digest of the item**, so the client hashes what it
received and compares against the name it asked for.
`reliquary-store/src/filesystem.rs` mints ids exactly that way —
`mint_id(created_at, digest_hex)`, sixteen characters of a SHA-256 verified
before the blob counts as stored. There is nothing for §7's ladder to add.

---

## 5. The table the supervisor derives

**`rivet_core::proxy`, a second time, and deliberately not a second pattern.**
Every row comes from an installed application's signed manifest — its
`proxy-path`, and its backup keys. Nothing is configured. An operator cannot add
a backup surface, cannot move one and cannot change what one claims to be:
doing any of those means shipping a new artifact that somebody signed.

The supervisor derives it beside the routing table and from the same manifests
— at boot, after an install, and after an uninstall, so a removed application's
surface leaves the table when the application does — and writes it
**root-owned at `0644`**.

```text
tempmon-rivet tempmon artifact sqlite api/v1/backup
tempmon-rivet tempmon restore api/v1/restore
reliquary-rivet reliquary feed api/v1/blobs
```

**The shape is a word in the third column and it decides the arity** — five
fields for an artifact, four for a feed or a restore (§12.3). A reader that does
not know the word drops the line rather than guessing how many fields follow it,
which is
`crate::proxy::parse`'s fail-closed rule pointed at a different question. A
malformed line is skipped and the rest still load, because a client that refused
the whole file over one row would fetch nothing from any application on the
machine.

### 5.1 It is at `/run/rivet/backups`, NOT beside an application

This is where it departs from `rivet_core::proxy`, and the departure is the
reason rather than an omission.

The routing table lives in `rivet-proxyd`'s own `<volume>/app/<name>` directory
because **the proxy is its reader and the proxy is on this machine**.
`cmd/rivet-proxyd/src/main.rs` explains the placement: root-owned and
application-readable, so a program running as an application's uid cannot
rewrite its own routes.

**This table has no reader on the machine at all.** Its consumer is a
workstation. There is therefore no application whose directory it belongs
beside, and choosing one arbitrarily would make the file vanish the day that one
was uninstalled.

So it goes to `/run/rivet`, beside `crate::display::HOURS_PATH` and
`OS_VERSION_PATH`, which is where this machine's derived runtime facts live —
and `/run/rivet` is root-owned, so the property the proxy table's placement was
chosen for holds here for free. It is **not** in `/etc`, for
`crate::display::HOURS_PATH`'s reason: `/etc` exists for paths a C runtime
demands and no C runtime asks what an application offers. It is **not** under
`/state`, because a table derived every convergence that survived a reboot would
name applications since uninstalled, on a machine that had not converged yet.

It is registered in `rivet_core::generated::GENERATED_PATHS`, so
`rivet explain /run/rivet/backups` answers.

### 5.2 An empty table is written rather than omitted

Unlike the routing table, which is not written at all where the proxy is not
installed. An operator asking a machine what it offers must be able to tell
**nothing** from **this build does not answer that question**, and an absent file
cannot make that distinction. `cargo rivet backup`'s refusal on a machine with
no file says exactly this.

---

## 6. A workstation reads it with `get file`, and no control-plane surface was added

`ADR-006` §8.10.5's `get file` already reads a text file off a machine whole,
and `cargo rivet screenshot` already does exactly this against a path on a
board. A dedicated `get backups` was considered and rejected:

* it would be a **second name for one fact** (`ADR-011`) whose only advantage is
  not having to type a path;
* the catalogue is the machine's published interface, and every subject added to
  it is a promise this project keeps for ever (`ADR-006` §6.4);
* and `get file` already carries `generated`, so a caller reading the table is
  told by the machine that it is a projection and what changes it.

**`ADR-006` §8.10's writing subjects refuse everything outside `/state`**, which
means no operator and no agent can write this table over the control plane. That
is not a limitation of this design; it is the property that makes the table
worth reading.

---

## 7. The validation ladder

**In order, and every rung is format-level.**

1. **non-empty**
2. **bytes received == `Content-Length`**
3. **format magic**
4. **the format's own internal length claim**

### 7.1 Rung 3 alone is a trap, and the ladder exists because of it

A truncated SQLite file **still starts with `SQLite format 3\0`**. It passes a
header check, it looks like a backup, and it rotates a real one out. Measured on
2026-09-08 against `rivet-01`: a real 2,678,784-byte TempMon backup cut in half
still carried the magic, and only rung 4 refused it.

For SQLite rung 4 is six lines and no library: the header carries the **page
size at bytes 16–17** and the **page count at bytes 28–31**, both big-endian, and
`page_size × page_count` must equal the file's length. That is an exact
completeness statement the format makes about itself.

Two details that a naive reader gets wrong and this one does not: a page size of
**1 means 65536** — the format's own escape for a value that does not fit in two
bytes — and a page count of **0 is not a claim at all**, because SQLite before
3.7.0 wrote it, so rung 4 does not run rather than passing vacuously.

### 7.2 Rung 2 is the rung every format shares

| format | rung 3 | rung 4 |
| :-- | :-- | :-- |
| `sqlite` | `SQLite format 3\0` | page size × page count == length |
| `tar` | `ustar` at offset 257 | a multiple of 512 ending in two zero blocks |
| `json` | first non-space byte is `{` or `[` | the matching closer is the last non-space byte |
| `csv` | **none — CSV declares nothing about itself** | **none** |

CSV gets rungs 1 and 2 and nothing else, and the report **says which rungs ran**
rather than printing one word for four very different amounts of evidence.

**A response with no `Content-Length` is refused.** For CSV nothing after rung 2
applies, so a body of unknown declared length is a body whose truncation is
undetectable — and storing it would be exactly the failure the ladder exists to
prevent. **No content coding is undone on the way in**: the workstation decodes
the body's framing (`Content-Length` or `chunked`) and nothing else, because a
client that transparently decompressed would be comparing a decompressed length
against a compressed header.

### 7.3 It never validates a schema, and that is the whole point

*"Is this a valid SQLite database"* **cannot rot.** *"Is this a valid Trends
backup"* **rots at every migration** — it starts refusing good backups the day
somebody adds a column, and the person who then deletes the check deletes the
truncation detection with it.

The corollary is §3's: **the format is declared by the application, not known by
the client.** The maintainer's stated worry was per-application validators
rotting. Putting `backup-format` in the manifest removes the worry structurally,
because the declaration ships with the payload that changed — an application
migrating from CSV to SQLite carries its own answer across the version boundary.

---

## 8. `cargo rivet backup`

```text
cargo rivet backup --host HOST:PORT --identity DIR --pin HEX --into DIR [--service NAME]... [confirm]
```

**It exists because a contract with no consumer is worse than no contract: it
looks built.** `ADR-045` shipped a health vocabulary and an SDK `report` call the
supervisor read nowhere, and that is the cautionary tale this verb was written
against.

Without `confirm` it fetches nothing and prints the plan.

### 8.1 One transport: the control plane

**The control plane is the control plane** (`.rulings/` R-234). Everything
`cargo rivet backup` and `cargo rivet restore` exchange with a machine goes over
`ADR-035`'s listener on port 7777, whose trust is the operator's **pinned
Ed25519 key** (`ADR-025` §2). `--host`, `--identity` and `--pin` are the whole
of the transport, and nothing else is presented.

* **The declaration** is read with one `get file /run/rivet/backups` (§6), a
  tool call.
* **The payload** is fetched with `GET /backup/<service>` on the same listener,
  beside the bulk endpoint (`ADR-035` §3.2). A JSON-RPC tool call is not a way to
  carry a multi-megabyte database; an HTTP body on the bulk endpoint's listener
  is, and the listener already carries one.

**On the machine, `rivet-mcpd` reaches the application the way `rivet-proxyd`
does** (`cmd/rivet-mcpd/src/backups.rs`): it looks the service up in the table,
takes the loopback port the proxy's routing table gives its route (`ADR-046`
§5), dials it over TLS presenting the machine's fleet certificate from the
proxy's own data directory and verifying the application against the proxy's
`upstream-name`, and sends `GET /<backup-path>` — the route's prefix removed,
exactly as the proxy removes it. **The application's answer is streamed back
verbatim** in 64 KiB pieces: status line, headers and body, untouched, so the
ladder's second rung compares against the application's own `Content-Length`.
Nothing on the board holds a whole backup.

**No credential is new.** The workstation presents the identity it already
presents for every control-plane call; the machine presents to the application
the certificate the proxy already presents, which the application already
requires (`ADR-046` §4.2). The operator's browser certificate plays no part, so
it can be retired at the proxy machine by machine (`ADR-046` §4.1).

**The machine refuses, by name and before dialling anything,** a connection
that presented no pinned key, a request naming an application its table does
not carry — the refusal names what it does offer — a feed, a restore to an
application with no `restore-path`, and an application the proxy does not
route. Every refusal it composes carries `Rivet-Answered-By: rivet-mcpd`
(`rivet_core::backup::ANSWERED_BY_HEADER`), so a workstation tells *this
machine refused* from *the application answered* even where the status codes
are the same. The format is never checked there: the ladder is the
workstation's (§7).

`--authority`, `--name` and `--port` named the browser certificate, the name on
the machine's certificate and the proxy's port. **They are refused by name**,
rather than accepted and ignored, so a script still passing them learns that
nothing presents a certificate any more.

### 8.2 Retention, cadence and scheduling are NOT this verb's

Ruled 2026-09-08. **It fetches once and stores.** How often, how many to keep,
and where they go afterwards belong to a separate program that makes those
decisions — Fleet Monitor. There is deliberately no `--keep`, no `--daily` and no
state file, because a verb that remembered anything would be the beginning of
one.

The consequence is visible in one place: **an existing file is refused rather
than overwritten**, and the refusal says that replacing one backup with another
is a retention decision this verb does not make.

### 8.3 What it writes, and how

`<--into>/<service>/<filename>` — one directory per application, so two
applications cannot name one file. The filename is the service's own
`Content-Disposition`, **sanitised to a single safe path component**, because
that name is a fact only the service holds: TempMon names its backup for the
instant it was taken, in UTC, so a directory of them sorts lexicographically into
chronological order. A name holding a separator, a dot segment or anything
outside `[A-Za-z0-9._-]` is **declined rather than repaired** — repairing a
hostile name is how a repaired name ends up being used — and the client composes
`<service>-<epoch seconds>.<extension>` instead.

Bytes are written to `<name>.partial` and renamed. A file that failed the ladder
must never sit under a name suggesting it is a backup, and neither must one from
a process killed mid-write.

**One application's failure does not stop the others**, and the exit status still
says the run failed. A run that abandoned four applications because the first was
down would be a run that quietly did nothing.

### 8.4 The feed half is DECLARED and NOT built

`rivet_core::backup` carries `BackupSurface::Feed` because the manifest has to be
able to express it — an application declaring a feed must be **refused or
understood**, never silently misread as an artifact. `cargo rivet backup` reports
a feed and does not fetch it, and its `--help` says so.

**An honestly deferred half beats two half-built ones.** A client that fetched
the first page of a feed and called it a backup would be the second kind, and
`ADR-045` is what that looks like in this tree.

§4 records what the feed needs, so the next person is not designing it from
scratch: a cursor the client presents, a low-water mark in the response that a
client below must treat as an error, and partial success advancing the cursor
only to the last contiguous item.

### 8.5 `--service NAME` chooses WHICH applications, and is repeatable

**It narrows the fetch, not the report.** Named once or more, only those
applications' declarations are ever handed to the code that opens a
connection — an unnamed application's bytes never cross the network, rather
than crossing it and being discarded once they arrive. A filter that fetched
everything and threw away what was not asked for would be indistinguishable
from having no filter at all on the one measurement that matters, which is
what crosses the wire, so `--service` has to sit in front of the fetch loop
and it does.

**The plan says what the filter is about to do**, listing which applications
will be fetched and which were excluded by name, so an operator can see the
filter took hold before spending the transfer to find out. This is the same
plan §8's opening paragraph already describes — `--service` changes what goes
into it, not whether it exists.

**A name matching nothing on the machine is refused, not fetched as an empty
set.** The refusal names both sides — what was asked for and what the machine
actually offers — because the one thing worse than a filter that fetches too
much is a filter that fetches nothing and says nothing: an application
renamed on one side and not the other produces exactly that condition, and an
operator reading a clean exit would believe a backup had run. Fleet Monitor's
own configuration file refuses an override that matches nothing for the same
reason; this is that rule applied to a filter given on this verb's command
line instead of one written into a file — the same failure mode, the same fix.

**This does not touch §8.2.** `--service` selects *which* applications this
one run considers; it says nothing about *how often* the run happens or *how
many* copies survive it, and there is still no `--keep`, no `--daily` and no
state file behind it.

---

## 9. Rejected

| | why not |
| :-- | :-- |
| **Rivet takes the backup itself** | it would have to know how. TempMon's database is in WAL mode with a live writer, so a file copy produces something that opens without complaint and holds less history than it should — the consistent copy has to be taken by whatever owns the data. `ADR-030` §2's exclusion is right and this does not touch it |
| **One key with five values** | §4. A resumable collection is a protocol difference, not a format one, and a value that changed what every other field means is a value that has no business in the same key |
| **A `get backups` control-plane subject** | §6. A second name for one fact, whose only advantage is not typing a path |
| **Validating the schema** | §7.3. It rots at every migration, and the person who deletes the rotted check deletes the truncation detection with it |
| **Trusting format magic alone** | §7.1. A truncated SQLite file passes it, and measurably did |
| **Accepting a body with no `Content-Length`** | §7.2. For CSV nothing after rung 2 applies |
| **The table beside an application, as `crate::proxy` puts it** | §5.1. Nothing on the machine reads it, so there is no application it belongs beside, and choosing one would make it vanish when that one was uninstalled |
| **Retention and cadence in this verb** | §8.2. They are the operator's decisions and belong to a program that makes them |
| **A per-application validator in the client** | §7.3. The worry that killed it was the maintainer's own, and `backup-format` in the manifest removes it structurally |
| **Rivet restores the data itself** — `apply service-stop`, write the file into the data directory, `apply service-start` | §12.1. The first row again, in the other direction: which file, and when replacing it is safe, is knowledge only the application has. R-183 D1 chose the application |
| **The restore route as a sixth field on the `artifact` line** | §12.3. A reader built before it drops a line with a trailing field whole, so every older `cargo rivet backup` would stop seeing an application the day it accepted a restore |
| **A restore route the proxy serves without an operator certificate** | §12.2. A restore replaces an application's data and `rivet-proxyd` is where an HTTP caller is authenticated (`ADR-046` §4), so the manifest refuses the declaration rather than trusting every application to gate the write |
| **Streaming the file while it is sent** | §12.6. What is sent must be what was checked, and neither `rivet-mcpd`'s relay nor `rivet-proxyd` decodes a chunked request body |
| **Fetching through `rivet-proxyd` with the operator's browser certificate** | §8.1. A second trust model for one operator, and the one thing keeping that certificate alive once passkeys sign a browser in (`.rulings/` R-234) |
| **A token signed by the operator key, presented to the proxy** | §8.1. A new credential and a new format for what the control plane already authenticates |

---

## 10. Version skew

`rivet_core::package::application::Manifest::parse` refuses a key this build does
not define. So **an application declaring a backup surface cannot be installed on
a machine whose image predates this ADR** — the manifest is a member, the
installer parses it after verifying and **before writing anything**
(`cmd/rivet-deploy/src/install.rs`), and the refusal names the key.

That is the fail-closed direction and it is the correct one: the alternative
would be a machine that accepted a declaration it would never act on. The
consequence for an operator is an ordering requirement — **the image first, then
the application** — and it is the same ordering `library` and `devices` imposed
when they arrived.

The other direction is free. A machine running this build and an application
built before it: the application declares nothing, offers nothing, and does not
appear in the table.

`restore-path` (§12) repeats the first statement, and adds two about the
workstation, which reads a table written by whatever image a machine runs:

* **an image built before `restore-path`** refuses an application declaring it,
  naming the key — the image first, then the application, again;
* **a `cargo rivet` built before it**, reading a table that carries a `restore`
  line, drops that line and keeps the `artifact` line, so its `backup` is
  unaffected (§12.3, and a unit test replays that reader's rule over this
  build's output);
* **a `cargo rivet` built with it**, reading a table from an image built before
  it, finds no `restore` line and reports that the application offers no
  restore — which is true of that machine.

---

## 11. What is proved, by what method, and what is not

`AGENTS.md` §6. **Nothing here is a claim that the whole chain has run on one
machine**, because it has not.

**Proved by unit test**, in the container against the Linux target and on the
host: the three keys round-trip through the canonical manifest text; every
combination that is not one of the two shapes is refused, by name; a backup path
that could escape its application is refused at the manifest; the derived table
is a function of the set of installed applications rather than of discovery
order; a malformed or unknown-shape row drops that line and no other; and every
rung of §7's ladder, including a synthetic truncated SQLite that passes rung 3
and is caught by rung 4.

**Proved by unit test, §8.5's filter:** one name, several names, and every name
absent leaves the table untouched — the identity case a no-`--service`
invocation takes; a name matching nothing is refused, naming both what was
asked for and what the machine offers, including the case where the machine
offers nothing at all; the plan's account of what is included and what
`--service` excluded; and that an excluded declaration is never handed to the
code that opens a connection, rather than being fetched and discarded.

**Proved against `rivet-01`, 2026-09-08:**

* `cargo rivet backup` reading `/run/rivet/backups` over the real control plane
  and refusing correctly, because that machine's image predates §5;
* a **real** 2,678,784-byte TempMon backup fetched from
  `https://rivet-01.local/tempmon/api/v1/backup` through `rivet-proxyd`,
  presenting the operator's certificate — the transport §8.1 has since replaced,
  so this settles the application's route and `ADR-046` §4.2's gate, not the
  control plane's relay;
* §7's ladder run over **those exact bytes**: accepted at four rungs; refused
  with a `Content-Length` one byte short; refused with none; and refused when cut
  in half, with the magic still present.

**Proved against `rivet-01`, 2026-09-09 — the plan half of §8.5:** the same
machine's table read over the real control plane three times without
`confirm`: once naming one of its three applications, whose plan listed that
one to fetch and named the other two as excluded by the filter; once naming
none, whose plan listed all three and carried no excluded section at all,
matching the unit test's proof that the no-`--service` case is the identity;
and once naming an application not on that machine, which was refused before
anything but the table was read, naming the one word asked for and the three
the machine actually offers.

**Proved on a booted machine, under QEMU** — `tools/qemu/adr-051-proof.sh`,
which is a script rather than a paragraph so that it can be re-run. Two aarch64
boots of `rivet-initd` and `rivet-sysd` differing by **one manifest** and by
nothing else: an application declaring `backup-path` and `backup-format` gets
`declarations 1`, the same application with those two lines removed gets
`declarations 0`, and both boots name the application, so both read the manifest.
The second direction is the half that matters — a run asserting only the first
would pass against a supervisor that wrote `1` unconditionally, which is
`ADR-045`'s failure exactly.

**Proved for the artifact:** `tempmon-rivet` cross-builds for
`aarch64-unknown-linux-musl` and packages into a signed `.rvt` whose manifest
member carries both keys and verifies against the application key
(`cargo rivet inspect --trusted-key`).

**NOT proved: no board has derived the table.** The QEMU boots settle that
`publish_backups` runs, reads a real installed manifest and derives the right
number of rows; they do not settle a Raspberry Pi, which is `ADR-026` §4's
right-hand column. Closing it needs a card image built from this tree and an A/B
update onto a machine, which is a decision about somebody's running machine
rather than an engineering step.

**NOT proved: the file's CONTENT on a machine.** The QEMU proof reads the
supervisor's journal line, because nothing on that guest can read
`/run/rivet/backups` back — an application is confined by `ADR-042`'s ruleset and
was never granted that path, and the control plane is not brought up there. A
`cargo rivet backup` against a board carrying this supervisor is what closes it,
and it is the same command §11's first bullet already runs.

**NOT proved:** nothing has installed `tempmon-rivet` with these keys. §10 is
why the ordering matters, and the image has to go first.

**NOT proved: `confirm` with `--service` against a real machine.** Running
`confirm` was out of scope for the session that built §8.5 — the fleet it
would have fetched from holds a maintainer's real personal data, and a
plan-only run was the one form permitted. That an excluded declaration never
reaches the code that fetches one is proved by unit test, over synthetic
declarations, not by a network trace of what did and did not cross the wire
on a real run. Closing it needs `confirm --service` run once against a
machine offering more than one application, with the network watched to
confirm only the named one's bytes moved.

---

## 12. Restore — `restore-path` and `cargo rivet restore`

`.rulings/` R-183 D1: an application restores its own data. A `restore-path`
manifest key, the mirror of `backup-path`, accepts the application's own backup
back; the application handles its own consistency; `cargo rivet restore` checks
the file against the declared `backup-format` before sending it, over the
control plane (§8.1), with `POST /restore/<service>`.

### 12.1 The application restores itself, and Rivet stops nothing

§9's first row is why Rivet does not take a backup: the consistent copy has to
be taken by whatever owns the data. **A restore is that argument in the other
direction.** Replacing a live SQLite database, a journal or a blob store means
knowing which files, in what order, and when the program holding them is not
mid-write — and only the application knows. So the application accepts the
bytes at a route it declares and does the replacement itself.

`apply service-stop` exists (`ADR-017` §6.5) and `cargo rivet restore` does not
call it, before, during or after. A restore that stopped the service and wrote
a file into `/state/app/<name>/data` would be Rivet deciding where inside a data
directory a backup belongs, which is the decision this ADR leaves with the
application.

### 12.2 The key, and what it is refused beside

```toml
[package.metadata.rivet]
proxy-path    = "tempmon"
backup-path   = "api/v1/backup"
backup-format = "sqlite"
restore-path  = "api/v1/restore"
```

* **Optional; absence is *accepts no restore*.** Every manifest emitted before
  2026-09-14 says that.
* **Relative to `proxy-path`, by `backup-path`'s predicate** — `is_proxy_prefix`,
  §3.1. It may name the same route as `backup-path`.
* **Only beside the artifact shape.** A restore is checked against
  `backup-format`, so `restore-path` with no `backup-path`/`backup-format` is
  refused naming `backup-path`, and beside `backup-feed` (which has no format) it
  is refused as a value the manifest cannot admit. In the code the field lives on
  `BackupSurface::Artifact`, so no value carries a restore without its format.
* **Only on a route that requires an operator certificate.** The manifest asks
  the proxy's own longest-match (`rivet_core::proxy::requires_operator_under`,
  `ADR-048` §5.2) over its `proxy-auth` and `proxy-auth-paths`, and refuses a
  restore route the proxy would serve to anybody. `proxy-auth = "none"` with
  `proxy-auth-paths = ["operator:api/v1/restore"]` is legal; `proxy-auth-paths =
  ["none:api"]` with the restore under `api` is not.

**Refused at package time and at install time.** `cargo rivet package` refuses
each case in words in front of the `Cargo.toml` the author edited
(`cmd/cargo-rivet/src/crate_meta.rs`), and `Manifest::parse` refuses the same
cases by key name, so a manifest produced some other way cannot reach a machine
carrying one either.

### 12.3 The table carries it as a line of its own

```text
tempmon-rivet tempmon artifact sqlite api/v1/backup
tempmon-rivet tempmon restore api/v1/restore
```

A `restore` line has four fields and attaches to the `artifact` line with the
**same service and the same route**, whichever order the two lines are in. A
restore line for a feed, for a service with no artifact line, or a second
restore line for one service attaches to nothing, and the artifact line is kept
either way — a malformed restore does not cost the backup.

**It is not a sixth field on the artifact line**, because §5's reader drops a
line carrying a trailing field whole: every `cargo rivet` built before
`restore-path` would stop seeing that application's backup. An unknown shape
word drops only its own line.

The supervisor's journal line for the table carries `restores` beside
`declarations`.

### 12.4 `cargo rivet restore`

```text
cargo rivet restore --host HOST:PORT --identity DIR --pin HEX --service NAME --from FILE [confirm]
```

`--host`, `--identity` and `--pin` are `backup`'s (§8.1), and so is the
refusal of `--authority`, `--name` and `--port`. `--service` is required and
given **once** — a restore replaces one application's data. `--from` is the
file.

**Without `confirm` nothing is sent**: the verb reads the table, checks the
file, and prints the control-plane route, the application route the machine will
send it to, the `Content-Type`, and which rungs of the ladder ran.

**Refused, in this order, before a byte is sent:**

1. `--from` is not a readable regular file — before the machine is asked;
2. the machine's table does not name the application (the refusal names what it
   does offer), names a feed, or names an artifact with no restore route;
3. the file is over 512 MiB (§12.6);
4. the file fails §7's ladder against the application's DECLARED
   `backup-format` (§12.5);
5. the file sits in a directory named for a DIFFERENT application the machine
   offers (§12.5).

**With `confirm`** it sends the file as the body of `POST /restore/<service>`;
the machine relays it to the application as §12.7's request and streams the
answer back. The verb prints the application's status code, its
`Content-Type`, and up to 64 KiB of its body as text, and **exits non-zero on
anything but 2xx** — including a redirect, which is not followed. A `202` is
reported as success with a note that the application said it had not finished.
An answer carrying `Rivet-Answered-By` is reported as the machine's refusal,
never as the application's.

### 12.5 What "matches the declared format" means, and how strong each check is

§7's ladder, run on the file, and nothing more:

| rung | on a restore | strength |
| :-- | :-- | :-- |
| 1 non-empty | as §7 | refuses only a zero-byte file |
| 2 length | bytes read == the size the filesystem reported when the file was opened | **weak**: catches a file that changed while it was read. There is no server's `Content-Length` to disagree with |
| 3 `sqlite` magic | `SQLite format 3\0` | refuses a file that is not SQLite; a truncated SQLite file passes it |
| 4 `sqlite` length | page size × page count == length | **strong** against truncation and appended bytes. Skipped for a page count of 0 (SQLite before 3.7.0) |
| 3 `tar` magic | `ustar` at offset 257 | refuses a file that is not a POSIX/GNU tar |
| 4 `tar` end | a multiple of 512 bytes ending in two zero blocks | catches an archive cut anywhere but after its end marker; says nothing about a member's contents |
| 3 `json` opener | first non-space byte `{` or `[` | a decidable first token, not a parse |
| 4 `json` closer | the matching closer is the last non-space byte | **weak**: a document truncated inside itself almost never ends on its outer closer, but nothing here parses JSON, so `{ not json }` passes |
| `csv` | rungs 1 and 2 only | **none past non-empty**: any non-empty file passes, and the plan says the format makes no statement about its own size |

**No format records which application wrote it.** A SQLite header, a tar
archive, a JSON document and a CSV file say nothing about their producer, so
whether a SQLite file is this application's or another's is not a question the
bytes answer. The one sign there is, is where `cargo rivet backup` put the file:
`<into>/<service>/<file>`. A file whose parent directory is named for a
different application **this machine offers** is refused. That is a layout
check, and weak: a file copied anywhere else passes. It catches one application's
name typed beside the other's path, and it is claimed to catch nothing more.

**No schema and no version is checked**, for §7.3's reason. Whether a backup
from an older release of an application fits the running one is the
application's refusal to make (§12.7 item 4).

### 12.6 Size, and why the file is not streamed

**512 MiB**, `cmd/cargo-rivet/src/backup.rs`'s `MAX_BODY` — the bound `backup`
holds on what it fetches, held in the other direction. A larger file is refused
by its size alone and never read.

**The file is read whole, checked, and sent from that same buffer** with a
`Content-Length`. What reaches the application is byte for byte what the ladder
passed; a streamed send would check one read of the file and send another. The
machine relays the body in 64 KiB pieces as it arrives and never holds it; it
refuses a body with no `Content-Length`, a chunked one, an empty one and one over
`rivet-mcpd`'s bulk ceiling before dialling the application — a known length is
required, not preferred.

**Five minutes with nothing moving** ends the exchange on the workstation — the
same `TIMEOUT` `backup` has, now a bound on a silent connection rather than on
the whole exchange, so a large body that keeps arriving is not cut off. The
relay on the machine copies until the application answers or closes, as
`rivet-proxyd` does.

### 12.7 The application's side — the contract

**Normative.** An application that declares `restore-path` MUST do what follows,
and an agent implementing `restore-path` in an application works from this
section. `tools/qemu/restore-probe` is a fixture that follows it on the
transport; it is not a model for a real application's storage.

1. **The request it receives.** `cargo rivet restore`'s request arrives from
   `rivet-mcpd`'s relay (§8.1); one a browser sends arrives from `rivet-proxyd`.
   Both remove `/<proxy-path>` from the target, so the application sees one
   request either way, on the listening socket its manifest's first TCP
   `listen` declares:

   ```text
   POST /<restore-path>?confirm=true HTTP/1.1
   Content-Type: <the media type of backup-format>
   Content-Length: <n>
   Connection: close
   ```

   | `backup-format` | `Content-Type` sent |
   | :-- | :-- |
   | `sqlite` | `application/vnd.sqlite3` |
   | `csv` | `text/csv` |
   | `json` | `application/json` |
   | `tar` | `application/x-tar` |

   The body is **the raw bytes of one backup**, exactly as `GET /<backup-path>`
   served them and `cargo rivet backup` stored them: not multipart, not
   base64, not compressed. There is no `Transfer-Encoding` and no
   `Expect: 100-continue`. `confirm=true` is always present; the application
   MAY require it and MAY ignore it. `Content-Type` is a claim — the
   application decides on the body.
2. **Authentication.** The request arrives over TLS from `rivet-proxyd`, which
   has already admitted the operator (§12.2 refuses any other route), or from
   `rivet-mcpd`, which has already admitted the operator's pinned key (§8.1).
   Both present the machine's fleet certificate. The application requires a
   caller certificate chaining to `tls-root` —
   `rivet_sdk::serve::Tls::handed("tls-cert", "tls-key")
   .requiring_a_caller_certificate_from("tls-root")`, `ADR-046` §4.2 — and
   **MUST NOT require any further credential** (a bearer token, a cookie, a
   CSRF token) on this route: `cargo rivet restore` sends none, and which
   operator called is not forwarded (`ADR-046` §4.4).

   **A restore another site caused never arrives.** A browser presents the
   operator's certificate to any request to the machine, so a page on another
   site could otherwise `POST` a `text/plain` body to this route with no
   preflight. `rivet-proxyd` refuses a request other than `GET`, `HEAD` or
   `OPTIONS` whose `Sec-Fetch-Site` is not `same-origin` or `none`, or which
   carries no `Sec-Fetch-Site` and an `Origin` that is not the machine's own —
   before any application is reached (`ADR-046` §4.5, `.rulings/` R-186 D1).
   **The application relies on that guard and needs no `Content-Type`, `Origin`
   or token check of its own** to be safe from it. One it already has may stay;
   the contract does not depend on it. The application's own page, served
   through the proxy and posting to a relative URL, is `same-origin` and passes.
   **A request from the network reaches the route through the proxy or not at
   all**, because the
   routed port of an application with a `proxy-path` is bound on `127.0.0.1`
   (`ADR-046` §7.1, `.rulings/` R-188). The guard does not cover a port the
   application declares `:direct`, so a restore served there would carry none
   of this.
3. **Method.** Only `POST`. The application SHOULD answer another method on
   this route with `405`.
4. **Validate the whole body before changing anything.** At least what the
   application's own loader needs: that it is its format, whole, and a schema
   or version it will load. **On refusal its data MUST be unchanged**, and it
   answers `4xx` with a body saying why — `400` for a body it cannot accept,
   `409` for a conflict (a version it will not load, a missing confirmation it
   requires), `413` for a body over its own size bound.
5. **Replace, consistently, by itself.** The application quiesces whatever it
   needs to — its own writers, readers, open handles, ingestion — and makes the
   replacement **atomic from its own point of view**: afterwards it holds
   either all of the old data or all of the new, never a mix, including across a
   crash or a power loss partway through (write beside the live data, `fsync`,
   rename; or one transaction). **Rivet does not stop, restart or signal the
   service.** The application MUST NOT exit to reload before it has answered,
   and SHOULD reload in-process.
6. **Replace, not merge.** After a `2xx` the application holds what the backup
   held. Files the backup never contained — its TLS files in `RIVET_APP_DATA`,
   its settings — are left as they are.
7. **Answer once the replacement is durable**, synchronously: `200` with a body
   saying what happened (JSON recommended — for example, counts of what was
   discarded and what was loaded) or `204`. `202` is reported to the operator as
   *not finished* and SHOULD NOT be used. A failure partway is `5xx`, and
   **afterwards the application holds the old data or the new, each whole, never
   a mix** — item 5's property, across the failure as across a crash. Which of
   the two is the application's: undoing a replacement that has not reached its
   commit point leaves the old, and **finishing one that has** (rolling forward)
   leaves the new. **Until the new data is whole, it MUST refuse service** —
   `503` to every request that would read or change the data
   — rather than serve a mix, and it finishes the replacement no later than its
   next start. `cargo rivet restore` prints the status and up to 64 KiB of the
   body, and treats anything but 2xx, including `3xx`, as failure.
8. **Size and time.** The application SHOULD accept a body as large as its own
   backup can be, up to 512 MiB, and SHOULD answer within 300 seconds of the
   body's last byte — the client gives up after five minutes with nothing
   moving.
9. **Idempotent.** Sending the same backup twice SHOULD leave the same state.

### 12.8 Where the pieces are

`restore-path` is read by three places that each hold a list of manifest keys:
`rivet_core::package::application` (`KEYS`, `Manifest::parse`/`render`, the
builder), `cmd/cargo-rivet/src/crate_meta.rs` (`DECLARED`, the author's
refusals), and `cargo rivet --help`'s key list in `cmd/cargo-rivet/src/main.rs`,
which a test holds to name every key in `DECLARED`. `cmd/rivet-sysd/src/runtime.rs` carries
it from the installed manifest into the table untouched. The contract's words —
the method, the query, the media types — are `rivet_core::backup::RESTORE_METHOD`,
`RESTORE_QUERY` and `BackupFormat::media_type`.

### 12.9 What is proved, by what method, and what is not

**Proved by unit test, on the host:** `restore-path` round-trips through the
canonical text and renders last; it is refused without the artifact pair, beside
a feed, on an escaping path (the builder and the parser both), and on a route
the proxy would serve without an operator in three spellings, while a gated
restore under an open route is accepted; `cargo rivet package` refuses the same
cases in words; the table writes the `restore` line, attaches it in either order,
drops it when it cannot attach unambiguously while keeping the backup, and a
reader applying the pre-§12 rule to this build's output keeps the artifact line
with its five fields. `cargo rivet restore`, against a fake control plane and a
fake application: the plan opens no sender and sends nothing; each refusal in
§12.4, including a truncated SQLite that keeps its magic, a file over the bound
refused without being read, and a file in another application's directory; a
`200` with the exact bytes sent to the named service; every non-2xx (and a
`301`) failing with the application's words; `204`, `202`, a transport failure,
and the machine's own refusal reported as the machine's.

**Proved by unit test, on the container's Linux target:** `publish_backups`
carries the restore route from an installed manifest into the table, and only
where it was declared.

**Proved over real TLS, §8.1's relay** (`cmd/rivet-mcpd/src/backups_tests.rs`,
in the container): `rivet-mcpd`'s planner and relay against an in-process
application that requires a caller certificate from a freshly minted P-256
authority — the proxy's leaf is presented and the application is verified
against the proxy's `upstream-name` (a setting naming something else is a `502`,
never a fall-back to trusting loopback); a backup's answer comes back byte for
byte and a megabyte arrives in pieces no larger than 64 KiB; a restore's body
reaches `POST /<restore-path>?confirm=true` whole, with the media type and the
length; an application's `409` is relayed as its own; and every refusal the
machine makes before dialling.

**Proved over real pinned TLS, the workstation's half**
(`cmd/cargo-rivet/src/backup_tests.rs`, `rivet-remote/src/tls_tests.rs`, on the
host): `cargo rivet backup`'s `execute` against a stand-in control plane that
records the certificate chain of every connection — the table and the payload
each presented exactly the operator's control-plane certificate and nothing
else, which is the regression guard for R-234 — and `Tls::http` sending a body
whole and reading an answer as it arrives.

**Proved on a booted machine, under QEMU** — `tools/qemu/adr-051-restore-proof.sh`,
PASS over §8.1's control-plane transport. One aarch64 guest with the control
plane, `rivet-proxyd` on 443 and the restore probe installed twice; the
authority minted by `rivet-provision ca` and installed by `cargo rivet secrets`.
The supervisor's line read `"declarations":2,"restores":1`; the table read over
the control plane carried the probe's restore line and none for the other;
`cargo rivet backup` fetched 2048 bytes over the control plane and `rivet-mcpd`
journalled relaying it on loopback; the same fetch with `HOME` pointed at an
empty directory arrived byte-identical, so no certificate the tool could find
was needed; `--authority`, `--name` and `--port` were each refused by name; the
plan and three refusals behaved as §12.4 says; after the data changed a second
backup differed; the confirmed restore printed the probe's `200` and
`rivet-mcpd` journalled relaying it; the probe's pid was the same before and
after; and a third backup was byte-identical to the first.

**Proved over real TLS in-process, and on the QEMU guest above, for the
cross-site guard (§12.7 item 2):** `ADR-046` §9.1 has the method. In the QEMU
run, `curl` with the operator certificate sent the first backup to the probe's
restore route twice — once with `Sec-Fetch-Site: cross-site`, once with only an
`Origin` from another site — and got `403` from the proxy both times; a backup
taken afterwards was still the changed data; the refusal was in the journal; a
`same-origin` `POST` carrying the machine's own `Origin` reached the probe; and
the confirmed `cargo rivet restore` that follows succeeded. The headers were set
by `curl`. **No browser was driven.**

**The three real applications, against item 5 and item 7.** Each declares
`restore-path`, and each one's own record states how it meets *old or new, each
whole, never a mix*; none of that was re-run here:

| application | how the replacement is made whole | a failure partway leaves |
| :-- | :-- | :-- |
| TempMon | one SQLite transaction in WAL mode, `synchronous = FULL` (`crates/tempmon-store/src/lib.rs`) | the old data before the commit record, the new after it |
| a third adopter | a temporary file, `fsync`, `rename(2)` over the journal, `fsync` of the directory, in its own repository | the old journal or the new |
| Reliquary | a commit marker, then per-blob renames rolled forward from it; the store serves nothing while the shelf is mixed (`crates/reliquary-store/src/restore.rs`) | the old shelf before the marker, the new after it, with requests refused until the renames finish |

**NOT proved here: any real application.** The table is each application's own
account, at the file named, and whatever proves it lives in that application's
repository. None has restored its data through `rivet-proxyd` under QEMU or on a
board.

**NOT proved: a board.** Nothing here ran on a Raspberry Pi or over a real
network from a Windows workstation, and no image carrying this supervisor has
been built.

**NOT proved: a large body through the proxy.** The QEMU body is 2048 bytes; the
512 MiB bound is proven only as a refusal, and `rivet-proxyd` forwarding a body
of hundreds of megabytes within five minutes is unmeasured.

---

## History

### 2026-10-03 — §8.1 had two transports; the payload moved onto the control plane

**§8.1 read:** ~~*"Two transports, and neither can do the other's job"* — the
table over the control plane, and the payload over HTTPS on 443 through
`rivet-proxyd`, presenting the operator's certificate (`browser.crt`,
`browser.key`, `root.crt` from `~/.rivet/ca`), with `--authority`, `--name` and
`--port` to find it.~~ That made backup the one tool still holding the operator's
browser certificate once passkeys could sign a browser in, so `ADR-063` §4.3
asked how tools leave it. Offered an operator-key-signed token, presenting the
operator key at the proxy, or carrying backups over the control plane, the
maintainer chose the third (`.rulings/` R-234): the payload rides the bulk
endpoint's listener, the machine reaches the application on loopback as the
proxy does, and nothing new is minted. §12.4, §12.6 and §12.7 items 1, 2 and 8
moved with it, and the HTTPS round trip that proved the old sender
(`restore_tests.rs`'s `over_https`) went with `ureq`.

### 2026-09-14 — §12.7 item 7 said a failure partway leaves the old data; the guard moved to the proxy

**Item 7 read:** *"A failure partway is `5xx`, and item 5's atomicity still
holds: the data is the old data."* Building Reliquary's restore found that its
commit point is a marker rename followed by per-blob renames, and that a medium
error during those renames can only be finished forward — rolling back would
mean keeping every discarded blob until the end, doubling a restore's disk, with
a rollback that can fail the same way. The property the item exists to protect
is *never a mix*, and Reliquary keeps it by refusing service until the shelf is
whole. `.rulings/` R-186 D2 reworded the item to *old or new, each whole, never a
mix*, and Reliquary's roll-forward stands.

**Item 2 asked nothing about another site.** The same build found that a page on
another site could make a browser holding the operator's certificate `POST` a
safelisted body to any application's restore route with no preflight, and that
`rivet-proxyd` checked neither `Origin` nor `Sec-Fetch-Site`. Reliquary guarded
itself by requiring `Content-Type: application/x-tar`; TempMon did not. A
per-application `Content-Type` rule in the contract was the alternative offered
and not chosen: `.rulings/` R-186 D1 put the guard in the proxy, for every
application, and item 2 names it.

### 2026-09-14 — `restore-path` and `cargo rivet restore` added (§12)

The contract stopped at the fetch. An application could offer its data and
`cargo rivet backup` could store it, and nothing sent a stored backup back:
`skills/taking-and-restoring-a-backup` named that as a gap, and §2's decision
read *"three keys, two shapes, one ladder, and one verb."* `.rulings/` R-183 D1
chose an application-mediated restore over a Rivet-mediated one (stop the
service, write the file, start it), because §9's first row holds in that
direction too, and §12 builds it.

### 2026-09-09 — `--service NAME` added (§8.5)

The verb fetched a machine whole, with no way to select applications. Fleet
Monitor's `exempt` setting could not mean what its own configuration file
already said it meant, as a direct consequence: an excluded application's
bytes still crossed the network and were discarded only after arriving,
because this verb had nothing to tell it not to fetch them. `--service` is
the fix, named in Fleet Monitor's own record as belonging here rather than in
a workaround on its side. Nothing in §8.2 changed — the flag selects WHICH
applications a run considers, never how often a run happens or how many
copies survive it.
