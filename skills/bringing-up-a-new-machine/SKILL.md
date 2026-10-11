---
name: bringing-up-a-new-machine
description: The whole arc for one machine, in order — write the card, first boot, deploy each application, mint and install its per-machine TLS material, switch on what a manifest depends on, give an older static machine a resolver, then reload and restart in the order that works. Use when asked to set up, rebuild or bring up a Rivet machine from scratch, and when a freshly deployed application crash-loops, draws nothing, is unreachable at `https://<host>/`, or fails every name lookup.
---

# Bringing up a new machine

**From a written card to a machine somebody can use.** Every other skill here
owns one operation; this one owns the **sequence** and the **traps between the
operations** — which step comes before which, and what each one leaves for the
next.

Where a step is already documented, this page points at it rather than
restating it:

| Step | Owned by |
| :-- | :-- |
| Writing the card, the interview, the refusals | `skills/provisioning-a-fleet-server` |
| What a card carries and what first boot does with it | `skills/carrying-applications-on-a-card` |
| Building, packaging, publishing, and one deploy in detail | `skills/deploying-an-application` |
| Reading a machine back afterwards | `skills/reading-a-board` |

## Why this exists, given that those were already written

On 2026-09-14 `skills/deploying-an-application` already carried the TLS
material, the proxy restart and the reload-before-restart ordering. On
2026-09-15 a real bring-up of a kiosk member met **all three anyway**, plus two
the skill did not carry.

The pages were not wrong. **Nothing tells an operator at the START of a rebuild
which documents they are about to need**, and every one of these steps is
invisible until something has already failed: an application crash-loops, a
screen stays empty, an index says nothing asks to be proxied, a lookup fails
inside somebody else's program. So the value here is order, not content.

`ADR-055` §7 item 3 is the standing caveat on this whole page: **a runbook is
the record of a defect, not its repair.** Every step below is a step whose
omission is silent. `ADR-055` §2 holds the three repairs; of them,
`.rulings/` R-194 D1 rules that minting TLS material stays an operator act and
that what `cargo rivet deploy` owes is a **refusal naming these commands**,
rather than a crash-loop diagnosed from a journal.

## Before you start

* **The fleet repository holds a published system image and every application
  this machine is to run.** `cargo rivet fleet list` is what it holds;
  `skills/provisioning-a-fleet-server`'s prerequisites table is how each got
  there. A missing image refuses the run before the interview; a missing
  carried product refuses it at the version question.
* **The tools on your PATH are this tree's build, and are a matched pair.**
  `cargo rivet --version` prints which file answered and which is first on
  PATH. The install is two lines — `cargo install --path cmd/cargo-rivet
  --force --locked` and `cargo install --path cmd/rivet-provision --force
  --locked` — because `provision` runs `rivet-provision` as its interview and
  refuses a pair whose builds disagree; `--locked` builds both against the
  committed `Cargo.lock`.
* **Know the machine's own pin.** The interview prints it when it mints the
  identity; `cargo rivet identity show --directory <identity dir>` reads it
  back out of `device.crt` afterwards, and `rivet-fleet get device --device
  <hostname>` prints the register's copy. `--pin` is always the **machine's**
  public key, never yours.

---

## 1. Write the card

```sh
cargo rivet provision
```

**Have the card in the reader before you start.** With a terminal, the run
offers the removable devices, prints the plan and asks `Write it? [y/N]`;
`confirm` only means that question is already answered, so an interactive run
needs no `confirm`. With **no removable device at all** the run ends after the
interview having written nothing — the document, the minted identity and the
staging bundle are kept, and it prints the `--document … --identity … --image …
confirm` command that writes the same card without re-answering anything.
Everything up to the write costs a full interview, so plugging the card in
first is cheaper than reading that line. (`.rulings/` R-193 D1 rules that a
run with a terminal should ask for the card instead; that is queued work, and
until it lands the refusal above is what happens.)

The interview asks nine things — hostname, addressing, address, gateway, mode,
fleet, timezone, auto-run, identity — and a tenth, **nameservers, for a static
address**: a bare Return takes the gateway. A DHCP machine is not asked, because
its lease names them. Step 7 is what to do for a machine provisioned before the
interview asked. **On a kiosk, answer Auto-run
`rivet-launcher`** (offered as `1`): the launcher ships disabled in the image,
and a blank answer enables nothing that would draw on the screen.

