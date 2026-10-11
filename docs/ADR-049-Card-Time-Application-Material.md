# ADR-049: Card-Time Application Material — the staging bundle and the ownership hop

* **Status:** Accepted. **Built, with tests, and not run on a board.** §9 says
  what is proved, by what method, and what no machine has done.
* **Date:** 2026-09-07
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Ruled:** by the maintainer, 2026-09-07 — the three first-boot states, the
  stand-alone register, the deletion discipline, and that `peers` was never a
  separate problem. `docs/design-candidates/Provisioning-Tool-Completion.md` §7
  carries his statements; this ADR is the decision they became.
* **Answers:** the maintainer's expectation that, when the device boots up,
  `fleetd` comes up running just like `mcpd` does (`.rulings/` R-303) —
  `SPEC-05` §1.1, and what has to be on the card for that to be true.
* **Builds on:** `ADR-019` (the courier and the destruction of the secret half),
  `ADR-004` (first-boot provisioning), `ADR-015` (the data directory, the uid
  and mode 0700), `ADR-034` (the register), `ADR-025` (fleet identity),
  `ADR-020` (secrets at rest)
* **Supersedes:** nothing. Every document above stands unamended.
* **Code:** `rivet-core/src/staging.rs` (the format and the paths),
  `rivet-core/src/state.rs` (`STAGING`), `cmd/rivet-image/src/boot.rs` and
  `src/main.rs` (`--staging`), `cmd/rivet-initd/src/staging.rs` (the courier),
  `cmd/rivet-sysd/src/applications.rs` (`deliver_staged`, the ownership hop),
  `cmd/rivet-provision/src/stage.rs` (what a role needs),
  `cmd/rivet-provision/src/stage_document.rs` (`rivet-provision stage`, §12),
  `cmd/cargo-rivet/src/provision.rs` (`ensure_staging`, §12),
  `tools/card/build.sh` (`RIVET_STAGING`)

---

## 1. Context

On 2026-09-07 a fleet server was brought up and did not work. Reaching a working
one took, **after the card had already booted**: three secret installations
under names no tool would guess, two directory creations, three file writes,
five ownership changes and two service restarts — every one of them typed by an
agent over the control plane. The maintainer's rule for that day was already
written down: every time he could not do something and the agent had to do it
for him was, frankly, the system failing. *(`.rulings/` R-301.)*

`SPEC-05` §10 is the manifest of what the hand did, read back off the running
machine with `get file` rather than reconstructed from a transcript. It is
therefore also the list of what the card must carry, and this ADR is how it gets
there.

### 1.1 What was missing, precisely

An application on a Rivet gets its own directory and creates its own state
correctly, unaided — `SPEC-05` §10.6 lists what nothing had to place:
`config.json`, a database and its WAL, an admin secret, a blob store. **The
platform's data-directory contract works.**

What is missing is narrower and it is the whole of this decision: **the
per-machine material the platform is supposed to hand an application before it
starts.** A fleet server's `rivet-fleetd` reads five files under three naming
schemes and a register in a fourth layout, none of which can be in a signed
payload every machine gets a copy of, because every one of them is about *this*
machine.

---

## 2. The decision

**A card may carry a staging bundle, and the material in it is delivered into
each application's own data directory, owned by that application, before the
application first starts.**

Four parts, in the order the material travels:

1. **`rivet-provision` composes the bundle** when it composes the document,
   because composing it wants the authority, the hostname and the machine's
   identity in one process at one moment, and that is where they meet. A
   document that arrives without one has it composed by `rivet-provision stage`
   from the document itself, which the card writer runs for it (§12).
2. **`rivet-image` writes it to the boot partition** under `--staging`, having
   parsed it first.
3. **PID 1 moves it to `/state/staging`** before any service runs, and then
   overwrites and unlinks the card's copy.
4. **The supervisor delivers it** into the data directory it has just created,
   with the uid it has just derived, and deletes the staged copy.

---

## 3. The mechanism is `--identity`'s, and no second one was invented

