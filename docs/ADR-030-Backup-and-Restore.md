# ADR-030: Backup and Restore

* **Status:** Accepted — scope ruled 2026-08-24; destination and restore ruled
  2026-09-14 (`.rulings/` R-183 D5); encryption and board identity ruled
  2026-09-14 (R-185). **Built 2026-09-14:** `cargo rivet backup-secrets` pulls
  a board's secret store to a workstation and seals it under a typed
  passphrase, and `cargo rivet restore-secrets` opens it and puts it back onto
  the same board — the one whose device identity key matches — and refuses any
  other. Partially supersedes `ADR-020` §9's *"backups are the operator's
  problem"*.
* **Date:** 2026-08-24
* **Target Audience:** AI Agent Engineering System & Human Maintainers

---

## 1. Context

**`/state` holds everything the machine itself keeps**: the provisioning
document it was configured with (`ADR-004` §6 step 2), the secret store
(`ADR-020` §5), the device identity seed (`ADR-020` §9.2), the journal
(`ADR-017` §7), and system extensions (`ADR-018` §6). Volumes (`ADR-031`) are
writable too, and hold applications and their data; §2 says why they are not
this ADR's.

**An A/B rollback cannot repair `/state`.** `ADR-018` §8 already ruled that a
failed `/state` degrades a boot rather than rolling it back, precisely because
rolling back would alternate between two healthy images for ever — an update
never touches `/state`, so it cannot have broken it and cannot fix it. **The
mechanism that protects `/system` therefore protects nothing in `/state`**, and
that asymmetry is the whole reason this ADR exists.

Until 2026-08-24 the record's only position was `ADR-020` §9: *"Backups are the
operator's problem."* That was defensible while a Rivet machine had no way to
send a file anywhere, and it stopped being defensible once `PLAN.md` §4.26 ruled
how files move.

---

## 2. The decision

**Rivet backs up the secret store, and it does not back up `/state`.**

| | In scope | Out of scope |
| :-- | :-- | :-- |
| Secret store, `/state/secrets` (`ADR-020` §5) | **yes** — `cargo rivet backup-secrets`, every file, sealed under a typed passphrase | |
| Device identity seed (`ADR-020` §9.2) | **yes** — it is `device.seed` in that store; see §5 | |
| Configuration document | covered elsewhere: `cargo rivet provision` commits every document it composes to the fleet repository (`ADR-052` §9 step 2). An edit made on the board afterwards is not in that copy | |
| Journal (`ADR-017` §7) | | no — it is a record of one machine's life, not its identity |
| System extensions (`ADR-018` §6) | | no — they are signed artifacts that can be fetched again (`ADR-029` §11) |
| Application data | | no — **it is not on the thing this ADR backs up.** An application and its data live together on a volume (`ADR-031`), and **`ADR-051` covers it instead** — see §2.2 |
| Whole `/state` | | **no — remains the operator's**, `ADR-020` §9 |

**The destination is a directory on the operator's workstation**, named every
time, pulled over the control plane (`ADR-035`). §4 says why.

**The backup is encrypted with a passphrase, although the store on the board is
not** (R-185 D1). `ADR-020` §11.2.3 rules secrets plain text on a Pi (hex for
values), and §11.2.5 records that §6's envelope encryption and §7's board-bound
key were not built — so the board's store is protected by being on the board.
A copy off the board is not protected that way, and it holds the device seed: a
stolen copy lets its holder present as that board until the board's key is
removed from every pin list (`ADR-025` §5). The copy is therefore sealed in the
passphrase envelope `cargo rivet key backup` uses for the signing seeds (R-183
D3; `ADR-009` §3.10), under a passphrase typed at a terminal. **A forgotten
passphrase loses the backup**, and both commands say so.

**No schedule and no retention.** Each command runs once, when asked, and keeps
nothing (R-183 D5). How often, and how many to keep, are decisions for whatever
drives it.

### 2.1 The name is load-bearing

**This is a backup of a board's secrets, not of a machine**, and the commands
are named `backup-secrets` and `restore-secrets` so that nobody reads them as
more. A command called `backup` beside a machine invites the belief that
restoring it reconstitutes the machine; §5 is why that belief would be false,
and a name is the cheapest place to stop it forming.