**A fresh fleet is a server first, its card written, then each member.** A
member's `peers` must name the key its server's `rivet-fleetd` presents, the
member interview reads that key from the Armada, and the server reaches the
Armada only when its card is written — the provisioning report says `armada:
Fleet "<server>" added …`. So write the server's card, boot it, see `get
status` answer (the member's enrollment is sent to it), and only then provision
the member; at its `w` the notes must say `added this member's fleet server's
operator key`. `skills/provisioning-a-fleet-server`'s *A fresh fleet* is the
checklist, with what to do when either line is missing.

A document handed to `cargo rivet provision --document` without its staging
bundle is not refused: `rivet-provision stage` builds one and the run says so
(`ADR-049` §12).

## 2. First boot: nothing to type

The machine installs what the card carried on its first convergence pass
(`skills/carrying-applications-on-a-card`). For a **member** that is nothing
at all: its front door, `rivet-proxyd`, is part of the image (`ADR-060`) and
starts because the document enables it, with the certificate the card staged
for it. No application that draws is there, so a kiosk machine's screen reads:

```
No Kiosk Applications Enabled
Install and enable a kiosk application to run it from the launcher.
```

**That is a true statement, not a fault.** The card carried nothing to draw;
steps 4, 6 and 8 are what put something on the screen.

## 3. Reach it by name

```sh
cargo rivet run <hostname> -- get status
```

A bare leading word is a device name, resolved through `rivet-fleet`'s
register into `--host`, `--identity` and `--pin`. **A DHCP machine has no
address in its record**, so the tool asks this workstation's resolver for
`<hostname>.local` — which the board's `rivet-mdnsd` answers — and prints the
address it dialled, IPv4 first. The register's pin still decides whether to
trust whatever answered there. If the name does not resolve, give
`--host <address>:7777` for that run.

`cargo rivet run`, `install`, `fleet push` and `fleet sync` all take the device
name. **`cargo rivet secrets` does not** — see step 5.

## 4. Deploy each application

**A person** runs the interview, which picks the applications and this host
from menus and asks before each step:

```sh
cargo rivet deploy
```

**An agent** runs the scripted verb once per application, and then the steps
it prints — `skills/deploying-an-application`'s *An agent's order*:

```sh
cargo rivet fleet list                                                  # the digests
cargo rivet install <hostname> --artifact <digest-or-prefix> confirm
```

`install` waits for its own install to finish, exits non-zero if it failed,
and prints what remains for that application on that machine, in order, as
exact commands — per-machine material (step 5 below), the enable, the
converge, restarts, the proxy restart a routed application needs, and on a
fleet server the fleet sync. **Both front ends plan from one planner**, so
those are the steps the interview would have offered. Steps 5 and 8 below are
the same work written out by hand, for a machine being repaired rather than
deployed to.

* **Re-sending an artifact the machine already runs is not a failure.** The
  machine refuses the install request before writing it, the tool reports
  `ALREADY RUNNING … NO INSTALL WAS REQUESTED`, and it exits zero. `--enable`
  still wrote its document edit, because that happens before the send, and
  the steps printed then include the converge that makes it take effect.
* **`--artifact` skips the library pre-check.** The check that refuses an
  application whose declared library closure the machine lacks runs on the
  build-and-publish path only. `ADR-055` §4 rules that the pre-flight should
  run on every route; the declared-material half does, the library half is
  unbuilt, and the machine's own installer is the backstop meanwhile.
* **Installing is not running, and neither is enabling.** A converge or a
  restart is what starts it — the printed steps, or step 8.

## 5. Mint and install each application's TLS material

**`install` prints these two commands, filled in, for every application whose
manifest declares `fleet-tls`**, and refuses `--enable` until they have run.
They are written out here for an application that predates the declaration,
and so declares nothing. It was once the step nothing told you about, and it
produced three crash-loops in a row. An application that serves HTTPS behind
`rivet-proxyd` — and one that *calls* another over HTTPS — reads `tls-cert`,
`tls-key` and `tls-root` out of its own data directory. **A signed payload
deliberately carries none of the three**: they are per-machine and secret, and
a payload is one artifact every machine gets a copy of (`ADR-055` §7 item 1).
Provisioning stages them for the role's own services — `rivet-fleetd` and the
image's `rivet-proxyd` (`ADR-049` §8, `ADR-060`) — and for nothing deployed
afterwards.

**One certificate per MACHINE, not per application.** Mint the server pair
once:

```sh
rivet-provision ca server <hostname> <ca dir>
```

By default it composes `<hostname>.<suffix>` for every suffix the authority
permits, read out of `root.crt`. Then install that one pair into each
application that needs it:

```sh
cargo rivet secrets <application> \
    --host <address>:7777 --identity <your identity dir> --pin <the MACHINE's 64 hex> \
    --authority <ca dir> --name <hostname>.<suffix> confirm