`cmd/rivet-image`'s `--identity DIR` already states the pattern in its own help:
material goes on the **boot** partition; PID 1 copies it to `/state/secrets` on
first boot and **deletes the seed from the card**. `ADR-019` §3 is the decision
behind it and `cmd/rivet-initd`'s `identity` module is the implementation, whose
ordering — copy, `sync`, blank, `sync`, unlink, `sync` — is the whole of its
safety and rests on two measurements taken on a real board.

**That is the pattern this reuses, deliberately and to the letter.** A device
identity has been travelling this road since 2026-08-28; a second road for a
second kind of secret would double the surface on which a private key can be
left readable in FAT free space.

### 3.1 What it could not reuse: the last hop

An application's data directory is mode `0700` owned by **that application's
uid** (`ADR-015` §5), and the uid is derived from the name by the supervisor at
discovery. It is not knowable when the card is written, and it is not knowable by
PID 1 either — at that point in the boot no application has been discovered and,
on a fresh card, none is installed. So:

```text
  boot partition -> PID 1 -> /state/staging -> the SUPERVISOR delivers into
                                               each application's data
                                               directory, with the uid it has
                                               just allocated
```

**The extra hop is not theoretical.** On 2026-09-07 the register and the
credentials were placed by hand as root and `rivet-fleetd` could not read one
file of them. It **logged nothing** either time it was restarted, because
`rivetos_register::store::load` treats a missing directory as an empty register
by design — correct for a first run, and silent for this. Five `apply set-owner`
calls fixed it. `deliver_staged` is those five calls, happening before the
application has ever started.

**The same hop serves an image service that runs under its own uid**
(`ADR-060`). `rivet-proxyd`'s share of the bundle is addressed to it by name
exactly as before; the supervisor makes the proxy's home at boot, before its
first start, and delivers the certificate, key and fleet root into `data/` with
the proxy's uid. Nothing about the bundle or PID 1's split changes with where
the program comes from.

---

## 4. The format, and why it is not TOML

One line per file: `<application> <relative/path> <hex of the contents>`, after
a `version = 1` line, with `#` comments. It lives in `rivet-core`, which
performs no I/O and depends on nothing but `secrecy` and `zeroize`.

**Because both readers are the two programs that must not grow a parser they
cannot test.** PID 1 and the supervisor are the processes whose failure is the
machine; a line-oriented grammar that fits in one function, in a crate whose
tests run everywhere, is what they can afford. The format is also diffable and
greppable, which a person holding a card in a workstation will want.

**Hex rather than raw bytes**, for `ADR-020` §11's reason and one more: the
bundle sits on a FAT partition where a NUL is a hazard to every tool that might
read it, and the material is already mixed — `server.key` is PEM while
`operator-device.crt.hex` is DER already written as hex text. One encoding ends
the question.

### 4.1 The path rule is the security boundary

The supervisor writes these paths **as root**, joined onto a directory it owns.
A `..` would put a file anywhere on the machine and a leading `/` would ignore
the join entirely. So `rivet_core::staging::check_path` refuses absolute paths,
`.` and `..` components, empty components, backslashes, whitespace, non-ASCII
and control characters — **at parse time**, where a caller cannot forget to ask.

### 4.2 No mode and no owner in the file

Every staged file lands mode `0600` inside a directory the application owns and
nobody else can open, so a per-file mode would be a second and weaker answer to
a question `ADR-015` has settled. Naming an owner would be worse: the card
cannot know the uid, and a number written at card time that later disagreed with
the supervisor's would produce exactly the silent unreadable directory this
exists to end.

---

## 5. One bundle per application under `/state/staging`

PID 1 splits the card's bundle and writes `/state/staging/<application>.txt`,
mode `0600` root.

**Split rather than unpacked into a tree**, and the reason is the reader on the
far end. Delivery happens inside the supervisor's discovery pass, and that
crate's `Kernel` port has no recursive directory walker in it — `list_dir`
answers directories only, and `list_dir_entries` is documented as *"used for
`ADR-015` §6's device grant and nothing else … it is not a general directory
walker and should not become the easy way to have one."* One `read_file_bytes`
and one `parse` is the whole read; the paths are checked by the parser rather
than by a walk that could meet a symlink; what is pending for an application is
one file an operator can `get file`; and delivery ends with one `remove_file`.