`AGENTS.md` §5's *refuse rather than guess* applied to naming: an operator who
reads "secrets" and wants more will ask; an operator who reads "backup" and gets
less will find out at the worst possible moment.

### 2.2 What covers application data instead — `ADR-051`

The exclusion above is correct and is unchanged. It left a hole all the same:
until 2026-09-08 nothing at all covered TempMon's readings or Reliquary's
blobs, and nothing was meant to.

**`ADR-051` is the answer, and it does not reverse this ADR.** An application
**offers** its data in its signed manifest — a route relative to its
`proxy-path`, and the format one fetch returns — the supervisor derives a
machine-wide table from every installed manifest and publishes it root-owned,
and something outside fetches it. Rivet still decides nothing about what is
kept, holds no copy, and schedules nothing; it carries a declaration.

The line between the two documents is worth stating once: **this ADR is about
what the MACHINE holds, and `ADR-051` is about what an APPLICATION holds.**

---

## 3. Why not the whole of `/state`

Three reasons, in ascending order of how much they matter.

**It is mostly not worth carrying.** The journal is the bulk of a long-lived
`/state` and is the least restorable thing in it — a record of what one machine
did, meaningless on another.

**Extensions are already signed artifacts.** `ADR-029` §11 makes an extension a
signed, versioned, independently installable file. Backing one up copies bytes
that can be fetched again from the thing that signed them, and a restore that
reinstalls from the trust set is *safer* than one that replays a local copy.

**The parts that cannot be re-derived are small.** What genuinely cannot be
recovered from anywhere else is the document the operator wrote and the secrets
the machine holds. That is a few kilobytes, and scoping to it makes the backup
cheap enough to take often — which is the property that decides whether a backup
exists when it is needed.

---

## 4. Where it goes: a workstation, pulling over the control plane

`PLAN.md` §4.26 answered *how does a file reach a Rivet host*. **Outbound is not
automatically the same question**, so it is asked rather than assumed:

| Destination | Verdict |
| :-- | :-- |
| The FAT boot partition | **No.** It is `ADR-002` §4.1's *inbound* provisioning channel, and the one filesystem an operator hand-edits. A backup written there would be indistinguishable from a file the operator put there, which is the second-authority failure `ADR-011` forbids — and a backup on the card survives nothing that loses the card. |
| A USB mass-storage device | **No.** Needs a block device, a filesystem driver, a mount policy and a hotplug story — `ADR-005` §2.1 has no udev on purpose. Every one of those is a new mechanism to protect a few kilobytes. |
| An admin server the board pushes to | **Not built, and not needed.** It was chosen for one property, being *off the board*, and it needs the board to hold a credential for it (`ADR-019`) and a server that exists. |
| **The operator's workstation, pulling over the control plane** | **Chosen** (R-183 D5). It is equally off the board, the channel exists and is already mutually authenticated (`ADR-025` §2, `ADR-035`), the board holds no new credential, and `cargo rivet backup`, `fleet drift` and `deploy` already reach a board this way. |

**A board that cannot be reached over its control plane does not get a
backup**, and that is correct rather than a gap: the alternative is a copy of
`/state` stored on `/state`, which survives nothing that would make anyone want
it.

### 4.1 How the pull works

```text
cargo rivet backup-secrets --host HOST:PORT --identity DIR --pin HEX --into DIR [confirm]
```

`cargo rivet backup`'s flag set, because it is that command's shape over the
control plane alone.

* **It reads through `get file`.** `ADR-006` §8.10.4's gate gives a read subject
  G1–G3 only — absolute, no control characters, no `.`/`..` — and `rivet explain
  /state/secrets` answers `E_UNKNOWN_SUBJECT`: it is not a view. So reading the
  store needed no change to the gate. **The control plane is operator-only**:
  `rivet-mcpd` admits only the keys in the board's `peers` file.
* **Without `confirm` no secret is read and no passphrase is asked for.** The
  plan lists the store — names and sizes — and reads `device.crt`, which is
  public.
