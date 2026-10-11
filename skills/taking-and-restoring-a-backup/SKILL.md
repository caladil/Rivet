---
name: taking-and-restoring-a-backup
description: Back up and restore what Rivet actually protects — an application's own data (cargo rivet backup/restore, ADR-051), the fleet certificate authority (rivet-provision ca backup/restore, SPEC-05 §4.0), the two signing seeds (cargo rivet key backup/restore, passphrase-encrypted), the fleet repository — its text half (git) and its object store (cargo rivet fleet objects backup/restore, ADR-052 §7.3) — and a board's own /state/secrets (cargo rivet backup-secrets/restore-secrets, passphrase-encrypted, same board only, ADR-030). Use when asked to back up, protect, snapshot, or restore a Rivet workstation, fleet, board or machine's data or secrets, or before a reset or re-provisioning.
---

# Taking and restoring a backup

**"Backup" is five unrelated things here, each with its own tool and its own
answer to "is there a restore."** There is no single command that covers all
of it, and no default destination anywhere in any of them — every command
below takes a directory the operator names. `<backup-dir>` in every example is
a placeholder; there is no location this skill or any tool suggests.

## When to use it, and when not to

Use this before a reset or authority re-mint (the authority backup is step 4.0
of `SPEC-05`'s twelve, and it runs **before** step 1 destroys the only copy),
before protecting a service's data ahead of an update, or when asked to
protect or back up a fleet, a board, or the workstation.

**Not for:** retention, cadence or scheduling — `ADR-051` rules those out of
`cargo rivet backup` by name, deliberately, as a different program's job; a
card image or provisioning (that is `cargo rivet provision`/`write-image`);
or the whole of `/state` — `ADR-030` §3 leaves that to the operator on
purpose, scoped down to what cannot be re-derived from anywhere else.

## What each kind covers, and what restores it

| Kind | Tool | Covers | Restore |
| :-- | :-- | :-- | :-- |
| An application's data | `cargo rivet backup`/`restore` | one service's declared backup surface — a SQLite/CSV/JSON/tar artifact, fetched over HTTPS | `cargo rivet restore … --service NAME --from FILE`, **only to an application whose manifest declares `restore-path`** — the application restores itself |
| The fleet authority (CA) | `rivet-provision ca backup`/`restore` | `~/.rivet/ca`: `root.crt`, `root.key`, `browser.crt`/`.key`/`.pfx`, `suffix`, one `server-<name>` pair per fleet server | `rivet-provision ca restore <backup-dir>` |
| The two signing seeds | `cargo rivet key backup`/`restore` | `~/.rivet/keys/rivet-system.seed`, `rivet-app.seed` — whichever actually exist, **passphrase-encrypted at rest** | `cargo rivet key restore <backup-dir> [--replace]` |
| The fleet repository's text | git | machine documents, register, artifact manifest | git, ordinary clone/pull |
| The fleet repository's object store | `cargo rivet fleet objects backup`/`restore` | `<fleet>/objects/<digest>` — referenced digests by default, `--all` for every digest ever published | `cargo rivet fleet objects restore <backup-dir> [--replace]` |
| A board's own secrets | `cargo rivet backup-secrets`/`restore-secrets` | every file in the board's `/state/secrets` — `device.seed`, `device.crt`, `peers`, stored values such as `<interface>.psk` — **passphrase-encrypted at rest**, one file | `cargo rivet restore-secrets … --from <backup-dir> [--replace]`, **onto the same board only** |

## 1. An application's data — `cargo rivet backup`

Reads the table an installed application's manifest publishes
(`backup-path` + `backup-format`, or the deferred `backup-feed` shape),
fetches each artifact over HTTPS through `rivet-proxyd` presenting your
operator certificate, checks it, and writes it under `--into`.

**The operator certificate is `browser.crt`, and it names no host.** The proxy
takes a fleet certificate carrying a `subjectAltName` for a machine's, and
answers `403` naming that host (`ADR-046` §4.1). A `server-<name>.crt` beside it
in `~/.rivet/ca` is a machine's leaf: presented by hand with `curl --cert`, it is
refused, and so is any certificate copied off a board's `tls-cert`.

```sh
cargo rivet backup --host <address>:7777 --identity <dir> --pin <hex> --into <backup-dir>
```

**Without `confirm` nothing is fetched.** This prints the plan: how many
applications offer a backup, where each would be fetched from, and what
`--service NAME` (repeatable) excluded, if it was given. Add `confirm` to
actually fetch and write:

```sh
cargo rivet backup --host <address>:7777 --identity <dir> --pin <hex> --into <backup-dir> confirm
```

**Observable proof:** a line per stored file — bytes received, the file path,
and which rungs of the validation ladder ran (non-empty, byte count matches
`Content-Length`, the format's magic, the format's own statement about its
size). A format with no fourth rung (CSV, for instance) says so — nothing
proves that file is whole beyond the byte count the server declared. The run
ends with a count: `N stored, N deferred, N failed`.

**Refusals, each observable:** `--service NAME` naming an application the
machine does not offer is an error naming both sides, never a silent empty
fetch; a file already at the destination stops the whole fetch for that
service rather than being overwritten (there is no `--replace` here —
retention is deliberately not this verb's job, so move the old file or
pick a different `--into`); an application declaring `backup-feed` is
listed and **not fetched** — this build implements the artifact half only.

**Never folder-copy a live database.** This verb never touches a service's
files directly — it fetches bytes the *application itself* served over its
own declared route, so taking a consistent snapshot of a live SQLite database
is that application's job, not this tool's. The SQLite rung only proves the
received bytes are a **whole, well-formed** file (page size × page count
equals the length) — it says nothing about whether the application's own
snapshot was itself consistent. If a service has no declared backup surface,
there is no sanctioned way to back up its data at all; copying its file off
`/state` by hand is outside anything this project's tools check.

### Restoring it — `cargo rivet restore`

`ADR-051` §12, `.rulings/` R-183 D1. **The application restores its own data**:
it declares a `restore-path` beside `backup-path` in its signed manifest, and
this verb checks the file and sends it there. Rivet stops nothing, restarts
nothing, and writes nothing into the application's data directory.

```sh
cargo rivet restore --host <address>:7777 --identity <dir> --pin <hex> --service <name> --from <backup-dir>/<name>/<file>
cargo rivet restore --host <address>:7777 --identity <dir> --pin <hex> --service <name> --from <backup-dir>/<name>/<file> confirm
```

`--authority`, `--name` and `--port` mean what they mean to `backup`.
`--service` is required, once.

**Without `confirm` nothing is sent**: the plan reads the machine's table,
checks the file, and prints the `POST` route, the `Content-Type`, and which
rungs of the format ladder ran.

**Refused before anything is sent:** a `--from` that is not a file; an
application the machine does not offer (naming what it does); **one that
declares no `restore-path`**, or a feed; a file over 512 MiB; a file that fails
the application's DECLARED `backup-format` — including a SQLite file cut short
that still starts with the right magic; and a file inside a directory named for
a *different* application the machine offers, which is where `backup` put that
one's. That last is a layout check and weak — no backup format records which
application wrote it, so a file copied anywhere else passes. **CSV has only the
first two rungs**: any non-empty CSV passes the format check.

**Observable proof:** with `confirm`, the application's own answer — status
code, `Content-Type`, and its body — printed as it gave it. **Anything but 2xx
is a failure** and the command exits non-zero. A `202` is reported as not
finished.

**TempMon and Reliquary each declare `restore-path`** (R-183 D2). An
installed build older than that refuses with *offers … NO RESTORE*; that refusal
is correct, not a defect. **Do not work around it** by copying a file into
`/state/app/<name>/data` over the control plane: that is the live-database
hazard above, pointed at a restore.

**A restore that fails partway leaves the old data or the new, each whole** —
`ADR-051` §12.7 item 7, R-186 D2. A `5xx` does not tell you which; take a fresh
`cargo rivet backup` to see what the application now holds, and expect a `503`
from an application still finishing a replacement it had committed to.

**A `403` naming "another site" is `rivet-proxyd`**, not the application: a
request other than `GET`/`HEAD`/`OPTIONS` that a browser reports another site
caused (`Sec-Fetch-Site`, or an `Origin` that is not the machine's) is refused
before any application sees it (`ADR-046` §4.5, R-186 D1). `cargo rivet restore`
sends neither header and is never refused by it; an application's own page
posting to its own route is `same-origin` and passes.

A board's *own* secrets are a different thing from `cargo rivet backup` —
see §5.

## 2. The fleet authority (CA) — `rivet-provision ca backup` / `ca restore`

Copies whatever is actually in `~/.rivet/ca` — never a hard-coded file list —
so it grows with the fleet without needing updating here.

```sh
rivet-provision ca backup <backup-dir>
```

**Observable:** every file copied, named, with its size. Refuses to overwrite
a backup already at `<backup-dir>` — naming it and its age — unless
`--replace` is given.

```sh
rivet-provision ca restore <backup-dir> [--replace] [--no-install]
```

Copies the files back into `~/.rivet/ca` and then reinstalls `root.crt` into
the Windows Root store and `browser.pfx` into Personal — **unless
`--no-install`**, which writes the files only and touches no store; use it
for every rehearsal. Refuses to restore over an authority already at
`~/.rivet/ca` unless `--replace` is given.

**`rivet-reset` looks for a backup before it destroys anything** (`SPEC-05`
§2.6):

```sh
rivet-reset --backup <backup-dir> --dry-run
```

`root.crt` is compared **byte for byte** against `<backup-dir>` and one level
of its subdirectories — never by name, never by opening a key — so an exact
match is what a real backup produces and nothing else does. If none is found
and you are not running `--dry-run`, `--proceed-without-backup` is required to
proceed at all; the report names what it found either way, before anything is
confirmed.

**Proven:** the file copy itself round-trips every byte
(`cmd/rivet-provision/src/backup.rs`'s own tests back up a throwaway
authority, delete it the way `rivet-reset` does, restore it, and check every
file), and the matcher is rehearsed against a scratch `~/.rivet` by
`tools/provision/rehearse-reset.sh`. **Not proven:** `ca restore`'s
certificate-store reinstall half. `SPEC-05` §2.5 says removal from the
maintainer's real Windows store is unverified because there is no sandboxed
store that would prove anything about the real one — the same is true of
installing back into it, and neither should be reported as working until run
for real.

## 3. The two signing seeds — `cargo rivet key backup` / `key restore`

`~/.rivet/keys/rivet-system.seed` and `rivet-app.seed` are kept apart on
purpose (`ADR-028`) because they fail separately — one signs system updates,
the other signs applications. `.rulings/` R-183 D3 rules this backup
**passphrase-encrypted**: a stolen seed backup is not recoverable the way a
stolen CA backup is (`ADR-028` §5.5.4 — revoking a compromised signing key is
an image update shipped to every board), so the blast radius does not tolerate
the plaintext convention `ca backup` uses.

```sh
cargo rivet key backup <backup-dir>
```

Encrypts whatever of the two seeds actually exists (never a hard-coded pair —
a workstation holding only one is backed up correctly) and writes one
`<name>.seed.enc` file per seed into `<backup-dir>`. Asks for a passphrase
**twice, with no echo, from a real terminal** — there is no `--passphrase`
flag and no environment variable, and none should be added without a ruling: a
high-value secret does not belong in a process's argv or its environment
block. Refuses to overwrite a backup already at `<backup-dir>` — naming it and
its age — unless `--replace` is given.

```text
  encrypted 1 seed(s) to <backup-dir>
      rivet-system.seed.enc            140 bytes

  1 file(s) written, each encrypted under the passphrase you just entered.
  A FORGOTTEN PASSPHRASE MEANS THIS BACKUP CANNOT BE RESTORED. There is no recovery
  route around it.
```

```sh
cargo rivet key restore <backup-dir> [--replace]
```

Asks for the passphrase once and decrypts each `<name>.seed.enc` back to
`~/.rivet/keys/<name>`. **Refuses a wrong passphrase, or a file altered since
it was written, by the identical sentence** — an AEAD's whole security
property is that authentication failure does not distinguish the two, and
telling them apart would leak which one happened — and writes nothing either
way. Refuses to overwrite an existing seed at the destination unless
`--replace` is given, the same reason `cargo rivet key new` already refuses:
there is no way to recover a seed that is lost, since every board that trusts
its public half would need to be re-trusted.

**The envelope is versioned and self-describing** — algorithm id, Argon2
parameters, salt and nonce are all in the file, ahead of the ciphertext, and
are themselves authenticated (an AEAD's associated data) so a hand-edited
parameter is refused exactly as a wrong passphrase is rather than silently
deriving a different key. See `cmd/cargo-rivet/src/envelope.rs`'s module
documentation for the byte layout, which `backup-secrets` (§5) shares.

**Deliberately not covered:** a seed kept at a path named only by
`$RIVET_APP_SEED`/`$RIVET_SYSTEM_SEED` rather than the conventional
`~/.rivet/keys` — this backs up the fixed location, matching `ca backup`'s own
precedent, not wherever an override might point.

## 4. The fleet repository — git text, and `cargo rivet fleet objects backup`/`restore`

`ADR-052`'s fleet repository (`~/.rivet/fleet` by default, or wherever
`cargo rivet fleet init --at DIR` put one) splits in two: machine documents,
the register and the artifact manifest are an ordinary **git** repository;
the `.rvt` and image bytes those documents name by digest live beside it
under `objects/<digest>`, which the repository's own `.gitignore` (one line,
`/objects/`) keeps out of every commit — `ADR-052` §4 makes that a checked
refusal, not a convention, because a signing seed committed to a pushed
history cannot be unpublished.

**Backing up the text half is ordinary git** — clone the repository, or add a
remote and push it, exactly as any other git repository. No Rivet-specific
tool wraps this because none is needed.

**The object store — `ADR-052` §7.3, `.rulings/` R-183 D4:**

```sh
cargo rivet fleet objects backup <backup-dir> [--all] [--replace] [--at DIR]
```

Copies bytes out of `objects/` into `<backup-dir>/objects/`. **Default scope
is only the digests `manifest.toml` currently references** — `ADR-052` §6.1's
own sizing rule (*"only the artifacts its own documents name"*) applied to a
backup: what would let this fleet recover as it stands today, not gigabytes of
superseded `-dirty-` builds nobody will install again. `--all` copies every
digest `objects/` holds regardless of whether anything still references it.
Refuses to overwrite an object already at `<backup-dir>` — naming it and its
age — unless `--replace` is given. An empty scope (nothing published yet, or
`--all` against an empty store) is reported as `0 objects`, not refused — the
same posture `fleet list` already takes for an empty manifest.

```text
  backed up 1 object(s) from <fleet>\objects to <backup-dir>\objects (referenced, 8490 bytes total)
      f0d23b87a37d343c47fcab5f4a316be6c5ebb640e2157310c8fbe6f602d93dd1           8490 bytes
```

```sh
cargo rivet fleet objects restore <backup-dir> [--replace] [--at DIR]
```

The inverse: **recomputes every backed-up object's own BLAKE3 and refuses —
naming every mismatch, nothing written — if any filename does not match the
digest of its own bytes**, the same door check the object store itself makes
(`ADR-052` §7.2.3), run here on the workstation rather than trusted from the
backup blindly. Refuses to overwrite a conflicting object already in this
repository unless `--replace` is given.

```text
cargo-rivet: 2 object(s) in <backup-dir>\objects do not match their own filename's
digest — refused, and NOTHING WAS RESTORED:
  f0d23b87a37d343c47fcab5f4a316be6c5ebb640e2157310c8fbe6f602d93dd1 — actual content digest is 2ba8...
```

**Machine-independent by construction** — an object's digest is the whole of
what identifies it, so a backup made on one workstation restores cleanly onto
a fleet repository initialised on another; `ADR-030` §5's same-board rule is
about a *board's* secrets and does not apply here.

## 5. A board's own secrets — `cargo rivet backup-secrets` / `restore-secrets`

`/state/secrets` on a board holds its device identity (`device.seed`,
`device.crt`), the control plane's `peers`, and stored values such as a
wireless `<interface>.psk`. `ADR-030`, `.rulings/` R-183 D5 and R-185: pulled
to the workstation over the control plane, **sealed under a passphrase typed at
a terminal**, and restored onto **the same board and no other**. No admin
server, no schedule.

```sh
cargo rivet backup-secrets --host <address>:7777 --identity <dir> --pin <hex> --into <backup-dir>
cargo rivet backup-secrets --host <address>:7777 --identity <dir> --pin <hex> --into <backup-dir> confirm
```

**Without `confirm` no secret is read and no passphrase is asked for** — the
plan lists names and sizes and reads only the public `device.crt`. With
`confirm` it asks for a passphrase **twice, with no echo, from a real
terminal**, then fetches every file and writes ONE file,
`<backup-dir>/secrets-backup.enc`: the same envelope `cargo rivet key backup`
uses, holding a manifest (the board's device public key, every file's size and
BLAKE3) and every file's bytes. Nothing about the secrets is readable on disk —
not the file names, not their sizes, not the board key.

**There is no `--passphrase` flag and no environment variable**, and none
should be added: a passphrase in argv or an environment block is readable by
anything else running as the same user. An agent cannot run the confirmed
backup or any restore without a terminal the operator types into — that is the
design, not a gap to script around. Hand the operator the command.

**A FORGOTTEN PASSPHRASE LOSES THE BACKUP.** There is no recovery route. The
tool says so before it asks and after it writes; say it to the operator too.

```text
backed up 4 file(s) from the board whose key is
  <board key>
into <backup-dir>/secrets-backup.enc (<n> bytes)

  device.crt                  <n> bytes
  device.seed                  32 bytes
  peers                        <n> bytes
  wlan0.psk                    <n> bytes

ENCRYPTED under the passphrase you just typed: the file names, sizes and the
board's key are inside it too. A FORGOTTEN PASSPHRASE MEANS THIS BACKUP CANNOT BE RESTORED. There is no recovery route around it.
```

**Refusals on backup:** a `<backup-dir>` that already holds a backup (move it or
name another); no terminal, or two entries that differ — before any secret is
fetched; an entry in the store that is not a regular file, or too large for the
restore to carry; a `device.crt` or `device.seed` on the board that does not
match `--pin`.

```sh
cargo rivet restore-secrets --host <address>:7777 --identity <dir> --pin <hex> --from <backup-dir>
cargo rivet restore-secrets --host <address>:7777 --identity <dir> --pin <hex> --from <backup-dir> --replace confirm
```

**Both need the passphrase, typed once** — the plan too, because it compares
the backup's bytes with the board's. **Without `confirm` nothing is written**:
the plan names each file `write`, `REPLACE`, or `identical, skipped`, and each
file on the board that the backup does not name as `left alone`.

**Refused before anything is sent:**

- **a wrong passphrase, or a backup altered since it was written** — the same
  refusal for both, on purpose (an AEAD does not tell them apart, and must not);
- **a different board** — the backup's recorded key is not the key `--pin`
  names. A board is the same board only if its device identity key matches
  (R-185 D2): a board re-carded with a new identity is a different board, and
  gets freshly provisioned secrets. There is no override: another board's seed
  would make two boards one identity;
- **a backup whose contents disagree with its own manifest** — a size or BLAKE3
  mismatch, or `device.crt`/`device.seed` naming another key than the manifest
  records; the whole backup is refused;
- **a `peers` file that does not name the key your `--identity` connects with**
  — after the board's next boot it would admit nobody who could fix it. The
  board refuses such a `peers` itself too (`apply restore-secret`);
- **a file whose bytes differ on the board**, unless `--replace`; without it
  nothing at all is written;
- **a backup directory an earlier build wrote unencrypted** (`secrets-backup.txt`
  beside `files/`) — refused by name; take a new backup, and have the operator
  remove the old directory, which holds that board's secrets in the clear.

**Observable proof:** one line per file — `written` or `REPLACED` and `read
back identical`, or `identical on the board — not written`, or `left alone`.

**Nothing is restarted.** `rivet-mcpd` and `rivet-netd` read these files when
they start, and `apply service-stop` refuses both (stopping the control plane
would end the session). A restored file is live at the board's **next boot**.
Do not tell the operator the board is "restored" in a sense that includes its
running services — that needs a reboot, and only the reboot proves the board
accepts the files.

**Proven:** unit tests against a fake control plane
(`cmd/cargo-rivet/src/machine_secrets_tests.rs`, `cmd/cargo-rivet/src/envelope.rs`)
and the board subject's (`rivet-core/src/operation/restore_secret_tests.rs`);
the typed prompt under a Windows ConPTY against a TLS server standing in for a
board (`cmd/cargo-rivet/tests/pty_secrets_backup.rs`); and under QEMU, with the
passphrase typed into a `script(1)` pty, by `tools/qemu/adr-030-secrets-proof.sh`.
**Not proven:** anything on a real board, a reboot after a restore, and the
prompt in a person's own terminal.

## 6. Recovering a whole host — `cargo rivet recover`

`ADR-063` §7, `.rulings/` R-235. It composes the restores above into one
recovery of a named host, from what is already taken: its recorded document and
artifacts (by digest) from the fleet repository, its `backup-secrets` directory,
and a folder of its applications' backups — `cargo rivet backup` output, or
Rivet Monitor's backup folder (`<folder>/<host>/<application>/`, newest file).

```sh
cargo rivet recover <host> --from <backup-dir> --secrets <secrets-dir> [--at <fleet>]
```

**Without `confirm` it plans, reads files only and reaches no machine.** It
refuses a host the fleet repository does not record, a host with no pin (in
the repository's register or given with `--pin`) or a `--pin` the register
disagrees with, an artifact the document installs that the repository lacks or
holds damaged, a secrets directory with no sealed backup, and data for an
application the document does not install.

**Two invocations.** With `confirm` alone it writes the card through
`provision --existing`, after checking the secrets backup is this host's — **a
person's step: it refuses without a terminal, and an agent never runs it.**
Once the machine has booted, `--after-boot confirm` runs, in order:
`restore-secrets` (the passphrase typed, so a terminal again), `install
--artifact` for each application and one `apply reload`, then each
application's `restore`. It stops at the first refusal, and running it again
skips what is already done.

**It asks before writing over data, every time.** Every application is asked
what it holds before any is sent anything. The backup's own bytes are skipped
and no data is restored; anything else — a fresh install's empty database
included, and an application that does not answer — is refused, nothing sent to
any application, unless `--replace <application>` names it. Take a fresh
`cargo rivet backup` first if what it holds might matter.

**Not in any backup: per-machine material** (an application's TLS pair from the
fleet's authority). Stage it with `cargo rivet secrets` and run `--after-boot`
again; an application declaring material is refused at step 3 until then, one
that needs it undeclared at step 4.

**Proven:** unit tests of the plan's refusals, the order of the steps and the
overwrite refusal (`cmd/cargo-rivet/src/recover_tests.rs`), and the secrets
binding (`machine_secrets_tests.rs`); and under QEMU,
`tools/qemu/adr-063-recover-proof.sh` — a guest backed up, destroyed, and
recovered by `--after-boot` with the same pin, the same artifact and
byte-identical data. **Not proven:** the card step (never run by an agent), any
of it on a real board, and a reboot after the recovery.

## Safety rules

- **Every destination is a directory the operator names, always.** No command
  above has a default, and `SPEC-05` §4.0 rules that out on purpose: a tool
  that invents a destination for secret keys is one whose output lands
  somewhere the operator did not choose.
- **Seeds, `root.key` and everything a `backup-secrets` directory holds are
  secrets.** Never print one, never paste one into chat, never commit one.
- **Nothing here overwrites silently.** `cargo rivet backup` and `cargo rivet
  backup-secrets` refuse a destination already holding a backup;
  `rivet-provision ca backup`/`restore`, `cargo rivet key backup`/`restore`,
  `cargo rivet fleet objects backup`/`restore` and `cargo rivet
  restore-secrets` all refuse to replace what is there unless `--replace`;
  `rivet-reset` requires `--proceed-without-backup` (or `--dry-run`, which
  removes nothing regardless) when it finds no backup to check against.
- **A confirmed `cargo rivet restore` replaces an application's data.** Run
  the plan first, and take a fresh `cargo rivet backup` of that application
  before confirming, so the data being replaced has a copy.
- **A board's secrets go back onto that board and no other** (`ADR-030` §5).
  The store is plain text on the board, so on another board they would *work* —
  and its device seed would make two boards one identity. **This does not apply
  to the two signing seeds or the fleet repository's object store** — a seed
  backup restores under its own passphrase on any workstation, and an object's
  BLAKE3 digest is what identifies it wherever it is copied.
- **A forgotten passphrase is unrecoverable, by design** — for a seed backup
  and for a board's secrets backup alike. `cargo rivet key backup` and `cargo
  rivet backup-secrets` say so in their own output; there is no recovery route,
  flag, or support path around it.
- **Never type, choose or store the passphrase for the operator**, and never
  look for a way to feed one to either command without a terminal. A passphrase
  reaches either command through its prompt, on purpose.

## Verifying a backup is usable

Only by what the tools themselves offer — there is no separate verification
step for any of this:

- `cargo rivet backup`'s own report is the proof: bytes received, which rungs
  of the format ladder ran, and an explicit note when a format "makes no
  statement about its own size" so nothing proves that file is whole beyond
  the declared byte count.
- For the authority, `rivet-reset --backup <backup-dir> --dry-run` is the
  built comparison: an exact, byte-for-byte match on `root.crt` or a named
  mismatch.
- For the two signing seeds, `cargo rivet key restore` IS the test: a wrong
  passphrase or a tampered file is refused before anything is written, so a
  restore that succeeds has already proven the backup opens under the
  passphrase that made it.
- For the fleet repository's object store, `cargo rivet fleet objects
  restore` recomputes and checks every digest before writing anything —
  the identical proof.
- For a board's secrets, the unconfirmed `cargo rivet restore-secrets`
  against the same board opens the backup under its passphrase, checks every
  size and digest and every identity file, and says per file whether the board
  already holds those bytes — a plan of all `identical, skipped` is a backup
  that opens and matches the board now.
- For an application's data, the unconfirmed `cargo rivet restore` checks the
  file against the application's declared format — which proves the file is
  whole where the format can say so, and nothing about whether the application
  will accept it. Only the application's own answer to a confirmed restore
  proves that, and a confirmed restore replaces its data.

## Gaps, stated rather than papered over

- An application's data restores only into an application that declares
  `restore-path`. TempMon and Reliquary do, and none has restored its
  data through the proxy on a machine. Rivet's side is proven by unit test, a
  real-TLS round trip on the host, and `tools/qemu/adr-051-restore-proof.sh`
  against a fixture application — not against a real application's data, and
  not on a board.
- The proxy's cross-site refusal is proven over real TLS in-process and, with
  `curl` carrying a browser's headers, under QEMU. No browser has been driven
  against it, and it has not run on a board.
- A board that has been re-carded with a new identity cannot take its old
  secrets back. That is ruled (R-185 D2), not a defect to work around: it gets
  freshly provisioned secrets.
- Nothing has restored a board's secrets on a real board, or rebooted one
  after a restore.
- `rivet-provision ca restore`'s certificate-store reinstall is unverified
  against a real Windows store, the same way `rivet-reset`'s removal is
  (`SPEC-05` §2.5) — **not closeable by any agent**, by that section's own
  words: there is no sandboxed store that would prove anything about the real
  one, and an agent must not write to it.

See `docs/COMMANDS.md`'s `cargo-rivet` and `rivet-provision` entries for every
flag each command prints today, `ADR-030`, `ADR-051` and `ADR-052` §4.1 for
why each boundary sits where it does, and `SPEC-05` §2.2, §2.6 and §4.0 for
the authority's own record.