```

* **`cargo rivet secrets` takes no device name.** It wants `--host`,
  `--identity` and `--pin` written out, unlike every command in steps 3, 4 and
  8 — the line `install` prints has them written out. `--authority` defaults
  to `~/.rivet/ca`; `--name` selects **which server
  pair**, when the authority directory holds more than one, and it is a name
  (`<hostname>.<suffix>`) rather than a file name.
* **It does not restart the application**, on purpose: an application reads
  these at start, so stopping one that is serving is the operator's call. Step
  8 is where that happens.
* The three names are the platform's. `rivet-fleetd` is the documented
  exception and reads `server.crt`, `server.key` and `ca.crt` instead
  (`ADR-055` §3.2), and it is staged at card time rather than installed here.

## 6. Switches the document has to carry

Two kinds, and neither is an install.

**A dependency that is installed and switched off.** A manifest's `connects`
is resolved against the services this machine *enables*, and the supervisor
refuses to start a consumer whose connection cannot be resolved. TempMon
declares `connects = ["rivet-bluetoothd:advertisements"]`, and
`rivet-bluetoothd` ships disabled because whether a machine listens for
advertisements is the operator's policy. The refusal names its own fix: it
says the consumer declares that `connects`, that `no such service is enabled`
on this machine, and that for a system service which exists and is switched
off the repair is `[services.<name>] enabled = true` in the configuration
document rather than an install. **`install` prints this edit as the first of
its steps** for an application whose manifest declares the `connects`, and the
interview offers it; by hand it is:

```sh
cargo rivet run <hostname> -- apply edit /state/config/machine.toml --append '
[services.rivet-bluetoothd]
enabled = true
' confirm
```

**The kiosk's auto-run.** A kiosk document starts with whatever the interview
answered — `rivet-launcher` shows its menu — and pointing it at a newly
deployed application is a replace-exactly-once edit:

```sh
cargo rivet run <hostname> -- apply edit /state/config/machine.toml \
    --expect 'auto-run = "rivet-launcher"' --replace 'auto-run = "<application>"' confirm
```

`--expect` must match **exactly once** or nothing is written, and the named
application must also be enabled: an `auto-run` naming an application that is
absent, or installed and disabled, is the same black screen by two routes
(`SPEC-05` §5.1).

**`--append` takes literal text and knows no escape sequences.** In PowerShell
a single-quoted string may span lines, so press Enter inside the quotes to get
each newline, as above. **Begin the text with a newline and end it with one.**
An append that would continue a last line with no newline after it is refused,
because text joined onto a trailing comment would vanish into the comment while
the document still parsed; the leading newline is what makes the edit valid
whatever the document ends with, and the trailing one leaves the next append
the same.

## 7. The resolver — asked by the interview, by hand only on an older machine

**A new machine needs nothing here.** The interview asks a **static** machine
for its nameservers — the tenth field, `Nameservers`, shown only when
Addressing is `static`, defaulting to the gateway — and writes them into the
document as `[network.resolver] nameservers`. A **DHCP** machine is not asked
and declares no resolver: its lease fills `/etc/resolv.conf`, and a table in
the document would override the lease.

**A static machine provisioned by an older interview has no resolver at all.**
That interview never asked, so its document names no nameservers and it has no
lease to learn them from. Every outbound lookup then fails **inside whichever
application made the call**: weather and the reading card report lookup errors that
read exactly like application defects (`ADR-055` §8.2). Read the document
first, and if it has no `[network.resolver]`, give it one:

```sh
cargo rivet run <hostname> -- get file /state/config/machine.toml
cargo rivet run <hostname> -- apply edit /state/config/machine.toml --append '
[network.resolver]
nameservers = ["<gateway or upstream>", "<second>"]
' confirm
```

IPv4 addresses, in the order they will be asked, **at most three** — musl
reads no more, so a fourth is refused rather than written and ignored. The
interview refuses the same things at its prompt.

**Then restart the machine, not the service.** `/etc/resolv.conf` is a
projection of the document, rendered once at boot before anything that reads
`/etc` has started (`ADR-008` §5.3.1) — so a document that names nameservers
and a `reload` leave the file exactly as the boot wrote it:

```sh
cargo rivet run <hostname> -- apply restart confirm
```

## 8. Reload, THEN restart — and which restart does what

```sh
cargo rivet run <hostname> -- apply reload confirm                            # the supervisor re-reads the document
cargo rivet run <hostname> -- apply service-restart rivet-launcher confirm    # the screen
cargo rivet run <hostname> -- apply service-restart rivet-proxyd confirm      # the front door
cargo rivet run <hostname> -- apply service-restart <application> confirm     # one that is `failed`
```

**Reload first, every time.** `apply service-restart` does not re-deliver
settings from a document you have just changed — it restarts the service
against whatever the supervisor parsed at its last spawn. A launcher restarted
before the reload re-reads the **old** `auto-run` and nothing appears to
change, which costs a round trip.

Then, and the three restarts are three different jobs:

* **The launcher is what puts a newly deployed application on the screen.** It
  asks the supervisor once, at start, what this machine can offer. The
  supervisor re-runs discovery after every install, so the machine's own
  answer was right within a second of the install finishing — the program
  drawing the screen is holding the answer it got when it started
  (`ADR-055` §5).
* **The proxy is what puts an application on `https://<host>/`.** It reads its
  routing table once at start. `rivet-sysd` rewrites that table at each
  finished install (`ADR-046` §5), so until the restart the index says nothing
  on this machine asks to be proxied, with routed applications installed.
