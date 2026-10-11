# Fleet Bring-Up Findings — the first production fleet, 2026-09-07

* **Status:** **Findings.** Observed, awaiting a ruling
* **Date:** 2026-09-07
* **Target Audience:** AI Agent Engineering System & Human Maintainers
* **Occasion:** re-minting the fleet CA and bringing `rivet-01` up as
  the production fleet server — proxy, Aura, launcher, fleetd, TempMon, another adopter

---

## 1. The rule these are graded by

**Ruled by the maintainer, 2026-09-07:** every time he could not do something and
the agent had to do it for him was, frankly, the system failing.
*(`.rulings/` R-301.)*

**This is the razor for everything below.** Not *did it eventually work* — it
all worked — but *could the maintainer have done it alone*. Every entry here is
a step that required hand-built commands, source reading, or an agent, and each
one is therefore a defect rather than a workaround that happened to succeed.

**And the counter-honesty:** a large share of this run's friction was **an agent
taking a shortcut past the provisioning tool**, not the tool failing at its job.
The CA was minted with the `rivet-provision ca …` subcommands and the machine
document was hand-written, rather than walking the menu. The menu writes
`browser.pfx`, installs it with `certutil`, and writes the fleet server's first
register record. Three of the "fixes" below are things it would simply have
done. `.claude` memory `no-implementor-shortcuts-in-first-run` says exactly
this, and it was ignored.

**So §2 is the platform's and §3 is the shortcut's**, and keeping them apart is
what stops this document being used to justify rewriting something that was not
broken.

---

## 2. Platform gaps — the maintainer could not have done these

### 2.1 `cargo rivet secrets` serves every application except `rivet-fleetd`

**Read §3.5 before this one.** The first version of this finding claimed
credential names were undiscoverable and was mostly wrong: `cargo rivet secrets
APPLICATION --host … confirm` installs `tls-cert`, `tls-key` and `tls-root` from
`~/.rivet/ca` in one command, and it is documented in `--help`.

**What is actually true is narrower and sharper.** `rivet-fleetd` reads
`server.crt`, `server.key` and `ca.crt`; `secrets` installs only the `tls-*`
triple and has no name mapping. So the **one** application whose names differ is
the one application the tool cannot install credentials for, and the only way to
learn its names is to start it, watch it fail, and read a path out of a journal
line.

**What should change:** either `rivet-fleetd` adopts the `tls-*` names, or
`secrets` learns fleetd's. One application with private naming is not worth a
general mechanism — but it is worth not being the *only* thing a person has to
hand-install on the machine that is the fleet's front door.

### 2.2 `apply service-restart` does not re-deliver settings

`rivet-proxyd` reported `upstream-name` unset through **two** restarts after the
value had been written into `/state/config/machine.toml` and confirmed present
by `get file`. Settings are delivered at spawn from the supervisor's parse, and
`service-restart` does not re-parse. `apply reload` first, then restart.

**Nothing says this**, in the refusal or in the `service-restart` answer, and the
answer is otherwise unusually forthcoming about what it did and did not do.

**What should change:** `service-restart` should either re-read the document or
say plainly that it did not.

### 2.3 A `connects` grant is only made at boot

`rivet-launcher` was installed onto a running machine, declared
`connects = ["rivet-sysd:kiosk"]`, and was refused `/run/rivet/kiosk` with
`Permission denied`. It drew an empty menu and reported *"0 application(s)
offered"*. Restarting the service did not fix it. **Only a reboot did** — the
grant is a chown performed when the supervisor binds the endpoint, and the
socket had been bound before the consumer existed.

The launcher's own warning names `connects` as the thing to declare, which the
manifest **already did** — so the message sends a reader to check something that
is already correct.

**What should change:** the supervisor should re-chown a bound endpoint when a
consumer for it appears, or the refusal should say *"this endpoint was bound
before this application was installed; restart the machine."*

### 2.4 `rivet-proxyd` does not re-derive routes when an application installs

`/aura/` answered 404 after `aura-controller` was installed and started. The
index page footer names the moment the table was derived — proxy start — and
nothing re-derives it. A proxyd restart fixed it.

**What should change:** derive on install, or on `reload`.

### 2.5 The proxy index advertises `:4443` unconditionally

`cmd/rivet-proxyd/src/main.rs` emits the direct fleet-portal row whether or not
`rivet-fleetd` is installed. On a machine without it the maintainer clicks a
link on the machine's own front page and reaches a refused connection.

**What should change:** emit the row when the application is present.

### 2.6 The register is on the workstation and the fleet server cannot see it

`ADR-034` §2.2 dropped Local mode: the register lives on a Rivet machine and the
workstation is a client. **`cmd/rivet-fleet` still writes a local register** —
`%APPDATA%\rivet-fleet\register` on this machine, holding the dev board — and
has **no verb that pushes to a fleet server**. `rivet-fleetd` on `rivet-01`
therefore serves an empty register and says so correctly.