* **Every entry is a regular file whose name `apply restore-secret` can write
  back, or the backup is refused.** A directory, a link, a listing the board
  cut short, or a file larger than the restore can carry would each make a
  backup reported as whole that is not.
* **With `confirm` it asks for the passphrase twice, before the first secret is
  fetched.** A prompt that refuses — no terminal, two entries that differ, an
  empty one — leaves nothing fetched and nothing written. There is no
  `--passphrase` flag and no environment variable: a secret in a process's argv
  or environment block is readable by anything else running as the same user.
* **The unit of encryption is the whole backup.** It writes one file,
  `<into>/secrets-backup.enc`: an envelope whose plaintext is a manifest
  followed by every file's bytes. **Nothing about the secrets is readable on
  disk** — not their names, not their sizes, and not the board key. The board
  key is public, and a readable copy would let a restore refuse another board
  before asking for the passphrase; but every restore needs the passphrase
  anyway, because even its plan compares the backup's bytes with the board's,
  and a second, unauthenticated statement of the binding would be one more thing
  to reconcile.
* **The plaintext never touches the workstation's disk.** The archive is built
  and sealed in memory; the envelope goes through a `.partial` and a rename,
  mode `0600` on Unix, and on Windows the directory's own ACL is the protection,
  as `cargo rivet key` says of a seed. The report prints names and sizes and no
  digest, because a digest of a short pre-shared key is something to guess
  against.
* **The manifest records** a format line (`rivet-machine-secrets 2`), the
  board's device public key (`board-key`, §5), the host as given, the time, and
  for every file its BLAKE3, its size and its name.
* **The envelope** is `cmd/cargo-rivet/src/envelope.rs`, the seed backup's
  own: Argon2id (m = 64 MiB, t = 3, p = 4) and XChaCha20-Poly1305, every
  parameter in the file's header and authenticated as associated data. Its magic
  is `RVTSECR` where a seed backup's is `RVTSEED`, so neither restore mistakes
  the other's file for its own. `ADR-009` §3.10 is the survey.
* **A directory that already holds a backup is refused**, not overwritten:
  replacing one backup with another is a retention decision.

---

## 5. Restore goes back onto the same board, and no other

```text
cargo rivet restore-secrets --host HOST:PORT --identity DIR --pin HEX --from DIR [--replace] [confirm]
```

### 5.1 Which board: the device key, proven by the handshake

**A board is the same board only if its device identity key matches** (R-185
D2). The key is the Ed25519 device public key `--pin` names and the TLS
handshake proves the board holds (`ADR-025` §2). It is the identity the fleet
already uses: the register pins it, and a request that arrives over the control
plane is attributed to it.

**A Pi re-carded with a new identity is a different board**, and a backup of
the old one is refused there; it gets freshly provisioned secrets.

**The hardware serial plays no part.** `ADR-020` §7's board provider was
designed to bind to it, but that provider was not built (`ADR-020` §11.2.5); a
board reports its serial as plain text nothing authenticates; a virtual machine
has none; and `/proc/cpuinfo` on an x86 machine carries none either.

**The key is cross-checked against what the backup carries.** At backup time,
the board's `device.crt` must carry the key `--pin` names and `device.seed` must
derive to it. At restore time, the backup's own `device.crt` and `device.seed`
must name the key its manifest records — so a manifest edited to name another
board is refused by the backup's own files.

### 5.2 Why another board is refused

**R-183 D5 rules the refusal**, and with the store in plain text the reason is
identity rather than readability. Restored onto another board, the secrets would
*work* — including `device.seed`, which would make two boards one identity to
every peer that pins it. `ADR-025` §5.2 already treats a board's key as the
board; a restore that copied it elsewhere would be an identity clone produced by
a backup tool. **If `ADR-020`'s envelope is ever built, the same refusal also
prevents the other failure** — secrets restored where they can never be read.

A board that has been given a new identity is, to the fleet, a different board
(R-185 D2), and its secrets are provisioned anew rather than restored from
another.

### 5.3 What the product does