### 5.1 The wait is the ordinary case, not the exception

**None of the applications a fleet server runs is on the card.** They arrive
afterwards as signed payloads, and `SPEC-05` §5.1 already names this: nothing at
card time can know whether a named application will be installed. So an
application's share of the staging normally sits under `/state` until an install
happens, and the supervisor re-runs discovery after every install
(`cmd/rivet-sysd/src/installed.rs`). Delivery therefore lands on the pass that
first sees the application — the same pass that creates the directory it goes
into.

**This is stated plainly because it bounds the claim.** This decision removes
the hand-work; it does not put `rivet-fleetd` on the card. §9.2 says what closes
that.

---

## 6. Convergence, and what is never overwritten

* **A file already in the data directory is kept, not overwritten.** By the time
  a second pass runs, the application may have rewritten it — a fleet server
  writes its own register — and a card-time copy overwriting a live one would
  silently roll a fleet back to the day the card was made.
* **The owner is set even for a file that was kept.** That is the repair path
  for the one failure this could not otherwise recover from: a delivery that
  wrote a file and then failed before its `chown`, leaving root-owned bytes in a
  `0700` directory and nothing saying so.
* **A failure leaves the bundle where it is.** PID 1 has already destroyed the
  card's copy, so `/state/staging` is the last one; removing it after a failed
  delivery would close the window with nobody having seen it. The failure is
  reported at `Error`, and the next boot or the next install tries again.
* **The service still starts anyway**, on `provide_data_dir`'s reasoning
  exactly: an application that cannot read its credentials fails in a way an
  operator can see, and one that is never started fails in a way that looks like
  the supervisor's fault.
* **A bundle that will not read delivers nothing at all.** A half-applied
  staging is a state nobody designed and nothing on the machine would say which
  half arrived.

---

## 7. The three first-boot states

The maintainer's three states, 2026-09-07 *(`.rulings/` R-303)*:

* **Stand Alone** — on first boot the machine comes up knowing everything it
  needs to get itself up, with its certificates, and able to communicate with
  `cargo-rivet` on the PC, which is its fleet server.
* **Fleet Server** — the machine comes up, provisions the new fleet register,
  enrolls itself, and is ready to talk to `cargo-rivet` with credentials.
* **Member Server** — the machine boots with the fleet's existing register and
  its certificates.

### 7.1 A stand-alone machine carries no register

Ruled: a machine that does not need a register does not carry one
(`.rulings/` R-303). The workstation holds the register for
a machine it administers directly, and a second copy on the machine would be the
second authority `ADR-034` §2.2 dropped Local mode to avoid. The symmetry with
the other two states was an assumption, and it was dropped rather than built.

### 7.2 A fleet server CARRIES a register; it does not create one

`cmd/rivet-fleetd/src/fleet.rs` refuses by design: *"It does not invent entries.
A fleet server that met an empty directory and minted a record for itself would
be manufacturing the one thing `ADR-034` §3.5 says it cannot have: a device
record is written by the tool that GENERATED the device's keypair, and this
program generated nobody's."*

So the Fleet Server case is **not** *come up and create a register*. It is *come
up carrying a one-record register the tool that minted your identity wrote*.
`rivet-provision` is that tool and this is that write, which is what makes all
three states one mechanism.

### 7.3 `peers` was never a separate problem

`--identity DIR` already takes *"device.seed, device.crt and a peers file beside
them. ALL THREE OR NONE"* and PID 1 already copies all three. So a fleet server
needs nothing new: `rivet-provision` adds the fleet server's own operator public
key to the peers file in the identity directory the card will be built from.

**It only ever adds.** A peers file is the whole list of who may reach a
machine's control plane, and dropping a line from a headless board's card is a
lockout whose only repair is another card.

**And it takes effect only on a boot.** `cmd/rivet-mcpd` builds its pin list at
startup, before the socket is bound, so this cannot be repaired after the fact
without risking exactly that lockout — which is what makes it card-time work
rather than a fix-up.

---

## 8. What a fleet server's card carries, and where the list came from