The bootstrap is documented in `cmd/rivet-fleetd/src/fleet.rs`: the fleet
server's own first record is written **onto the card** by `rivet-provision`,
because at that moment nothing is listening. **There is no recovery path for a
machine that was provisioned without it** — the window has closed and no verb
reopens it.

**What should change:** an enrollment operation over the control plane, and a
`rivet-fleet` verb that targets a fleet server rather than a directory. This is
the largest gap in this document.

**PROVEN BY DEMONSTRATION, later the same day.** `rivet-fleet`'s whole surface
was read: `get`, `set`, `list`, `add`, `remove`, `apply`, `export` and every
subcommand. **There is no verb that sends a register anywhere.** `apply` offered
`document`, `deploy`, `repo-delete` and `trial-restart` that day (`deploy` was
removed on 2026-09-12, `PLAN-Feature-Set-2026-09-12` §3.4); `export` offers
`provisioning` and `card-plan`. Getting `rivet-01` into its own fleet server's
register required creating two directories and three files by hand over the
control plane and then chowning all five — see the finding below on the control plane writing as root.

### 2.6.1 `add document` cannot adopt, because adoption needs what adoption finds

`ADR-034` §6.2's adoption path is `rivet-fleet add document --device D`. It
refuses with:

> *"rivet-01's record carries no address, so there is nowhere to reach it …
> or its record has no `[interfaces]` at all. Nothing has been sent."*

**And no verb puts an address into a record.** `set` offers `listen`, `pin`,
`fleet`, `service`, `fleet-service` and `repo`; `set listen` moves the control
plane on the register *and the machine together*, so it is not a way to describe
a machine that already works. An `[interfaces]` block reaches a record only
through `add document` — which cannot run without one.

**The deadlock was broken by hand-editing the record's TOML.** With `listen` and
`[interfaces.eth0]` written in by an editor, `add document` then worked
perfectly: it read the machine, printed an exact plan, and recorded nine
services.

**What should change:** either `set address`/`add interface`, or `add document`
should accept `--host` for a machine whose address the operator knows.

### 2.6.2 The register schema cannot hold what the machines actually carry

`add document` reported four values it could not record, and was right to say
so plainly:

* `applications.rivet-proxyd.upstream-name` — *"the register has no field for"*
* `kiosk.auto-run` — the same
* `network.resolver.nameservers` — *"the register's interface record has no
  `nameservers` — it holds method, address and gateway and nothing else"*

**So `apply document` will always refuse for this machine**, and the register can
never be authoritative for a Rivet that runs a kiosk or an application with
settings — which is every interesting Rivet.

**A related defect, smaller:** adoption turned `network.resolver` into an
invented interface, `[interfaces.resolver] method = "dhcp"`, while simultaneously
reporting that it could not record the resolver. The record now describes an
invented network interface.

### 2.7 Cross-compiling an adopter needs two environment variables, typed

Any application whose graph contains a C build script — `ring`, so anything
using `ureq` or `rustls` — fails until `CC_aarch64_unknown_linux_musl` and
`AR_aarch64_unknown_linux_musl` are exported. **The refusal is excellent**: it
names both variables, the Buildroot path they come from, and why the tool will
not set them (`ADR-011`). It still has to be typed for every adopter build.

**What should change:** nothing, arguably — the refusal is doing its job and the
reasoning against setting it is sound. Recorded because it is friction the
maintainer would hit, and `tools/container/exec.sh` could export both when
`RIVET_APP_SRC` is set without making `cargo build` and `cargo rivet package`
disagree.

### 2.8 Two `cargo-rivet` binaries on this machine are stale

The PATH `cargo-rivet` is **0.1.66** with nine verbs and no `secrets`; the
container's is stale too. Both refuse valid manifest keys. The repository's own
build must be used by path. This is already in `.claude` memory
(`rivet-container-cargo-rivet-is-stale`) and keeps costing time, which is
evidence the memory is not the right place for it.

---

### 2.9 `apply create-file` takes one word, so multi-line content has no verb

`rivet apply create-file <path> [<text>]` — *"the text is OPTIONAL and one
word."* There is no way to create a file with a line break in it.

**The working combination is undocumented:** `apply edit <path> --append <text>`
accepts multi-line text **and creates the file if it is absent**, answering
`"created": true`. That is how three TOML files were placed on `rivet-01`.
Nothing in either verb's refusal points at the other.

### 2.10 The control plane writes as root; applications cannot read what it wrote

Every file written by `apply edit` and `apply create-file` is `root:root`,
mode `0600`. An application's data directory is `0700` owned by that
application (`ADR-015` §5.1.1). **So anything placed into an application's data
directory over the control plane is invisible to the application it was placed
for**, and nothing says so.

`rivet-fleetd` was restarted twice against a register it could not open, logging
no error either time — an unreadable register and an absent one are the same
answer, because `store::load` treats a missing directory as an empty register by
design (`rivet-register/src/store.rs:117`), which is right for a first run and
silent for this.

`apply set-owner <path> <application> confirm` fixes it and worked first time.

**What should change:** a write into `/state/app/<name>/data` should either
chown to that application or say that it did not.