1. **The backup states its binding, inside the envelope.** The manifest records
   `board-key`, and the AEAD authenticates it with everything else, so a restore
   can tell before it sends anything whether this is the board it came from.
2. **A restore needs the passphrase, and asks for it once, before the board is
   asked anything.** The file is checked to be a secrets-backup envelope first,
   so a file of the wrong kind is refused without a prompt.
3. **A restore onto a different board refuses**, naming both keys, and sends
   nothing. There is no flag that overrides it.
4. **A restore says what it did per file**: written, replaced, identical and
   skipped, or on the board and not in the backup and left alone. `ADR-006`
   §4.2: an operation that partly happened is not a result with a false in it.

Before the board is asked anything, a restore also refuses:

* **a wrong passphrase, or an envelope altered since it was written — with the
  same words for both.** An AEAD's security property is that the two are
  indistinguishable; telling them apart would hand an attacker an oracle. **This
  is what detects tampering**, including someone who rewrites the manifest along
  with the files, because they cannot re-seal without the passphrase;
* **an archive that does not describe its own files** — bytes the manifest does
  not account for, or a file whose BLAKE3 differs from the manifest. Inside an
  authenticated envelope this can only be a defect in what wrote it, and saying
  which file is wrong is cheap. The whole backup is refused, not the damaged file;
* **`device.seed` or `device.crt` that disagree with the recorded key**, so a
  manifest re-sealed by someone holding the passphrase to name another board is
  still refused by the backup's own files;
* **a `peers` file that does not name the key `--identity` connects with.**
  `rivet-mcpd` admits only the keys in `peers` and loads the whole file when it
  starts, so such a file would leave the board admitting nobody who could undo
  it after its next boot;
* **a directory in the unencrypted layout** an earlier, unreleased build wrote
  (`secrets-backup.txt` beside `files/`), by name and before any prompt, with the
  instruction to take a new backup.

Then it compares the backup with what the board holds. A file the board already
has with the same bytes is not written. **A file whose bytes differ is replaced
only with `--replace`**, the rule `cargo rivet key restore` and `cargo rivet
fleet objects restore` follow; without it nothing at all is written. Each file
that is written is read back and compared.

### 5.4 How it writes: `apply restore-secret`

Each file goes back through **`rivet apply restore-secret <name> --hex <hex>
confirm`** (`ADR-006` §8.10.19): `/state/secrets/<name>`, root-owned, mode
`0600`, written to a temporary and renamed into place so no reader sees half a
file. The path is composed from the name. The board refuses a `peers` file that
does not name the session's own pinned key, independently of the workstation's
check. Which board the bytes came from is the workstation's refusal, because a
command carrying bytes cannot tell where they came from.

### 5.5 What is running while it writes

**Nothing is stopped or restarted, and the restore says so.** The readers of
`/state/secrets` are `rivet-mcpd` (its identity and `peers`) and `rivet-netd` (a
pre-shared key). Both are in the reachability set `apply service-stop` refuses
(`ADR-017` §6), both read their files when they start, and stopping `rivet-mcpd`
would end the session doing the restore. Applications never open this directory
(`ADR-020` §8). **A restored file is therefore live at the board's next boot**,
and the atomic rename means nothing reads a partial file before then.

### 5.6 What has run, and what has not — `AGENTS.md` §6

* **The board subject, against `rivet-core`'s in-memory filesystem**: 14 tests —
  a 32-byte seed with a null byte arriving whole, root-owned, mode 0600, in one
  act; the plan never echoing the value; a composed path no name can escape; a
  missing store refused rather than made; a link or directory at the name
  refused; the `peers` lock-out refused on the plan path and the apply path.
* **The envelope** (`cmd/cargo-rivet/src/envelope.rs`, 11 tests): an envelope
  written by the seed-backup code before the envelope had its own module still
  opens, and this build writes the same header; any length of plaintext round
  trips; a wrong passphrase and a flipped ciphertext byte refuse identically; a
  flipped salt byte, another kind's magic, every truncation, trailing bytes and
  an unknown version are refused; with no terminal both prompts refuse by name.