Not invented here. `SPEC-05` §10 is the manifest read back off `rivet-01`.

| application | file | what it is |
| :--- | :--- | :--- |
| `rivet-fleetd` | `server.crt`, `server.key` | its TLS certificate and key, for the browser |
| `rivet-fleetd` | `ca.crt` | the authority both ends are checked against |
| `rivet-fleetd` | `operator-device.seed.hex` | its operator private key, **hex text** |
| `rivet-fleetd` | `operator-device.crt.hex` | its operator certificate, DER as **hex text** |
| `rivet-fleetd` | `register/register.toml`, `register/devices/<id>.toml` | a one-record register — its own |
| `rivet-proxyd` | `tls-cert`, `tls-key`, `tls-root` | the same leaf, under the names that program reads |

**Three naming schemes in one directory, and a fourth for the proxy.** That is
not tidiable from here: `cmd/rivet-fleetd` opens `server.crt` and
`cmd/rivet-proxyd` opens `tls-cert`, and each program is the authority on the
names it reads. This writes what they read.

**One leaf, not two.** `rivet-fleetd` and `rivet-proxyd` are handed the same
certificate under their own names, because minting a second for one machine
would be two certificates for one name — `ADR-011` in the place it is hardest to
debug, a browser trusting one page of a machine and not another. A fleet-root
server re-issues that one leaf itself when it is missing, near its end or
lacking the standard fields, and writes it where this hand-over put it
(`ADR-063` §4.5).

**The operator credential is the one nobody had noticed.** Without it the fleet
portal lists machines and can drive none of them: every page answers *"this
machine did not answer"*. A fleet server asks **itself** over the control plane
exactly as it asks anything else — `/run/rivet/query` is `0600 root:root` and
serves only `status` — so there is deliberately no self-shortcut to fall back
on.

### 8.1 The register record's identifier is derived from the pin

Stable and opaque, which is both properties `ADR-025` §4 asks of an identifier.
Re-provisioning the same machine produces the same identifier rather than a
second record for one board, and nothing about the machine can be read out of
it.

**The identifier is the pin's first eight bytes, so it is only as unique as the
pin.** Each machine is therefore provisioned with its own identity: the
interview's Identity answer defaults to `~/.rivet/identity/<hostname>` and is
minted at `w` when that directory holds no identity (`.rulings/` R-189 D2).

**A record can carry a pin whose identifier it does not share.** A record
enrolled by `rivet-fleet add device` has a randomly minted identifier, so
comparing identifiers does not find a second machine holding the same key.
The enrollment check therefore compares pins too: an identity whose pin is
already on a record under another hostname is refused, at the interview's `w`
and again before `cargo rivet provision` writes a card, naming that record's
hostname and identifier.

### 8.2 `[applications.rivet-proxyd] upstream-name` is composed with it

Not part of the staging, and fixed in the same pass because it is the same
defect. `cmd/rivet-proxyd` has no default and derives nothing — *"appending
`.local` to the kernel hostname would assume a suffix `ADR-034` makes a fleet
property"* — so a card composed without it boots to a fleet server whose front
door starts five times and is then marked `failed`. The value is the name on the
fleet certificate, which the tool has already composed from the authority's own
suffixes at the moment it writes the document.

---

## 9. What is built, what is proved, and what is unproven

### 9.1 Built and tested

* `rivet_core::staging` — the format, the path rule, the split. 17 tests,
  including every path that would escape a data directory and a byte-exact
  round trip.
* `cmd/rivet-image` — `--staging PATH`, parsed before a partition is written so
  a bad bundle is refused on the workstation rather than on a headless board.
  Tests assert the bundle reaches the boot partition, that it does not require
  an identity beside it, and that its name is one this crate's own FAT rules
  accept.
* `cmd/rivet-initd` — the courier. 10 tests, including the ordering guarantee
  (`/state` written and flushed before the card's copy is blanked, and blanked
  before it is unlinked) and a test that asserts PID 1 writes **nothing** under
  `/state/app`.