* **A service marked `failed` needs a restart of its own.** A reload does not
  revive one — which is what every application repaired in step 5 needs, since
  it crash-looped before its certificates arrived.

`ADR-055` §5 proposes that the machine converge these readers itself, and
§6.2 is the question the maintainer rules before it is built: converging the
launcher takes the screen from whatever is drawing, and on a machine running a
soak that costs the run.

## 9. Settings that are the application's own, not Rivet's

Rivet hands an application its data directory and its per-machine material;
what the application then wants is its own business, and two of Aura's bite in
a way that looks like a platform fault.

* **The indoor reading must be asked for over `https://`**, because TempMon on
  a Rivet serves TLS once step 5 has given it certificates. `tempmon_url` is
  `https://127.0.0.1:<its port>` — the supervisor opens 8081 for it, 8080
  being the controller's own portal — and `tempmon_name` must be the name the
  machine's certificate carries (`<hostname>.<suffix>`), because the machine's
  certificate names that and musl cannot resolve it, so the URL has to be the
  address and the verified name has to be given separately. An empty
  `tempmon_name` means *verify the URL's host*, which on a loopback address is
  not what is wanted.
* **Credentials the application keeps for itself** — Aura's reading-source
  credential, TempMon's sensor labels — are entered through the application's
  own portal, which is reachable once the proxy has been restarted in step 8.

Every application on the machine created its own configuration, database and
secrets unaided; `SPEC-05` §10.6 is the evidence that this list is short.

## 10. Check it, and know what the answer means

```sh
cargo rivet run <hostname> -- get status
```

Read, in this order: each service's own state (`started`, and `restarts: 0`
rather than a number climbing), then `auto_run` against `display_leased`, then
`health`.

* **`auto_run` is what the document asks for; `display_leased` is what the
  launcher actually did.** They disagree whenever a selection has not
  happened, was revoked, or was made by a person at the menu — so never infer
  one from the other, from `devices = ["display"]`, or from which kiosk
  application is `started`.
* **`health: healthy` is narrower than it sounds.** It is the worst state
  among the services the document marks `critical`, so a machine with no
  `critical` services answers `healthy` whatever else is wrong (`ADR-045` §4).
  That is the honest answer to an operator who has said nothing matters, and
  the remedy is to mark what does. Every service's own `health` is in
  `services[]` and none of it is hidden.
* `get install` reports the **latest** install, not necessarily yours, and
  `get log` defaults to 200 records. `skills/reading-a-board` carries both.

---

## The PowerShell quoting trap

**Use single quotes around any argument containing double quotes.**

```powershell
--expect 'auto-run = "rivet-launcher"'        # correct
--expect "auto-run = \"rivet-launcher\""      # WRONG: the backslash arrives literally
```

PowerShell does not consume the backslash, so `\"rivet-launcher\"` reaches the
tool as text with backslashes in it, `--expect` matches nothing, and the
refusal is about the word rather than about the quoting. A single-quoted
string also passes `$`, backticks and braces through untouched, and may span
lines — which is how step 6 and step 7 append a two-line table. For text that
is awkward either way, assign it to a variable first and pass the variable.

**And the `\` at the end of a wrapped line above is shell style, not
PowerShell's.** Type those commands on one line there; PowerShell's own
continuation character is a backtick.

## What this page does not settle

* **What `install` prints is typed, not run for you.** `ADR-055` §2 holds the
  three repairs: a manifest declares its per-machine material (built — minting
  stays the operator's, R-194 D1, and `install` names the commands), a
  switched-off dependency is named (the interview offers its enable before the
  install; `install` prints it after), and the machine converges the readers
  holding a stale copy (unbuilt, so the proxy and launcher restarts in step 8
  are still the operator's).
* **A rebuild that follows this page has not been walked end to end by anyone
  but the machine it was written from.** It is one real bring-up of one kiosk
  member, written as it happened; a second machine of a different role will
  meet steps this one did not.
* **`SPEC-05` §6 rules out hand-edited TOML for provisioning a fleet server**,
  and steps 6 and 7 edit a document by hand on a running machine. That is not
  a contradiction of the specification — it is the gap `ADR-055` was opened
  about — but it is the reason to prefer the interview or the register for
  anything either can answer, and to treat an `apply edit` as the thing a
  later build should make unnecessary.