* **Both commands, against a fake control plane** (`cmd/cargo-rivet`'s
  `machine_secrets_tests.rs`, throwaway identities minted by `cargo rivet
  identity new`, the passphrase handed in by the test): 30 tests — the plan
  reading no secret and asking no passphrase; one sealed file with no secret,
  name, digest or board key readable in it; a refused prompt fetching no
  secret; the round trip; `--replace`; a wrong passphrase and a tampered
  envelope refusing identically with nothing sent; a truncated envelope and a
  seed backup refused before any prompt; the unencrypted layout refused; and
  after decryption a different board, bytes the manifest does not account for, a
  digest mismatch, a relabelled manifest and the lock-out.
* **The prompt under a real terminal** (`cmd/cargo-rivet/tests/pty_secrets_backup.rs`,
  `#[ignore]`d): the shipped binary under a Windows ConPTY, against a `rustls`
  server presenting a throwaway board identity — a passphrase typed twice seals
  the store, typed once restores a lost file that reads back identical; a wrong
  one sends nothing at all; two differing entries write nothing and fetch no
  secret.
* **Under QEMU**, `tools/qemu/adr-030-secrets-proof.sh`, each command run under
  `script(1)`'s pty with the passphrase typed into it: a confirmed backup with no
  terminal refused; one `RVTSECR` file with the extra secret's value, its name
  and the board key unreadable in it; the backup's bytes equal to the guest's
  (every file `identical, skipped` in a restore plan); the guest refusing a
  lock-out `peers`; a removed and an overwritten file restored with `--replace`
  and the store identical to the backup afterwards; a wrong passphrase and a
  one-byte change to the envelope refused alike, nothing sent; the unencrypted
  layout refused; a second guest with another identity refusing the first
  guest's backup as a different board once opened, its store unchanged.
* **NOT proven by any of these:** anything on a Raspberry Pi or an SD card; a
  reboot after a restore, so that `rivet-mcpd` and `rivet-netd` start from the
  restored files is not observed; the prompt under a person's own terminal, and
  `pty_secrets_backup.rs` under a Unix pty (the QEMU proof's `script(1)` is the
  Unix pty that has run); how private the backup file is on a Windows
  workstation, where the directory's ACL decides and nothing here sets one.

---

## 6. Open

1. **The artifact is unsigned.** A backup is authenticated by its passphrase —
   whoever holds the passphrase can write one the restore accepts — and is not
   signed. `ADR-029` §11's signed container runs in the other trust direction —
   signed by a publisher and verified by a machine — and whether one container
   could serve a backup written by a machine and verified by itself deserves its
   own answer rather than an assumption.
2. **Whether the journal is ever in scope.** §2 excludes it on the grounds that
   it describes one machine's life. If `ADR-022` §7's on-disk journal becomes the
   place incidents are investigated from, that reasoning weakens.

---

## History

Superseded text, moved here out of the body so the body above states only what
is true now. **The body is complete without this section**; nothing here is a
live requirement.

**Superseded text keeps its strike-through.** That is not decoration:
`tools/docs/claims.py` masks `~~…~~` spans before it resolves a citation or
checks a path, so struck text is how a dead claim tells the checker it is dead.

### In §1 — `/state` was the only writable filesystem

**FALSE since 2026-08-24**, when `ADR-031` added volumes. **This ADR's whole
existence was derived from it** (`AGENTS.md` §7). The superseded sentence read:

~~`ADR-018` §4 makes `/state` the only writable filesystem on a Rivet
machine.~~

### In §1 — the file-transfer ruling was cited to the wrong document

**Corrected 2026-08-29.** The superseded citation was ~~`ADR-026` §4.26~~;
`ADR-026` is the development and test loop and stops at §4.6. Found by
`tools/docs/claims.py`.

### In §2 — why application data is excluded

**The reason was superseded 2026-08-24 and the exclusion was not.** The
superseded reason read:

~~`ADR-015` has no data model yet, and inventing one here would be the second
authority `ADR-011` forbids~~

`ADR-031` is that data model as far as *location* goes, so application data is
excluded because it is not on the thing this ADR backs up.

### In §2 and §4 — the destination was an admin server, and open questions about it