### 2.11 The fleet portal's empty-state text is in the page whether or not it applies

`rivet-fleetd` serves a single-page application that fetches `/api/fleet`. The
sentence *"The register is empty. rivet-provision writes the first record."* is a
JavaScript string in the served HTML **at all times**. Anything that reads the
page rather than the endpoint — a script, an agent, a `curl | grep` — will
conclude the register is empty when it is full. It cost two wrong conclusions in
one session.

**Recorded as a trap rather than a defect**, because the page is not wrong; it is
a reminder that the API is the observable and the HTML is not.

---

## 3. Consequences of skipping the provisioning menu

**These are not platform defects. They are what the tool would have prevented.**

### 3.1 The new CA carries the same subject as the retired one

Both are `CN=home-fleet`. A browser selects client certificates by matching the
issuer **name**, not the key — so every certificate either authority ever issued
appeared in the picker at once. The maintainer was offered **four**, none of
which could authenticate: three from throwaway rehearsal CAs also named
`home-fleet`, and one from the retired authority.

**Worth ruling anyway:** re-minting an authority under the name of the one it
replaces is a foreseeable trap, and the tool could refuse it, suffix it, or warn.

### 3.2 The operator certificate was installed without its private key

`CN=Joel Lindell` from the new CA was in the Personal store with
`HasPrivateKey: False`, so Chrome never offered it — the one correct certificate
was the one that could not be seen. No `browser.pfx` existed in the CA
directory, which is where `browser::install` writes and keeps it.

Recovery required building a PKCS#12 with `openssl` and importing it with
`certutil` by hand — **and the keyless copy had to be deleted first**, because
`certutil -importpfx` reported success while leaving the key unattached.

**What should change:** a verb that re-runs the browser-enrollment step alone.
There is no way to do this today short of re-minting the whole authority, which
would destroy the fleet.

### 3.3 `ca server` overwrote a three-name leaf with a one-name leaf

The authority records three suffixes (`local`, `home-fleet.home.arpa`,
`home-fleet.lan`). Running `ca server "rivet-01.local"` minted a certificate
with a **single** SAN, silently replacing the correct one. Only
`https://rivet-01.local/` validates; the address and the other two suffixes
warn.

**What should change:** `ca server` should compose names from the recorded
suffixes by default rather than taking a literal list.

### 3.4 Rehearsal runs write into the operator's real certificate store

`tools/provision/rehearse-menu.sh` left three `CN=Rehearsal Operator`
certificates in `CurrentUser\My`, each from a throwaway CA named `home-fleet`.
A rehearsal must not touch the real store.

### 3.5 The agent never read `cargo rivet --help`, and then filed a defect

**The worst entry in this document, and it is about how the document was
written.** Credentials were installed on five applications by a hand-written
shell script that hex-encoded PEM files into `apply install-secret`. The first
draft of §2.1 then reported the resulting friction as a platform gap.

`cargo rivet secrets APPLICATION --host … confirm` does the whole job and is in
the top-level `--help`, which was never opened — the same `--help` that opens
by explaining that a stale PATH binary is the usual reason a verb seems to be
missing, which is exactly the trap that had already been hit twice this session.

**Why it is recorded rather than quietly fixed.** A wrong finding is more
expensive than no finding: it sends whoever acts on it to build a mechanism for
an imaginary problem, and it discredits the entries around it. The
maintainer caught this one. The next one may not be caught.

**The rule it produces:** before reporting that a tool cannot do something, run
its `--help`. An agent's workaround is not evidence about the platform until
that has been done.

---

## 4. What went right, so it is not lost in the list

* **Every refusal that mattered named its own fix.** The `upstream-name` error
  printed the exact TOML. The cross-compile refusal named both variables and the
  Buildroot path. `apply install` explained what it would and would not do.
  Where this project fails, it fails legibly — that is the difference between a
  ninety-second fix and an afternoon.
* **The journal was sufficient every time.** Nothing needed a shell on the board,
  and no diagnosis needed a screen.
* **`RIVET_APP_SRC` made adopter builds work first time.** TempMon and another adopter
  each cross-compiled and packaged on the first attempt.
* **The gate is correct.** `/aura/` and the other gated adopter answer 403 without an
  operator certificate and 200 with one; `/tempmon/` answers 200 either way,
  which is what its `proxy-auth = "none"` asks for.

---

## 5. The order these are worth fixing in

1. **§2.6, the register** — the fleet server cannot be enrolled at all, and no
   amount of care at card-build time helps a machine already running.
2. **§3.2, browser re-enrollment** — a fleet is unusable without it and the only
   recovery destroys the fleet.
3. **§2.2, §2.3, §2.4** — three convergence surprises, each cheap to fix and each
   costing a diagnosis cycle to a person who does not already know.
4. **§3.1, §3.3, §3.4** — provisioning correctness, once somebody is next in
   that code.
5. **§2.1, fleetd's credential names** — one application, one afternoon, and
   `cargo rivet secrets` already covers everything else.