* `cmd/rivet-sysd` — `deliver_staged`. 6 tests, including that every delivered
  file and every directory on the way is chowned to the application's uid, that
  a file the application already holds is kept and still chowned, that a failed
  delivery keeps the bundle and still starts the service, and that a bundle
  naming another application delivers nothing.
* `cmd/rivet-provision` — the composer. 10 tests against a scratch authority,
  including that the staged seed and the staged certificate are the same
  keypair, that the register holds exactly one record filed under its own
  identifier and carrying the machine's pin, and that the peers file only gains
  lines.

### 9.2 Unproven

**No machine has booted from a card built this way.** What is asserted above is
that the bytes are composed, carried, moved and owned correctly according to
fakes and to a card image read back off the disk. What is **not** asserted is
that a Raspberry Pi boots from such a card and that `rivet-fleetd` reaches
`started`.

Closing it takes exactly three things and nothing else: a card written from an
image built with `RIVET_STAGING`, a **test** board to put it in, and

    rivet run --host <addr> --identity <dir> --pin <hex> -- get status

showing `rivet-fleetd` `"state":"started"` with `/api/fleet` returning the
record that was injected. **The portal's HTML is not evidence** — it ships its
empty-register sentence as a JavaScript string at all times.

And the first boot of a fresh card cannot show `rivet-fleetd` started at all,
because no application is on the card (§5.1). The observable on that boot is
`/state/staging/rivet-fleetd.txt` present and `/state/staging` empty of anything
else; the observable **after** `cargo rivet deploy` installs the application is
the file gone and the credentials in place, owned, with no `apply set-owner`
typed by anybody.

---

## 10. What this does NOT do

* **It does not put an application on the card.** Closing that needs the card
  build and the document to know about each other, which is `SPEC-05` §4.8 and
  §4.9 and is a separate decision.
* **It does not stage anything for a stand-alone machine.** That role enables
  no service that reads per-machine credentials, so there is no data directory
  for anything to be delivered into, and staging a leaf for a service that is
  not on the document would put a private key on a card to be delivered to
  nobody. The tool says so on screen rather than writing an empty bundle. A
  **member** is not this case: it enables the front door, and §12 says what it
  carries.
* **It does not give a member the fleet's existing register.** There is no verb
  that reads a register off a running fleet server —
  `docs/design-candidates/Fleet-Bring-Up-Findings.md` §2.6 proved that by
  reading `rivet-fleet`'s whole surface — so a member's register cannot be
  fetched, and material that cannot be fetched cannot be staged. `SPEC-05` §8
  already puts enrolling a member out of scope.
* **It does not replace `cargo rivet secrets`.** On a fleet whose root rests on
  a workstation, the other four applications on a fleet server take their TLS
  material after boot, one command each, and `SPEC-05` §10.1 records that as the
  one part of the hand-work that was not a defect. On a fleet-root server the
  machine issues each of them its own certificate at install and at every boot
  (`ADR-063` §4.5), so nothing is staged for them at all.
* **It does not encrypt anything.** `ADR-020` §11 ruled the card is the whole
  boundary, and hex is an encoding that stops the accidental case. A staged
  private key is `0600` root under `/state` until it is delivered and `0600`
  application afterwards, which is exactly what a wireless passphrase already
  gets.

---

## 11. Why a new ADR, rather than an amendment to one of three

Each of the three documents this builds on owns exactly one hop, and **none of
them owns the journey.**

* **`ADR-019` owns the courier.** Its subject is fleet enrollment: the machine's
  own identity, and the promise that a card recovered from a running device
  carries no secret. This inherits §3's discipline unchanged — but the material
  here is addressed to an *application*, and §3 has no vocabulary for an
  application, a uid, or a supervisor.
* **`ADR-004` owns first boot.** Step 3 of its §6 is the sibling of this
  decision's third hop, and the courier is written as one. But `ADR-004` ends
  where PID 1 ends, and half of this decision is about what happens afterwards —
  possibly days afterwards, at an install.
* **`ADR-015` owns the data directory.** The uid, the mode and the ownership are
  its §5, and this changes none of them; it depends on them. Writing a card-time
  provisioning decision into the packaging ADR would put it where nobody
  provisioning a card would look.