**Superseded 2026-09-14 by R-183 D5**: *"machine secrets are pulled to the
workstation over the control plane and restored onto the same board, refusing
any other board. No admin server, no schedule."* The admin server was never
built; a workstation pull has the one property it was chosen for, being off the
board, and needs no credential on the board. The superseded text read:

~~**The destination is the admin server**, over `PLAN.md` §4.26's channel used
in the outbound direction.~~

~~| **The admin server** | **Chosen.** The channel exists, the machine already
trusts it (`ADR-019` fleet enrolment), and it is the only destination that is
*off the board* — which is the only property that makes a backup a backup. |~~

and §6 held two questions the ruling answered — schedule (none) and who
initiates (the workstation pulls):

~~**Schedule and retention.** Whether a backup is taken on a timer, on change,
or only when asked; and what the admin server keeps. Nothing here depends on
it, so it waits for an admin server that exists.~~

~~**Push or pull.** §4 rules the *destination*, not who initiates. A pull needs
the fleet API `ADR-006` §5 deferred; a push needs the machine to hold a
credential for the admin server, which is `ADR-019`'s business.~~

### In §2 and §5 — the secrets were expected to be ciphertext, unreadable elsewhere

**Superseded 2026-09-14, when the backup was built against what `ADR-020`
actually is.** `ADR-020` §11.2.3 (2026-08-30) ruled secrets plain text and
§11.2.5 records that the envelope and the board provider were not built, so the
premise below was false by the time anything could act on it. The refusal of
another board survives, for the identity reason in §5.2. The superseded text
read:

~~| Secret store (`ADR-020` §5) | **yes**, as ciphertext | |~~

~~**A restore onto different hardware loses every secret.** `ADR-020` §6 wraps
the store with a key from a `KeyProvider`, and §7's board provider binds that
key to the board. … The consequence is that the ciphertext in a backup is
undecryptable by any machine but the one that wrote it.~~

~~the secret store — **present, and permanently unreadable**; the device
identity seed — **present, and permanently unreadable**, so the replacement
machine presents a *different* identity~~

### In §5 — a restore onto a different board was to proceed on confirmation

**Superseded 2026-09-14 by R-183 D5**, which rules the restore *"refusing any
other board"* with no override. The superseded rule read:

~~**A restore onto a different board refuses by default**, names what would be
lost, and requires confirmation — `ADR-012` §7.1's shape, which the CLI already
implements for every changing operation.~~

### In §6 — key escrow

**Reframed 2026-09-14.** With no wrapping key on the board (`ADR-020` §11.2.5)
there is nothing to escrow; the live question became whether the copy on the
workstation is encrypted, which R-185 D1 then answered (next entry). The
superseded item read:

~~**Key escrow — deferred, not rejected.** The question is whether a wrapping
key may leave the board under operator control, and it is an `ADR-020`-sized
decision.~~

### In §2, §4 and §6 — the backup was not encrypted

**Superseded 2026-09-14 by R-185 D1**: *"`cargo rivet backup-secrets` encrypts
with a passphrase, the same format and typed prompt as `cargo rivet key backup`
(R-183 D3), and `restore-secrets` needs it. A forgotten passphrase loses the
backup, and the tool says so."* The first build copied the store as the board
holds it, reasoning that a copy of plain text need be no better protected than
the plain text. What that missed is where the copy lives: the board's store is
protected by being on the board, and a directory on a workstation is not — while
it holds the device seed that lets its holder present as that board (`ADR-025`
§5). The maintainer ruled the copy sealed, with the seed backup's envelope rather
than a second format. That build was never released, so its layout is refused
rather than read. The superseded text read:

~~**The backup is not encrypted, because the store is not.** … So there is no
ciphertext to carry: the backup is the store's files byte for byte, and is
exactly as sensitive as the card.~~

~~**It writes** `<into>/files/<name>` for each file, then
`<into>/secrets-backup.txt` last, so a manifest's presence means every file it
names was written.~~

~~**Whether the backup should be encrypted at rest on the workstation.** … Nothing
here encrypts it, and the question is the maintainer's.~~