A decision written into any one of the three would be invisible from the other
two, and this is precisely a decision about how the three meet. So: a new record
that cites all three and amends none.

---

## 12. When a bundle exists, and what builds one

**A fleet server's card and a fleet member's card each carry a bundle, whatever
else they carry.** The role decides it, not the payloads: both roles enable
`rivet-proxyd`, which is part of the system image (`ADR-060`) and is handed its
certificate, key and fleet root from the bundle. So a member's card, which
carries no application at all (`ADR-050` §5.1), still carries a bundle. A
stand-alone card carries none (§10).

| role | the bundle holds |
| :--- | :--- |
| server | `rivet-proxyd`'s `tls-cert`, `tls-key`, `tls-root`; `rivet-fleetd`'s five files (§8); a register holding the server's own record and nothing else |
| member | `rivet-proxyd`'s `tls-cert`, `tls-key`, `tls-root` — its own leaf, for its own name |
| stand-alone | nothing, and no file is written |

**A fresh server's register holds itself only.** Members are enrolled into it
over the control plane once they are provisioned (`ADR-059` §6), never staged
onto the server's card.

### 12.1 Two ways a bundle is composed, one composer

* **The interview's `w`** composes it beside the document it writes,
  `<host>.staged.txt` beside `<host>.provisioning.toml`.
* **`rivet-provision stage --document PATH --identity DIR [--at DIR]`**
  composes it for a document that already exists — composed by an agent,
  edited by hand, copied from another workstation. It reads the role, the
  hostname, the listener and the addressing out of the document and the
  authority from `<at>/ca`, and runs the same composer `w` runs, with the same
  two side effects: the identity's `peers` gains the key the machine's fleet
  server presents, and the machine's leaf is kept in the authority directory
  for a later deploy. It lists what it staged by name and size, never by
  content, and refuses by path when the authority or the identity is missing.

**`cargo rivet provision --document` and `cargo rivet write-image --document`
run `stage` themselves** when a server or member document has no bundle beside
it, then carry on with the write and say what was built. They find
`rivet-provision` as the interview is found — the installed sibling binary,
checked to be the same build — so the bundle one writes is the bundle the other
would have written. A missing bundle is therefore never the reason a card is
refused. Two cases are refused rather than built:

* **`--staging PATH` naming nothing.** A bundle named on the command line and
  absent is a mistake to report, not a gap to fill.
* **`cargo rivet provision --existing HOSTNAME`.** It rewrites a machine that is
  already provisioned, and building a bundle there would mint a fresh
  `rivet-fleetd` operator key for a server whose members already trust the old
  one.

### 12.2 A member trusts the server that manages it

A member's `peers` must name the key its fleet server's `rivet-fleetd` presents,
or the server can list the member and manage nothing on it. Both composers read
that key out of the Armada under the credential root (`ADR-059` §5), and add it
when the Armada holds exactly one Fleet that records one. **A fleet server
reaches the Armada when its card is written** — `ADR-059` §3's enrollment of a
server, done at the moment the card is known to exist: after a verified write,
`cargo rivet provision` enrolls the server into the register with the operator
key its bundle carries, and adds it to the Armada as a Fleet when the Armada
holds no Fleet or already holds that one. An Armada holding other Fleets is
left alone and the report names `cargo rivet armada add`, because adding a
second would leave the next member unable to tell which server to trust.

So the order for a fresh fleet is **server first, card written, then member** —
a member provisioned before its server's card is written finds no Fleet and is
staged trusting the human operator alone, and the interview says so.

---

## History

### 2026-09-24 — §10 said a member stages nothing

§10 read: *"It does not stage anything for a member or a stand-alone machine.
Those roles enable no application that reads per-machine credentials — a member
gets `rivet-mdnsd` alone."* That stopped being true when a member's document
began enabling `rivet-proxyd` (`ADR-050` §5.1) and its front door needed its own
certificate from the bundle (`.rulings/` R-160 R25); since `ADR-060` the proxy
is an image service and the bundle is how it is handed that certificate. §10
now covers the stand-alone machine alone, and §12 states what each role's
bundle holds and what builds it.
